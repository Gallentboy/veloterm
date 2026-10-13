import { FitAddon } from '@xterm/addon-fit';
import { Terminal as XTerm, type IDisposable } from '@xterm/xterm';
import { ClipboardPaste, Copy, Eraser } from 'lucide-react';
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from 'react';
import { useTranslation } from 'react-i18next';
import { cn } from '@/lib/utils';
import { loadHostStore, resolveJumpHostChain } from '@/hosts/storage';
import { formatShellShortLabel } from '@/terminal/terminalPanelState';
import {
  terminalSessionRegistry,
  type TerminalSessionRegistryEntry,
} from '@/terminal/terminalSessionRegistry';
import { mergeTerminalTheme } from '@/terminal/terminalTheme';
import {
  createTerminalByteChannel,
  encodeBinaryString,
  encodeTerminalInput,
} from '@/terminal/transport';
import type { ITerminalTransport, TerminalSize } from '@/terminal/types';

const DEFAULT_TERMINAL_FONT_FAMILY =
  'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", "Microsoft YaHei UI", "Microsoft YaHei", "PingFang SC", "Noto Sans CJK SC", monospace';

const TERMINAL_RESIZE_DRAG_THROTTLE_MS = 300;
/** 进程启动后若在极短时间内立即异常退出，保留终端回显错误信息而不触发自动关 Tab 重建循环 */
const MIN_UPTIME_FOR_AUTO_CLOSE_MS = 600;

type TerminalResizeReason = 'drag' | 'final' | 'visible' | 'init' | 'observer';

function normalizeTerminalFontSize(value: number | undefined): number | undefined {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 6 || value > 72) {
    return undefined;
  }
  return value;
}

export interface TerminalSessionProps {
  sessionId: number;
  transport: ITerminalTransport;
  cwd?: string;
  hostId?: number;
  isVisible: boolean;
  isPanelResizing?: boolean;
  onShellLabelChange?: (sessionId: number, shellLabel: string | null) => void;
  onSizeChange?: (sessionId: number, size: TerminalSize) => void;
  onWebglStatusChange?: (sessionId: number, active: boolean) => void;
  onExit?: (sessionId: number, exitCode: number) => void;
}

/**
 * 单个终端会话视图组件（移植自 ZCode `TerminalSession.tsx` 并集成 WebGL2 硬件加速）：
 * - 通过 `terminalSessionRegistry` 单例持有 `XTerm` + `FitAddon` + `WebglAddon` + `hostEl`；
 * - 兼容 React 19 `<StrictMode>` 双重挂载：卸载时仅 detach DOM、不中断进行中的异步 PTY 创建；
 * - 当前可见会话自动激活 `@xterm/addon-webgl`（含 LRU 上下文池管理与 `onContextLoss` 自动恢复）；
 * - 主题切换时同步更新配色并调用 `clearWebglTextureAtlas` 刷新 WebGL 字形纹理图集；
 * - 拖拽侧边栏时按 300ms 节流 `fit()` 与后端 `resize`，松手后立即 flush 最终尺寸。
 */
