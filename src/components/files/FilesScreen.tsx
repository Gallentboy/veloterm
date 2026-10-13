import { Columns2, FolderSync, Plus, Server } from 'lucide-react';
import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react';
import { useTranslation } from 'react-i18next';
import { SftpSessionView } from '@/components/files/SftpSessionView';
import { SftpTabTrigger } from '@/components/files/SftpTabTrigger';
import { Button } from '@/components/ui/button';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { EmptyState } from '@/components/ui/EmptyState';
import { useHostsStore } from '@/hosts/HostsProvider';
import { cn } from '@/lib/utils';
import {
  FILES_PANE_MIN_WIDTH_PX,
  persistFilesPaneState,
  persistLeftPaneWidth,
  readStoredFilesPaneState,
  readStoredLeftPaneWidth,
  type FilesPaneSide,
  type FilesPaneState,
} from '@/services/filesPaneState';
import { useSftpStore } from '@/services/SftpProvider';

/** 中缝分隔条列宽（4px 命中区，与侧边栏拖宽条一致）。 */
const FILES_PANE_DIVIDER_WIDTH_CSS = '0.25rem';
/** 键盘微调左栏宽度的步长（对齐 AppShell 侧边栏 SIDEBAR_KEYBOARD_STEP_PX）。 */
const FILES_PANE_KEYBOARD_STEP_PX = 16;

/** 中缝拖拽会话（对齐 AppShell SidebarResizeSession：拖拽中不进 React 状态）。 */
interface PaneResizeSession {
  containerWidthPx: number;
  pointerId: number;
  startWidthPx: number;
  startX: number;
}

/** 双侧都保留 FILES_PANE_MIN_WIDTH_PX 的前提下，夹取左栏宽度。 */
function clampLeftPaneWidth(widthPx: number, containerWidthPx?: number): number {
  const maxWidthPx =
    containerWidthPx && containerWidthPx > 0
      ? Math.max(FILES_PANE_MIN_WIDTH_PX, containerWidthPx - FILES_PANE_MIN_WIDTH_PX)
      : Number.POSITIVE_INFINITY;
  return Math.round(Math.max(FILES_PANE_MIN_WIDTH_PX, Math.min(widthPx, maxWidthPx)));
}

/** 把会话分配到指定窗格槽位（显式分支，避免联合类型动态 key 破坏类型收窄）。 */
function assignSessionToPane(
  state: FilesPaneState,
  side: FilesPaneSide,
  sessionId: number,
): FilesPaneState {
  return side === 'left'
    ? { ...state, leftSessionId: sessionId }
    : { ...state, rightSessionId: sessionId };
}

export interface FilesScreenProps {
  isVisible?: boolean;
  onNavigateHosts?: () => void;
  /** 打开可编辑文件到编辑器页（由 AppShell 注入：openFile + 切页） */
  onOpenEditor?: (file: { hostId: number; remotePath: string }) => void;
}

/**
 * 文件页（双栏 Dual-Pane 布局，聚焦窗格 + Tab 池模型）：
 *
 * - 顶部 Tab 栏管理全部 SFTP 会话（对齐 ZCode Terminal 顶栏规范）；
 * - 主区为左右两个窗格 + 可拖拽中缝：点击池中 Tab 会把它「换入」当前聚焦的窗格，
 *   原窗格会话退回隐藏 Keep-alive 池，目录状态不丢失；
 * - 全部会话以稳定 key 渲染在同一个 Grid 容器内，窗格分配只是切换 gridColumn，
 *   因此换入/换出窗格不会卸载重挂 SftpSessionView；
 * - 中缝拖宽逻辑与 AppShell 侧边栏一致：拖拽中直写容器 grid 模板（不进 React
 *   状态避免整树重渲），释放时才提交 state 并持久化；
 * - 窗格分配/聚焦侧与左栏宽度均持久化，重启后恢复双栏布局。
 */
