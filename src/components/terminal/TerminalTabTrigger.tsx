import { SquareTerminal, Unplug, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { MarqueeText } from '@/components/ui/MarqueeText';
import { loadHostStore } from '@/hosts/storage';
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

  const envLevel =
    session.hostId != null
      ? loadHostStore().hosts.find((h) => h.id === session.hostId)?.envLevel
      : undefined;

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
        'group relative inline-flex h-7 w-auto max-w-60 min-w-0 flex-none shrink-0 cursor-default items-center justify-start gap-1.5 overflow-hidden whitespace-nowrap rounded-lg border border-transparent bg-transparent pl-2 pr-1 text-ui-base font-medium text-foreground-subtle transition-all select-none outline-none',
        'hover:text-foreground',
        !isActive && 'hover:bg-hover',
        isActive && 'bg-selected text-foreground',
      )}
    >
      {/* 断开态用 Unplug + warning 色指示（激活/非激活都保持 warning 色），正常态用默认终端图标 */}
      {session.status === 'disconnected' ? (
        <Unplug className="size-3.5 shrink-0 text-warning" />
      ) : (
        <SquareTerminal className="size-3.5 shrink-0 text-foreground-subtle" />
      )}
      {envLevel === 'prod' ? (
        <span className="inline-flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 font-mono text-[9px] font-bold uppercase tracking-wider bg-red-500/15 text-red-500 border border-red-500/30 shadow-[0_0_8px_rgba(239,68,68,0.2)]">
          <span className="size-1.5 rounded-full bg-red-500 status-pulse-rose" />
          PROD
        </span>
      ) : envLevel === 'staging' ? (
        <span className="inline-flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 font-mono text-[9px] font-bold uppercase tracking-wider bg-amber-500/15 text-amber-500 border border-amber-500/30 shadow-[0_0_8px_rgba(245,158,11,0.2)]">
          <span className="size-1.5 rounded-full bg-amber-500 status-pulse-amber" />
          STAGE
        </span>
      ) : null}
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
      <ControlHintTooltip title={closeLabel} side="bottom" align="end">
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label={closeLabel}
          // 非激活 Tab 关闭按钮悬停/聚焦才显现；门控必须直接挂按钮——
          // ControlHintTooltip 的 className 作用于弹出面板而非包裹层
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
      </ControlHintTooltip>
    </div>
  );
}
