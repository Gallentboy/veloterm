import { Columns2, FolderSync, Plus, Server } from 'lucide-react';
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react';
import { useTranslation } from 'react-i18next';
import { SftpSessionView } from '@/components/files/SftpSessionView';
import { SftpTabTrigger } from '@/components/files/SftpTabTrigger';
import {
  registerDropTarget,
  type SftpTabDragPayload,
} from '@/services/dragDropService';
import { NewSessionMenu } from '@/components/NewSessionMenu';
import { Button } from '@/components/ui/button';
import { EmptyState } from '@/components/ui/EmptyState';
import { useHostsStore } from '@/hosts/HostsProvider';
import { hasPrimaryModifier } from '@/lib/keyboard';
import { cn } from '@/lib/utils';
import { useDismissOnOutsidePointer } from '@/hooks/useDismissOnOutsidePointer';
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
const FILES_PANE_DIVIDER_WIDTH_CSS = '0.75rem';
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
  const { hosts } = useHostsStore();
  const {
    sessions,
    activeSession,
    createSession,
    closeSession,
    activateSession,
    updateSessionPath,
    renameSession,
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
  // Tab 拖入分栏：当前被拖 Tab 的会话 id 与悬停落点侧（null = 无拖拽/无悬停）
  const [draggingTabId, setDraggingTabId] = useState<number | null>(null);
  const [tabDropSide, setTabDropSide] = useState<FilesPaneSide | null>(null);
  // Tab 右键菜单（ZCode 风格，提供「退出分栏」；池中 Tab 置灰）
  const [paneMenu, setPaneMenu] = useState<{ x: number; y: number; sessionId: number } | null>(
    null,
  );
  const paneMenuRef = useRef<HTMLDivElement | null>(null);

  const closePaneMenu = useCallback(() => {
    setPaneMenu(null);
  }, []);

  // 点击外部 / Escape 关闭：document 捕获阶段监听，不受子树 stopPropagation 影响
  useDismissOnOutsidePointer(Boolean(paneMenu), paneMenuRef, closePaneMenu);

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


  // Tab 点击/会话激活的统一入口（焦点面板 + Tab 模型）：
  // 已在某窗格 → 聚焦该窗格；仍在池中 → 换入当前聚焦窗格（显式点击才换，被换会话退回池中）；
  // 自动分配（新建连接等）不经过这里，避免连接创建时顶替现有窗格
  const handleTabActivate = useCallback(
    (sessionId: number) => {
      setPane((current) => {
        if (current.leftSessionId === sessionId) {
          return current.focusedPane === 'left' ? current : { ...current, focusedPane: 'left' };
        }
        if (current.rightSessionId === sessionId) {
          return current.focusedPane === 'right' ? current : { ...current, focusedPane: 'right' };
        }
        return current.focusedPane === 'left'
          ? { ...current, leftSessionId: sessionId }
          : { ...current, rightSessionId: sessionId };
      });
      activateSession(sessionId);
    },
    [activateSession],
  );

  // 快捷键支持：主修饰键+W 关闭当前 Tab、主修饰键+Shift+[ / ] 切换前后 Tab（切换会进入聚焦窗格）
  useEffect(() => {
    if (!isVisible) return;

    const onKeyDown = (event: KeyboardEvent) => {
      const mod = hasPrimaryModifier(event);
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

  // 新会话自动展示（仅空窗格兜底，绝不顶替、不新建分栏）：
  // 两侧窗格都为空时，激活会话落入聚焦窗格（覆盖首次连接、主机页直达等路径，保证连接后立即可见）；
  // 任一窗格已有会话时保持池中——展示用点击 Tab（换入聚焦侧）或拖拽（分栏/替换），
  // 自动路径（新建连接等）不动现有窗格，分栏只能通过拖动创建
  useEffect(() => {
    const activeId = activeSession?.id;
    if (activeId == null) return;
    setPane((current) => {
      if (current.leftSessionId === activeId || current.rightSessionId === activeId) {
        return current;
      }
      if (current.leftSessionId != null || current.rightSessionId != null) {
        return current;
      }
      return current.focusedPane === 'left'
        ? { ...current, leftSessionId: activeId }
        : { ...current, rightSessionId: activeId };
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
  // 仅一侧有会话时退化为单栏铺满：隐藏中缝分隔条，另一侧不再渲染占位
  const bothPanesOccupied = Boolean(leftSession && rightSession);

  // ── Tab 拖入分栏（统一拖放总线，见 services/dragDropService）──
  // 已在某窗格的 Tab 拖动为 NOOP（不响应悬停与放置）；
  // 双栏已存在：池中 Tab 落到某侧即替换该侧，被替换会话退回池中（仍以 Tab 展示）；
  // 单栏铺满状态：按落点在主区的左/右半区决定新会话进哪一侧，原主机占另一侧，直接形成双栏。
  const isPooledTabDragging =
    draggingTabId != null &&
    draggingTabId !== pane.leftSessionId &&
    draggingTabId !== pane.rightSessionId;

  // 落点在主区的左半还是右半（单栏模式据此分栏）
  const dropSideByX = (clientX: number): FilesPaneSide => {
    const rect = mainAreaRef.current?.getBoundingClientRect();
    const mid = rect ? rect.left + rect.width / 2 : 0;
    return clientX < mid ? 'left' : 'right';
  };

  // 窗格落点目标：跟随当前占用两侧的会话注册（会话/池拖拽态变化时重建，重建开销仅注册表增删）
  const cellElsRef = useRef(new Map<number, HTMLDivElement>());
  useEffect(() => {
    const unregisters = (['left', 'right'] as const).flatMap((side) => {
      const occupiedId = side === 'left' ? pane.leftSessionId : pane.rightSessionId;
      if (occupiedId == null) return [];
      return [
        registerDropTarget({
          accepts: ['sftp-tab'],
          getElement: () => cellElsRef.current.get(occupiedId) ?? null,
          onHoverChange: (hovered, position) => {
            if (!hovered || !isPooledTabDragging || !position) {
              setTabDropSide(null);
              return;
            }
            setTabDropSide(bothPanesOccupied ? side : dropSideByX(position.x));
          },
          onDrop: (event) => {
            setTabDropSide(null);
            if (event.kind !== 'sftp-tab' || !isPooledTabDragging) return;
            const droppedId = (event.payload as SftpTabDragPayload).sessionId;
            if (droppedId == null || !sessions.some((s) => s.id === droppedId)) return;

            setPane((current) => {
              // NOOP：已在任一窗格的 Tab 落点无效
              if (droppedId === current.leftSessionId || droppedId === current.rightSessionId) {
                return current;
              }
              const currentlyBoth = current.leftSessionId != null && current.rightSessionId != null;
              if (!currentlyBoth) {
                // 单栏铺满：按落点半区分栏，原主机让到/留在另一侧，形成双栏
                const dropSide = dropSideByX(event.position.x);
                const occupiedSide: FilesPaneSide = current.leftSessionId != null ? 'left' : 'right';
                const displacedId =
                  occupiedSide === 'left' ? current.leftSessionId : current.rightSessionId;
                if (dropSide === occupiedSide && displacedId != null) {
                  return dropSide === 'left'
                    ? { ...current, leftSessionId: droppedId, rightSessionId: displacedId, focusedPane: 'left' }
                    : { ...current, rightSessionId: droppedId, leftSessionId: displacedId, focusedPane: 'right' };
                }
                return dropSide === 'left'
                  ? { ...current, leftSessionId: droppedId, focusedPane: 'left' }
                  : { ...current, rightSessionId: droppedId, focusedPane: 'right' };
              }
              // 双栏已存在：替换指定侧，被替换会话退回池中
              return side === 'left'
                ? { ...current, leftSessionId: droppedId, focusedPane: 'left' }
                : { ...current, rightSessionId: droppedId, focusedPane: 'right' };
            });
            activateSession(droppedId);
          },
        }),
      ];
    });
    return () => unregisters.forEach((u) => u());
    // eslint-disable-next-line react-hooks/exhaustive-deps -- isPooledTabDragging/dropSideByX 每次渲染重建，行为取最新值
  }, [pane.leftSessionId, pane.rightSessionId, sessions, bothPanesOccupied, draggingTabId, activateSession]);

  // 右键菜单：退出分栏（清空对应窗格槽位；退出的是激活会话且另一侧仍有会话时，
  // 激活就近落到另一侧，避免高亮一个不可见会话）
  const handleExitPane = useCallback(
    (sessionId: number) => {
      closePaneMenu();
      setPane((current) => {
        const exitingLeft = current.leftSessionId === sessionId;
        const exitingRight = current.rightSessionId === sessionId;
        if (!exitingLeft && !exitingRight) return current;
        const next: FilesPaneState = exitingLeft
          ? { ...current, leftSessionId: null }
          : { ...current, rightSessionId: null };
        const survivorId = exitingLeft ? next.rightSessionId : next.leftSessionId;
        if (survivorId != null && activeSession?.id === sessionId) {
          activateSession(survivorId);
        }
        return next;
      });
    },
    [activateSession, activeSession?.id],
  );

  const handleTabContextMenu = useCallback(
    (event: ReactMouseEvent<HTMLDivElement>, sessionId: number) => {
      event.preventDefault();
      // 菜单已打开时再次右键同一 Tab → 直接关闭（否则 pointerdown 先关、contextmenu
      // 又原地重开，视觉上等于「第一次点了没反应」）
      if (paneMenu?.sessionId === sessionId) {
        closePaneMenu();
        return;
      }
      // 视口边缘钳制，避免菜单溢出屏幕
      const x = Math.min(event.clientX, window.innerWidth - 180);
      const y = Math.min(event.clientY, window.innerHeight - 120);
      setPaneMenu({ x, y, sessionId });
    },
    [closePaneMenu, paneMenu?.sessionId],
  );

  // 空窗格占位元格（轻量内联空态，无描边装饰，配色对齐 ZCode 中性风格）
  const renderEmptyPaneCell = (side: FilesPaneSide) => {
    return (
      <div
        key={`pane-empty-${side}`}
        data-pane-empty={side}
        onPointerDown={() => focusPane(side)}
        style={{ gridColumn: '1', gridRow: '1' }}
        className="flex min-h-0 min-w-0 flex-col items-center justify-center overflow-hidden rounded-lg bg-input p-6 text-center select-none"
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

  // 主区 Grid 模板：双侧均有会话时为「左列定宽/弹性 + 4px 中缝 + 右列弹性」；
  // 仅一侧有会话时退化为单列铺满（分隔条随之隐藏）
  const gridTemplateColumns = bothPanesOccupied
    ? leftPaneWidthPx == null
      ? `minmax(${FILES_PANE_MIN_WIDTH_PX}px, 1fr) ${FILES_PANE_DIVIDER_WIDTH_CSS} minmax(${FILES_PANE_MIN_WIDTH_PX}px, 1fr)`
      : `${leftPaneWidthPx}px ${FILES_PANE_DIVIDER_WIDTH_CSS} minmax(${FILES_PANE_MIN_WIDTH_PX}px, 1fr)`
    : 'minmax(0, 1fr)';

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
      className="flex h-full min-h-0 flex-col select-none overflow-hidden bg-background"
    >
      {/* 顶部多 Tab 栏（完全对齐 ZCode Terminal 顶栏规范） */}
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/50 px-3 py-1.5">
        <div className="scrollbar-slim flex min-w-0 flex-1 items-center gap-1 overflow-x-auto overflow-y-hidden">
          {sessions.length > 0 ? (
            <div
              role="tablist"
              aria-label={t('files.title')}
              className="flex h-7 w-max items-center gap-1"
            >
              {sessions.map((session) => {
                // 标题不拼序号：customTitle（重命名）优先，其次主机名；key 始终用唯一会话 id
                const title = session.customTitle || session.hostName;
                return (
                  <SftpTabTrigger
                    key={session.id}
                    session={session}
                    title={title}
                    closeLabel={t('files.closeTab', { title })}
                    isActive={session.id === activeSession?.id}
                    onActivate={handleTabActivate}
                    onClose={closeSession}
                    onDragStateChange={(dragging) =>
                      setDraggingTabId(dragging ? session.id : null)
                    }
                    onContextMenu={(event) => handleTabContextMenu(event, session.id)}
                    onRename={(next) => renameSession(session.id, next)}
                  />
                );
              })}
            </div>
          ) : null}

          {/* 新建 SFTP Tab 按钮（带主机选择下拉） */}
          {hosts.length > 0 && (
            <NewSessionMenu
              title={t('files.newTab')}
              hostSectionLabel={t('files.remoteHosts')}
              hosts={hosts}
              onSelectHost={handleOpenHostSession}
            />
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
          // 主区域外圈四角圆角（与工程整体 --workspace-panel-radius 一致），
          // overflow-hidden 使面板内容在四角被裁成圆角；分栏内侧圆角由面板自身类提供
          className="relative grid min-h-0 flex-1 overflow-hidden rounded-(--workspace-panel-radius)"
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
            return (
              <div
                key={session.id}
                ref={(el) => {
                  if (el) cellElsRef.current.set(session.id, el);
                  else cellElsRef.current.delete(session.id);
                }}
                role="tabpanel"
                aria-hidden={paneSide == null}
                data-state={isTabActive ? 'active' : 'inactive'}
                onPointerDown={paneSide ? () => focusPane(paneSide) : undefined}
                style={{
                  gridColumn: bothPanesOccupied ? (paneSide === 'right' ? '3' : '1') : '1',
                  gridRow: '1',
                }}
                className={cn(
                  // 面板底色 = ZCode 对话输入栏同款 --color-input（浅色 #ffffff / 深色 #2b2b2b），
                  // 无论单栏铺满还是双栏分列恒为此色；面板间缝隙透出页面底色
                  'flex min-h-0 min-w-0 flex-col overflow-hidden bg-input',
                  paneSide == null && 'hidden',
                  // 圆角与工程整体一致（--workspace-panel-radius）：
                  // 双栏时只圆内侧两角（左栏圆右、右栏圆左），单栏铺满时四角全圆
                  bothPanesOccupied
                    ? paneSide === 'left'
                      ? 'rounded-r-(--workspace-panel-radius)'
                      : 'rounded-l-(--workspace-panel-radius)'
                    : 'rounded-(--workspace-panel-radius)',
                  // 双栏模式：池中 Tab 悬停落点整栏高亮（单栏模式由半区 overlay 呈现）
                  bothPanesOccupied &&
                    paneSide != null &&
                    tabDropSide === paneSide &&
                    'bg-hover/40',
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

          {/* 空窗格占位：仅双侧都无会话的极端瞬态作兑底（单侧模式由另一侧铺满，不再占位） */}
          {!leftSession && !rightSession ? renderEmptyPaneCell(pane.focusedPane) : null}

          {/* 中缝分隔条：仅双侧均有会话时渲染，悬停/聚焦/拖拽显示指示线，左右键盘微调 */}
          {bothPanesOccupied ? (
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
              'group/handle relative z-10 flex h-full w-full shrink-0 cursor-ew-resize touch-none items-center justify-center bg-transparent outline-none focus:outline-none focus-visible:ring-0',
              // 分栏时常驻 1px 中性分割线（ZCode 侧栏风格）；悬停/聚焦/拖拽时加深为 2px 圆角指示线
              "after:pointer-events-none after:absolute after:left-1/2 after:-translate-x-1/2 after:w-px after:bg-foreground-subtlest/60 after:transition-[width,opacity,background-color] after:content-[''] after:inset-y-2",
              'hover:after:w-0.5 hover:after:rounded-full hover:after:bg-foreground-subtlest focus-visible:after:w-0.5 focus-visible:after:rounded-full focus-visible:after:bg-foreground-subtlest',
              isDividerActive && 'after:w-0.5 after:rounded-full after:bg-foreground-subtlest',
            )}
          />
          ) : null}

          {/* 单栏铺满状态拖入池中 Tab：按落点半区预览即将形成的左右分栏（中性色，无描边） */}
          {!bothPanesOccupied && tabDropSide != null ? (
            <div
              aria-hidden
              className="pointer-events-none absolute inset-y-0 z-10 bg-hover/30"
              style={tabDropSide === 'left' ? { left: 0, width: '50%' } : { right: 0, width: '50%' }}
            />
          ) : null}
        </div>
      )}

      {/* Tab 右键菜单（ZCode 风格）：退出分栏；池中 Tab 置灰不可点 */}
      {paneMenu ? (
        <div
          ref={paneMenuRef}
          role="menu"
          style={{ left: paneMenu.x, top: paneMenu.y }}
          onPointerDown={(event) => event.stopPropagation()}
          className="fixed z-50 min-w-36 overflow-hidden rounded-lg border border-border bg-menu p-1 text-ui-sm text-foreground shadow-lg"
        >
          {(() => {
            const inPane =
              paneMenu.sessionId === pane.leftSessionId ||
              paneMenu.sessionId === pane.rightSessionId;
            return (
              <button
                type="button"
                role="menuitem"
                disabled={!inPane}
                onClick={() => handleExitPane(paneMenu.sessionId)}
                className={cn(
                  'flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors',
                  inPane
                    ? 'hover:bg-menu-hover'
                    : 'cursor-not-allowed opacity-40',
                )}
              >
                <Columns2 className="size-3.5" />
                <span>{t('files.pane.exitPane')}</span>
              </button>
            );
          })()}
        </div>
      ) : null}
    </section>
  );
}
