import { SquareTerminal, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Button } from '@/components/ui/button';
import { MarqueeText } from '@/components/ui/MarqueeText';
import { cn } from '@/lib/utils';
import type { TerminalSessionDescriptor } from '@/terminal/terminalPanelState';

export interface TerminalTabTriggerProps {
  session: TerminalSessionDescriptor;
  title: string;
  closeLabel: string;
  isActive: boolean;
  onActivate: (sessionId: number) => void;
  onClose: (sessionId: number) => void;
  /** 双击重命名提交（null = 清空回退默认名） */
  onRename?: (title: string | null) => void;
}

/**
 * 终端顶部单个 Tab 触发器（移植自 ZCode `TerminalTabTrigger.tsx`）：
 * - 激活态 `bg-selected text-foreground`，非激活态悬停 `hover:bg-hover`；
 * - 标题超长时截断省略号，悬停走马灯滚动展示全文（MarqueeText）；
 * - 双击标题行内重命名（Enter/失焦提交，Esc 取消，空值回退默认名）；
 * - 关闭按钮在激活态常显，非激活态仅 hover/focus 时浮现；支持鼠标中键快速关闭。
 */
export function TerminalTabTrigger({
  session,
  title,
  closeLabel,
  isActive,
  onActivate,
  onClose,
  onRename,
}: TerminalTabTriggerProps) {
  const [isEditing, setIsEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);
  // Esc 取消标记：取消后紧随的失焦不应再提交（每次进入编辑时复位）
  const cancelledRef = useRef(false);

  useEffect(() => {
    if (isEditing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [isEditing]);

  const beginRename = () => {
    cancelledRef.current = false;
    setDraft(title);
    setIsEditing(true);
  };

  const commitRename = () => {
    if (!isEditing) return;
    setIsEditing(false);
    if (cancelledRef.current) return;
    const next = draft.trim();
    onRename?.(next.length > 0 ? next : null);
  };

  const cancelRename = () => {
    cancelledRef.current = true;
    setIsEditing(false);
  };

  return (
    <div
      role="tab"
      tabIndex={isActive ? 0 : -1}
      aria-selected={isActive}
      data-active={isActive ? '' : undefined}
      data-state={isActive ? 'active' : 'inactive'}
      onDoubleClick={(event) => {
        // 关闭按钮上的双击不触发重命名
        if ((event.target as HTMLElement).closest('button')) return;
        beginRename();
      }}
      onClick={() => {
        if (!isEditing) onActivate(session.id);
      }}
      onAuxClick={(event) => {
        // 鼠标中键关闭 Tab
        if (event.button === 1) {
          event.preventDefault();
          event.stopPropagation();
          onClose(session.id);
        }
      }}
      onKeyDown={(event) => {
        if (isEditing) return;
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          onActivate(session.id);
        }
      }}
      className={cn(
        'group relative inline-flex h-7 w-auto max-w-120 min-w-0 flex-none shrink-0 cursor-default items-center justify-start gap-1.5 overflow-hidden whitespace-nowrap rounded-lg border border-transparent bg-transparent pl-2 pr-1 text-ui-base font-medium text-foreground-subtle transition-all select-none outline-none',
        'hover:text-foreground',
        !isActive && 'hover:bg-hover',
        isActive && 'bg-selected text-foreground',
      )}
    >
      <SquareTerminal className="size-3.5 shrink-0 text-foreground-subtle" />
      {isEditing ? (
        <input
          ref={inputRef}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commitRename}
          onKeyDown={(event) => {
            event.stopPropagation();
            if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
              event.preventDefault();
              commitRename();
            } else if (event.key === 'Escape') {
              event.preventDefault();
              cancelRename();
            }
          }}
          onPointerDown={(event) => event.stopPropagation()}
          onClick={(event) => event.stopPropagation()}
          onDoubleClick={(event) => event.stopPropagation()}
          aria-label={title}
          className="h-5 min-w-0 flex-1 rounded border border-border bg-input px-1 text-ui-base font-medium text-foreground outline-none"
        />
      ) : (
        <MarqueeText text={title} className="min-w-0 flex-1" />
      )}
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
