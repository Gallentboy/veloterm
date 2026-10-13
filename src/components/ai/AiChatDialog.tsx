import { Loader2, Plus, Send, Sparkles, TriangleAlert } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  aiChat,
  listAiProviders,
  type AiChatMessage,
  type AiProvider,
} from '@/services/aiService';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';

interface ChatItem {
  role: 'user' | 'assistant';
  content: string;
  streaming?: boolean;
  error?: boolean;
}

interface AiChatDialogProps {
  open: boolean;
  onClose: () => void;
  /** 外部注入的初始提问（如 SSH 错误上下文） */
  initialPrompt?: string | null;
}

/**
 * AI 问答回话弹窗：流式渲染助手回复，多 Key 轮询状态显示在回复角落。
 * 后端走 rig 统一接口（openai 兼容/anthropic/gemini），供应商在设置页配置。
 */
export function AiChatDialog({ open, onClose, initialPrompt }: AiChatDialogProps) {
  const { t } = useTranslation();
  const [items, setItems] = useState<ChatItem[]>([]);
  const [input, setInput] = useState('');
  const [isStreaming, setIsStreaming] = useState(false);
  const [providers, setProviders] = useState<AiProvider[]>([]);
  const [providerId, setProviderId] = useState<number | null>(null);
  const [lastKeyIndex, setLastKeyIndex] = useState<number | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const sentInitialRef = useRef(false);

  useEffect(() => {
    if (!open) return;
    void listAiProviders()
      .then((list) => {
        setProviders(list);
        const preferred = list.find((p) => p.isDefault) ?? list[0];
        setProviderId(preferred?.id ?? null);
      })
      .catch(() => setProviders([]));
  }, [open]);

  // 自动发送外部注入的初始提问（如「SSH 连接失败，帮我分析」）
  useEffect(() => {
    if (open && initialPrompt && !sentInitialRef.current && providers.length > 0) {
      sentInitialRef.current = true;
      void send(initialPrompt);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- 仅在弹窗打开且配置就绪时发一次
  }, [open, initialPrompt, providers.length]);

  // 流式时自动滚到底部
  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
  }, [items]);

  useEffect(() => {
    if (!open) {
      sentInitialRef.current = false;
    }
  }, [open]);

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
          preamble: '你是 VeloTerm 桌面终端应用的 AI 助手，擅长 Linux/SSH/运维问题排查。回答简洁、直接给可执行的命令或步骤。',
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
    <Modal
      title={t('ai.chat.title')}
      onClose={onClose}
      className="w-[560px]"
      footer={
        <div className="flex w-full items-center gap-2">
          <select
            value={providerId ?? ''}
            onChange={(e) => setProviderId(e.target.value ? Number(e.target.value) : null)}
            className="h-8 shrink-0 max-w-36 rounded-md border border-border bg-input px-2 text-ui-sm text-foreground outline-none cursor-pointer"
            aria-label={t('ai.chat.provider')}
          >
            {providers.length === 0 ? (
              <option value="">{t('ai.chat.noProvider')}</option>
            ) : (
              providers.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name} · {p.model}
                </option>
              ))
            )}
          </select>
          <input
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                void send(input);
              }
            }}
            placeholder={
              providers.length === 0 ? t('ai.chat.goSettings') : t('ai.chat.placeholder')
            }
            disabled={providers.length === 0}
            className="h-8 min-w-0 flex-1 rounded-md border border-border bg-input px-2.5 text-ui-base text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover"
          />
          <Button
            size="icon-md"
            disabled={isStreaming || !input.trim() || providers.length === 0}
            onClick={() => void send(input)}
            aria-label={t('ai.chat.send')}
          >
            {isStreaming ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <Send className="size-4" />
            )}
          </Button>
        </div>
      }
    >
      <div ref={listRef} className="flex min-h-[280px] flex-col gap-3 overflow-y-auto pr-1">
        {items.length === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 py-10 select-none">
            <Sparkles className="size-8 text-foreground-subtlest" />
            <span className="text-ui-sm text-foreground-subtlest">{t('ai.chat.empty')}</span>
          </div>
        ) : (
          items.map((item, index) => (
            <div
              key={index}
              className={
                item.role === 'user'
                  ? 'self-end max-w-[85%] rounded-xl rounded-br-sm bg-primary px-3 py-2 text-ui-sm text-primary-foreground whitespace-pre-wrap break-words'
                  : 'self-start max-w-[92%] rounded-xl rounded-bl-sm border border-border bg-surface px-3 py-2 text-ui-sm text-foreground whitespace-pre-wrap break-words'
              }
            >
              {item.content ||
                (item.streaming ? (
                  <Loader2 className="size-4 animate-spin text-foreground-subtlest" />
                ) : null)}
              {item.error && (
                <TriangleAlert className="mr-1 inline size-3.5 text-destructive align-[-2px]" />
              )}
              {item.role === 'assistant' && !item.streaming && item.content && !item.error && provider && provider.apiKeys.length > 1 && lastKeyIndex !== null && (
                <span className="mt-1.5 block text-ui-xs text-foreground-subtlest">
                  {provider.model} · Key #{lastKeyIndex + 1}
                </span>
              )}
            </div>
          ))
        )}
      </div>

      {items.length > 0 && (
        <button
          type="button"
          onClick={() => setItems([])}
          className="mt-2 self-start text-ui-xs text-foreground-subtlest hover:text-foreground"
        >
          <Plus className="mr-0.5 inline size-3 rotate-45 align-[-2px]" />
          {t('ai.chat.clear')}
        </button>
      )}
    </Modal>
  );
}
