import { Ban, Check, Radio, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { broadcastSignalToSessions } from '@/services/terminalBroadcastService';
import { useTerminalStore } from '@/terminal/TerminalProvider';
import type { TerminalSessionDescriptor } from '@/terminal/terminalPanelState';

export interface BroadcastCommandBarProps {
  sessions: TerminalSessionDescriptor[];
  onClose: () => void;
}

/**
 * 终端联动控制条（吸附于终端区底部）：
 * - 控制条可见即联动生效：勾选集合（store 的 broadcastTargets）内任一终端的
 *   用户输入会被实时镜像到其余终端（镜像逻辑在 TerminalSession 的 onData 里，
 *   这里只负责目标集合的增删与协同中断）；
 * - 无命令输入栏：联动是逐键镜像，不是整条命令广播；
 * - ⌃C 中断向全部勾选目标并发发送 SIGINT，用于协同停住 top / ping / tail -f。
 */
export function BroadcastCommandBar({ sessions, onClose }: BroadcastCommandBarProps) {
  const { transport, broadcastTargets, setBroadcastTargets } = useTerminalStore();
  const [isExecuting, setIsExecuting] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);

  // 打开时默认全选一次（store 在关闭时已清空）；之后仅修剪已关闭的会话，
  // 不把用户主动取消勾选产生的空集合重置回全选（空集合 = 联动暂停）
  const initializedRef = useRef(false);
  useEffect(() => {
    const activeIds = sessions.map((s) => s.id);
    if (activeIds.length === 0) return;
    if (!initializedRef.current) {
      initializedRef.current = true;
      setBroadcastTargets(activeIds);
      return;
    }
    const filtered = broadcastTargets.filter((id) => activeIds.includes(id));
    if (filtered.length !== broadcastTargets.length) {
      setBroadcastTargets(filtered);
    }
  }, [sessions, broadcastTargets, setBroadcastTargets]);

  const handleToggleSession = (sessionId: number) => {
    setBroadcastTargets(
      broadcastTargets.includes(sessionId)
        ? broadcastTargets.filter((id) => id !== sessionId)
        : [...broadcastTargets, sessionId],
    );
  };

  const handleToggleAll = () => {
    const allIds = sessions.map((s) => s.id);
    setBroadcastTargets(
      broadcastTargets.length === allIds.length ? [] : allIds,
    );
  };

  const executeSendSignal = useCallback(
    async (signalByte: number = 0x03, label: string = 'Ctrl+C (SIGINT)') => {
      if (broadcastTargets.length === 0 || isExecuting) return;

      setIsExecuting(true);
      try {
        const result = await broadcastSignalToSessions(transport, broadcastTargets, signalByte);
        setFeedback(`已向 ${result.successCount}/${result.sessionCount} 个终端发送 ${label}`);
        setTimeout(() => setFeedback(null), 3000);
      } catch (err) {
        setFeedback(`发送信号失败: ${String(err)}`);
      } finally {
        setIsExecuting(false);
      }
    },
    [broadcastTargets, isExecuting, transport],
  );

  const allSelected = broadcastTargets.length === sessions.length && sessions.length > 0;
  const syncActive = broadcastTargets.length > 1;

  return (
    <div
      role="region"
      aria-label="终端联动控制条"
      className="relative shrink-0 border-t border-border/80 bg-background/95 backdrop-blur-md p-2.5 shadow-lg select-none z-30"
    >
      <div className="flex flex-wrap items-center gap-1.5 text-ui-xs">
        <div className="flex items-center gap-1 font-medium text-foreground pr-1 shrink-0">
          <Radio
            className={cn('size-3.5 text-primary', syncActive && 'animate-pulse')}
          />
          <span>联动目标 ({broadcastTargets.length}/{sessions.length}):</span>
        </div>

        <button
          type="button"
          onClick={handleToggleAll}
          className={cn(
            'inline-flex h-6 items-center gap-1 rounded-md border px-2 text-ui-xs transition-colors',
            allSelected
              ? 'border-border bg-selected text-foreground'
              : 'border-border/60 text-foreground-subtle hover:bg-hover hover:text-foreground',
          )}
        >
          {allSelected && <Check className="size-3" />}
          <span>全部终端</span>
        </button>

        {sessions.map((session) => {
          const isChecked = broadcastTargets.includes(session.id);
          const title = session.customTitle || session.shellLabel || '终端';
          return (
            <button
              key={session.id}
              type="button"
              onClick={() => handleToggleSession(session.id)}
              className={cn(
                'inline-flex h-6 items-center gap-1 rounded-md border px-2 text-ui-xs transition-colors truncate max-w-[140px]',
                isChecked
                  ? 'border-border bg-selected text-foreground'
                  : 'border-border/60 text-foreground-subtle hover:bg-hover hover:text-foreground',
              )}
            >
              {isChecked && <Check className="size-3 shrink-0" />}
              <span className="truncate">{title}</span>
            </button>
          );
        })}

        {feedback ? (
          <span className="ml-auto text-success font-medium text-ui-xs animate-in fade-in">
            {feedback}
          </span>
        ) : (
          !syncActive && (
            <span className="ml-auto text-foreground-subtlest text-ui-xs">
              勾选至少两个终端后，在任一终端输入即实时同步到其余终端
            </span>
          )
        )}

        <ControlHintTooltip title="向勾选的所有目标终端并发发送中断信号 (Ctrl+C)" side="bottom">
          <Button
            type="button"
            variant="outline"
            size="md"
            onClick={() => void executeSendSignal(0x03, 'Ctrl+C (SIGINT)')}
            disabled={broadcastTargets.length === 0 || isExecuting}
            className="shrink-0 gap-1.5"
          >
            <Ban className="size-3.5" />
            <span>⌃C 中断</span>
          </Button>
        </ControlHintTooltip>

        <ControlHintTooltip title="关闭联动" shortcut="Esc" side="bottom" align="end">
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            onClick={onClose}
            className="size-8 rounded-lg text-foreground-subtle hover:text-foreground shrink-0"
            aria-label="关闭联动"
          >
            <X className="size-4" />
          </Button>
        </ControlHintTooltip>
      </div>
    </div>
  );
}
