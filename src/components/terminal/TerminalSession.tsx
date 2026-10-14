import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { Terminal as XTerm, type IDisposable } from '@xterm/xterm';
import { ClipboardPaste, Copy, Eraser, Type } from 'lucide-react';
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from 'react';
import { useTranslation } from 'react-i18next';
import { readClipboardText, writeClipboardText } from '@/lib/clipboard';
import { cn } from '@/lib/utils';
import { useDismissOnOutsidePointer } from '@/hooks/useDismissOnOutsidePointer';
import { TerminalSearchBar } from '@/components/terminal/TerminalSearchBar';
import { loadHostStore, resolveJumpHostChain } from '@/hosts/storage';
import { formatShellShortLabel } from '@/terminal/terminalPanelState';
import {
  terminalSessionRegistry,
  type TerminalSessionRegistryEntry,
} from '@/terminal/terminalSessionRegistry';
import { mergeTerminalTheme } from '@/terminal/terminalTheme';
import { buildSshErrorPrompt, setAiErrorContext } from '@/services/aiContext';
import {
  createTerminalByteChannel,
  encodeBinaryString,
  encodeTerminalInput,
} from '@/terminal/transport';
import type { ITerminalTransport, TerminalSize } from '@/terminal/types';

const DEFAULT_TERMINAL_FONT_FAMILY =
  'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", "Microsoft YaHei UI", "Microsoft YaHei", "PingFang SC", "Noto Sans CJK SC", monospace';

const TERMINAL_RESIZE_DRAG_THROTTLE_MS = 300;
/** WebGL 上下文创建/重建开销较大，延迟到 fit 与首次重绘之后再到闲时挂载 */
const WEBGL_ENSURE_DEFER_MS = 100;
/** 进程启动后若在极短时间内立即异常退出，保留终端回显错误信息而不触发自动关 Tab 重建循环 */
const MIN_UPTIME_FOR_AUTO_CLOSE_MS = 600;
/** xterm 的 resize 会对整个 scrollback 同步重排（reflow），行数直接决定布局切换时的卡顿成本，控制在一个够用的档位 */
const TERMINAL_SCROLLBACK_LINES = 2000;

type TerminalResizeReason = 'drag' | 'final' | 'visible' | 'init' | 'observer';

function normalizeTerminalFontSize(value: number | undefined): number | undefined {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 6 || value > 72) {
    return undefined;
  }
  return value;
}

/** 终端字体缩放范围（与 SFTP 一致：50%–150%，作用于 xterm fontSize，取整像素） */
const TERMINAL_FONT_SCALE_MIN = 0.5;
const TERMINAL_FONT_SCALE_MAX = 1.5;
/** 100% 磁吸阈值：捏合/滚轮滑过 1.0 附近时自动吸附，提升回正效率 */
const FONT_SCALE_SNAP_THRESHOLD = 0.04;
/** xterm 允许的最小字号下限（防止极小字号渲染异常） */
const TERMINAL_FONT_MIN_PX = 6;

export interface TerminalSessionProps {
  sessionId: number;
  transport: ITerminalTransport;
  cwd?: string;
  hostId?: number;
  isVisible: boolean;
  isPanelResizing?: boolean;
  /** 字体缩放是否可用：九宫格模式下禁用（标签页模式启用），字号仍保持 */
  fontScaleEnabled?: boolean;
  onShellLabelChange?: (sessionId: number, shellLabel: string | null) => void;
  onSizeChange?: (sessionId: number, size: TerminalSize) => void;
  onRendererStatusChange?: (sessionId: number, kind: 'webgl' | 'canvas2d' | 'dom') => void;
  onExit?: (sessionId: number, exitCode: number) => void;
}

