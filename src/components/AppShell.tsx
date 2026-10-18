import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react';
import { listen } from '@tauri-apps/api/event';
import { DesktopTopOverlay } from '@/components/DesktopTopOverlay';
import { EditorScreen } from '@/components/editor/EditorScreen';
import { FilesScreen } from '@/components/files/FilesScreen';
import { GroupsScreen } from '@/components/hosts/GroupsScreen';
import { HostsScreen } from '@/components/hosts/HostsScreen';
import { ZkScreen } from '@/components/zk/ZkScreen';
import { SettingsScreen } from '@/components/SettingsScreen';
import { TerminalScreen } from '@/components/terminal';
import { WelcomeScreen } from '@/components/WelcomeScreen';
import { AiChatPanel } from '@/components/ai/AiChatPanel';
import { CommandPalette } from '@/components/commands/CommandPalette';
import { CommandsScreen } from '@/components/commands/CommandsScreen';
import { WorkspaceHeader } from '@/components/WorkspaceHeader';
import { WorkspaceSidebar, type Page } from '@/components/WorkspaceSidebar';
import { useAppPanels } from '@/hooks/useAppPanels';
import { useTranslation } from 'react-i18next';
import { hasPrimaryModifier } from '@/lib/keyboard';
import { isMacOS } from '@/lib/platform';
import { cn } from '@/lib/utils';
import { getDbKv, setDbKv } from '@/services/dbStorage';
import { useSftpStore } from '@/services/SftpProvider';
import { useEditorStore } from '@/editor/EditorProvider';
import { useTerminalStore } from '@/terminal/TerminalProvider';

/** 主面板圆角（ZCode workspaceShellWindowChrome 规则）：macOS Tahoe 26+ 为 12px。 */
const WORKSPACE_PANEL_RADIUS_PX = 12;

/** 侧边栏宽度：默认 264px，最小 180px，上限为容器 50%。 */
const SIDEBAR_DEFAULT_WIDTH_PX = 264;
const SIDEBAR_MIN_WIDTH_PX = 180;
const SIDEBAR_MAX_WIDTH_RATIO = 0.5;
const SIDEBAR_KEYBOARD_STEP_PX = 16;
const SIDEBAR_WIDTH_STORAGE_KEY = 'veloterm:workspace-shell:sidebar-width-px';
const SIDEBAR_PANEL_WIDTH_CSS_VAR = '--sidebar-panel-width';
const SIDEBAR_RESIZING_ATTR = 'data-workspace-sidebar-resizing';

/** AI 右侧面板宽度：默认 380px，最小 280px，上限为容器 45%。 */
const AI_PANEL_DEFAULT_WIDTH_PX = 380;
const AI_PANEL_MIN_WIDTH_PX = 280;
const AI_PANEL_MAX_RATIO = 0.45;
const AI_PANEL_WIDTH_CSS_VAR = '--ai-panel-width';

function clampAiPanelWidth(widthPx: number, containerWidthPx?: number): number {
  const maxWidthPx =
    containerWidthPx && containerWidthPx > 0
      ? Math.max(AI_PANEL_MIN_WIDTH_PX, containerWidthPx * AI_PANEL_MAX_RATIO)
      : Number.POSITIVE_INFINITY;
  return Math.round(Math.max(AI_PANEL_MIN_WIDTH_PX, Math.min(widthPx, maxWidthPx)));
}

/** 收起时保留 4px 面板留白，主面板获得同心内缩边距（与 ZCode 桌面 shell 一致）。 */
const SIDEBAR_COLLAPSED_WIDTH_PX = 4;

interface SidebarResizeSession {
  containerWidthPx: number;
  pointerId: number;
  startWidthPx: number;
  startX: number;
}

function clampSidebarWidth(widthPx: number, containerWidthPx?: number): number {
  const maxWidthPx =
    containerWidthPx && containerWidthPx > 0
      ? Math.max(SIDEBAR_MIN_WIDTH_PX, containerWidthPx * SIDEBAR_MAX_WIDTH_RATIO)
      : Number.POSITIVE_INFINITY;
  return Math.round(Math.max(SIDEBAR_MIN_WIDTH_PX, Math.min(widthPx, maxWidthPx)));
}

