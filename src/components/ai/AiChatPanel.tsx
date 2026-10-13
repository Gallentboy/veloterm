import { Loader2, Send, Settings, Sparkles, TriangleAlert, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  aiChat,
  listAiProviders,
  type AiChatMessage,
  type AiProvider,
} from '@/services/aiService';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

interface ChatItem {
  role: 'user' | 'assistant';
  content: string;
  streaming?: boolean;
  error?: boolean;
}

interface AiChatPanelProps {
  open: boolean;
  onClose: () => void;
  /** 外部注入的初始提问（如 SSH 错误上下文） */
  initialPrompt?: string | null;
  /** 跳转 AI 供应商设置页 */
  onOpenSettings?: () => void;
}

/**
 * AI 问答右侧面板（ZCode 布局范式：右侧栏常驻，不遮挡主内容）：
 * 流式渲染助手回复、供应商切换、多 Key 轮询状态。
 * 面板跨页面导航保持挂载，对话上下文不丢。
 */
export function AiChatPanel({ open, onClose, initialPrompt, onOpenSettings }: AiChatPanelProps) {
  const { t } = useTranslation();
  const [items, setItems] = useState<ChatItem[]>([]);
  const [input, setInput] = useState('');
  const [isStreaming, setIsStreaming] = useState(false);
  const [providers, setProviders] = useState<AiProvider[]>([]);
  const [providerId, setProviderId] = useState<number | null>(null);
  const [lastKeyIndex, setLastKeyIndex] = useState<number | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const sentInitialRef = useRef<string | null>(null);

  useEffect(() => {
    void listAiProviders()
      .then((list) => {
        setProviders(list);
        setProviderId((current) => {
          if (current != null && list.some((p) => p.id === current)) {
            return current;
          }
          const preferred = list.find((p) => p.isDefault) ?? list[0];
          return preferred?.id ?? null;
        });
      })
      .catch(() => setProviders([]));
  }, []);

  // 外部注入的初始提问：与上次已发送的不同才自动发（面板常驻后同一错误只发一次）
  useEffect(() => {
    if (open && initialPrompt && sentInitialRef.current !== initialPrompt && providers.length > 0) {
      sentInitialRef.current = initialPrompt;
      void send(initialPrompt);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- 依赖面板打开与配置就绪时机
  }, [open, initialPrompt, providers.length]);

  // 流式时自动滚到底部
  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
  }, [items]);

  async function send(text: string) {
    const content = text.trim();
    if (!content || isStreaming) return;

    const history: AiChatMessage[] = [
      ...items
        .filter((item) => !item.error && item.content)
        .map((item) => ({ role: item.role, content: item.content })),
      { role: 'user' as const, content },
    ];

    setInput('');
    setIsStreaming(true);
    setItems((prev) => [
      ...prev,
      { role: 'user', content },
      { role: 'assistant', content: '', streaming: true },
    ]);

    const updateAssistant = (fn: (current: ChatItem) => ChatItem) => {
      setItems((prev) => {
        const next = [...prev];
        const last = next[next.length - 1];
        if (last?.role === 'assistant') {
          next[next.length - 1] = fn(last);
        }
        return next;
      });
    };

    try {
      await aiChat(
        {
          providerId,
          history,
          preamble:
            '你是 VeloTerm 桌面终端应用的 AI 助手，擅长 Linux/SSH/运维问题排查。回答简洁、直接给可执行的命令或步骤。',
        },
        (chunk) => {
          if (chunk.keyIndex !== null && chunk.keyIndex !== undefined) {
            setLastKeyIndex(chunk.keyIndex);
          }
          if (chunk.delta) {
            updateAssistant((current) => ({
              ...current,
              content: current.content + chunk.delta,
            }));
          }
          if (chunk.error) {
            updateAssistant((current) => ({
              ...current,
              content: chunk.error ?? 'error',
              error: true,
              streaming: false,
            }));
          }
        },
      );
    } catch (err) {
      updateAssistant((current) => ({
        ...current,
        content: current.content || String(err),
        error: true,
        streaming: false,
      }));
    } finally {
      updateAssistant((current) => ({ ...current, streaming: false }));
      setIsStreaming(false);
    }
  }

  const provider = providers.find((p) => p.id === providerId);

  return (
    <aside
      aria-label={t('ai.chat.title')}
      className={cn(
        'flex h-full w-[360px] shrink-0 flex-col overflow-hidden rounded-(--workspace-panel-radius) border border-border bg-background transition-[width,opacity] duration-200 ease-out',
        !open && 'pointer-events-none w-0 border-transparent opacity-0',
      )}
    >
      {/* 头部：标题 + 供应商 + 设置/关闭 */}
      <div className="flex h-12 shrink-0 items-center justify-between gap-2 border-b border-border/50 px-3">
        <div className="flex min-w-0 items-center gap-1.5">
          <Sparkles className="size-4 shrink-0 text-primary" />
          <span className="text-ui-base font-medium text-foreground select-none">
            {t('ai.chat.title')}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-0.5">
          <select
            value={providerId ?? ''}
            onChange={(e) => setProviderId(e.target.value ? Number(e.target.value) : null)}
            className="h-7 max-w-36 rounded-md border border-border bg-input px-1.5 text-ui-sm text-foreground outline-none cursor-pointer"
            aria-label={t('ai.chat.provider')}
          >
            {providers.length === 0 ? (
              <option value="">{t('ai.chat.noProvider')}</option>
            ) : (
              providers.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))
            )}
          </select>
          {onOpenSettings ? (
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={onOpenSettings}
              aria-label={t('ai.provider.title')}
              title={t('ai.provider.title')}
            >
              <Settings className="size-3.5" />
            </Button>
          ) : null}
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={onClose}
            aria-label={t('common.close')}
          >
            <X className="size-3.5" />
          </Button>
        </div>
      </div>

      {/* 消息列表 */}
      <div
        ref={listRef}
        className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto p-3"
      >
        {items.length === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 py-10 select-none">
            <Sparkles className="size-7 text-foreground-subtlest" />
            <span className="text-ui-sm text-foreground-subtlest">{t('ai.chat.empty')}</span>
            {providers.length === 0 && onOpenSettings ? (
              <Button variant="outline" onClick={onOpenSettings} className="mt-2">
                <Settings className="size-3.5" />
                {t('ai.chat.goSettingsButton')}
              </Button>
            ) : null}
          </div>
        ) : (
          items.map((item, index) => (
            <div
              key={index}
              className={
                item.role === 'user'
                  ? 'self-end max-w-[88%] rounded-xl rounded-br-sm bg-primary px-2.5 py-1.5 text-ui-sm text-primary-foreground whitespace-pre-wrap break-words'
                  : 'self-start max-w-[95%] rounded-xl rounded-bl-sm border border-border bg-surface px-2.5 py-1.5 text-ui-sm text-foreground whitespace-pre-wrap break-words'
              }
            >
              {item.content ||
                (item.streaming ? (
                  <Loader2 className="size-4 animate-spin text-foreground-subtlest" />
                ) : null)}
              {item.error && (
                <TriangleAlert className="mr-1 inline size-3.5 text-destructive align-[-2px]" />
              )}
              {item.role === 'assistant' &&
                !item.streaming &&
                item.content &&
                !item.error &&
                provider &&
                provider.apiKeys.length > 1 &&
                lastKeyIndex !== null && (
                  <span className="mt-1.5 block text-ui-xs text-foreground-subtlest">
                    {provider.model} · Key #{lastKeyIndex + 1}
                  </span>
                )}
            </div>
          ))
        )}
      </div>

      {/* 输入区 */}
      <div className="flex shrink-0 items-end gap-1.5 border-t border-border/50 p-2.5">
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void send(input);
            }
          }}
          rows={Math.min(4, Math.max(1, input.split('\n').length))}
          placeholder={
            providers.length === 0 ? t('ai.chat.goSettings') : t('ai.chat.placeholder')
          }
          disabled={providers.length === 0}
          className="max-h-24 min-w-0 flex-1 resize-none rounded-md border border-border bg-input px-2 py-1.5 text-ui-sm text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover"
        />
        <Button
          size="icon-md"
          disabled={isStreaming || !input.trim() || providers.length === 0}
          onClick={() => void send(input)}
          aria-label={t('ai.chat.send')}
          className="shrink-0"
        >
          {isStreaming ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <Send className="size-4" />
          )}
        </Button>
      </div>
    </aside>
  );
}
