import {
  AlertTriangle,
  Ban,
  Check,
  Radio,
  Send,
  X,
} from 'lucide-react';
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
} from 'react';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import {
  appendBroadcastHistory,
  broadcastSignalToSessions,
  broadcastToSessions,
  getBroadcastHistory,
  isDangerousCommand,
} from '@/services/terminalBroadcastService';
import { useTerminalStore } from '@/terminal/TerminalProvider';
import type { TerminalSessionDescriptor } from '@/terminal/terminalPanelState';

export interface BroadcastCommandBarProps {
  sessions: TerminalSessionDescriptor[];
  onClose: () => void;
}

export function BroadcastCommandBar({ sessions, onClose }: BroadcastCommandBarProps) {
  const { transport } = useTerminalStore();

  const [selectedSessionIds, setSelectedSessionIds] = useState<number[]>(() =>
    sessions.map((s) => s.id),
  );
  const [command, setCommand] = useState('');
  const [isExecuting, setIsExecuting] = useState(false);
  const [dangerousModalOpen, setDangerousModalOpen] = useState(false);
  const [history, setHistory] = useState<string[]>(() => getBroadcastHistory());
  const [historyIndex, setHistoryIndex] = useState<number>(-1);
  const [feedback, setFeedback] = useState<string | null>(null);

  const inputRef = useRef<HTMLInputElement>(null);

  // 当外部会话列表变化时，自动修剪无效的目标 ID
  useEffect(() => {
    setSelectedSessionIds((prev) => {
      const activeIds = new Set(sessions.map((s) => s.id));
      const filtered = prev.filter((id) => activeIds.has(id));
      // 如果之前全选，新开终端也默认加入
      if (prev.length === 0 && sessions.length > 0) {
        return sessions.map((s) => s.id);
      }
      return filtered.length > 0 ? filtered : sessions.map((s) => s.id);
    });
  }, [sessions]);

  // 打开时自动聚焦输入框
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const handleToggleSession = (sessionId: number) => {
    setSelectedSessionIds((prev) =>
      prev.includes(sessionId) ? prev.filter((id) => id !== sessionId) : [...prev, sessionId],
    );
  };

  const handleToggleAll = () => {
    if (selectedSessionIds.length === sessions.length) {
      setSelectedSessionIds([]);
    } else {
      setSelectedSessionIds(sessions.map((s) => s.id));
    }
  };

  const executeSend = useCallback(async () => {
    const trimmed = command.trim();
    if (!trimmed || selectedSessionIds.length === 0 || isExecuting) return;

    setIsExecuting(true);
    setDangerousModalOpen(false);
    try {
      appendBroadcastHistory(trimmed);
      setHistory(getBroadcastHistory());
      setHistoryIndex(-1);

      const result = await broadcastToSessions(transport, selectedSessionIds, trimmed);
      setFeedback(`已向 ${result.successCount}/${result.sessionCount} 个终端广播发送成功`);
      setCommand('');
      setTimeout(() => setFeedback(null), 3000);
    } catch (err) {
      setFeedback(`广播失败: ${String(err)}`);
    } finally {
      setIsExecuting(false);
      inputRef.current?.focus();
    }
  }, [command, isExecuting, selectedSessionIds, transport]);

  const executeSendSignal = useCallback(
    async (signalByte: number = 0x03, label: string = 'Ctrl+C (SIGINT)') => {
      if (selectedSessionIds.length === 0 || isExecuting) return;

      setIsExecuting(true);
      try {
        const result = await broadcastSignalToSessions(transport, selectedSessionIds, signalByte);
        setFeedback(`已向 ${result.successCount}/${result.sessionCount} 个终端广播发送 ${label}`);
        setTimeout(() => setFeedback(null), 3000);
      } catch (err) {
        setFeedback(`发送信号失败: ${String(err)}`);
      } finally {
        setIsExecuting(false);
        inputRef.current?.focus();
      }
    },
    [isExecuting, selectedSessionIds, transport],
  );

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    const trimmed = command.trim();
    if (!trimmed || selectedSessionIds.length === 0) return;

    // 若用户输入 ^C 或 ctrl+c，直接触发广播中断信号
    if (trimmed.toLowerCase() === '^c' || trimmed.toLowerCase() === 'ctrl+c') {
      setCommand('');
      void executeSendSignal(0x03, 'Ctrl+C (SIGINT)');
      return;
    }

    if (isDangerousCommand(trimmed)) {
      setDangerousModalOpen(true);
      return;
    }
    void executeSend();
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
      return;
    }

    // 广播输入框支持 Ctrl+C：一键向所有选中的目标终端广播发送 SIGINT 中断信号（如停止 top）
    if (e.ctrlKey && !e.metaKey && e.key.toLowerCase() === 'c') {
      e.preventDefault();
      e.stopPropagation();
      void executeSendSignal(0x03, 'Ctrl+C (SIGINT)');
      return;
    }

    // 广播输入框支持 Ctrl+D：向所有选中的目标终端广播发送 EOF
    if (e.ctrlKey && !e.metaKey && e.key.toLowerCase() === 'd') {
      e.preventDefault();
      e.stopPropagation();
      void executeSendSignal(0x04, 'Ctrl+D (EOF)');
      return;
    }

    if (e.key === 'ArrowUp') {
      if (history.length === 0) return;
      e.preventDefault();
      const nextIdx = historyIndex + 1 < history.length ? historyIndex + 1 : historyIndex;
      setHistoryIndex(nextIdx);
      if (history[nextIdx] !== undefined) {
        setCommand(history[nextIdx]!);
      }
      return;
    }

    if (e.key === 'ArrowDown') {
      if (history.length === 0) return;
      e.preventDefault();
      const nextIdx = historyIndex - 1;
      setHistoryIndex(nextIdx);
      if (nextIdx >= 0 && history[nextIdx] !== undefined) {
        setCommand(history[nextIdx]!);
      } else {
        setCommand('');
        setHistoryIndex(-1);
      }
    }
  };

  const allSelected = selectedSessionIds.length === sessions.length && sessions.length > 0;

  return (
    <div
      role="region"
      aria-label="终端协同广播工具栏"
      className="relative shrink-0 border-t border-border/80 bg-background/95 backdrop-blur-md p-2.5 shadow-lg select-none z-30"
    >
      {/* 顶部目标选择器 */}
      <div className="flex flex-wrap items-center gap-1.5 pb-2 text-ui-xs">
        <div className="flex items-center gap-1 font-medium text-foreground pr-1 shrink-0">
          <Radio className="size-3.5 text-primary animate-pulse" />
          <span>广播目标 ({selectedSessionIds.length}/{sessions.length}):</span>
        </div>

        <button
          type="button"
          onClick={handleToggleAll}
          className={cn(
            'inline-flex h-6 items-center gap-1 rounded-md border px-2 text-ui-xs font-medium transition-colors',
            allSelected
              ? 'border-primary/50 bg-primary/10 text-primary font-semibold'
              : 'border-border/70 bg-surface/40 text-foreground-subtle hover:text-foreground hover:bg-hover',
          )}
        >
          {allSelected && <Check className="size-3" />}
          <span>全部终端</span>
        </button>

        {sessions.map((session) => {
          const isChecked = selectedSessionIds.includes(session.id);
          const title = session.customTitle || session.shellLabel || '终端';
          return (
            <button
              key={session.id}
              type="button"
              onClick={() => handleToggleSession(session.id)}
              className={cn(
                'inline-flex h-6 items-center gap-1 rounded-md border px-2 text-ui-xs font-medium transition-colors truncate max-w-[140px]',
                isChecked
                  ? 'border-primary/50 bg-primary/10 text-primary font-semibold'
                  : 'border-border/70 bg-surface/40 text-foreground-subtle hover:text-foreground hover:bg-hover',
              )}
            >
              {isChecked && <Check className="size-3 shrink-0" />}
              <span className="truncate">{title}</span>
            </button>
          );
        })}

        {feedback && (
          <span className="ml-auto text-success font-medium text-ui-xs animate-in fade-in">
            {feedback}
          </span>
        )}
      </div>

      {/* 命令输入栏 */}
      <form onSubmit={handleSubmit} className="flex items-center gap-2">
        <div className="relative flex flex-1 items-center">
          <span className="pointer-events-none absolute left-2.5 font-mono text-ui-sm text-foreground-subtle select-none">
            ❯
          </span>
          <input
            ref={inputRef}
            type="text"
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            onKeyDown={handleKeyDown}
            disabled={isExecuting}
            placeholder={
              selectedSessionIds.length === 0
                ? '请先勾选至少一个广播目标终端...'
                : '输入广播命令 (Enter 发送，按 ⌃C 或点击「⌃C 中断」发送中断信号，↑/↓ 调阅历史)...'
            }
            className="w-full rounded-lg border border-border/80 bg-surface/30 py-1.5 pl-7 pr-3 font-mono text-ui-sm text-foreground outline-none transition-colors focus:border-primary/70 focus:bg-background"
          />
        </div>

        <Button
          type="submit"
          variant="default"
          size="sm"
          disabled={!command.trim() || selectedSessionIds.length === 0 || isExecuting}
          className="h-8 gap-1.5 rounded-lg px-3.5 text-ui-xs font-medium shrink-0 shadow-xs"
        >
          <Send className="size-3.5" />
          <span>发送广播</span>
        </Button>

        {/* 广播发送 Ctrl+C 中断信号按钮 */}
        <ControlHintTooltip title="向勾选的所有目标终端广播发送中断信号 (Ctrl+C)" side="bottom">
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => void executeSendSignal(0x03, 'Ctrl+C (SIGINT)')}
            disabled={selectedSessionIds.length === 0 || isExecuting}
            className="h-8 gap-1.5 rounded-lg border border-destructive/40 bg-destructive/10 text-destructive hover:bg-destructive/20 hover:text-destructive px-2.5 text-ui-xs font-medium shrink-0 shadow-xs transition-colors"
          >
            <Ban className="size-3.5" />
            <span>⌃C 中断</span>
          </Button>
        </ControlHintTooltip>

        <ControlHintTooltip title="关闭广播栏" shortcut="Esc" side="bottom" align="end">
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            onClick={onClose}
            className="size-8 rounded-lg text-foreground-subtle hover:text-foreground shrink-0"
            aria-label="关闭广播栏"
          >
            <X className="size-4" />
          </Button>
        </ControlHintTooltip>
      </form>

      {/* 高危命令拦截二次确认弹窗 */}
      {dangerousModalOpen && (
        <div
          role="dialog"
          aria-modal="true"
          className="fixed inset-0 z-[70] flex items-center justify-center bg-black/40 p-4 animate-in fade-in duration-150 backdrop-blur-2xs"
        >
          <div className="flex w-[440px] max-w-full flex-col overflow-hidden rounded-2xl border border-destructive/30 bg-popover p-5 shadow-2xl animate-in zoom-in-95 duration-150">
            <div className="flex items-center gap-2 text-destructive font-semibold text-ui-base">
              <AlertTriangle className="size-5 shrink-0" />
              <span>高危命令广播安全警告</span>
            </div>

            <p className="mt-2 text-ui-sm text-foreground">
              您正准备向 <strong className="text-destructive">{selectedSessionIds.length}</strong> 个终端并发执行以下高危破坏性指令：
            </p>

            <pre className="mt-3 rounded-lg border border-destructive/20 bg-destructive/10 p-3 font-mono text-ui-xs text-destructive break-all whitespace-pre-wrap select-all">
              {command}
            </pre>

            <div className="mt-3 text-ui-xs text-foreground-subtle">
              受影响的目标终端：
              <div className="mt-1 flex flex-wrap gap-1">
                {sessions
                  .filter((s) => selectedSessionIds.includes(s.id))
                  .map((s) => (
                    <span
                      key={s.id}
                      className="rounded bg-surface px-1.5 py-0.5 font-mono text-[11px] text-foreground"
                    >
                      {s.customTitle || s.shellLabel || '终端'}
                    </span>
                  ))}
              </div>
            </div>

            <div className="mt-5 flex items-center justify-end gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => setDangerousModalOpen(false)}
                className="h-8 px-3 rounded-lg"
              >
                取消
              </Button>
              <Button
                type="button"
                variant="destructive"
                size="sm"
                onClick={() => void executeSend()}
                className="h-8 gap-1.5 px-3 rounded-lg"
              >
                <span>确认向全部 {selectedSessionIds.length} 个终端执行</span>
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