function readStoredSidebarWidth(): number {
  const raw = getDbKv(SIDEBAR_WIDTH_STORAGE_KEY);
  if (!raw) {
    return SIDEBAR_DEFAULT_WIDTH_PX;
  }
  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? clampSidebarWidth(parsed) : SIDEBAR_DEFAULT_WIDTH_PX;
}

function persistSidebarWidth(widthPx: number): void {
  setDbKv(SIDEBAR_WIDTH_STORAGE_KEY, String(widthPx));
}

/**
 * 应用外壳（布局移植自 ZCode WorkspaceShellLayout）：
 * 窗口底色 → 可拖宽/收起侧边栏 → 圆角主面板，左上角为红绿灯安全区 + 侧边栏开关浮层。
 * 侧边栏拖宽逻辑与 ZCode 一致：拖拽中直写 CSS 变量（不进 React 状态避免整树重渲），
 * 释放时才提交 state 并持久化；拖拽中通过 data 标记禁用宽度过渡。
 */
export function AppShell() {
  const { t } = useTranslation();
  // 编辑器会话（Provider 挂在 App 层，这里拿 openFile 供文件页入口调用）
  const { openFile: openEditorFile } = useEditorStore();

  // 后台预热 Monaco 异步模块块：首屏后立即排队，进入文件页（编辑意图信号）再确保一次。
  // dev 模式 Monaco 依赖图是几千个模块请求，越早开始拉取，首次点「编辑」等待越短。
  useEffect(() => {
    const timer = setTimeout(() => {
      void import('@/components/editor/MonacoDocumentCard');
    }, 300);
    return () => clearTimeout(timer);
  }, []);

  const { isSidebarVisible, toggleSidebar } = useAppPanels();
  const { createSession } = useTerminalStore();
  const { createSession: createSftpSession } = useSftpStore();
  const [page, setPage] = useState<Page>('home');
  const [isAiChatOpen, setAiChatOpen] = useState(false);
  const [isPaletteOpen, setPaletteOpen] = useState(false);
  const [aiInitialPrompt, setAiInitialPrompt] = useState<string | null>(null);

  // 「关于」独立窗口打开时淡化主窗（modal 视觉）：Rust 侧在关于窗创建/销毁时
  // 广播 about-visibility；遮罩阻断主窗交互，点击遮罩即关闭关于窗（与 Esc/
  // Enter/OK 同一销毁路径，对齐 ZCode 的 modal 关闭手感）
  const [isAboutOpen, setIsAboutOpen] = useState(false);
  useEffect(() => {
    const unlisten = listen<boolean>('about-visibility', ({ payload }) => {
      setIsAboutOpen(Boolean(payload));
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  const openAiChat = (prompt?: string) => {
    setAiInitialPrompt(prompt ?? null);
    setAiChatOpen(true);
  };

  // AI 问答是终端页专属能力：切到其他页面自动收起面板与入口
  useEffect(() => {
    if (page !== 'terminal' && isAiChatOpen) {
      setAiChatOpen(false);
    }
  }, [page]); // eslint-disable-line react-hooks/exhaustive-deps -- 仅依赖页面切换

  // 进入文件页 = 用户可能马上要编辑文件，无延迟再触发一次（与上面去重，模块只拉一次）
  const [hasWarmedOnFiles, setHasWarmedOnFiles] = useState(false);
  useEffect(() => {
    if (page === 'files' && !hasWarmedOnFiles) {
      setHasWarmedOnFiles(true);
      void import('@/components/editor/MonacoDocumentCard');
    }
  }, [page, hasWarmedOnFiles]);
  const [sidebarWidthPx, setSidebarWidthPx] = useState<number>(() => readStoredSidebarWidth());
  const [aiPanelWidthPx, setAiPanelWidthPx] = useState<number>(AI_PANEL_DEFAULT_WIDTH_PX);
  const aiPanelWidthPxRef = useRef(aiPanelWidthPx);
  const aiResizeSessionRef = useRef<SidebarResizeSession | null>(null);
  const [isAiResizing, setIsAiResizing] = useState(false);

  const shellRef = useRef<HTMLDivElement>(null);
  const sidebarPanelRef = useRef<HTMLDivElement>(null);
  const sidebarWidthPxRef = useRef(sidebarWidthPx);
  const resizeSessionRef = useRef<SidebarResizeSession | null>(null);
  const [isResizing, setIsResizing] = useState(false);

  useEffect(() => {
    sidebarWidthPxRef.current = sidebarWidthPx;
  }, [sidebarWidthPx]);

  useEffect(() => {
    aiPanelWidthPxRef.current = aiPanelWidthPx;
  }, [aiPanelWidthPx]);

  const handleOpenNewTerminal = useCallback(() => {
    createSession();
    setPage('terminal');
  }, [createSession]);

  // 主修饰键(⌘/Ctrl)+B 切换侧边栏、+N 新建/打开终端、+K 命令面板、+I AI 面板、+, 打开设置
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!hasPrimaryModifier(event) || event.altKey) {
        return;
      }
      const key = event.key.toLowerCase();
      if (key === 'b') {
        event.preventDefault();
        toggleSidebar();
      } else if (key === 'k') {
        event.preventDefault();
        setPaletteOpen((current) => !current);
      } else if (key === 'i') {
        event.preventDefault();
        setAiChatOpen((current) => !current);
      } else if (key === 'n') {
        event.preventDefault();
        handleOpenNewTerminal();
      } else if (key === ',') {
        event.preventDefault();
        setPage('settings');
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [handleOpenNewTerminal, toggleSidebar]);

  const applySidebarPanelWidthVar = useCallback(
    (widthPx: number, visible: boolean) => {
      shellRef.current?.style.setProperty(
        SIDEBAR_PANEL_WIDTH_CSS_VAR,
        `${visible ? widthPx : SIDEBAR_COLLAPSED_WIDTH_PX}px`,
      );
    },
    [],
  );

  // 可见性与宽度变化时同步 CSS 变量（拖拽路径绕过这里直写）
  useEffect(() => {
    applySidebarPanelWidthVar(sidebarWidthPx, isSidebarVisible);
  }, [applySidebarPanelWidthVar, sidebarWidthPx, isSidebarVisible]);

  const applyAiPanelWidthVar = useCallback((widthPx: number) => {
    shellRef.current?.style.setProperty(AI_PANEL_WIDTH_CSS_VAR, `${widthPx}px`);
  }, []);

  // AI 面板宽度同步 + 响应式：容器不足以同时容纳主内容最小宽与 AI 面板时自动收窄
  useEffect(() => {
    applyAiPanelWidthVar(isAiChatOpen ? aiPanelWidthPx : 0);
  }, [applyAiPanelWidthVar, aiPanelWidthPx, isAiChatOpen]);

  useEffect(() => {
    const onResize = () => {
      const containerWidthPx = shellRef.current?.getBoundingClientRect().width ?? 0;
      if (containerWidthPx <= 0) return;
      const available = containerWidthPx
        - (isSidebarVisible ? sidebarWidthPx : SIDEBAR_COLLAPSED_WIDTH_PX)
        - 320; // 主内容最小宽
      if (available < aiPanelWidthPxRef.current) {
        const clamped = clampAiPanelWidth(Math.max(AI_PANEL_MIN_WIDTH_PX, available), containerWidthPx);
        if (clamped !== aiPanelWidthPxRef.current) {
          aiPanelWidthPxRef.current = clamped;
          setAiPanelWidthPx(clamped);
        }
      }
    };
    onResize();
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, [isSidebarVisible, sidebarWidthPx]);

  const setResizeActive = useCallback((active: boolean) => {
    const elements = [shellRef.current, sidebarPanelRef.current];
    for (const element of elements) {
      if (!element) {
        continue;
      }
      if (active) {
        element.setAttribute(SIDEBAR_RESIZING_ATTR, 'true');
      } else {
        element.removeAttribute(SIDEBAR_RESIZING_ATTR);
      }
    }
    setIsResizing(active);
  }, []);

  const handleResizePointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (!isSidebarVisible || (event.pointerType === 'mouse' && event.button !== 0)) {
        return;
      }
      const containerWidthPx = shellRef.current?.getBoundingClientRect().width ?? 0;
      if (containerWidthPx <= 0) {
        return;
      }
      resizeSessionRef.current = {
        containerWidthPx,
        pointerId: event.pointerId,
        startWidthPx: sidebarWidthPxRef.current,
        startX: event.clientX,
      };
      setResizeActive(true);
      event.currentTarget.setPointerCapture(event.pointerId);
      event.preventDefault();
    },
    [isSidebarVisible, setResizeActive],
  );

  const handleResizePointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const session = resizeSessionRef.current;
      if (!session || session.pointerId !== event.pointerId) {
        return;
      }
      event.preventDefault();
      const nextWidthPx = clampSidebarWidth(
        session.startWidthPx + event.clientX - session.startX,
        session.containerWidthPx,
      );
      // 拖拽中的宽度只是瞬时布局值，直写 CSS 变量；释放时再提交 React state。
      sidebarWidthPxRef.current = nextWidthPx;
      applySidebarPanelWidthVar(nextWidthPx, true);
      event.currentTarget.setAttribute('aria-valuenow', String(nextWidthPx));
    },
    [applySidebarPanelWidthVar],
  );

  const finishResize = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const session = resizeSessionRef.current;
      if (!session || session.pointerId !== event.pointerId) {
        return;
      }
      resizeSessionRef.current = null;
      setResizeActive(false);
      const finalWidthPx = sidebarWidthPxRef.current;
      setSidebarWidthPx((current) => (current === finalWidthPx ? current : finalWidthPx));
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
      persistSidebarWidth(finalWidthPx);
    },
    [setResizeActive],
  );

  const handleResizeKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLDivElement>) => {
      if (!isSidebarVisible) {
        return;
      }
      const containerWidthPx = shellRef.current?.getBoundingClientRect().width ?? 0;
      const maxWidthPx = Math.max(SIDEBAR_MIN_WIDTH_PX, containerWidthPx * SIDEBAR_MAX_WIDTH_RATIO);
      let nextWidthPx: number | null = null;
      if (event.key === 'ArrowLeft') {
        nextWidthPx = sidebarWidthPxRef.current - SIDEBAR_KEYBOARD_STEP_PX;
      } else if (event.key === 'ArrowRight') {
        nextWidthPx = sidebarWidthPxRef.current + SIDEBAR_KEYBOARD_STEP_PX;
      } else if (event.key === 'Home') {
        nextWidthPx = SIDEBAR_MIN_WIDTH_PX;
      } else if (event.key === 'End') {
        nextWidthPx = maxWidthPx;
      }
      if (nextWidthPx === null) {
        return;
      }
      event.preventDefault();
      const clamped = clampSidebarWidth(nextWidthPx, containerWidthPx);
      sidebarWidthPxRef.current = clamped;
      setSidebarWidthPx(clamped);
      persistSidebarWidth(clamped);
      event.currentTarget.setAttribute('aria-valuenow', String(clamped));
    },
    [isSidebarVisible],
  );

  // ── AI 右侧面板拖宽/窄（镜像左侧栏逻辑，方向相反）──
  const handleAiResizePointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (!isAiChatOpen || (event.pointerType === 'mouse' && event.button !== 0)) {
        return;
      }
      const containerWidthPx = shellRef.current?.getBoundingClientRect().width ?? 0;
      if (containerWidthPx <= 0) {
        return;
      }
      aiResizeSessionRef.current = {
        containerWidthPx,
        pointerId: event.pointerId,
        startWidthPx: aiPanelWidthPxRef.current,
        startX: event.clientX,
      };
      setIsAiResizing(true);
      event.currentTarget.setPointerCapture(event.pointerId);
      event.preventDefault();
    },
    [isAiChatOpen],
  );

  const handleAiResizePointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const session = aiResizeSessionRef.current;
      if (!session || session.pointerId !== event.pointerId) {
        return;
      }
      event.preventDefault();
      // 向左拖（clientX 减小）变宽
      const nextWidthPx = clampAiPanelWidth(
        session.startWidthPx + session.startX - event.clientX,
        session.containerWidthPx,
      );
      aiPanelWidthPxRef.current = nextWidthPx;
      applyAiPanelWidthVar(nextWidthPx);
      event.currentTarget.setAttribute('aria-valuenow', String(nextWidthPx));
    },
    [applyAiPanelWidthVar],
  );

  const finishAiResize = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const session = aiResizeSessionRef.current;
      if (!session || session.pointerId !== event.pointerId) {
        return;
      }
      aiResizeSessionRef.current = null;
      setIsAiResizing(false);
      const finalWidthPx = aiPanelWidthPxRef.current;
      setAiPanelWidthPx((current) => (current === finalWidthPx ? current : finalWidthPx));
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
    },
    [],
  );

  // 窗口底座（ZCode DesktopWindowFrame 同款）：mac 走半透明 alt 叠 vibrancy，
  // 无 vibrancy 的平台用不透明 win-alt 兜底，避免半透明混出异常灰块。
  return (
    <div
        className={cn(
          'flex h-dvh flex-col overflow-hidden',
          isMacOS ? 'bg-background-alt' : 'bg-background-win-alt',
        )}
      >
        <div
          ref={shellRef}
          style={
            {
              '--sidebar-panel-width': `${isSidebarVisible ? sidebarWidthPx : SIDEBAR_COLLAPSED_WIDTH_PX}px`,
              '--ai-panel-width': `${isAiChatOpen && page === 'terminal' ? aiPanelWidthPx : 0}px`,
              '--workspace-panel-radius': `${WORKSPACE_PANEL_RADIUS_PX}px`,
            } as CSSProperties
          }
          className="relative flex h-full w-full overflow-hidden"
        >
          <div
            ref={sidebarPanelRef}
            className={cn(
              'w-(--sidebar-panel-width) max-w-[50%] flex-none overflow-hidden transition-[width,opacity] duration-200 ease-out',
              !isSidebarVisible && 'pointer-events-none opacity-0',
              // 拖拽中禁用宽度过渡，跟随指针不滞后
              isResizing && 'transition-opacity',
            )}
          >
            <WorkspaceSidebar
              activePage={page}
              onPageChange={setPage}
              className="w-(--sidebar-panel-width)"
            />
          </div>

          {isSidebarVisible ? (
            <div
              role="separator"
              aria-orientation="vertical"
              aria-label={t('sidebar.resize')}
              aria-valuemin={SIDEBAR_MIN_WIDTH_PX}
              aria-valuemax={Math.round(window.innerWidth * SIDEBAR_MAX_WIDTH_RATIO)}
              aria-valuenow={sidebarWidthPx}
              tabIndex={0}
              onPointerDown={handleResizePointerDown}
              onPointerMove={handleResizePointerMove}
              onPointerUp={finishResize}
              onPointerCancel={finishResize}
              onKeyDown={handleResizeKeyDown}
              className={cn(
                'group/handle relative z-10 flex h-full w-1 shrink-0 cursor-ew-resize touch-none items-center justify-center bg-transparent outline-none focus:outline-none focus-visible:ring-0',
                // 悬停/聚焦/拖拽时显示 2px 圆角指示线，上下按面板圆角内缩
                "after:pointer-events-none after:absolute after:w-0.5 after:rounded-full after:bg-foreground-subtlest/50 after:opacity-0 after:transition-opacity after:content-[''] after:inset-y-[var(--workspace-panel-radius)]",
                'hover:after:opacity-100 focus-visible:after:opacity-100',
                isResizing && 'after:opacity-100',
              )}
            />
          ) : null}

          {/* 内容列内缩：右 4px（ZCode 一致）、左/上/下 0——底部原与 ZCode 一致留
              4px，但终端资源监控条贴面板最底，这 4px 使整条相对窗口底部视觉抬高
              （用户实测：pb=0 即居中），故底部清零让面板贴住窗口底缘 */}
          <div className="flex min-w-[320px] flex-1 flex-col pr-1">
            <div className="h-1 w-full shrink-0" data-tauri-drag-region />
            <div
              className="flex min-h-0 flex-1"
              data-ai-panel-resizing={isAiResizing ? 'true' : undefined}
            >
            <section className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-(--workspace-panel-radius) border border-border bg-background">
              <WorkspaceHeader
                page={page}
                isSidebarVisible={isSidebarVisible}
                onOpenAiChat={page === 'terminal' ? () => openAiChat() : undefined}
                isAiPanelOpen={isAiChatOpen}
                onToggleAiPanel={page === 'terminal' ? () => setAiChatOpen((current) => !current) : undefined}
              />
              <main className="min-h-0 flex-1 overflow-y-auto relative">
                {/* SFTP 文件视图持久挂载，切换页面保持后台状态、文件路径与传输任务 */}
                <div className={cn('h-full min-h-0 w-full', page !== 'files' && 'hidden')}>
                  <FilesScreen
                    isVisible={page === 'files'}
                    onNavigateHosts={() => setPage('hosts')}
                    onOpenEditor={(file) => {
                      openEditorFile(file);
                      setPage('editor');
                    }}
                  />
                </div>
                {page === 'home' ? (
                  <WelcomeScreen
                    onNewTerminal={handleOpenNewTerminal}
                    onOpenSettings={() => setPage('settings')}
                  />
                ) : page === 'terminal' ? (
                  <TerminalScreen
                    isVisible={page === 'terminal'}
                    isPanelResizing={isResizing}
                    onNavigateHosts={() => setPage('hosts')}
                    onOpenAiChat={openAiChat}
                    onOpenCommandPalette={() => setPaletteOpen(true)}
                  />
                ) : page === 'editor' ? (
                  <EditorScreen />
                ) : page === 'files' ? null : page === 'hosts' ? (
                  <HostsScreen
                    onConnectHost={(host) => {
                      createSession({ hostId: host.id, customTitle: host.name });
                      setPage('terminal');
                    }}
                    onOpenSftp={(host) => {
                      createSftpSession({ hostId: host.id, hostName: host.name });
                      setPage('files');
                    }}
                  />
                ) : page === 'groups' ? (
                  <GroupsScreen />
                ) : page === 'commands' ? (
                  <CommandsScreen />
                ) : page === 'zk' ? (
                  <ZkScreen />
                ) : (
                  <SettingsScreen />
                )}
              </main>
            </section>

            {/* AI 右侧面板拖拽条（镜像左侧栏，方向相反）；门控与面板生效态
                一致（仅终端页且开启时）——否则非终端页会残留 4px 隐形拖拽条 */}
            {isAiChatOpen && page === 'terminal' ? (
              <div
                role="separator"
                aria-orientation="vertical"
                aria-label={t('ai.panel.resize')}
                aria-valuemin={AI_PANEL_MIN_WIDTH_PX}
                aria-valuemax={Math.round(window.innerWidth * AI_PANEL_MAX_RATIO)}
                aria-valuenow={aiPanelWidthPx}
                tabIndex={0}
                onPointerDown={handleAiResizePointerDown}
                onPointerMove={handleAiResizePointerMove}
                onPointerUp={finishAiResize}
                onPointerCancel={finishAiResize}
                className={cn(
                  'group/ai-handle relative z-10 flex h-full w-1 shrink-0 cursor-ew-resize touch-none items-center justify-center bg-transparent outline-none focus:outline-none focus-visible:ring-0',
                  "after:pointer-events-none after:absolute after:w-0.5 after:rounded-full after:bg-foreground-subtlest/50 after:opacity-0 after:transition-opacity after:content-[''] after:inset-y-[var(--workspace-panel-radius)]",
                  'hover:after:opacity-100 focus-visible:after:opacity-100',
                  isAiResizing && 'after:opacity-100',
                )}
              />
            ) : null}

            {/* AI 右侧面板（ZCode 布局范式：常驻侧栏、跨页面保活、宽度可拖）；
                左缘 4px 间隙由 ml-1 提供（ZCode 用内容列 p-1 的右留白） */}
            <AiChatPanel
              open={isAiChatOpen && page === 'terminal'}
              width={aiPanelWidthPx}
              isResizing={isAiResizing}
              isTerminalPage={page === 'terminal'}
              onClose={() => setAiChatOpen(false)}
              initialPrompt={aiInitialPrompt}
              onOpenSettings={() => {
                setAiChatOpen(false);
                setPage('settings');
              }}
            />
            </div>
          </div>

          <DesktopTopOverlay isSidebarVisible={isSidebarVisible} onToggleSidebar={toggleSidebar} />

          <CommandPalette open={isPaletteOpen} onClose={() => setPaletteOpen(false)} />

          {/* 「关于」窗口打开时主窗遮罩：淡化下层 + 阻断交互。点击遮罩不关闭——
              对话框只能由 OK 按钮（或 Esc/Enter 键盘）收尾。常驻挂载、以透明度
              过渡开合，时长与关于卡片动效对齐（入场 320ms），开关事件由卡片
              动画首帧同帧广播——两窗动画同步开始不割裂 */}
          <div
            aria-hidden="true"
            className={cn(
              'fixed inset-0 z-[90] cursor-default bg-black/45 transition-opacity duration-[320ms]',
              isAboutOpen ? 'opacity-100' : 'pointer-events-none opacity-0',
            )}
          />
        </div>
      </div>
  );
}
