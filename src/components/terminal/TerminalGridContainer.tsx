import { Maximize2, Minimize2, Terminal, X } from 'lucide-react';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { TerminalSession } from '@/components/terminal/TerminalSession';
import type { TerminalLayoutMode } from '@/terminal/TerminalProvider';
import type { ITerminalTransport, TerminalSize } from '@/terminal/types';
import type { TerminalSessionDescriptor } from '@/terminal/terminalPanelState';

export interface TerminalGridContainerProps {
  layoutMode: TerminalLayoutMode;
  sessions: TerminalSessionDescriptor[];
  activeSessionId: number;
  maximizedSessionId: number | null;
  transport: ITerminalTransport;
  /** 终端联动目标集合（透传给 TerminalSession：非空且含本会话时输入实时镜像） */
  broadcastTargets: number[];
  isPanelResizing: boolean;
  isVisible: boolean;
  onActivateSession: (sessionId: number) => void;
  onMaximizeSession: (sessionId: number | null) => void;
  onCloseSession: (sessionId: number) => void;
  onShellLabelChange: (sessionId: number, shellLabel: string | null) => void;
  onSizeChange: (sessionId: number, size: TerminalSize) => void;
  onRendererStatusChange: (sessionId: number, kind: 'webgl' | 'canvas2d' | 'dom') => void;
  onExit: (sessionId: number, exitCode: number) => void;
  /** SSH 会话断开回调（标记断开态，Tab 保留） */
  onSessionDisconnected: (sessionId: number, reason: string) => void;
  /** 重连请求（⌘R/Ctrl+R 快捷键触发：终端内由 xterm 键处理分支，焦点在终端外时全局快捷键）：
   *  释放旧会话并重新发起连接 */
  onReconnectSession: (sessionId: number) => void;
}

/**
 * 根据当前会话总数计算自适应响应式网格行列配置（支持左右分屏、四宫格、六宫格、九宫格）
 */
function getGridClass(count: number): string {
  if (count <= 1) return 'grid-cols-1 grid-rows-1';
  if (count === 2) return 'grid-cols-1 md:grid-cols-2 grid-rows-1';
  if (count <= 4) return 'grid-cols-1 md:grid-cols-2 grid-rows-2';
  if (count <= 6) return 'grid-cols-2 md:grid-cols-3 grid-rows-2';
  return 'grid-cols-2 md:grid-cols-3 grid-rows-3';
}

/**
 * 统一终端会话布局容器（宫格平铺 / 标签页两种模式共用同一棵组件树）：
 * - 宫格与标签页切换仅改变容器与单元格的 CSS 类，所有 `TerminalSession` 挂载点持久保活，
 *   避免整棵子树卸载重挂载导致 hostEl 进出暂存区、ResizeObserver 重建与 resize 链路重置；
 * - 单元格尺寸变化统一交给 `TerminalSession` 内部的 ResizeObserver 感知并 fit。
 */