export function FilesScreen({ isVisible = true, onNavigateHosts, onOpenEditor }: FilesScreenProps) {
  const { t } = useTranslation();
  const selectId = useId();
  const { hosts } = useHostsStore();
  const {
    sessions,
    activeSession,
    createSession,
    closeSession,
    activateSession,
    updateSessionPath,
  } = useSftpStore();

  // 双栏窗格分配与聚焦侧（Tab 池模型：激活的会话落在聚焦窗格）
  const [pane, setPane] = useState<FilesPaneState>(() => readStoredFilesPaneState());
  // 左栏宽度：null = 用户未自定义（默认左右各半的弹性布局）
  const [leftPaneWidthPx, setLeftPaneWidthPx] = useState<number | null>(() =>
    readStoredLeftPaneWidth(),
  );
  const [isDividerActive, setIsDividerActive] = useState(false);
  // 主区实际宽度（ResizeObserver 跟踪）：用于分隔条 aria 上限与容器变窄时的防御性收缩
  const [mainAreaWidthPx, setMainAreaWidthPx] = useState(0);

  const mainAreaRef = useRef<HTMLDivElement>(null);
  // 左栏宽度探针：占据第 1 列的 0 高度元素，用于读取当前实际列宽（拖拽起点/键盘步进）
  const leftColumnProbeRef = useRef<HTMLDivElement>(null);
  const leftPaneWidthPxRef = useRef(0);
  const dividerSessionRef = useRef<PaneResizeSession | null>(null);

  const hasSessions = sessions.length > 0;

  // 窗格分配持久化（含聚焦侧，重启恢复双栏布局）
  useEffect(() => {
    persistFilesPaneState(pane);
  }, [pane]);

  // 聚焦某个窗格：仅切换焦点侧，不抢活（会话激活仍由 Tab 点击/快捷键驱动）
  const focusPane = useCallback((side: FilesPaneSide) => {
    setPane((current) =>
      current.focusedPane === side ? current : { ...current, focusedPane: side },
    );
  }, []);

  // Tab 点击/会话激活的统一入口：
  // 已在某窗格 → 聚焦该窗格；仍在池中 → 换入当前聚焦窗格（原会话退回池中）
  const handleTabActivate = useCallback(
    (sessionId: number) => {
      setPane((current) => {
        if (current.leftSessionId === sessionId) {
          return current.focusedPane === 'left' ? current : { ...current, focusedPane: 'left' };
        }
        if (current.rightSessionId === sessionId) {
          return current.focusedPane === 'right' ? current : { ...current, focusedPane: 'right' };
        }
        return assignSessionToPane(current, current.focusedPane, sessionId);
      });
      activateSession(sessionId);
    },
    [activateSession],
  );

  // 快捷键支持：⌘W 关闭当前 Tab、⌘⇧[ / ⌘⇧] 切换前后 Tab（切换会进入聚焦窗格）
  useEffect(() => {
    if (!isVisible) return;

    const onKeyDown = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;
      if (!mod) return;
      const key = event.key.toLowerCase();

      if (!event.altKey && !event.shiftKey && key === 'w') {
        if (activeSession) {
          event.preventDefault();
          closeSession(activeSession.id);
        }
        return;
      }

      if (event.shiftKey && (event.key === '[' || event.key === '{')) {
        event.preventDefault();
        if (sessions.length > 1 && activeSession) {
          const idx = sessions.findIndex((s) => s.id === activeSession.id);
          const prevIdx = (idx - 1 + sessions.length) % sessions.length;
          const target = sessions[prevIdx];
          if (target) handleTabActivate(target.id);
        }
        return;
      }

      if (event.shiftKey && (event.key === ']' || event.key === '}')) {
        event.preventDefault();
        if (sessions.length > 1 && activeSession) {
          const idx = sessions.findIndex((s) => s.id === activeSession.id);
          const nextIdx = (idx + 1) % sessions.length;
          const target = sessions[nextIdx];
          if (target) handleTabActivate(target.id);
        }
      }
    };

    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [activeSession, closeSession, handleTabActivate, isVisible, sessions]);

  // 窗格槽位清理：会话被关闭后置空对应窗格；同一会话不允许同时占据两个窗格
  // （后者是异常持久化数据的兜底：保留聚焦侧，另一侧置空）
  useEffect(() => {
    setPane((current) => {
      let next = current;
      if (
        next.leftSessionId != null &&
        !sessions.some((s) => s.id === next.leftSessionId)
      ) {
        next = { ...next, leftSessionId: null };
      }
      if (
        next.rightSessionId != null &&
        !sessions.some((s) => s.id === next.rightSessionId)
      ) {
        next = { ...next, rightSessionId: null };
      }
      if (next.leftSessionId != null && next.leftSessionId === next.rightSessionId) {
        next =
          next.focusedPane === 'left'
            ? { ...next, rightSessionId: null }
            : { ...next, leftSessionId: null };
      }
      return next === current ? current : next;
    });
  }, [sessions]);

  // 焦点跟随激活会话：激活会话位于另一侧窗格时，把聚焦切过去（Tab 高亮跟随焦点，
  // 覆盖关闭 Tab 后 store 就近激活相邻会话等未经过 handleTabActivate 的路径）
  useEffect(() => {
    const activeId = activeSession?.id;
    if (activeId == null) return;
    setPane((current) => {
      if (current.focusedPane === 'left' && current.rightSessionId === activeId) {
        return { ...current, focusedPane: 'right' };
      }
      if (current.focusedPane === 'right' && current.leftSessionId === activeId) {
        return { ...current, focusedPane: 'left' };
      }
      return current;
    });
  }, [activeSession?.id]);

  // 新会话/外部激活兜底：激活会话不在任何窗格时，落入当前聚焦窗格
  // （覆盖新建会话自动激活、主机页直接 createSftpSession 等路径；幂等）
  useEffect(() => {
    const activeId = activeSession?.id;
    if (activeId == null) return;
    setPane((current) => {
      if (current.leftSessionId === activeId || current.rightSessionId === activeId) {
        return current;
      }
      return assignSessionToPane(current, current.focusedPane, activeId);
    });
  }, [activeSession?.id]);

  // 主区尺寸跟踪：供分隔条 aria 数值与容器变窄时对已存左栏宽度做防御性收缩
  // （只收不存：临时收窄窗口不覆盖用户持久化的宽度偏好）
  useEffect(() => {
    if (!hasSessions) return;
    const element = mainAreaRef.current;
    if (!element) return;
    const observer = new ResizeObserver((entries) => {
      const widthPx = Math.round(entries[0]?.contentRect.width ?? 0);
      setMainAreaWidthPx((current) => (current === widthPx ? current : widthPx));
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [hasSessions]);

  useEffect(() => {
    if (mainAreaWidthPx <= 0 || leftPaneWidthPx == null) return;
    const clamped = clampLeftPaneWidth(leftPaneWidthPx, mainAreaWidthPx);
    if (clamped !== leftPaneWidthPx) {
      setLeftPaneWidthPx(clamped);
    }
  }, [leftPaneWidthPx, mainAreaWidthPx]);

  // 中缝拖拽（对齐 AppShell 侧边栏拖宽）：pointer capture + 拖拽中直写 DOM
  const handleDividerPointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.pointerType === 'mouse' && event.button !== 0) {
        return;
      }
      const containerWidthPx = mainAreaRef.current?.getBoundingClientRect().width ?? 0;
      if (containerWidthPx <= 0) {
        return;
      }
      // 记录拖拽起始实测列宽（兼防「按下即释放」时提交出 0 宽度）
      const startWidthPx = leftColumnProbeRef.current?.getBoundingClientRect().width ?? 0;
      leftPaneWidthPxRef.current = startWidthPx;
      dividerSessionRef.current = {
        containerWidthPx,
        pointerId: event.pointerId,
        startWidthPx,
        startX: event.clientX,
      };
      setIsDividerActive(true);
      event.currentTarget.setPointerCapture(event.pointerId);
      event.preventDefault();
    },
    [],
  );

  const handleDividerPointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const session = dividerSessionRef.current;
      if (!session || session.pointerId !== event.pointerId) {
        return;
      }
      event.preventDefault();
      const nextWidthPx = clampLeftPaneWidth(
        session.startWidthPx + event.clientX - session.startX,
        session.containerWidthPx,
      );
      // 拖拽中的宽度只是瞬时布局值，直写容器 grid 模板；释放时再提交 React state。
      leftPaneWidthPxRef.current = nextWidthPx;
      if (mainAreaRef.current) {
        mainAreaRef.current.style.gridTemplateColumns = `${nextWidthPx}px ${FILES_PANE_DIVIDER_WIDTH_CSS} minmax(${FILES_PANE_MIN_WIDTH_PX}px, 1fr)`;
      }
      event.currentTarget.setAttribute('aria-valuenow', String(nextWidthPx));
    },
    [],
  );

  const finishDividerDrag = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const session = dividerSessionRef.current;
      if (!session || session.pointerId !== event.pointerId) {
        return;
      }
      dividerSessionRef.current = null;
      setIsDividerActive(false);
      const finalWidthPx = leftPaneWidthPxRef.current;
      setLeftPaneWidthPx((current) => (current === finalWidthPx ? current : finalWidthPx));
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
      persistLeftPaneWidth(finalWidthPx);
    },
    [],
  );

  // 键盘微调：←/→ ±16px，Home/End 到最小/最大（当前宽度从探针实测，兼容未自定义的弹性布局）
  const handleDividerKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLDivElement>) => {
      const containerWidthPx = mainAreaRef.current?.getBoundingClientRect().width ?? 0;
      const currentWidthPx = leftColumnProbeRef.current?.getBoundingClientRect().width ?? 0;
      if (currentWidthPx <= 0) {
        return;
      }
      let nextWidthPx: number | null = null;
      if (event.key === 'ArrowLeft') {
        nextWidthPx = currentWidthPx - FILES_PANE_KEYBOARD_STEP_PX;
      } else if (event.key === 'ArrowRight') {
        nextWidthPx = currentWidthPx + FILES_PANE_KEYBOARD_STEP_PX;
      } else if (event.key === 'Home') {
        nextWidthPx = FILES_PANE_MIN_WIDTH_PX;
      } else if (event.key === 'End') {
        nextWidthPx = containerWidthPx - FILES_PANE_MIN_WIDTH_PX;
      }
      if (nextWidthPx === null) {
        return;
      }
      event.preventDefault();
      const clamped = clampLeftPaneWidth(nextWidthPx, containerWidthPx);
      leftPaneWidthPxRef.current = clamped;
      setLeftPaneWidthPx(clamped);
      persistLeftPaneWidth(clamped);
    },
    [],
  );

  // 新建指定主机的 SFTP Tab（store 自动激活，由上面的激活兜底 effect 分配到聚焦窗格）
  const handleOpenHostSession = (hostId: number) => {
    const host = hosts.find((h) => h.id === hostId);
    if (!host) return;
    createSession({
      hostId: host.id,
      hostName: host.name,
      initialPath: '/',
    });
  };

  // 各窗格当前绑定的会话（找不到 = 空窗格）
  const leftSession =
    pane.leftSessionId == null
      ? undefined
      : sessions.find((s) => s.id === pane.leftSessionId);
  const rightSession =
    pane.rightSessionId == null
      ? undefined
      : sessions.find((s) => s.id === pane.rightSessionId);

  // 空窗格占位元格（轻量内联空态，聚焦态同样高亮）
  const renderEmptyPaneCell = (side: FilesPaneSide) => {
    const isPaneFocused = pane.focusedPane === side;
    return (
      <div
        key={`pane-empty-${side}`}
        data-pane-empty={side}
        onPointerDown={() => focusPane(side)}
        style={{ gridColumn: side === 'left' ? '1' : '3', gridRow: '1' }}
        className={cn(
          'flex min-h-0 min-w-0 flex-col items-center justify-center overflow-hidden rounded-lg p-6 text-center select-none',
          isPaneFocused && 'ring-1 ring-primary/40',
        )}
      >
        <div className="flex size-12 items-center justify-center rounded-2xl border border-border/60 bg-surface/50 shadow-xs">
          <Columns2 className="size-6 text-foreground-subtle" strokeWidth={1.5} />
        </div>
        <h3 className="mt-3.5 text-sm font-medium tracking-tight text-foreground">
          {t('files.pane.emptyTitle')}
        </h3>
        <p className="mt-1.5 max-w-xs text-ui-xs text-foreground-subtle leading-relaxed">
          {t('files.pane.emptyDesc')}
        </p>
        <div className="mt-5">
          <Button
            type="button"
            variant="outline"
            disabled={!onNavigateHosts}
            onClick={() => {
              focusPane(side);
              onNavigateHosts?.();
            }}
            className="h-8 gap-1.5 rounded-lg border-border/80 bg-surface/40 px-3 text-ui-sm font-medium text-foreground shadow-2xs transition-all hover:bg-hover active:bg-hover/80"
          >
            <Server className="size-4 shrink-0 text-foreground-subtle" />
            <span>{t('files.pane.emptyAction')}</span>
          </Button>
        </div>
      </div>
    );
  };

  // 主区 Grid 模板：左列（定宽或弹性）+ 4px 中缝 + 右列弹性（双侧保底最小宽度）
  const gridTemplateColumns =
    leftPaneWidthPx == null
      ? `minmax(${FILES_PANE_MIN_WIDTH_PX}px, 1fr) ${FILES_PANE_DIVIDER_WIDTH_CSS} minmax(${FILES_PANE_MIN_WIDTH_PX}px, 1fr)`
      : `${leftPaneWidthPx}px ${FILES_PANE_DIVIDER_WIDTH_CSS} minmax(${FILES_PANE_MIN_WIDTH_PX}px, 1fr)`;

  // 分隔条 aria 数值（无自定义宽度时用主区实测宽度的一半作当前值）
  const maxLeftPaneWidthPx =
    mainAreaWidthPx > 0
      ? Math.max(FILES_PANE_MIN_WIDTH_PX, Math.round(mainAreaWidthPx - FILES_PANE_MIN_WIDTH_PX))
      : undefined;
  const ariaLeftPaneWidthPx =
    leftPaneWidthPx ?? (mainAreaWidthPx > 0 ? Math.round(mainAreaWidthPx / 2) : undefined);

  return (
    <section
      data-testid="files-screen"
      className="flex h-full min-h-0 flex-col overflow-hidden bg-background"
    >
      {/* 顶部多 Tab 栏（完全对齐 ZCode Terminal 顶栏规范） */}
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/50 px-3 py-1.5">
        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto overflow-y-hidden [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
          {sessions.length > 0 ? (
            <div
              role="tablist"
              aria-label={t('files.title')}
              className="flex h-7 w-max items-center gap-1"
            >
              {sessions.map((session) => (
                <SftpTabTrigger
                  key={session.id}
                  session={session}
                  title={session.hostName}
                  closeLabel={t('files.closeTab', { title: session.hostName })}
                  isActive={session.id === activeSession?.id}
                  onActivate={handleTabActivate}
                  onClose={closeSession}
                />
              ))}
            </div>
          ) : (
            <span className="text-ui-xs text-foreground-subtle select-none pl-1">
              {t('files.title')}
            </span>
          )}

          {/* 新建 SFTP Tab 按钮（带主机选择下拉） */}
          {hosts.length > 0 && (
            <div className="relative inline-flex items-center shrink-0">
              <ControlHintTooltip title={t('files.newTab')} side="bottom">
                <div className="relative flex items-center justify-center">
                  <Button
                    type="button"
                    size="icon-xs"
                    variant="ghost"
                    className="h-7 w-7 rounded-lg text-foreground-subtle hover:text-foreground hover:bg-hover active:bg-hover/80"
                    aria-label={t('files.newTab')}
                  >
                    <Plus className="size-3.5" />
                  </Button>
                  <select
                    id={selectId}
                    aria-label={t('files.newTab')}
                    value=""
                    onChange={(e) => {
                      const hostId = Number(e.target.value);
                      if (hostId) handleOpenHostSession(hostId);
                    }}
                    className="absolute inset-0 w-full h-full opacity-0 cursor-pointer text-xs"
                  >
                    <option value="" disabled>
                      {t('files.newTab')}
                    </option>
                    {hosts.map((host) => (
                      <option key={host.id} value={host.id}>
                        {host.name} ({host.username}@{host.host})
                      </option>
                    ))}
                  </select>
                </div>
              </ControlHintTooltip>
            </div>
          )}
        </div>

        {/* 右侧主机标识与状态指示 */}
        <div className="flex shrink-0 items-center gap-2">
          {activeSession ? (
            <span className="rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle select-none">
              {activeSession.hostName}
            </span>
          ) : null}
        </div>
      </div>

      {/* 主区：无会话时显示引导空态，有会话时进入双栏布局 */}
      {sessions.length === 0 ? (
        <EmptyState
          icon={FolderSync}
          title={t('files.emptyTitle')}
          description={t('files.emptyDesc')}
          primaryAction={
            hosts.length > 0
              ? {
                  label: t('files.openSftp'),
                  icon: Plus,
                  onClick: () => handleOpenHostSession(hosts[0]!.id),
                }
              : undefined
          }
          secondaryAction={
            onNavigateHosts
              ? {
                  label: hosts.length > 0 ? t('terminal.connectHost') : t('files.goToHosts'),
                  icon: Server,
                  variant: hosts.length > 0 ? 'outline' : 'default',
                  onClick: onNavigateHosts,
                }
              : undefined
          }
        />
      ) : (
        <div
          ref={mainAreaRef}
          className="grid min-h-0 flex-1 overflow-hidden"
          style={{ gridTemplateColumns, gridTemplateRows: 'minmax(0, 1fr)' }}
        >
          {/* 左栏宽度探针：占据第 1 列的 0 高度元素，拖拽起点/键盘步进时读取实际列宽 */}
          <div
            ref={leftColumnProbeRef}
            aria-hidden
            className="pointer-events-none h-0 w-full"
            style={{ gridColumn: '1', gridRow: '1' }}
          />

          {/* 全部会话以稳定 key 渲染：窗格分配仅切换 gridColumn，换入/换出不卸载重挂 */}
          {sessions.map((session) => {
            const paneSide: FilesPaneSide | null =
              session.id === pane.leftSessionId
                ? 'left'
                : session.id === pane.rightSessionId
                  ? 'right'
                  : null;
            const isTabActive = session.id === activeSession?.id;
            const isPaneFocused = paneSide != null && paneSide === pane.focusedPane;
            return (
              <div
                key={session.id}
                role="tabpanel"
                aria-hidden={paneSide == null}
                data-state={isTabActive ? 'active' : 'inactive'}
                onPointerDown={paneSide ? () => focusPane(paneSide) : undefined}
                style={{ gridColumn: paneSide === 'right' ? '3' : '1', gridRow: '1' }}
                className={cn(
                  'flex min-h-0 min-w-0 flex-col overflow-hidden rounded-lg',
                  paneSide == null && 'hidden',
                  isPaneFocused && 'ring-1 ring-primary/40',
                )}
              >
                <SftpSessionView
                  sessionId={String(session.hostId)}
                  hostId={session.hostId}
                  initialPath={session.currentPath}
                  isVisible={Boolean(isVisible && paneSide != null)}
                  onPathChange={(path) => updateSessionPath(session.id, path)}
                  onOpenEditor={onOpenEditor}
                  onPaneFocus={paneSide ? () => focusPane(paneSide) : undefined}
                />
              </div>
            );
          })}

          {/* 空窗格占位（仅对应窗格没有绑定会话时渲染） */}
          {!leftSession ? renderEmptyPaneCell('left') : null}
          {!rightSession ? renderEmptyPaneCell('right') : null}

          {/* 中缝分隔条：悬停/聚焦/拖拽显示指示线，左右键盘微调 */}
          <div
            role="separator"
            aria-orientation="vertical"
            aria-valuemin={FILES_PANE_MIN_WIDTH_PX}
            aria-valuemax={maxLeftPaneWidthPx}
            aria-valuenow={ariaLeftPaneWidthPx}
            tabIndex={0}
            onPointerDown={handleDividerPointerDown}
            onPointerMove={handleDividerPointerMove}
            onPointerUp={finishDividerDrag}
            onPointerCancel={finishDividerDrag}
            onKeyDown={handleDividerKeyDown}
            style={{ gridColumn: '2', gridRow: '1' }}
            className={cn(
              'group/handle relative z-10 flex h-full w-1 shrink-0 cursor-ew-resize touch-none items-center justify-center bg-transparent outline-none focus:outline-none focus-visible:ring-0',
              // 悬停/聚焦/拖拽时显示 2px 圆角指示线
              "after:pointer-events-none after:absolute after:w-0.5 after:rounded-full after:bg-foreground-subtlest/50 after:opacity-0 after:transition-opacity after:content-[''] after:inset-y-2",
              'hover:after:opacity-100 focus-visible:after:opacity-100',
              isDividerActive && 'after:opacity-100',
            )}
          />
        </div>
      )}
    </section>
  );
}