export function TerminalSession({
  sessionId,
  transport,
  cwd,
  hostId,
  isVisible,
  isPanelResizing = false,
  onShellLabelChange,
  onSizeChange,
  onWebglStatusChange,
  onExit,
}: TerminalSessionProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const terminalIdRef = useRef<number | null>(null);
  const isVisibleRef = useRef(isVisible);
  const isPanelResizingRef = useRef(isPanelResizing);
  const resizeRAFRef = useRef(0);
  const resizeThrottleTimerRef = useRef<number | null>(null);
  const lastSentTerminalSizeRef = useRef<TerminalSize | null>(null);
  const pendingTerminalSizeRef = useRef<TerminalSize | null>(null);
  const resizeInFlightRef = useRef(false);
  const focusRAFRef = useRef(0);

  const onShellLabelChangeRef = useRef(onShellLabelChange);
  const onSizeChangeRef = useRef(onSizeChange);
  const onWebglStatusChangeRef = useRef(onWebglStatusChange);
  const onExitRef = useRef(onExit);
  onShellLabelChangeRef.current = onShellLabelChange;
  onSizeChangeRef.current = onSizeChange;
  onWebglStatusChangeRef.current = onWebglStatusChange;
  onExitRef.current = onExit;

  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
    hasSelection: boolean;
  } | null>(null);

  const notifyWebglStatus = useCallback(
    (active: boolean) => {
      onWebglStatusChangeRef.current?.(sessionId, active);
    },
    [sessionId],
  );

  const flushTerminalResize = useCallback(() => {
    if (resizeInFlightRef.current) {
      return;
    }
    const terminalId = terminalIdRef.current;
    const pendingSize = pendingTerminalSizeRef.current;
    if (terminalId === null || !pendingSize) {
      return;
    }

    pendingTerminalSizeRef.current = null;
    resizeInFlightRef.current = true;

    void transport
      .resize({
        id: terminalId,
        cols: pendingSize.cols,
        rows: pendingSize.rows,
      })
      .catch((error) => {
        console.warn('[TerminalSession] resize failed:', error);
      })
      .finally(() => {
        resizeInFlightRef.current = false;
        if (pendingTerminalSizeRef.current) {
          flushTerminalResize();
        }
      });
  }, [transport]);

  const queueTerminalResize = useCallback(
    (size: TerminalSize) => {
      onSizeChangeRef.current?.(sessionId, size);
      const lastSentSize = lastSentTerminalSizeRef.current;
      const pendingSize = pendingTerminalSizeRef.current;
      if (
        (lastSentSize?.cols === size.cols && lastSentSize.rows === size.rows && !pendingSize) ||
        (pendingSize?.cols === size.cols && pendingSize.rows === size.rows)
      ) {
        return;
      }

      pendingTerminalSizeRef.current = size;
      lastSentTerminalSizeRef.current = size;
      flushTerminalResize();
    },
    [flushTerminalResize, sessionId],
  );

  const clearResizeThrottleTimer = useCallback(() => {
    if (resizeThrottleTimerRef.current) {
      window.clearTimeout(resizeThrottleTimerRef.current);
      resizeThrottleTimerRef.current = null;
    }
  }, []);

  const requestFitAndResize = useCallback(
    (reason: TerminalResizeReason = 'observer') => {
      if (resizeRAFRef.current) {
        cancelAnimationFrame(resizeRAFRef.current);
      }

      resizeRAFRef.current = requestAnimationFrame(() => {
        resizeRAFRef.current = 0;
        const el = containerRef.current;
        const term = termRef.current;
        const fitAddon = fitAddonRef.current;
        if (
          !isVisibleRef.current ||
          !el ||
          !term ||
          !fitAddon ||
          el.clientWidth <= 0 ||
          el.clientHeight <= 0
        ) {
          return;
        }

        const prevCols = term.cols;
        const prevRows = term.rows;
        try {
          fitAddon.fit();
        } catch (error) {
          console.warn('[TerminalSession] fit failed:', error);
          return;
        }

        // 当从其他 Tab 切回当前 Tab（reason === 'visible' | 'init'）且窗口尺寸未变时，
        // fitAddon.fit() 因行列数不变会直接跳过内部 resize，需显式同步 WebGL RenderModel 与顶点 UV 缓存。
        if (
          (reason === 'visible' || reason === 'init') &&
          term.cols === prevCols &&
          term.rows === prevRows
        ) {
          terminalSessionRegistry.refreshWebglViewport(sessionId);
        }

        queueTerminalResize({ cols: term.cols, rows: term.rows });
      });
    },
    [queueTerminalResize, sessionId],
  );

  const scheduleFitAndResize = useCallback(
    (reason: TerminalResizeReason = 'observer') => {
      if (!isPanelResizingRef.current || reason === 'final') {
        clearResizeThrottleTimer();
        requestFitAndResize(reason);
        return;
      }

      if (resizeThrottleTimerRef.current) {
        return;
      }

      resizeThrottleTimerRef.current = window.setTimeout(() => {
        resizeThrottleTimerRef.current = null;
        requestFitAndResize('drag');
      }, TERMINAL_RESIZE_DRAG_THROTTLE_MS);
    },
    [clearResizeThrottleTimer, requestFitAndResize],
  );

  const requestFocus = useCallback(() => {
    if (focusRAFRef.current) {
      cancelAnimationFrame(focusRAFRef.current);
    }

    focusRAFRef.current = requestAnimationFrame(() => {
      focusRAFRef.current = 0;
      const term = termRef.current;
      if (!isVisibleRef.current || !term) {
        return;
      }
      term.focus();
    });
  }, []);

  useEffect(() => {
    isVisibleRef.current = isVisible;
    if (isVisible) {
      // Tab 切为可见时确保挂载/恢复 WebGL2 硬件加速渲染器
      terminalSessionRegistry.ensureWebgl(sessionId, notifyWebglStatus);
      scheduleFitAndResize('visible');
      requestFocus();
    } else {
      setContextMenu(null);
    }
  }, [isVisible, notifyWebglStatus, requestFocus, scheduleFitAndResize, sessionId]);

  useEffect(() => {
    const wasResizing = isPanelResizingRef.current;
    isPanelResizingRef.current = isPanelResizing;
    if (wasResizing && !isPanelResizing && isVisibleRef.current) {
      scheduleFitAndResize('final');
    }
  }, [isPanelResizing, scheduleFitAndResize]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const existingEntry = terminalSessionRegistry.get(sessionId);

    // ===== 1. 复用快路径：从其他页面切回「终端」页或 StrictMode 二次挂载时，直接挂载已保活的 hostEl =====
    if (existingEntry) {
      termRef.current = existingEntry.term;
      fitAddonRef.current = existingEntry.fitAddon;
      terminalIdRef.current = existingEntry.terminalId;
      existingEntry.onTerminalIdReady = (readyId: number) => {
        terminalIdRef.current = readyId;
        flushTerminalResize();
        scheduleFitAndResize('init');
        if (isVisibleRef.current) {
          requestFocus();
        }
      };

      terminalSessionRegistry.attachDom(sessionId, el);
      existingEntry.term.options.theme = mergeTerminalTheme(existingEntry.profileTheme);
      terminalSessionRegistry.refreshWebglViewport(sessionId);

      const reuseThemeObserver = new MutationObserver(() => {
        existingEntry.term.options.theme = mergeTerminalTheme(existingEntry.profileTheme);
        terminalSessionRegistry.clearWebglTextureAtlas(sessionId);
      });
      reuseThemeObserver.observe(document.documentElement, {
        attributes: true,
        attributeFilter: ['class'],
      });

      let reuseResizeRAF = 0;
      const reuseResizeObserver = new ResizeObserver(() => {
        if (!isVisibleRef.current) return;
        if (reuseResizeRAF) cancelAnimationFrame(reuseResizeRAF);
        reuseResizeRAF = requestAnimationFrame(() => scheduleFitAndResize('observer'));
      });
      reuseResizeObserver.observe(el);

      if (isVisibleRef.current) {
        terminalSessionRegistry.ensureWebgl(sessionId, notifyWebglStatus);
        if (el.clientWidth > 0 && el.clientHeight > 0) {
          try {
            existingEntry.fitAddon.fit();
            terminalSessionRegistry.refreshWebglViewport(sessionId);
            queueTerminalResize({
              cols: existingEntry.term.cols,
              rows: existingEntry.term.rows,
            });
          } catch (error) {
            console.warn('[TerminalSession] reuse fit failed:', error);
          }
        }
        requestFocus();
      } else {
        notifyWebglStatus(terminalSessionRegistry.isWebglActive(sessionId));
      }

      return () => {
        if (existingEntry.onTerminalIdReady) {
          existingEntry.onTerminalIdReady = undefined;
        }
        reuseThemeObserver.disconnect();
        reuseResizeObserver.disconnect();
        clearResizeThrottleTimer();
        if (reuseResizeRAF) cancelAnimationFrame(reuseResizeRAF);
        if (resizeRAFRef.current) cancelAnimationFrame(resizeRAFRef.current);
        if (focusRAFRef.current) cancelAnimationFrame(focusRAFRef.current);
        terminalSessionRegistry.detachDom(sessionId);
        termRef.current = null;
        fitAddonRef.current = null;
      };
    }

    // ===== 2. 首次创建：初始化常驻 hostEl、XTerm、WebGL2/Fit 插件与 Channel<Vec<u8>> 传输通道 =====
    const hostEl = document.createElement('div');
    hostEl.className = 'terminal-xterm-shell h-full min-h-0 w-full overflow-hidden';
    el.appendChild(hostEl);

    let isEntryDisposed = false;
    const createdAt = performance.now();
    const registryDisposers: IDisposable[] = [];

    const term = new XTerm({
      fontSize: 13,
      fontFamily: DEFAULT_TERMINAL_FONT_FAMILY,
      cursorBlink: true,
      allowProposedApi: true,
      scrollback: 5000,
      theme: mergeTerminalTheme(),
    });
    termRef.current = term;

    const fitAddon = new FitAddon();
    fitAddonRef.current = fitAddon;
    term.loadAddon(fitAddon);
    term.open(hostEl);

    const entry: TerminalSessionRegistryEntry = {
      key: sessionId,
      term,
      fitAddon,
      webglAddon: null,
      terminalId: null,
      cwd: cwd ?? '~',
      hostEl,
      onTerminalIdReady: (readyId: number) => {
        terminalIdRef.current = readyId;
        flushTerminalResize();
        scheduleFitAndResize('init');
        if (isVisibleRef.current) {
          requestFocus();
        }
      },
      dispose: () => {},
    };

    entry.dispose = () => {
      isEntryDisposed = true;
      entry.onTerminalIdReady = undefined;
      for (const d of registryDisposers) {
        try {
          d.dispose();
        } catch (error) {
          console.warn('[TerminalSession] dispose subscription failed:', error);
        }
      };
      if (entry.terminalId !== null) {
        void transport.dispose({ id: entry.terminalId });
      }
      try {
        term.dispose();
      } catch (error) {
        console.warn('[TerminalSession] term.dispose failed:', error);
      }
    };

    terminalSessionRegistry.register(sessionId, entry);

    // 挂载 WebGL2 硬件加速渲染插件（当前可见 Tab 立即启用）
    if (isVisibleRef.current) {
      terminalSessionRegistry.ensureWebgl(sessionId, notifyWebglStatus);
    }

    let initialTerminalSize: TerminalSize | null = null;
    if (isVisibleRef.current && hostEl.clientWidth > 0 && hostEl.clientHeight > 0) {
      try {
        fitAddon.fit();
        initialTerminalSize = { cols: term.cols, rows: term.rows };
        lastSentTerminalSizeRef.current = initialTerminalSize;
        onSizeChangeRef.current?.(sessionId, initialTerminalSize);
      } catch (error) {
        console.warn('[TerminalSession] initial fit failed:', error);
      }
    }
    if (isVisibleRef.current) {
      requestFocus();
    }

    // 拦截 Ctrl/Cmd+C（有选区时复制，无选区透传 SIGINT）与 Ctrl/Cmd+V（防止浏览器原生 paste 重复粘贴）
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== 'keydown') return true;
      if (!(e.metaKey || e.ctrlKey)) return true;
      const key = e.key.toLowerCase();
      if (key === 'c' && term.hasSelection()) {
        void navigator.clipboard.writeText(term.getSelection()).catch((err) => {
          console.warn('[TerminalSession] copy via shortcut failed:', err);
        });
        return false;
      }
      if (key === 'v') {
        e.preventDefault();
        e.stopPropagation();
        navigator.clipboard
          .readText()
          .then((text) => {
            if (text) term.paste(text);
          })
          .catch((err) => console.warn('[TerminalSession] paste via shortcut failed:', err));
        return false;
      }
      return true;
    });

    const themeObserver = new MutationObserver(() => {
      term.options.theme = mergeTerminalTheme(entry.profileTheme);
      terminalSessionRegistry.clearWebglTextureAtlas(sessionId);
    });
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class'],
    });

    // 建立接收后端 `Channel<Vec<u8>>` 输出的字节通道，直接将 `Uint8Array` 写入 xterm
    const outputChannel = createTerminalByteChannel((bytes: Uint8Array) => {
      if (isEntryDisposed) return;
      term.write(bytes);
    });

    const initialCreateSize = initialTerminalSize ?? { cols: term.cols, rows: term.rows };
    const hostStore = hostId != null ? loadHostStore() : null;
    const resolvedHost =
      hostStore && hostId != null ? hostStore.hosts.find((h) => h.id === hostId) : undefined;
    const resolvedJumpChain =
      hostStore && resolvedHost ? resolveJumpHostChain(hostStore.hosts, resolvedHost) : [];
    const resolvedTarget = resolvedHost
      ? ({
          type: 'ssh' as const,
          hostId: resolvedHost.id,
          host: resolvedHost.host,
          port: resolvedHost.port,
          username: resolvedHost.username,
          auth: resolvedHost.auth,
          allowInsecureAlgos: resolvedHost.allowInsecureAlgos ?? false,
          hostKeyVerification: resolvedHost.hostKeyVerification ?? 'auto_accept',
          jumpHosts: resolvedJumpChain.map((hop) => ({
            host: hop.host,
            port: hop.port,
            username: hop.username,
            auth: hop.auth,
            allowInsecureAlgos: hop.allowInsecureAlgos ?? false,
            hostKeyVerification: hop.hostKeyVerification ?? 'auto_accept',
          })),
        })
      : undefined;

    void transport
      .create({
        sessionId,
        cols: initialCreateSize.cols,
        rows: initialCreateSize.rows,
        cwd,
        hostId,
        target: resolvedTarget,
        onOutput: outputChannel,
        onExit: (exitCode) => {
          if (isEntryDisposed) return;
          const uptimeMs = performance.now() - createdAt;
          const exitHandler = onExitRef.current;
          // 若 Shell 正常退出且并非启动瞬间闪退，则触发自动关闭 Tab；否则在终端内打印退出码供排查
          if (exitHandler && exitCode === 0 && uptimeMs >= MIN_UPTIME_FOR_AUTO_CLOSE_MS) {
            exitHandler(sessionId, exitCode);
          } else {
            term.write(`\r\n[Process exited with code ${exitCode}]\r\n`);
          }
        },
      })
      .then(({ id, shell, fontFamily, fontSize, theme }) => {
        if (isEntryDisposed) {
          void transport.dispose({ id });
          return;
        }
        entry.terminalId = id;

        if (fontFamily && fontFamily !== term.options.fontFamily) {
          term.options.fontFamily = fontFamily;
        }
        const nextFontSize = normalizeTerminalFontSize(fontSize);
        if (nextFontSize && nextFontSize !== term.options.fontSize) {
          term.options.fontSize = nextFontSize;
        }
        if (theme) {
          entry.profileTheme = theme;
          term.options.theme = mergeTerminalTheme(entry.profileTheme);
        }

        const nextShellLabel = formatShellShortLabel(shell);
        onShellLabelChangeRef.current?.(sessionId, nextShellLabel);
        entry.onTerminalIdReady?.(id);

        // 将用户输入统一编码为 Uint8Array（对应后端 Vec<u8>）发送
        registryDisposers.push(
          term.onData((data) => {
            const bytes = encodeTerminalInput(data);
            void transport.write({ id, data: bytes });
          }),
        );

        registryDisposers.push(
          term.onBinary((binary) => {
            const bytes = encodeBinaryString(binary);
            void transport.write({ id, data: bytes });
          }),
        );
      })
      .catch((error) => {
        if (isEntryDisposed) return;
        const message = error instanceof Error ? error.message : String(error);
        console.error('[TerminalSession] failed to create terminal:', error);
        term.write(`\r\n[Terminal failed to start]\r\n${message}\r\n`);
      });

    let resizeRAF = 0;
    const resizeObserver = new ResizeObserver(() => {
      if (!isVisibleRef.current) return;
      if (resizeRAF) cancelAnimationFrame(resizeRAF);
      resizeRAF = requestAnimationFrame(() => scheduleFitAndResize('observer'));
    });
    resizeObserver.observe(el);

    return () => {
      entry.onTerminalIdReady = undefined;
      themeObserver.disconnect();
      resizeObserver.disconnect();
      clearResizeThrottleTimer();
      if (resizeRAF) cancelAnimationFrame(resizeRAF);
      if (resizeRAFRef.current) cancelAnimationFrame(resizeRAFRef.current);
      if (focusRAFRef.current) cancelAnimationFrame(focusRAFRef.current);
      // 仅将 DOM 移入暂存区，不中断异步创建中的 PTY（真正销毁仅在 closeSession -> registry.release 时发生）
      terminalSessionRegistry.detachDom(sessionId);
      termRef.current = null;
      fitAddonRef.current = null;
    };
  }, [
    clearResizeThrottleTimer,
    cwd,
    flushTerminalResize,
    hostId,
    notifyWebglStatus,
    queueTerminalResize,
    requestFocus,
    scheduleFitAndResize,
    sessionId,
    transport,
  ]);

  // 右键菜单关闭监听
  useEffect(() => {
    if (!contextMenu) return;
    const close = () => setContextMenu(null);
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('pointerdown', close);
    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('pointerdown', close);
      window.removeEventListener('keydown', onKeyDown);
    };
  }, [contextMenu]);

  const handleContextMenu = (event: ReactMouseEvent<HTMLDivElement>) => {
    event.preventDefault();
    const term = termRef.current;
    setContextMenu({
      x: event.clientX,
      y: event.clientY,
      hasSelection: Boolean(term?.hasSelection()),
    });
  };

  const handleCopy = () => {
    const term = termRef.current;
    setContextMenu(null);
    if (!term?.hasSelection()) return;
    void navigator.clipboard.writeText(term.getSelection()).catch((err) => {
      console.warn('[TerminalSession] copy via context menu failed:', err);
    });
    requestFocus();
  };

  const handlePaste = () => {
    const term = termRef.current;
    setContextMenu(null);
    if (!term) return;
    navigator.clipboard
      .readText()
      .then((text) => {
        if (text) term.paste(text);
        requestFocus();
      })
      .catch((err) => console.warn('[TerminalSession] paste via context menu failed:', err));
  };

  const handleClear = () => {
    const term = termRef.current;
    setContextMenu(null);
    term?.clear();
    requestFocus();
  };

  return (
    <div
      onContextMenu={handleContextMenu}
      className="relative h-full min-h-0 w-full overflow-hidden"
    >
      <div
        ref={containerRef}
        className="terminal-xterm-shell h-full min-h-0 w-full overflow-hidden"
      />

      {contextMenu ? (
        <div
          role="menu"
          style={{
            left: Math.min(contextMenu.x, window.innerWidth - 160),
            top: Math.min(contextMenu.y, window.innerHeight - 120),
          }}
          onPointerDown={(e) => e.stopPropagation()}
          className="fixed z-50 min-w-36 overflow-hidden rounded-lg border border-border bg-menu p-1 text-ui-sm text-foreground shadow-lg"
        >
          <button
            type="button"
            role="menuitem"
            disabled={!contextMenu.hasSelection}
            onClick={handleCopy}
            className={cn(
              'flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors',
              contextMenu.hasSelection
                ? 'hover:bg-menu-hover'
                : 'cursor-not-allowed opacity-40',
            )}
          >
            <Copy className="size-3.5" />
            <span>{t('terminal.contextMenu.copy')}</span>
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={handlePaste}
            className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-menu-hover"
          >
            <ClipboardPaste className="size-3.5" />
            <span>{t('terminal.contextMenu.paste')}</span>
          </button>
          <div className="my-1 h-px bg-border/60" />
          <button
            type="button"
            role="menuitem"
            onClick={handleClear}
            className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-menu-hover"
          >
            <Eraser className="size-3.5" />
            <span>{t('terminal.contextMenu.clear')}</span>
          </button>
        </div>
      ) : null}
    </div>
  );
}