export function TerminalGridContainer({
  layoutMode,
  sessions,
  activeSessionId,
  maximizedSessionId,
  transport,
  broadcastTargets,
  isPanelResizing,
  isVisible,
  onActivateSession,
  onMaximizeSession,
  onCloseSession,
  onShellLabelChange,
  onSizeChange,
  onRendererStatusChange,
  onExit,
  onSessionDisconnected,
  onReconnectSession,
}: TerminalGridContainerProps) {
  const isGrid = layoutMode === 'grid';
  const hasMaximized = maximizedSessionId !== null;
  // 单屏最大化时退化为 1×1 网格，避免被多列模板约束在单列内
  const gridClass = hasMaximized ? 'grid-cols-1 grid-rows-1' : getGridClass(sessions.length);

  return (
    <div
      role={isGrid ? 'grid' : undefined}
      aria-label={isGrid ? '多终端平铺网格视图' : undefined}
      className={cn(
        'min-h-0 w-full flex-1 overflow-hidden',
        isGrid ? cn('grid gap-2 p-2', gridClass) : 'flex flex-col p-3 pt-2',
      )}
    >
      {sessions.map((session) => {
        const isActive = session.id === activeSessionId;
        const isMaximized = session.id === maximizedSessionId;
        const title = session.customTitle || session.shellLabel || '终端';
        // 会话可见性：宫格模式全部可见（单屏最大化期间仅保留该会话）；标签页模式仅激活 Tab 可见
        const sessionVisible = isGrid ? !hasMaximized || isMaximized : isActive;

        return (
          <div
            // reconnectNonce 变化时强制 remount TerminalSession 走全新创建路径
            // （reconnectSession 已 release 旧注册表条目：新 xterm + 新 SSH 连接）
            key={session.reconnectNonce ? `${session.id}:${session.reconnectNonce}` : session.id}
            role={isGrid ? 'gridcell' : 'tabpanel'}
            aria-hidden={!sessionVisible}
            data-state={isActive ? 'active' : 'inactive'}
            onClick={() => onActivateSession(session.id)}
            className={cn(
              'group relative flex min-h-0 min-w-0 flex-col overflow-hidden',
              !sessionVisible && 'hidden',
              isGrid
                ? isMaximized
                  ? 'rounded-xl border border-border/80 bg-background shadow-xs'
                  : cn(
                      'rounded-lg border bg-background transition-colors',
                      // 选中单元格用工程「强调边框」token（亮/暗双值），不叠主色描边：
                      // 与 tab 选中态（bg-selected）同一套克制语汇，活动指示由顶栏与状态点承担
                      isActive
                        ? 'border-border-hover ring-1 ring-border-hover/60 shadow-xs'
                        : 'border-border/70 hover:border-border',
                    )
                : 'h-full w-full flex-1',
            )}
          >
            {/* 宫格单元格顶栏（标签页模式与单屏最大化时隐藏） */}
            <div
              onDoubleClick={() => onMaximizeSession(session.id)}
              className={cn(
                'flex h-6.5 shrink-0 items-center justify-between border-b px-2 text-ui-xs select-none transition-colors cursor-pointer',
                (!isGrid || isMaximized) && 'hidden',
                isActive
                  ? 'border-border/80 bg-surface/60 text-foreground font-medium'
                  : 'border-border/40 bg-surface/20 text-foreground-subtle hover:text-foreground',
              )}
            >
              <div className="flex items-center gap-1.5 min-w-0 overflow-hidden">
                <span
                  className={cn(
                    'size-1.5 rounded-full shrink-0',
                    isActive ? 'bg-primary' : 'bg-foreground-subtlest',
                  )}
                />
                <span className="truncate font-mono text-[11px]">{title}</span>
              </div>

              <div className="flex items-center gap-0.5 opacity-60 group-hover:opacity-100 transition-opacity">
                <ControlHintTooltip title="放大该终端（双击顶栏）" side="bottom">
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-xs"
                    onClick={(e) => {
                      e.stopPropagation();
                      onMaximizeSession(session.id);
                    }}
                    className="size-5 rounded text-foreground-subtle hover:text-foreground"
                  >
                    <Maximize2 className="size-2.5" />
                  </Button>
                </ControlHintTooltip>
                <ControlHintTooltip title="关闭该终端" side="bottom">
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-xs"
                    onClick={(e) => {
                      e.stopPropagation();
                      onCloseSession(session.id);
                    }}
                    className="size-5 rounded text-foreground-subtle hover:text-destructive hover:bg-destructive/10"
                  >
                    <X className="size-2.5" />
                  </Button>
                </ControlHintTooltip>
              </div>
            </div>

            {/* 单屏最大化顶栏（仅宫格模式下被放大的会话显示） */}
            <div
              className={cn(
                'flex h-7 shrink-0 items-center justify-between border-b border-border/60 bg-surface/30 px-2.5 text-ui-xs',
                !(isGrid && isMaximized) && 'hidden',
              )}
            >
              <div className="flex items-center gap-2 min-w-0">
                <Terminal className="size-3.5 text-primary shrink-0" />
                <span className="font-semibold text-foreground truncate">{title}</span>
                <span className="rounded bg-primary/10 px-1.5 py-0.2 font-mono text-[10px] text-primary">
                  单屏最大化
                </span>
              </div>
              <div className="flex items-center gap-1">
                <ControlHintTooltip title="还原网格多屏" shortcut="Esc" side="bottom">
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-xs"
                    onClick={(e) => {
                      e.stopPropagation();
                      onMaximizeSession(null);
                    }}
                    className="h-6 w-6 rounded text-foreground-subtle hover:text-foreground"
                  >
                    <Minimize2 className="size-3" />
                  </Button>
                </ControlHintTooltip>
              </div>
            </div>

            {/* 终端会话实际视口 */}
            <div
              className={cn(
                'min-h-0 flex-1 overflow-hidden',
                isGrid && (isMaximized ? 'p-2' : 'p-1'),
              )}
            >
              <TerminalSession
                sessionId={session.id}
                transport={transport}
                broadcastTargets={broadcastTargets}
                cwd={session.cwd}
                hostId={session.hostId}
                isVisible={isVisible && sessionVisible}
                isPanelResizing={isPanelResizing}
                // 字体缩放仅在标签页模式启用；九宫格/单屏最大化期间禁用（字号保持）
                fontScaleEnabled={!isGrid}
                // 底部远程资源监控条：仅标签页模式的当前可见会话展示（九宫格平铺自动隐藏）
                resourceBarVisible={!isGrid && sessionVisible}
                onShellLabelChange={onShellLabelChange}
                onSizeChange={onSizeChange}
                onRendererStatusChange={onRendererStatusChange}
                onExit={onExit}
                isDisconnected={session.status === 'disconnected'}
                onDisconnected={onSessionDisconnected}
                onReconnectRequest={onReconnectSession}
              />
            </div>

            {/* 断开态不再叠加 DOM 横幅：断开信息与重连快捷键提示由 TerminalSession
                直接写进 xterm 终端缓冲（像真实终端掉线那样）；重连入口 = ⌘R/Ctrl+R，
                关闭 Tab 走标签栏原有的关闭按钮 */}
          </div>
        );
      })}
    </div>
  );
}
