import { listen } from '@tauri-apps/api/event';
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { Terminal as XTerm, type IDisposable } from '@xterm/xterm';
import { Circle, CircleStop, ClipboardPaste, Copy, Eraser, MonitorPlay, Type } from 'lucide-react';
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from 'react';
import { useTranslation } from 'react-i18next';
import { readClipboardText, writeClipboardText } from '@/lib/clipboard';
import { hasPrimaryModifier, shortcutLabel } from '@/lib/keyboard';
import { isMacOS, platformKind } from '@/lib/platform';
import { formatPathsForTerminal, type TerminalPathStyle } from '@/lib/shellPath';
import { cn } from '@/lib/utils';
import { useDismissOnOutsidePointer } from '@/hooks/useDismissOnOutsidePointer';
import { useDropTarget } from '@/hooks/useDropTarget';
import { highlightOutput, registerOutputLinkProvider } from '@/terminal/outputHighlight';
import { onSshKind } from '@/services/sshLifecycle';
import { restoreSessionSnapshot, saveSessionSnapshot } from '@/terminal/sessionSnapshot';
import { TerminalSearchBar } from '@/components/terminal/TerminalSearchBar';
import { TerminalResourceBar } from '@/components/terminal/TerminalResourceBar';
import { RecordingPill } from '@/components/terminal/RecordingPill';
import { CastPlayerModal } from '@/components/terminal/CastPlayerModal';
import { SafetyGuardModal } from '@/components/terminal/SafetyGuardModal';
import {
  SafetyInputTracker,
  extractCommandLineFromTerminal,
  inspectCommandSafety,
  type SafetyInspectResult,
  type Osc133TrackerState,
} from '@/services/safetyGuardService';
import { loadHostStore, resolveJumpHostChain } from '@/hosts/storage';
import { formatShellShortLabel } from '@/terminal/terminalPanelState';
import {
  terminalSessionRegistry,
  type TerminalSessionRegistryEntry,
} from '@/terminal/terminalSessionRegistry';
import { mergeTerminalTheme } from '@/terminal/terminalTheme';
import { buildSshErrorPrompt, setAiErrorContext } from '@/services/aiContext';
import { listCommands, markCommandUsed } from '@/services/commandService';
import { mirrorInputToSessions, mirrorClearToSessions } from '@/services/terminalBroadcastService';
import {
  createTerminalByteChannel,
  encodeBinaryString,
  encodeTerminalInput,
} from '@/terminal/transport';
import {
  dropCastSession,
  getActiveRecorder,
  getLastRecording,
  startCastRecording,
  stopCastRecording,
  type CastRecording,
} from '@/terminal/castRecorder';
import { exportCastFile, exportLogFile } from '@/terminal/castExport';
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
/** 就绪钩子：首屏输出（登录横幅 + 提示符）停止多久视为 shell 就绪，可以安全注入命令 */
const HOOK_QUIET_MS = 500;
/** 就绪钩子兜底上限：持续刷屏或无输出的异常场景下，限时后也按就绪处理 */
const HOOK_MAX_WAIT_MS = 6000;

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
  /** 终端联动目标集合（store 广播状态）：非空且包含本会话时，用户输入实时镜像到其余目标 */
  broadcastTargets: number[];
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
  /** 当前会话是否处于断开态（SSH 连接断开，等待重连） */
  isDisconnected?: boolean;
  /** SSH 会话断开回调（保留 Tab 与终端现场，标记断开态） */
  onDisconnected?: (sessionId: number, reason: string) => void;
  /** 终端聚焦时 ⌘R/Ctrl+R 重连请求 */
  onReconnectRequest?: (sessionId: number) => void;
  /** 底部远程资源监控条是否展示（仅标签页模式的当前可见 SSH 会话为 true；
   * 九宫格平铺自动隐藏，由 TerminalGridContainer 按布局模式计算） */
  resourceBarVisible?: boolean;
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
  broadcastTargets,
  cwd,
  hostId,
  isVisible,
  isPanelResizing = false,
  fontScaleEnabled = true,
  onShellLabelChange,
  onSizeChange,
  onRendererStatusChange,
  onExit,
  isDisconnected = false,
  onDisconnected,
  onReconnectRequest,
  resourceBarVisible = false,
}: TerminalSessionProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const terminalIdRef = useRef<number | null>(null);
  const isVisibleRef = useRef(isVisible);
  // 联动目标集合的实时快照：onData 在挂载时注册一次，闭包不能捕获旧集合
  const broadcastTargetsRef = useRef(broadcastTargets);
  useEffect(() => {
    broadcastTargetsRef.current = broadcastTargets;
  }, [broadcastTargets]);
  const isPanelResizingRef = useRef(isPanelResizing);
  // ── ZMODEM 传输期输入锁 ──
  // 传输中用户键入会混入协议流、破坏 ZMODEM 帧；故 onData/onBinary 统一在
  // writeInputAndMirror 入口拦截（直接 return，不写后端 → 不污染）。
  // 解锁三重保障：① 后端 isFinished 事件（成功/失败/取消都发）；
  // ② 看门狗超时兜底（事件丢失/后端崩溃也不会永久锁死）；③ 组件卸载清理。
  const zmodemLockRef = useRef(false);
  const [zmodemLocked, setZmodemLocked] = useState(false);
  const zmodemWatchdogRef = useRef<number | null>(null);
  // 光标隐藏状态：与输入锁**同一时机**（同一事件、同一解锁路径、同一看门狗）
  const zmodemCursorHiddenRef = useRef(false);
  // 提示节流：连续敲键时只在首次给出提示，避免刷屏
  const zmodemHintShownRef = useRef(false);
  const [zmodemHint, setZmodemHint] = useState(false);

  useEffect(() => {
    const unlock = () => {
      zmodemLockRef.current = false;
      zmodemHintShownRef.current = false;
      setZmodemLocked(false);
      setZmodemHint(false);
      if (zmodemWatchdogRef.current !== null) {
        window.clearTimeout(zmodemWatchdogRef.current);
        zmodemWatchdogRef.current = null;
      }
      // 恢复光标（与解锁同一路径，幂等）
      if (zmodemCursorHiddenRef.current) {
        zmodemCursorHiddenRef.current = false;
        termRef.current?.write('\x1b[?25h');
      }
    };
    /** 传输期隐藏光标（幂等；与输入锁同步进入） */
    const lockAndHideCursor = () => {
      zmodemLockRef.current = true;
      setZmodemLocked(true);
      if (!zmodemCursorHiddenRef.current) {
        zmodemCursorHiddenRef.current = true;
        termRef.current?.write('\x1b[?25l');
      }
    };
    let disposed = false;
    // 本会话当前是否处于传输中（组级锁：组内任一成员传输 → 全组锁输入）
    let selfTransferring = false;
    const unlistenPromise = listen<{ sessionId: string; isFinished: boolean }>(
      'zmodem-progress',
      (event) => {
        if (disposed) return;
        // 后端事件带的是 terminalId；本组件的 sessionId 是前端编号，
        // 经 registry 的 entry.terminalId 对齐（两套独立计数）
        const myTerminalId = terminalSessionRegistry.getTerminalId(sessionId);
        const mine = myTerminalId !== null && event.payload.sessionId === `terminal-${myTerminalId}`;
        const inGroup =
          broadcastTargetsRef.current.length > 1 &&
          broadcastTargetsRef.current.includes(sessionId);
        // 组内其他成员的事件同样影响本会话的锁状态
        if (!mine && !inGroup) return;
        if (mine) selfTransferring = !event.payload.isFinished;
        if (selfTransferring) {
          lockAndHideCursor();
          if (zmodemWatchdogRef.current !== null) {
            window.clearTimeout(zmodemWatchdogRef.current);
          }
          zmodemWatchdogRef.current = window.setTimeout(unlock, 10 * 60 * 1000);
        } else if (mine) {
          // 本会话结束：组内若还有其他传输，保持锁定（无法精确得知，保守解锁）
          unlock();
        }
      },
    );
    return () => {
      disposed = true;
      unlock();
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [sessionId]);

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

  // ── 终端录制（Asciinema v2 .cast）──
  // 录制器实例存模块级 Map（castRecorder.ts），跨重挂载存活；这里只持 UI 状态。
  // recStopped：停止后胶囊以「已停止」形态保留为操作面板锚点，面板关闭即复位。
  const [recActive, setRecActive] = useState(false);
  const [recElapsedMs, setRecElapsedMs] = useState(0);
  const [recStopped, setRecStopped] = useState(false);
  const [recHasLast, setRecHasLast] = useState(false);
  const [playerRecording, setPlayerRecording] = useState<CastRecording | null>(null);

  // ── 生产环境安全拦截与视觉护罩 ──
  const safetyTrackerRef = useRef(new SafetyInputTracker());
  const osc133TrackerRef = useRef<Osc133TrackerState>({
    hasOsc133: false,
    commandStartX: 0,
    commandStartY: 0,
    inCommand: false,
  });
  const [safetyModal, setSafetyModal] = useState<{
    isOpen: boolean;
    command: string;
    inspectResult: SafetyInspectResult;
    pendingBytes: Uint8Array;
  } | null>(null);

  const currentHost = hostId != null ? loadHostStore().hosts.find((h) => h.id === hostId) : undefined;
  const envLevel = currentHost?.envLevel ?? 'dev';
  const isSafetyShieldEnabled =
    (currentHost?.safetyGuardEnabled ?? true) && (envLevel === 'prod' || envLevel === 'staging');

  const onShellLabelChangeRef = useRef(onShellLabelChange);
  const onSizeChangeRef = useRef(onSizeChange);
  const onRendererStatusChangeRef = useRef(onRendererStatusChange);
  const onExitRef = useRef(onExit);
  const isDisconnectedRef = useRef(isDisconnected);
  const onDisconnectedRef = useRef(onDisconnected);
  const onReconnectRequestRef = useRef(onReconnectRequest);
  onShellLabelChangeRef.current = onShellLabelChange;
  onSizeChangeRef.current = onSizeChange;
  onRendererStatusChangeRef.current = onRendererStatusChange;
  onExitRef.current = onExit;
  isDisconnectedRef.current = isDisconnected;
  onDisconnectedRef.current = onDisconnected;
  onReconnectRequestRef.current = onReconnectRequest;

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
      // 录制旁路：真实 resize 落定后记 "r" 事件（与上次相同尺寸在 recorder 内去重）
      getActiveRecorder(sessionId)?.feedResize(size.cols, size.rows);
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

  // 拖拽文件进终端 → 把路径写进命令行（对齐原生 cmd/zsh 行为）：
  // 路径格式跟随目标会话的 shell 类型——Windows 本地（固定 cmd.exe）用按需双引号，
  // macOS 本地与 SSH 远程用 POSIX 反斜杠转义；多文件空格分隔依次写入。
  // term.paste 走 onData 通路：联动模式下同步镜像到其余勾选终端（与粘贴一致）
  useDropTarget({
    ref: rootRef,
    accepts: ['files'],
    onDrop: (event) => {
      if (event.kind !== 'files' || event.paths.length === 0) return;
      const term = termRef.current;
      if (!term) return;
      const style: TerminalPathStyle =
        hostId != null ? 'posix' : platformKind === 'windows' ? 'windows' : 'posix';
      term.paste(formatPathsForTerminal(event.paths, style));
      requestFocus();
    },
  });

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

    // ===== 终端就绪钩子注入控制器 =====
    // 命令必须等 shell 首屏输出静止后再发：若字节在 readline 接管 tty 之前到达，
    // 会先被内核 tty 回显一次，随后 readline 读到输入缓冲又重绘一次——用户看到
    // 命令出现两遍。策略：每次输出重置「静止计时」，静止 HOOK_QUIET_MS 即视为
    // 提示符已就绪发送；HOOK_MAX_WAIT_MS 兜底（持续刷屏 / 无输出也不会永远不发）
    const hookInjector = {
      terminalId: null as number | null,
      commandId: null as number | null,
      command: null as string | null,
      sent: false,
      /** shell 通道已就绪（LifecycleKind::ShellReady）——认证期不得注入，
       *  否则命令+回车会被 kbd-int 的 AUTH_INPUT_WAITER 当成密码提交 */
      shellReady: false,
      quietTimer: null as number | null,
      deadlineTimer: null as number | null,
    };

    const hookClearTimers = () => {
      if (hookInjector.quietTimer != null) {
        window.clearTimeout(hookInjector.quietTimer);
        hookInjector.quietTimer = null;
      }
      if (hookInjector.deadlineTimer != null) {
        window.clearTimeout(hookInjector.deadlineTimer);
        hookInjector.deadlineTimer = null;
      }
    };

    const hookTrySend = () => {
      if (isEntryDisposed || hookInjector.sent) return;
      // 双保险：deadline 兜底计时也不能绕过 shellReady（认证可能远超 6s）
      if (!hookInjector.shellReady) return;
      const { terminalId, commandId, command } = hookInjector;
      if (terminalId == null || command == null) return;
      hookInjector.sent = true;
      hookClearTimers();
      const bytes = encodeTerminalInput(`${command}\r`);
      void transport.write({ id: terminalId, data: bytes });
      if (commandId != null) {
        void markCommandUsed(commandId);
      }
    };

    /** 输出到达：重置静止计时（仅在待注入时生效） */
    const hookOnOutput = () => {
      if (hookInjector.command == null || hookInjector.sent || isEntryDisposed) return;
      // 认证期禁止注入：密码过期时 kbd-int 提示（"当前的密码:"）也走输出流，
      // 若在此计时会把就绪钩子命令+回车当成密码提交给 AUTH_INPUT_WAITER，
      // 直接导致认证失败断开（实测：提示一出现就自动回车）。只有 shell 通道
      // 真正就绪（LifecycleKind::ShellReady）后才允许开始静止计时
      if (!hookInjector.shellReady) return;
      if (hookInjector.quietTimer != null) {
        window.clearTimeout(hookInjector.quietTimer);
      }
      hookInjector.quietTimer = window.setTimeout(hookTrySend, HOOK_QUIET_MS);
    };

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

    // IP/URL 可点击链接（悬停下划线由 xterm 自带，随 term.dispose 一并释放）
    registerOutputLinkProvider(term);

    // 注册 OSC 133 语义化 Shell Integration 监听器（支持精准命令起点定位）
    const oscTracker = osc133TrackerRef.current;
    const osc133Disposer = term.parser.registerOscHandler(133, (data) => {
      const code = data[0];
      if (code === 'B') {
        const buffer = term.buffer.active;
        oscTracker.hasOsc133 = true;
        oscTracker.inCommand = true;
        oscTracker.commandStartX = buffer.cursorX;
        oscTracker.commandStartY = buffer.cursorY + buffer.baseY;
        return true;
      } else if (code === 'C' || code === 'D' || code === 'A') {
        oscTracker.inCommand = false;
        return true;
      }
      return false;
    });
    registryDisposers.push(osc133Disposer);

    // 重连 remount 时恢复上一实例的滚动缓冲（快照在 registry.release →
    // entry.dispose 时存入；首次挂载无快照，此处 no-op）
    restoreSessionSnapshot(sessionId, term);

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

    let cleanupOutputWriter: (() => void) | undefined;

    entry.dispose = () => {
      isEntryDisposed = true;
      cleanupOutputWriter?.();
      hookClearTimers();
      entry.onTerminalIdReady = undefined;
      // 会话真正销毁：清理录制器与上次成品（重挂载路径不会到这里）
      dropCastSession(sessionId);
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
      // 重连（release → remount）前保留滚动缓冲；关闭标签页同样会走到
      // 这里存一份，由 sessionSnapshot 的容量上限兜底淘汰
      saveSessionSnapshot(sessionId, term);
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

    /**
     * 写本终端并（联动生效时）镜像给其余勾选终端。
     *
     * 快捷键拦截路径（Ctrl+C 等）必须走这里而不是裸 transport.write：拦截后
     * 手动写的字节不经过 xterm 的 onData，而镜像逻辑只挂在 onData/onBinary 上，
     * 裸写会导致「联动开着但 Ctrl+C 只中断了自己」。
     *
     * 仅用于等价于用户键入的输入；程序化注入（就绪钩子命令）不走此路——
     * 它本就只应作用于本终端。
     */
    const writeInputAndMirror = (bytes: Uint8Array) => {
      // ZMODEM 传输期：拦截一切用户输入（不写后端 → 协议流零污染）。
      // 首次拦截给出可见提示，之后静默（避免连续敲键刷屏）。
      if (zmodemLockRef.current) {
        if (!zmodemHintShownRef.current) {
          zmodemHintShownRef.current = true;
          setZmodemHint(true);
        }
        return;
      }
      const tid = terminalIdRef.current ?? terminalSessionRegistry.getTerminalId(sessionId);
      if (tid === null) return;
      void transport.write({ id: tid, data: bytes });
      const targets = broadcastTargetsRef.current;
      if (targets.length > 1 && targets.includes(sessionId)) {
        void mirrorInputToSessions(transport, targets, sessionId, bytes);
      }
    };

    // 拦截与适配终端专属快捷键：
    // 1. Ctrl+C:
    //    - Windows：有选区时按原生 conhost/Windows Terminal 语义复制并清选区；
    //      无选区仍写 0x03 (SIGINT) 中断进程
    //    - macOS：恒为 SIGINT（复制走 ⌘C，Ctrl 留给终端控制字符）
    // 2. 复制快捷键：
    //    - macOS 下为 Cmd+C (⌘C)
    //    - 跨平台通用 Ctrl+Shift+C（Windows 菜单指示也用这个）
    //    有选区时复制选中文本到剪贴板，无选区时不执行操作。
    // 3. 粘贴快捷键：
    //    - macOS 下为 Cmd+V (⌘V)
    //    - Windows 下 Ctrl+V（conhost/Windows Terminal 默认键位）与
    //      Ctrl+Shift+V 均可（WebView2 的原生 paste 事件不可靠，统一走 Rust 剪贴板）
    //    从剪贴板读取文本并通过 term.paste() 写入终端，阻止原生浏览器粘贴防止重复。
    // 4. 全选快捷键：
    //    - macOS 下为 Cmd+A (⌘A)
    //    - Windows 下为 Win+A (⊞A)
    //    - 兼顾跨平台通用 Ctrl+Shift+A
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== 'keydown') return true;
      const key = e.key.toLowerCase();

      // 1. Ctrl+C: 有选区且在 Windows → 复制（原生语义）；否则 SIGINT 中断/终止进程
      if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && key === 'c') {
        e.preventDefault();
        e.stopPropagation();
        if (platformKind === 'windows' && term.hasSelection()) {
          const selectedText = term.getSelection();
          term.clearSelection();
          void writeClipboardText(selectedText).catch((err) => {
            console.warn('[TerminalSession] copy via Ctrl+C failed:', err);
          });
        } else {
          term.clearSelection();
          // 经 writeInputAndMirror：联动开启时 0x03 同步发往其余勾选终端
          writeInputAndMirror(new Uint8Array([3]));
        }
        return false;
      }

      // 2. 复制快捷键：Cmd+C (macOS) / Ctrl+Shift+C
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

      // 3. 粘贴快捷键：Cmd+V (macOS) / Ctrl+Shift+V / Windows 裸 Ctrl+V
      const isPaste =
        (e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && key === 'v') ||
        (e.ctrlKey && e.shiftKey && !e.metaKey && !e.altKey && key === 'v') ||
        (platformKind === 'windows' &&
          e.ctrlKey &&
          !e.shiftKey &&
          !e.metaKey &&
          !e.altKey &&
          key === 'v');
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

      // 5. 重连快捷键（主修饰键+R）：仅断开态拦截——
      //    活动会话不拦截 Ctrl+R（保留 shell 反向搜索 ^R）；断开态下终端输入无意义，拦截并请求重连
      if (
        !e.altKey &&
        !e.shiftKey &&
        hasPrimaryModifier(e) &&
        key === 'r' &&
        isDisconnectedRef.current
      ) {
        e.preventDefault();
        e.stopPropagation();
        onReconnectRequestRef.current?.(sessionId);
        return false;
      }

      // 6. 主修饰键+F：打开终端搜索栏（拦截浏览器原生 find-in-page）
      if (hasPrimaryModifier(e) && !e.altKey && !e.shiftKey && key === 'f') {
        e.preventDefault();
        e.stopPropagation();
        setIsSearchOpen(true);
        return false;
      }

      // 7. Esc：搜索栏打开时优先关闭搜索栏（而非其他全局 Esc 语义）
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

    // 建立接收后端 `Channel<Vec<u8>>` 输出的字节通道，直接将 `Uint8Array` 写入 xterm。
    // 性能优化：
    // 1. 复用持久单例 TextDecoder 并开启 stream: true，消除每次回调频繁 new 的 GC 压力并防止跨包多字节字符切分乱码；
    // 2. 突发洪峰流量保护（Flood Protection）：统计 100ms 窗口吞吐，>64KB 时自动 bypass 智能高亮行扫描；
    // 3. requestAnimationFrame 批量合并写入缓冲，降低密集小包频繁调用 xterm.write 引起的渲染重绘开销。
    const decoder = new TextDecoder('utf-8', { fatal: false });
    let pendingWrite = '';
    let rafHandle: number | null = null;
    let burstByteCount = 0;
    let burstWindowStart = performance.now();
    let isBurstMode = false;

    const flushWrite = () => {
      rafHandle = null;
      if (isEntryDisposed || !pendingWrite) return;
      const toWrite = pendingWrite;
      pendingWrite = '';
      term.write(toWrite);
    };

    const scheduleWrite = (text: string) => {
      pendingWrite += text;
      if (pendingWrite.length >= 65536) {
        if (rafHandle !== null) {
          cancelAnimationFrame(rafHandle);
          rafHandle = null;
        }
        flushWrite();
        return;
      }
      if (rafHandle === null) {
        rafHandle = requestAnimationFrame(flushWrite);
      }
    };

    cleanupOutputWriter = () => {
      if (rafHandle !== null) {
        cancelAnimationFrame(rafHandle);
        rafHandle = null;
      }
      pendingWrite = '';
    };

    const outputChannel = createTerminalByteChannel((bytes: Uint8Array) => {
      if (isEntryDisposed) return;
      const now = performance.now();
      if (now - burstWindowStart > 100) {
        isBurstMode = burstByteCount > 64 * 1024;
        burstByteCount = 0;
        burstWindowStart = now;
      }
      burstByteCount += bytes.length;

      const text = decoder.decode(bytes, { stream: true });
      // 录制旁路：取「高亮注入前」的原始文本（highlightOutput 会注入 ANSI 颜色码，
      // 录进去会污染回放保真）；ZMODEM 传输期跳过（二进制协议帧回放即乱码）
      const recorder = getActiveRecorder(sessionId);
      if (recorder && !zmodemLockRef.current) {
        recorder.feedOutput(text);
      }
      const processed = isBurstMode ? text : highlightOutput(text);
      scheduleWrite(processed);
      // 就绪钩子待发时，每次输出重置静止计时（输出停止 = 首屏完成，可安全注入）
      hookOnOutput();
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
        // SSH 会话断开：保留 Tab 与终端现场，标记断开态等用户重连，绝不自动关 Tab。
        // 提示直接写进 xterm 缓冲（像真实终端掉线那样），不叠加任何 DOM 覆盖层；
        // 重连入口是快捷键（终端内 ⌘R/Ctrl+R，焦点在终端外时全局 ⌘R 也生效）
        if (hostId != null) {
          term.write(
            `\r\n\x1b[33m⏏ 连接已断开（退出码 ${exitCode}）\x1b[0m\r\n` +
              `\x1b[90m按 ${shortcutLabel('R')} 重新连接\x1b[0m\r\n`,
          );
          onDisconnectedRef.current?.(sessionId, `连接已断开（退出码 ${exitCode}）`);
          return;
        }
          // 本地会话：若 Shell 正常退出且并非启动瞬间闪退，则触发自动关闭 Tab；否则打印退出码供排查
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

        // 将用户输入统一编码为 Uint8Array（对应后端 Vec<u8>）发送；
        // 联动模式下同时镜像给其余目标终端（只发输入不注入输出，回显由各自
        // pty 完成，接收方 onData 不会被程序化写入触发，无回环）。
        // 走 writeInputAndMirror 统一入口：快捷键拦截路径（Ctrl+C）共用同一逻辑
        registryDisposers.push(
          term.onData((data) => {
            const rawBytes = encodeTerminalInput(data);
            if (isSafetyShieldEnabled) {
              const isEnterHit = data.includes('\r') || data.includes('\n');
              if (isEnterHit) {
                // 生产/预发高危拦截检查：优先从真实屏幕行提取（涵盖按⬆️调历史命令、Tab补全、光标编辑等场景）
                const extracted = extractCommandLineFromTerminal(term, osc133TrackerRef.current);
                const trackerCmd = safetyTrackerRef.current.feed(data).commandToInspect;
                const cmdToInspect = extracted.command || trackerCmd;

                if (cmdToInspect) {
                  void inspectCommandSafety(cmdToInspect, envLevel, extracted.fromOsc133).then((inspectRes) => {
                    if (inspectRes.level !== 'safe') {
                      setSafetyModal({
                        isOpen: true,
                        command: cmdToInspect,
                        inspectResult: inspectRes,
                        pendingBytes: rawBytes,
                      });
                    } else {
                      writeInputAndMirror(rawBytes);
                    }
                  });
                  return;
                }
              } else {
                safetyTrackerRef.current.feed(data);
              }
            }
            writeInputAndMirror(rawBytes);
          }),
        );

        registryDisposers.push(
          term.onBinary((binary) => {
            writeInputAndMirror(encodeBinaryString(binary));
          }),
        );

        // 终端就绪钩子：主机配置引用了命令库命令（仅存主键 id）时，等 shell 首屏
        // 输出静止后再把命令作为终端输入发出（等价于用户敲入并回车）。
        // 不能创建后立即发：命令字节若在 readline 接管 tty 前到达，会被内核回显
        // + readline 重绘各显示一次，用户会看到命令出现两遍。命令已被删则静默跳过
        const readyCommandId = resolvedHost?.readyCommandId ?? null;
        if (readyCommandId != null) {
          // shell 通道真正就绪（认证全部完成后首个 shell 输出）才允许注入：
          // 认证期的 kbd-int 提示同样走输出流，若按输出计时会把钩子命令当成
          // 密码提交（见 hookOnOutput 注释）。监听失败则 hookInjector.shellReady
          // 永不置位 → 钩子静默不发（宁可不用，不可误发密码）
          void onSshKind('shell-ready', (event) => {
            if (isEntryDisposed) return;
            const myTerminalId = terminalSessionRegistry.getTerminalId(sessionId);
            if (myTerminalId === null || event.sessionId !== myTerminalId) return;
            hookInjector.shellReady = true;
            hookOnOutput();
          })
            .then((unlisten) => {
              if (isEntryDisposed) {
                unlisten();
                return;
              }
              registryDisposers.push({ dispose: unlisten });
            })
            .catch(() => undefined);
          void listCommands()
            .then((all) => all.find((cmd) => cmd.id === readyCommandId))
            .then((cmd) => {
              if (isEntryDisposed || !cmd) return;
              hookInjector.terminalId = id;
              hookInjector.commandId = cmd.id;
              hookInjector.command = cmd.command;
              // 兜底上限：shell-ready 后持续刷屏 / 无输出的极端场景
              hookInjector.deadlineTimer = window.setTimeout(hookTrySend, HOOK_MAX_WAIT_MS);
              // 若 shell-ready 已先到（事件早于命令加载完成），立即开始静止计时
              if (hookInjector.shellReady) {
                hookInjector.quietTimer = window.setTimeout(hookTrySend, HOOK_QUIET_MS);
              }
            })
            .catch(() => undefined);
        }
      })
      .catch((error) => {
        if (isEntryDisposed) return;
        const message = error instanceof Error ? error.message : String(error);
        console.error('[TerminalSession] failed to create terminal:', error);
        term.write(`\r\n[Terminal failed to start]\r\n${message}\r\n`);
        // SSH 会话启动失败：暂存错误现场，供终端面板「问 AI」一键带入；
        // 同时标记断开态供 ⌘R 重连（断开提示直接写在上方 xterm 输出里）
        if (hostId != null) {
          term.write(`\x1b[90m按 ${shortcutLabel('R')} 重新连接\x1b[0m\r\n`);
          const hostLabel = resolvedHost
            ? `${resolvedHost.username || 'root'}@${resolvedHost.host}:${resolvedHost.port}`
            : `hostId=${hostId}`;
          setAiErrorContext(
            'SSH 终端启动失败',
            buildSshErrorPrompt({ hostLabel, errorMessage: message }),
          );
          onDisconnectedRef.current?.(sessionId, `连接失败：${message}`);
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
      hookClearTimers();
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
    // Windows 本地终端（cmd）：term.clear() 只清前端视口、旧内容仍留在回滚区，
    // 而 cmd 用户习惯的 cls 是整缓冲清除——把 cls 当键入发给 PTY。程序化注入走
    // transport.write 直写（与就绪钩子同路）：不进 onData，联动模式不会把
    // cls 文本镜像到其余终端（那些目标仍走前端清屏）。
    // 注意 id 语义：transport.write 的 id 是**后端分配的 terminalId**（异步就绪，
    // 见 onTerminalIdReady），不是前端 Tab 的 sessionId——两者各自从 1 自增，
    // 首个终端碰巧同值，但开/关过 Tab 后序列错开，用 sessionId 会把 cls 写到
    // 别的后端终端（当前终端收不到清屏序列，前端缓冲残留 → 视口错位）。
    // 未就绪（terminalId 尚为 null）时退回前端清屏。
    const tid = terminalIdRef.current ?? terminalSessionRegistry.getTerminalId(sessionId);
    if (hostId == null && platformKind === 'windows' && tid !== null) {
      void transport.write({ id: tid, data: encodeTerminalInput('cls\r') });
    } else {
      term?.clear();
    }
    // 联动开启时清屏同步到组内其余终端（纯前端缓冲清除，与输入镜像同一门控）
    mirrorClearToSessions(broadcastTargetsRef.current, sessionId);
    requestFocus();
  };

  // ── 终端录制（Asciinema v2）：右键菜单开关，胶囊与操作面板见 RecordingPill ──

  const recordingTitle = useCallback((): string => {
    if (hostId != null) {
      const host = loadHostStore().hosts.find((h) => h.id === hostId);
      if (host) return `${host.username || 'root'}@${host.host}`;
    }
    return 'local';
  }, [hostId]);

  const startRecording = useCallback(() => {
    const term = termRef.current;
    if (!term) return;
    if (startCastRecording(sessionId, term.cols, term.rows, recordingTitle())) {
      setRecActive(true);
      setRecStopped(false);
      setRecHasLast(false);
      setRecElapsedMs(0);
    }
  }, [recordingTitle, sessionId]);

  /** 停止录制；返回成品（在录返回新成品，否则回落上次成品） */
  const stopRecording = useCallback((): CastRecording | null => {
    const rec = stopCastRecording(sessionId);
    if (rec) {
      setRecActive(false);
      setRecStopped(true);
      setRecHasLast(true);
      setRecElapsedMs(0);
      return rec;
    }
    return getLastRecording(sessionId);
  }, [sessionId]);

  const openRecordingPreview = useCallback(() => {
    const rec = stopRecording();
    if (rec) setPlayerRecording(rec);
  }, [stopRecording]);

  const handleExportCast = useCallback(() => {
    const rec = stopRecording();
    if (rec) void exportCastFile(rec);
  }, [stopRecording]);

  const handleExportLog = useCallback(() => {
    const rec = stopRecording();
    if (rec) void exportLogFile(rec);
  }, [stopRecording]);

  const closeStoppedPill = useCallback(() => setRecStopped(false), []);

  const handleSafetyConfirm = useCallback(() => {
    if (safetyModal) {
      const tid = terminalIdRef.current ?? terminalSessionRegistry.getTerminalId(sessionId);
      if (tid !== null) {
        void transport.write({ id: tid, data: safetyModal.pendingBytes });
      }
      setSafetyModal(null);
      requestFocus();
    }
  }, [requestFocus, safetyModal, sessionId, transport]);

  const handleSafetyCancel = useCallback(() => {
    // 发送 0x03 (Ctrl+C) 清理终端当前行
    const tid = terminalIdRef.current ?? terminalSessionRegistry.getTerminalId(sessionId);
    if (tid !== null) {
      void transport.write({ id: tid, data: new Uint8Array([3]) });
    }
    safetyTrackerRef.current.clear();
    setSafetyModal(null);
    requestFocus();
  }, [requestFocus, sessionId, transport]);

  // 录制计时：每秒从 recorder 取真实经过时间（胶囊展示）
  useEffect(() => {
    if (!recActive) return;
    const timer = window.setInterval(() => {
      setRecElapsedMs(getActiveRecorder(sessionId)?.elapsedMs() ?? 0);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [recActive, sessionId]);

  return (
    <div
      ref={rootRef}
      onContextMenu={handleContextMenu}
      className={cn(
        'relative flex h-full min-h-0 w-full flex-col overflow-hidden',
        envLevel === 'prod' && 'ring-1 ring-inset ring-red-500/40',
        envLevel === 'staging' && 'ring-1 ring-inset ring-amber-500/40',
      )}
    >
      {envLevel === 'prod' ? (
        <div className="h-[2px] w-full shrink-0 bg-gradient-to-r from-red-600 via-rose-500 to-red-600 opacity-80" />
      ) : envLevel === 'staging' ? (
        <div className="h-[2px] w-full shrink-0 bg-gradient-to-r from-amber-600 via-yellow-500 to-amber-600 opacity-80" />
      ) : null}
      <div
        ref={containerRef}
        className="terminal-xterm-shell min-h-0 w-full flex-1 overflow-hidden"
      />

      {/* ZMODEM 传输期输入锁提示：键入被拦截时短暂显示（不写入终端缓冲，
          避免污染终端内容/协议流） */}
      {zmodemLocked && zmodemHint ? (
        <div
          role="status"
          aria-live="polite"
          className="pointer-events-none absolute left-1/2 top-2 z-20 -translate-x-1/2 rounded-full border border-border bg-menu px-2.5 py-0.5 font-mono text-ui-xs text-foreground-subtle shadow-md"
        >
          {t('terminal.zmodem.inputLocked')}
        </div>
      ) : null}

      {/* 远程主机资源监控条（终端内容最底一行）：仅 SSH 会话 + 标签页模式展示；
          挂载/卸载改变 xterm 容器高度，ResizeObserver 会自动 refit */}
      {resourceBarVisible && hostId != null ? (
        <TerminalResourceBar sessionId={sessionId} disconnected={isDisconnected} />
      ) : null}

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
            {/* 快捷键指示：macOS ⌘C；Windows 菜单指示用 Ctrl+Shift+C（Ctrl+C 有选区时
                也复制，但菜单只标主键位，对齐 Windows Terminal 惯例） */}
            <span className="ml-auto font-mono text-ui-xs text-foreground-subtlest">
              {isMacOS ? shortcutLabel('C') : shortcutLabel('C', { shift: true })}
            </span>
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={handlePaste}
            className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-menu-hover"
          >
            <ClipboardPaste className="size-3.5" />
            <span>{t('terminal.contextMenu.paste')}</span>
            {/* 快捷键指示：macOS ⌘V；Windows 菜单指示用 Ctrl+Shift+V（裸 Ctrl+V
                同样可粘贴，菜单只标主键位，对齐 Windows Terminal 惯例） */}
            <span className="ml-auto font-mono text-ui-xs text-foreground-subtlest">
              {isMacOS ? shortcutLabel('V') : shortcutLabel('V', { shift: true })}
            </span>
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
          <div className="my-1 h-px bg-border/60" />
          {recActive ? (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                closeContextMenu();
                stopRecording();
                requestFocus();
              }}
              className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-menu-hover"
            >
              <CircleStop className="size-3.5 text-red-500" />
              <span>{t('terminal.recording.stop')}</span>
            </button>
          ) : (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                closeContextMenu();
                startRecording();
                requestFocus();
              }}
              className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-menu-hover"
            >
              <Circle className="size-3.5" />
              <span>{t('terminal.recording.start')}</span>
            </button>
          )}
          {recHasLast ? (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                closeContextMenu();
                const rec = stopRecording();
                if (rec) setPlayerRecording(rec);
              }}
              className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-menu-hover"
            >
              <MonitorPlay className="size-3.5" />
              <span>{t('terminal.recording.replayLast')}</span>
            </button>
          ) : null}
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

      {/* 终端录制：胶囊（录制中红点计时 / 已停止作为操作面板锚点）+ 回放弹窗 */}
      {recActive || recStopped ? (
        <RecordingPill
          active={recActive}
          elapsedMs={recElapsedMs}
          onPreview={openRecordingPreview}
          onExportCast={handleExportCast}
          onExportLog={handleExportLog}
          onResume={() => undefined}
          onIdleClose={closeStoppedPill}
        />
      ) : null}
      {playerRecording ? (
        <CastPlayerModal
          recording={playerRecording}
          onClose={() => setPlayerRecording(null)}
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

      {/* 生产环境高危命令拦截确认浮窗 */}
      {safetyModal ? (
        <SafetyGuardModal
          isOpen={safetyModal.isOpen}
          command={safetyModal.command}
          inspectResult={safetyModal.inspectResult}
          hostName={currentHost?.name ?? 'Host'}
          envLevel={envLevel}
          onConfirm={handleSafetyConfirm}
          onCancel={handleSafetyCancel}
        />
      ) : null}
    </div>
  );
}
