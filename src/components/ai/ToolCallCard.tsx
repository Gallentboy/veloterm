import { ChevronRight, Play } from 'lucide-react';
import { useState } from 'react';
import { cn } from '@/lib/utils';
import { Spinner } from '@/components/ui/spinner';

export interface ToolCallCardData {
  name: string;
  arguments: string;
  result?: string;
  running?: boolean;
}

/**
 * 工具调用卡片：默认只显示工具名 + 状态图标（执行中旋转 / 完成绿勾），
 * 有参数或结果时整卡可点击展开/收起（ChevronRight 随展开旋转）。
 */
export function ToolCallCard({ toolCall }: { toolCall: ToolCallCardData }) {
  const [expanded, setExpanded] = useState(false);
  const hasDetails =
    (Boolean(toolCall.arguments) && toolCall.arguments !== '{}') || Boolean(toolCall.result);

  return (
    <div
      className={cn(
        'self-start w-[92%] rounded-lg border border-border/70 bg-surface/40 select-none transition-colors',
        hasDetails && 'cursor-pointer hover:border-border',
      )}
      onClick={() => {
        if (hasDetails) {
          setExpanded((value) => !value);
        }
      }}
    >
      <div className="flex items-center gap-1.5 px-2.5 py-1.5 text-ui-xs text-foreground-subtle">
        {toolCall.running ? (
          <Spinner className="size-3 text-primary" />
        ) : (
          <Play className="size-3 text-success" />
        )}
        <span className="font-mono font-medium">{toolCall.name}</span>
        {hasDetails ? (
          <ChevronRight
            className={cn(
              'ml-auto size-3 shrink-0 text-foreground-subtlest transition-transform',
              expanded && 'rotate-90',
            )}
          />
        ) : null}
      </div>
      {expanded && hasDetails ? (
        <div className="border-t border-border/50 px-2.5 py-1.5">
          {toolCall.arguments && toolCall.arguments !== '{}' ? (
            <pre className="max-h-20 overflow-auto font-mono text-ui-xs text-foreground-subtlest whitespace-pre-wrap">
              {toolCall.arguments}
            </pre>
          ) : null}
          {toolCall.result ? (
            <pre className="mt-1 max-h-40 overflow-auto rounded border border-border/50 bg-background px-1.5 py-1 font-mono text-ui-xs text-foreground-subtle whitespace-pre-wrap">
              {toolCall.result}
            </pre>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