/**
 * 单个终端会话视图组件（移植自 ZCode `TerminalSession.tsx` 并集成 WebGL2 硬件加速）：
 * - 通过 `terminalSessionRegistry` 单例持有 `XTerm` + `FitAddon` + `WebglAddon` + `hostEl`；
 * - 兼容 React 19 `<StrictMode>` 双重挂载：卸载时仅 detach DOM、不中断进行中的异步 PTY 创建；
 * - 当前可见会话自动激活 `@xterm/addon-webgl`（含 LRU 上下文池管理与 `onContextLoss` 自动恢复）；
 * - 主题切换时同步更新配色并调用 `clearWebglTextureAtlas` 刷新 WebGL 字形纹理图集；
 * - 拖拽侧边栏时按 300ms 节流 `fit()` 与后端 `resize`，松手后立即 flush 最终尺寸；
 * - 所有 fit 触发源（ResizeObserver / 可见性 / 拖拽结束）统一合并到同一帧 RAF 执行一次，
 *   布局切换不再依赖多轮延时补偿重试；WebGL 渲染器延迟到空闲时机惰性挂载，
 *   避免上下文创建阻塞 fit 与首帧重绘。
 */
export function TerminalSession({
  sessionId,
  transport,
  cwd,
  hostId,
  isVisible,
  isPanelResizing = false,
  fontScaleEnabled = true,
  onShellLabelChange,
  onSizeChange,
  onRendererStatusChange,
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
  const webglDeferTimerRef = useRef<number | null>(null);
  const lastSentTerminalSizeRef = useRef<TerminalSize | null>(null);
  const pendingTerminalSizeRef = useRef<TerminalSize | null>(null);
  const resizeInFlightRef = useRef(false);
  const focusRAFRef = useRef(0);

  // ── 字体缩放（每会话独立，存于注册表跨布局/页面切换保持；九宫格模式下禁用）──
  const rootRef = useRef<HTMLDivElement>(null);
  const [fontScale, setFontScale] = useState(
    () => terminalSessionRegistry.get(sessionId)?.fontScale ?? 1,
  );
  const fontScaleRef = useRef(fontScale);
  const baseFontSizeRef = useRef(
    terminalSessionRegistry.get(sessionId)?.baseFontSize ?? 13,
  );
  const fontScaleEnabledRef = useRef(fontScaleEnabled);
  fontScaleEnabledRef.current = fontScaleEnabled;
  const gestureStartScaleRef = useRef(1);
  const [fontBadgeVisible, setFontBadgeVisible] = useState(false);
  const fontBadgeTimerRef = useRef<number | null>(null);
  // 终端搜索（Ctrl+F）：searchAddon 引用 + 打开态；registry.showSearch 经 openSearch 桥进入
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const searchAddonRef = useRef<SearchAddon | null>(null);

  const onShellLabelChangeRef = useRef(onShellLabelChange);
  const onSizeChangeRef = useRef(onSizeChange);
  const onRendererStatusChangeRef = useRef(onRendererStatusChange);
  const onExitRef = useRef(onExit);
  onShellLabelChangeRef.current = onShellLabelChange;
  onSizeChangeRef.current = onSizeChange;
  onRendererStatusChangeRef.current = onRendererStatusChange;
  onExitRef.current = onExit;

  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
    hasSelection: boolean;
  } | null>(null);
  const contextMenuRef = useRef<HTMLDivElement | null>(null);
  /** 打开菜单的锚点元素：再次右键同一锚点时视为「关闭」而非原地重开 */
  const contextMenuAnchorRef = useRef<Element | null>(null);

  const notifyRendererStatus = useCallback(
    (kind: 'webgl' | 'canvas2d' | 'dom') => {
      onRendererStatusChangeRef.current?.(sessionId, kind);
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

  const executeFitNow = useCallback(
    (_reason: TerminalResizeReason = 'observer') => {
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
        return false;
      }

      try {
        // 关键：若元素曾被移入 display: none 暂存区或刚完成 DOM 挂载，xterm 的字符尺寸测量可能为 0 或失效。
        // 在 fit 之前显式调用 measure() 强制重新测量单字符宽与高，确保 proposeDimensions 计算出正确的 cols/rows。
        const core = (term as unknown as { _core?: { _charSizeService?: { measure?: () => void } } })._core;
        core?._charSizeService?.measure?.();
        fitAddon.fit();
      } catch (error) {
        console.warn('[TerminalSession] fit failed:', error);
        return false;
      }

      // 同步 WebGL 视口与渲染模型
      terminalSessionRegistry.refreshWebglViewport(sessionId);
      queueTerminalResize({ cols: term.cols, rows: term.rows });
      return true;
    },
    [queueTerminalResize, sessionId],
  );

  const requestFitAndResize = useCallback(
    (_reason: TerminalResizeReason = 'observer') => {
      // 单一 fit 通道：所有触发源统一合并到下一帧执行一次，同帧内的多次触发自动去重；
      // 布局尺寸在 ResizeObserver 回调触发时已结算完毕，无需再多轮延时补偿复核。
      if (resizeRAFRef.current) {
        cancelAnimationFrame(resizeRAFRef.current);
      }
      resizeRAFRef.current = requestAnimationFrame(() => {
        resizeRAFRef.current = 0;
        executeFitNow();
      });
    },
    [executeFitNow],
  );

  const clearWebglDeferTimer = useCallback(() => {
    if (webglDeferTimerRef.current !== null) {
      window.clearTimeout(webglDeferTimerRef.current);
      webglDeferTimerRef.current = null;
    }
  }, []);

  // WebGL 上下文（含 LRU 淘汰后重建）创建开销大且非首帧必需：延迟挂载，
  // 确保 fit/resize 与终端重绘先落地，不让上下文初始化阻塞布局切换与 Tab 激活。
  const scheduleWebglEnsure = useCallback(() => {
    clearWebglDeferTimer();
    webglDeferTimerRef.current = window.setTimeout(() => {
      webglDeferTimerRef.current = null;
      if (!isVisibleRef.current) return;
      terminalSessionRegistry.ensureRenderer(sessionId, notifyRendererStatus);
    }, WEBGL_ENSURE_DEFER_MS);
  }, [clearWebglDeferTimer, notifyRendererStatus, sessionId]);

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

  // 应用字体缩放：钳制倍率 → 取整像素字号写入 xterm → 清 WebGL 字形图集 →
  // 下一帧重测字符尺寸并 fit + 下发 PTY resize（行列与字号匹配，防止重排乱码）
  const applyTerminalFontScale = useCallback(
    (nextScale: number) => {
      const term = termRef.current;
      if (!term) return;
      const clamped = Math.min(
        TERMINAL_FONT_SCALE_MAX,
        Math.max(TERMINAL_FONT_SCALE_MIN, nextScale),
      );
      // 100% 磁吸：滑入 1.0 ± 4% 区间时吸附到精确 1.0，松手即回正
      const snapped =
        Math.abs(clamped - 1) < FONT_SCALE_SNAP_THRESHOLD ? 1 : clamped;
      const nextFontSize = Math.max(
        TERMINAL_FONT_MIN_PX,
        Math.round(baseFontSizeRef.current * snapped),
      );
      // 取整像素后反算倍率可能超出钳制边界（如 13px × 1.5 = 19.5 → round 20 → 20/13 ≈ 154%），
      // 对显示倍率再做一次钳制，保证 UI 展示的百分比不越过上下限
      const effectiveScale = Math.min(
        TERMINAL_FONT_SCALE_MAX,
        Math.max(TERMINAL_FONT_SCALE_MIN, nextFontSize / baseFontSizeRef.current),
      );
      fontScaleRef.current = effectiveScale;
      setFontScale(effectiveScale);
      term.options.fontSize = nextFontSize;

      const entry = terminalSessionRegistry.get(sessionId);
      if (entry) {
        entry.fontScale = effectiveScale;
        entry.baseFontSize = baseFontSizeRef.current;
      }
      // 字号变化需重建字形纹理图集（与主题切换同路径，同步所有会话渲染模型）
      terminalSessionRegistry.clearWebglTextureAtlas(sessionId);
      // 重测 + fit + PTY resize：保证 cols/rows 与新字号匹配
      requestFitAndResize('observer');

      setFontBadgeVisible(true);
      if (fontBadgeTimerRef.current !== null) {
        window.clearTimeout(fontBadgeTimerRef.current);
      }
      fontBadgeTimerRef.current = window.setTimeout(
        () => setFontBadgeVisible(false),
        1000,
      );
    },
    [requestFitAndResize, sessionId],
  );

  // 字体缩放手势（借用 Chrome 两指捏合通道，与 SFTP 同款）：
  // WebKit(WKWebView) gesture 事件 + Chrome 合成 ctrl+滚轮；九宫格模式下禁用
  useEffect(() => {
    const element = rootRef.current;
    if (!element) return;
    const onWheel = (event: WheelEvent) => {
      if (!event.ctrlKey && !event.metaKey) return;
      if (!fontScaleEnabledRef.current) return;
      event.preventDefault();
      applyTerminalFontScale(fontScaleRef.current * Math.exp(-event.deltaY * 0.002));
    };
    const onGestureStart = (event: Event) => {
      if (!fontScaleEnabledRef.current) return;
      event.preventDefault();
      gestureStartScaleRef.current = fontScaleRef.current;
    };
    const onGestureChange = (event: Event) => {
      if (!fontScaleEnabledRef.current) return;
      event.preventDefault();
      const scale = (event as unknown as { scale?: number }).scale ?? 1;
      applyTerminalFontScale(gestureStartScaleRef.current * scale);
    };
    element.addEventListener('wheel', onWheel, { passive: false });
    element.addEventListener('gesturestart', onGestureStart);
    element.addEventListener('gesturechange', onGestureChange);
    return () => {
      element.removeEventListener('wheel', onWheel);
      element.removeEventListener('gesturestart', onGestureStart);
      element.removeEventListener('gesturechange', onGestureChange);
      if (fontBadgeTimerRef.current !== null) {
        window.clearTimeout(fontBadgeTimerRef.current);
      }
    };
  }, [applyTerminalFontScale]);

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
      // Tab 切为可见时先 fit 与聚焦，WebGL 渲染器延迟到空闲时机再挂载/恢复
      scheduleFitAndResize('visible');
      requestFocus();
      scheduleWebglEnsure();
    } else {
      contextMenuAnchorRef.current = null;
      setContextMenu(null);
    }
  }, [isVisible, requestFocus, scheduleFitAndResize, scheduleWebglEnsure, sessionId]);

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
      searchAddonRef.current = existingEntry.searchAddon;
      existingEntry.openSearch = () => setIsSearchOpen(true);
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

      const reuseResizeObserver = new ResizeObserver(() => {
        if (!isVisibleRef.current) return;
        scheduleFitAndResize('observer');
      });
      reuseResizeObserver.observe(el);

      if (isVisibleRef.current) {
        requestFitAndResize('visible');
        requestFocus();
        scheduleWebglEnsure();
      } else {
        notifyRendererStatus(terminalSessionRegistry.getRendererKind(sessionId));
      }

      return () => {
        if (existingEntry.onTerminalIdReady) {
          existingEntry.onTerminalIdReady = undefined;
        }
        existingEntry.openSearch = undefined;
        reuseThemeObserver.disconnect();
        reuseResizeObserver.disconnect();
        clearResizeThrottleTimer();
        clearWebglDeferTimer();
        if (resizeRAFRef.current) cancelAnimationFrame(resizeRAFRef.current);
        if (focusRAFRef.current) cancelAnimationFrame(focusRAFRef.current);
        terminalSessionRegistry.detachDom(sessionId);
        termRef.current = null;
        fitAddonRef.current = null;
        searchAddonRef.current = null;
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
      // 细竖线光标：方块光标的宽度恒等于字符单元格宽度，xterm 无法做「半宽方块」，
      // 只能改用 bar 形态；cursorWidth 仅在 bar 下生效。
      // 非聚焦态同样用 bar，避免失焦后回退成默认的 outline 描边方块（又变宽）。
      cursorStyle: 'bar',
      cursorWidth: 2,
      cursorInactiveStyle: 'bar',
      allowProposedApi: true,
      scrollback: TERMINAL_SCROLLBACK_LINES,
      theme: mergeTerminalTheme(),
    });
    termRef.current = term;

    const fitAddon = new FitAddon();
    fitAddonRef.current = fitAddon;
    term.loadAddon(fitAddon);
    // 文本搜索：与 xterm 实例同生命周期挂载（纯 CPU，九宫格多会话零成本）
    const searchAddon = new SearchAddon();
    searchAddonRef.current = searchAddon;
    term.loadAddon(searchAddon);
    term.open(hostEl);

    const entry: TerminalSessionRegistryEntry = {
      key: sessionId,
      term,
      fitAddon,
      webglAddon: null,
      canvasAddon: null,
      rendererKind: 'dom',
      searchAddon,
      terminalId: null,
      cwd: cwd ?? '~',
      hostEl,
      baseFontSize: 13,
      fontScale: 1,
      onTerminalIdReady: (readyId: number) => {
        terminalIdRef.current = readyId;
        flushTerminalResize();
        scheduleFitAndResize('init');
        if (isVisibleRef.current) {
          requestFocus();
        }
      },
      openSearch: () => setIsSearchOpen(true),
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

    // 挂载 WebGL2 硬件加速渲染插件（当前可见 Tab 延迟到空闲时机惰性启用）
    if (isVisibleRef.current) {
      scheduleWebglEnsure();
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

    // 拦截与适配终端专属快捷键：
    // 1. Ctrl+C: 在终端中是标准的 SIGINT 中断信号（如终止 top / 终止当前进程）
    //    无论是否有文本选区，均直接向 PTY 写入 0x03 (ETX)，绝不能被复制逻辑拦截！
    // 2. 复制快捷键：
    //    - macOS 下为 Cmd+C (⌘C)
    //    - Windows 下为 Win+C (⊞C)
    //    - 兼顾跨平台通用 Ctrl+Shift+C
    //    有选区时复制选中文本到剪贴板，无选区时不执行操作。
    // 3. 粘贴快捷键：
    //    - macOS 下为 Cmd+V (⌘V)
    //    - Windows 下为 Win+V (⊞V)
    //    - 兼顾跨平台通用 Ctrl+Shift+V
    //    从剪贴板读取文本并通过 term.paste() 写入终端，阻止原生浏览器粘贴防止重复。
    // 4. 全选快捷键：
    //    - macOS 下为 Cmd+A (⌘A)
    //    - Windows 下为 Win+A (⊞A)
    //    - 兼顾跨平台通用 Ctrl+Shift+A
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== 'keydown') return true;
      const key = e.key.toLowerCase();

      // 1. Ctrl+C: 中断/终止进程（SIGINT）
      if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && key === 'c') {
        e.preventDefault();
        e.stopPropagation();
        term.clearSelection();
        const tid = entry.terminalId ?? terminalSessionRegistry.getTerminalId(sessionId);
        if (tid !== null) {
          void transport.write({ id: tid, data: new Uint8Array([3]) });
        }
        return false;
      }

      // 2. 复制快捷键：Cmd+C (macOS) / Win+C (Windows) / Ctrl+Shift+C
      const isCopy =
        (e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && key === 'c') ||
        (e.ctrlKey && e.shiftKey && !e.metaKey && !e.altKey && key === 'c');
      if (isCopy) {
        e.preventDefault();
        e.stopPropagation();
        if (term.hasSelection()) {
          const selectedText = term.getSelection();
          void writeClipboardText(selectedText).catch((err) => {
            console.warn('[TerminalSession] copy via shortcut failed:', err);
          });
        }
        return false;
      }

      // 3. 粘贴快捷键：Cmd+V (macOS) / Win+V (Windows) / Ctrl+Shift+V
      const isPaste =
        (e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && key === 'v') ||
        (e.ctrlKey && e.shiftKey && !e.metaKey && !e.altKey && key === 'v');
      if (isPaste) {
        e.preventDefault();
        e.stopPropagation();
        // 原生剪贴板读取（Rust 侧）：不再触发 WKWebView 粘贴授权弹层，Cmd+V 一键直达
        readClipboardText()
          .then((text) => {
            if (text) {
              term.paste(text);
            }
          })
          .catch((err) => console.warn('[TerminalSession] paste via shortcut failed:', err));
        return false;
      }

      // 4. 全选快捷键：Cmd+A (macOS) / Win+A (Windows) / Ctrl+Shift+A
      const isSelectAll =
        (e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && key === 'a') ||
        (e.ctrlKey && e.shiftKey && !e.metaKey && !e.altKey && key === 'a');
      if (isSelectAll) {
        e.preventDefault();
        e.stopPropagation();
        term.selectAll();
        return false;
      }

      // 5. Ctrl/⌘+F：打开终端搜索栏（拦截浏览器原生 find-in-page）
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && key === 'f') {
        e.preventDefault();
        e.stopPropagation();
        setIsSearchOpen(true);
        return false;
      }

      // 6. Esc：搜索栏打开时优先关闭搜索栏（而非其他全局 Esc 语义）
      if (!e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && e.key === 'Escape') {
        if (isSearchOpen) {
          e.preventDefault();
          e.stopPropagation();
          setIsSearchOpen(false);
          return false;
        }
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
          // Profile 字号作为缩放基准（仅当用户尚未缩放时更新基准，保持当前倍率）
          if (fontScaleRef.current === 1) {
            baseFontSizeRef.current = nextFontSize;
            entry.baseFontSize = nextFontSize;
          }
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
        // SSH 会话启动失败：暂存错误现场，供终端面板「问 AI」一键带入
        if (hostId != null) {
          const hostLabel = resolvedHost
            ? `${resolvedHost.username || 'root'}@${resolvedHost.host}:${resolvedHost.port}`
            : `hostId=${hostId}`;
          setAiErrorContext(
            'SSH 终端启动失败',
            buildSshErrorPrompt({ hostLabel, errorMessage: message }),
          );
        }
      });

    const resizeObserver = new ResizeObserver(() => {
      if (!isVisibleRef.current) return;
      scheduleFitAndResize('observer');
    });
    resizeObserver.observe(el);

    return () => {
      entry.onTerminalIdReady = undefined;
      entry.openSearch = undefined;
      themeObserver.disconnect();
      resizeObserver.disconnect();
      clearResizeThrottleTimer();
      clearWebglDeferTimer();
      if (resizeRAFRef.current) cancelAnimationFrame(resizeRAFRef.current);
      if (focusRAFRef.current) cancelAnimationFrame(focusRAFRef.current);
      // 仅将 DOM 移入暂存区，不中断异步创建中的 PTY（真正销毁仅在 closeSession -> registry.release 时发生）
      terminalSessionRegistry.detachDom(sessionId);
      termRef.current = null;
      fitAddonRef.current = null;
      searchAddonRef.current = null;
    };
  }, [
    clearResizeThrottleTimer,
    clearWebglDeferTimer,
    cwd,
    flushTerminalResize,
    hostId,
    notifyRendererStatus,
    queueTerminalResize,
    requestFocus,
    scheduleFitAndResize,
    scheduleWebglEnsure,
    sessionId,
    transport,
  ]);

  // 右键菜单关闭监听
  const closeContextMenu = useCallback(() => {
    contextMenuAnchorRef.current = null;
    setContextMenu(null);
  }, []);

  // 点击外部 / Escape 关闭（document 捕获阶段，不受子树 stopPropagation 影响）
  useDismissOnOutsidePointer(Boolean(contextMenu), contextMenuRef, closeContextMenu);

  const handleContextMenu = (event: ReactMouseEvent<HTMLDivElement>) => {
    event.preventDefault();
    // 菜单已打开时再次右键同一锚点 → 直接关闭。否则 pointerdown 先关、contextmenu 又
    // 在原地重开，视觉上等于「第一次点了没反应」。
    if (contextMenu && contextMenuAnchorRef.current === event.currentTarget) {
      closeContextMenu();
      return;
    }
    contextMenuAnchorRef.current = event.currentTarget;
    const term = termRef.current;
    setContextMenu({
      x: event.clientX,
      y: event.clientY,
      hasSelection: Boolean(term?.hasSelection()),
    });
  };

  const handleCopy = () => {
    const term = termRef.current;
    closeContextMenu();
    if (!term?.hasSelection()) return;
    void writeClipboardText(term.getSelection()).catch((err) => {
      console.warn('[TerminalSession] copy via context menu failed:', err);
    });
    requestFocus();
  };

  const handlePaste = () => {
    const term = termRef.current;
    closeContextMenu();
    if (!term) return;
    readClipboardText()
      .then((text) => {
        if (text) term.paste(text);
        requestFocus();
      })
      .catch((err) => console.warn('[TerminalSession] paste via context menu failed:', err));
  };

  const handleClear = () => {
    const term = termRef.current;
    closeContextMenu();
    term?.clear();
    requestFocus();
  };

  return (
    <div
      ref={rootRef}
      onContextMenu={handleContextMenu}
      className="relative h-full min-h-0 w-full overflow-hidden"
    >
      <div
        ref={containerRef}
        className="terminal-xterm-shell h-full min-h-0 w-full overflow-hidden"
      />

      {contextMenu ? (
        <div
          ref={contextMenuRef}
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
          <div className="my-1 h-px bg-border/60" />
          <button
            type="button"
            role="menuitem"
            disabled={fontScale === 1}
            onClick={() => {
              closeContextMenu();
              applyTerminalFontScale(1);
            }}
            className={cn(
              'flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors',
              fontScale === 1 ? 'cursor-not-allowed opacity-40' : 'hover:bg-menu-hover',
            )}
          >
            <Type className="size-3.5" />
            <span>{t('terminal.contextMenu.resetZoom')}</span>
            <span className="ml-auto font-mono text-ui-xs text-foreground-subtlest">
              {Math.round(fontScale * 100)}%
            </span>
          </button>
        </div>
      ) : null}

      {/* 终端搜索栏（Ctrl+F）：作用于本会话，overlay 不影响 fit 尺寸 */}
      {searchAddonRef.current ? (
        <TerminalSearchBar
          searchAddon={searchAddonRef.current}
          visible={isSearchOpen}
          onClose={() => setIsSearchOpen(false)}
        />
      ) : null}

      {/* 字体倍率气泡（Chrome 同款反馈）：捏合/ctrl+滚轮时短暂显示当前百分比 */}
      <div
        aria-hidden={!fontBadgeVisible}
        className={cn(
          'pointer-events-none absolute bottom-4 left-1/2 z-30 -translate-x-1/2 rounded-full border border-border bg-menu px-2.5 py-0.5 font-mono text-ui-xs text-foreground shadow-md transition-opacity duration-200',
          fontBadgeVisible ? 'opacity-100' : 'opacity-0',
        )}
      >
        {Math.round(fontScale * 100)}%
      </div>
    </div>
  );
}
