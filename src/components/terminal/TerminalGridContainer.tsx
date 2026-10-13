import { Maximize2, Minimize2, Terminal, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { TerminalSession } from '@/components/terminal/TerminalSession';
import type { ITerminalTransport, TerminalSize } from '@/terminal/types';
import type { TerminalSessionDescriptor } from '@/terminal/terminalPanelState';

export interface TerminalGridContainerProps {
  sessions: TerminalSessionDescriptor[];
  activeSessionId: number;
  maximizedSessionId: number | null;
  transport: ITerminalTransport;
  isPanelResizing: boolean;
  isVisible: boolean;
  onActivateSession: (sessionId: number) => void;
  onMaximizeSession: (sessionId: number | null) => void;
  onCloseSession: (sessionId: number) => void;
  onShellLabelChange: (sessionId: number, shellLabel: string | null) => void;
  onSizeChange: (sessionId: number, size: TerminalSize) => void;
  onWebglStatusChange: (sessionId: number, active: boolean) => void;
  onExit: (sessionId: number, exitCode: number) => void;
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

export function TerminalGridContainer({
  sessions,
  activeSessionId,
  maximizedSessionId,
  transport,
  isPanelResizing,
  isVisible,
  onActivateSession,
  onMaximizeSession,
  onCloseSession,
  onShellLabelChange,
  onSizeChange,
  onWebglStatusChange,
  onExit,
}: TerminalGridContainerProps) {
  // 如果某个分屏被放大为全屏，仅呈现该分屏并提供还原按钮
  if (maximizedSessionId !== null) {
    const session = sessions.find((s) => s.id === maximizedSessionId);
    if (session) {
      const title = session.customTitle || session.shellLabel || `终端 ${session.index}`;
      return (
        <div className="relative flex h-full min-h-0 w-full flex-col overflow-hidden rounded-xl border border-border/80 bg-background shadow-xs">
          {/* 单屏最大化顶栏 */}
          <div className="flex h-7 shrink-0 items-center justify-between border-b border-border/60 bg-surface/30 px-2.5 text-ui-xs">
            <div className="flex items-center gap-2 min-w-0">
              <Terminal className="size-3.5 text-primary shrink-0" />
              <span className="font-semibold text-foreground truncate">{title}</span>
              <span className="rounded bg-primary/10 px-1.5 py-0.2 font-mono text-[10px] text-primary">
                单屏最大化
              </span>
            </div>
            <div className="flex items-center gap-1">
              <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                onClick={() => onMaximizeSession(null)}
                className="h-6 w-6 rounded text-foreground-subtle hover:text-foreground"
                title="还原网格多屏 (Esc)"
              >
                <Minimize2 className="size-3" />
              </Button>
            </div>
          </div>

          <div className="min-h-0 flex-1 overflow-hidden p-2">
            <TerminalSession
              sessionId={session.id}
              transport={transport}
              cwd={session.cwd}
              hostId={session.hostId}
              isVisible={isVisible}
              isPanelResizing={isPanelResizing}
              onShellLabelChange={onShellLabelChange}
              onSizeChange={onSizeChange}
              onWebglStatusChange={onWebglStatusChange}
              onExit={onExit}
            />
          </div>
        </div>
      );
    }
  }

  const gridClass = getGridClass(sessions.length);

  return (
    <div
      role="grid"
      aria-label="多终端平铺网格视图"
      className={cn('grid h-full min-h-0 w-full gap-2 p-2 overflow-hidden', gridClass)}
    >
      {sessions.map((session) => {
        const isActive = session.id === activeSessionId;
        const title = session.customTitle || session.shellLabel || `终端 ${session.index}`;

        return (
          <div
            key={session.id}
            role="gridcell"
            onClick={() => onActivateSession(session.id)}
            className={cn(
              'group relative flex min-h-0 min-w-0 flex-col overflow-hidden rounded-lg border bg-background transition-all',
              isActive
                ? 'border-primary/60 ring-1 ring-primary/40 shadow-xs'
                : 'border-border/70 hover:border-border',
            )}
          >
            {/* 单元格分屏顶栏 */}
            <div
              onDoubleClick={() => onMaximizeSession(session.id)}
              className={cn(
                'flex h-6.5 shrink-0 items-center justify-between border-b px-2 text-ui-xs select-none transition-colors cursor-pointer',
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
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  onClick={(e) => {
                    e.stopPropagation();
                    onMaximizeSession(session.id);
                  }}
                  className="size-5 rounded text-foreground-subtle hover:text-foreground"
                  title="放大该终端 (双击顶栏)"
                >
                  <Maximize2 className="size-2.5" />
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  onClick={(e) => {
                    e.stopPropagation();
                    onCloseSession(session.id);
                  }}
                  className="size-5 rounded text-foreground-subtle hover:text-destructive hover:bg-destructive/10"
                  title="关闭该终端"
                >
                  <X className="size-2.5" />
                </Button>
              </div>
            </div>

            {/* 终端会话实际视口 */}
            <div className="min-h-0 flex-1 overflow-hidden p-1">
              <TerminalSession
                sessionId={session.id}
                transport={transport}
                cwd={session.cwd}
                hostId={session.hostId}
                isVisible={isVisible}
                isPanelResizing={isPanelResizing}
                onShellLabelChange={onShellLabelChange}
                onSizeChange={onSizeChange}
                onWebglStatusChange={onWebglStatusChange}
                onExit={onExit}
              />
            </div>
          </div>
        );
      })}
    </div>
  );
}
