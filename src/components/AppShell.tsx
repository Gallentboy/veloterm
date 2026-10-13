import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react';
import { DesktopTopOverlay } from '@/components/DesktopTopOverlay';
import { FilesScreen } from '@/components/files/FilesScreen';
import { GroupsScreen } from '@/components/hosts/GroupsScreen';
import { HostsScreen } from '@/components/hosts/HostsScreen';
import { SettingsScreen } from '@/components/SettingsScreen';
import { TerminalScreen } from '@/components/terminal';
import { WelcomeScreen } from '@/components/WelcomeScreen';
import { WorkspaceHeader } from '@/components/WorkspaceHeader';
import { WorkspaceSidebar, type Page } from '@/components/WorkspaceSidebar';
import { useAppPanels } from '@/hooks/useAppPanels';
import { useTranslation } from 'react-i18next';
import { isMacOS } from '@/lib/platform';
import { cn } from '@/lib/utils';
import { getDbKv, setDbKv } from '@/services/dbStorage';
import { useSftpStore } from '@/services/SftpProvider';
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
  const { isSidebarVisible, toggleSidebar } = useAppPanels();
  const { createSession } = useTerminalStore();
  const { createSession: createSftpSession } = useSftpStore();
  const [page, setPage] = useState<Page>('home');
  const [sidebarWidthPx, setSidebarWidthPx] = useState<number>(() => readStoredSidebarWidth());

  const shellRef = useRef<HTMLDivElement>(null);
  const sidebarPanelRef = useRef<HTMLDivElement>(null);
  const sidebarWidthPxRef = useRef(sidebarWidthPx);
  const resizeSessionRef = useRef<SidebarResizeSession | null>(null);
  const [isResizing, setIsResizing] = useState(false);

  useEffect(() => {
    sidebarWidthPxRef.current = sidebarWidthPx;
  }, [sidebarWidthPx]);

  const handleOpenNewTerminal = useCallback(() => {
    createSession();
    setPage('terminal');
  }, [createSession]);

  // ⌘B 切换侧边栏、⌘N 新建/打开终端、⌘, 打开设置
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey) {
        return;
      }
      const key = event.key.toLowerCase();
      if (key === 'b') {
        event.preventDefault();
        toggleSidebar();
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

        {/* 内容列内缩与 ZCode 一致：右/下 4px、左 0，顶部经 4px 拖拽条让面板上缘留出等宽间隙 */}
        <div className="flex min-w-[320px] flex-1 flex-col p-1 pl-0 pt-0">
          <div className="h-1 w-full shrink-0" data-tauri-drag-region />
          <section className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-(--workspace-panel-radius) border border-border bg-background">
            <WorkspaceHeader page={page} isSidebarVisible={isSidebarVisible} />
            <main className="min-h-0 flex-1 overflow-y-auto relative">
              {/* SFTP 文件视图持久挂载，切换页面保持后台状态、文件路径与传输任务 */}
              <div className={cn('h-full min-h-0 w-full', page !== 'files' && 'hidden')}>
                <FilesScreen
                  isVisible={page === 'files'}
                  onNavigateHosts={() => setPage('hosts')}
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
                />
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
              ) : (
                <SettingsScreen />
              )}
            </main>
          </section>
        </div>

        <DesktopTopOverlay isSidebarVisible={isSidebarVisible} onToggleSidebar={toggleSidebar} />
      </div>
    </div>
  );
}
