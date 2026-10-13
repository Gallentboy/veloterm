import { SquareTerminal, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import type { TerminalSessionDescriptor } from '@/terminal/terminalPanelState';

export interface TerminalTabTriggerProps {
  session: TerminalSessionDescriptor;
  title: string;
  closeLabel: string;
  isActive: boolean;
  onActivate: (sessionId: number) => void;
  onClose: (sessionId: number) => void;
}

/**
 * 终端顶部单个 Tab 触发器（移植自 ZCode `TerminalTabTrigger.tsx`）：
 * - 激活态 `bg-selected text-foreground`，非激活态悬停 `hover:bg-hover`；
 * - 标题超长时右侧渐隐遮罩；
 * - 关闭按钮在激活态常显，非激活态仅 hover/focus 时浮现；支持鼠标中键快速关闭。
 */
export function TerminalTabTrigger({
  session,
  title,
  closeLabel,
  isActive,
  onActivate,
  onClose,
}: TerminalTabTriggerProps) {
  return (
    <div
      role="tab"
      tabIndex={isActive ? 0 : -1}
      aria-selected={isActive}
      data-active={isActive ? '' : undefined}
      data-state={isActive ? 'active' : 'inactive'}
      onClick={() => onActivate(session.id)}
      onAuxClick={(event) => {
        // 鼠标中键关闭 Tab
        if (event.button === 1) {
          event.preventDefault();
          event.stopPropagation();
          onClose(session.id);
        }
      }}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          onActivate(session.id);
        }
      }}
      className={cn(
        'group relative inline-flex h-7 w-auto max-w-40 min-w-0 flex-none shrink-0 cursor-default items-center justify-start gap-1.5 overflow-hidden whitespace-nowrap rounded-lg border border-transparent bg-transparent pl-2 pr-1 text-ui-base font-medium text-foreground-subtle transition-all select-none outline-none',
        'hover:text-foreground',
        !isActive && 'hover:bg-hover',
        isActive && 'bg-selected text-foreground',
      )}
    >
      <SquareTerminal className="size-3.5 shrink-0 text-foreground-subtle" />
      <span
        data-terminal-tab-content=""
        className="flex min-w-0 flex-1 overflow-hidden whitespace-nowrap [mask-image:linear-gradient(to_right,black_calc(100%-0.5rem),transparent)]"
      >
        <span className="shrink-0 whitespace-nowrap">{title}</span>
      </span>
      <Button
        type="button"
        variant="ghost"
        size="icon-xs"
        aria-label={closeLabel}
        title={closeLabel}
        className={cn(
          'shrink-0 rounded-md',
          !isActive &&
            'pointer-events-none opacity-0 group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100',
        )}
        onPointerDown={(event) => {
          event.stopPropagation();
        }}
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          onClose(session.id);
        }}
      >
        <X className="size-3" />
      </Button>
    </div>
  );
}
