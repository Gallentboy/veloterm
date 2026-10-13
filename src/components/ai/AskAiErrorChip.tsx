import { Sparkles, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useSyncExternalStore } from 'react';
import { useTranslation } from 'react-i18next';
import {
  clearAiErrorContext,
  getAiErrorContext,
  subscribeAiErrorContext,
  type AiErrorContext,
} from '@/services/aiContext';

interface AskAiErrorChipProps {
  onAsk: (prompt: string) => void;
}

/** 快照缓存：上下文对象引用仅在 set/clear 时更换，满足 useSyncExternalStore 稳定性 */
let contextSnapshot: AiErrorContext | null = getAiErrorContext();

subscribeAiErrorContext(() => {
  contextSnapshot = getAiErrorContext();
});

/**
 * 终端面板右下角浮层：SSH 会话启动失败后出现「问 AI」快捷入口，
 * 一键把错误现场（主机/错误信息）作为初始提问带入 AI 对话。
 */
export function AskAiErrorChip({ onAsk }: AskAiErrorChipProps) {
  const { t } = useTranslation();
  const context = useSyncExternalStore(subscribeAiErrorContext, () => contextSnapshot);
  const [dismissedAt, setDismissedAt] = useState(0);

  // 新错误产生时重置关闭状态
  useEffect(() => {
    if (context) {
      setDismissedAt(0);
    }
  }, [context?.occurredAt]);

  if (!context || context.occurredAt <= dismissedAt) {
    return null;
  }

  return (
    <div className="absolute bottom-3 right-3 z-30 flex items-center gap-1.5 rounded-full border border-border bg-popover py-1 pl-2.5 pr-1 shadow-lg animate-in fade-in slide-in-from-bottom-2 duration-200">
      <button
        type="button"
        onClick={() => onAsk(context.detail)}
        className="flex items-center gap-1.5 text-ui-sm text-foreground transition-colors hover:text-primary"
        title={context.title}
      >
        <Sparkles className="size-3.5 text-primary" />
        {t('ai.errorChip.ask')}
        <span className="max-w-40 truncate text-ui-xs text-foreground-subtlest">
          {context.title}
        </span>
      </button>
      <button
        type="button"
        onClick={() => {
          setDismissedAt(context.occurredAt);
          clearAiErrorContext();
        }}
        aria-label={t('ai.errorChip.dismiss')}
        className="inline-flex size-5 items-center justify-center rounded-full text-foreground-subtlest transition-colors hover:bg-surface-hover hover:text-foreground"
      >
        <X className="size-3" />
      </button>
    </div>
  );
}
