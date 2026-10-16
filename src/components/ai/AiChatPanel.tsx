import { Loader2, Play, Send, Settings, Sparkles, SquareTerminal, TriangleAlert, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import {
  aiAgentChat,
  listAiProviders,
  reportAiToolResult,
  type AiChatMessage,
  type AiProvider,
} from '@/services/aiService';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { ToolCallCard } from '@/components/ai/ToolCallCard';
import { MarkdownContent } from '@/components/ai/MarkdownContent';
import { useTerminalStore } from '@/terminal/TerminalProvider';
import {
  executeAgentTool,
  getActiveTerminalText,
  readTerminalOutput,
  sendToTerminal,
} from '@/services/aiTerminalTools';

interface ChatItem {
  role: 'user' | 'assistant';
  content: string;
  streaming?: boolean;
  error?: boolean;
  /** AI 正在执行终端工具（显示为工具调用卡片） */
  toolCall?: { name: string; arguments: string; result?: string; running?: boolean };
}

interface AiChatPanelProps {
  open: boolean;
  /** 当前是否在终端页：AI 问答入口仅在终端页展示 */
  isTerminalPage?: boolean;
  /** 面板宽度（px，由 AppShell 拖拽管理） */
  width: number;
  /** 拖拽中禁用宽度过渡 */
  isResizing?: boolean;
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
export function AiChatPanel({ open, width: _width, isResizing = false, isTerminalPage = false, onClose, initialPrompt, onOpenSettings }: AiChatPanelProps) {
  const { t } = useTranslation();
  const [items, setItems] = useState<ChatItem[]>([]);
  const [input, setInput] = useState('');
  const [isStreaming, setIsStreaming] = useState(false);
  const [providers, setProviders] = useState<AiProvider[]>([]);
  const [providerId, setProviderId] = useState<number | null>(null);
  const [lastKeyIndex, setLastKeyIndex] = useState<number | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const terminalStore = useTerminalStore();
  const [toolResult, setToolResult] = useState<string | null>(null);
  const [isToolRunning, setIsToolRunning] = useState<string | null>(null);

  const activeTerminalKey = terminalStore.activeSessionId;

  // ── AI 终端工具（系统提示词描述的能力，由面板上的快捷按钮触发）──

  async function runToolGetScreen() {
    if (activeTerminalKey == null) return;
    setIsToolRunning('getScreen');
    try {
      const text = await getActiveTerminalText(activeTerminalKey, { lines: 120 });
      setToolResult(text);
    } finally {
      setIsToolRunning(null);
    }
  }

  async function runToolRunCommand() {
    const command = input.trim();
    if (!command || activeTerminalKey == null) return;
    setIsToolRunning('runCommand');
    try {
      const result = await sendToTerminal(activeTerminalKey, `${command}
`);
      if (result.ok) {
        setInput('');
        setToolResult(`[已发送命令: ${command}]

等待输出中…`);
        const output = await readTerminalOutput(activeTerminalKey, { waitForMs: 2500 });
        setToolResult(`[已发送命令: ${command}]

${output}`);
      } else {
        setToolResult(`[发送失败: ${result.error}]`);
      }
    } finally {
      setIsToolRunning(null);
    }
  }
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
    // 只 push user 气泡；assistant 文本气泡由 updateAssistant 在首个 delta 到达时自动创建，
    // 避免预建的空气泡在工具调用场景下无人写入而永远转圈。
    setItems((prev) => [...prev, { role: 'user', content }]);

    const updateAssistant = (fn: (current: ChatItem) => ChatItem) => {
      setItems((prev) => {
        const next = [...prev];
        const last = next[next.length - 1];
        // 最后一条是纯文本 assistant（无 toolCall）才更新；
        // 工具卡片后到达的文本 delta 需要新开一条文本气泡
        if (last?.role === 'assistant' && !last.toolCall) {
          next[next.length - 1] = fn(last);
        } else {
          next.push({
            role: 'assistant',
            content: '',
            streaming: true,
          });
          // 建新条后再应用 fn（把 delta 写入新条）
          const created = next[next.length - 1];
          next[next.length - 1] = fn(created);
        }
        return next;
      });
    };

    try {
      // 智能体模式：AI 可自主调用终端工具（读屏/执行命令/读输出），
      // 工具调用经 Channel 推到前端执行，结果回传后 AI 继续推理，直到给出纯文本。
      await aiAgentChat(
        {
          providerId,
          history,
          preamble: buildSystemPreamble(isTerminalPage, activeTerminalKey),
        },
        async (chunk) => {
          if (chunk.keyIndex !== null && chunk.keyIndex !== undefined) {
            setLastKeyIndex(chunk.keyIndex);
          }
          if (chunk.delta) {
            updateAssistant((current) => ({
              ...current,
              content: current.content + chunk.delta,
            }));
          }
          if (chunk.toolCall && activeTerminalKey != null) {
            const { callId, name, arguments: args } = chunk.toolCall;
            // 追加工具调用卡片（running 态）
            setItems((prev) => [
              ...prev,
              {
                role: 'assistant' as const,
                content: '',
                toolCall: {
                  name,
                  arguments: JSON.stringify(args, null, 2),
                  running: true,
                },
              },
            ]);
            // 前端执行工具并回传结果
            const result = await executeAgentTool(
              activeTerminalKey,
              name,
              (args ?? {}) as Record<string, unknown>,
            );
            setItems((prev) => {
              const next = [...prev];
              for (let i = next.length - 1; i >= 0; i--) {
                const item = next[i];
                if (item && item.toolCall && item.toolCall.running && item.toolCall.name === name) {
                  next[i] = {
                    ...item,
                    toolCall: { ...item.toolCall, result, running: false },
                  };
                  break;
                }
              }
              return next;
            });
            void reportAiToolResult(callId, result);
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
      // 兜底清掉所有残留的 streaming 标记（工具卡片 running + 文本气泡 streaming）
      setItems((prev) =>
        prev.map((item) =>
          item.toolCall
            ? { ...item, toolCall: { ...item.toolCall, running: false } }
            : { ...item, streaming: false },
        ),
      );
      setIsStreaming(false);
    }
  }

  const provider = providers.find((p) => p.id === providerId);

  return (
    <aside
      aria-label={t('ai.chat.title')}
      className={cn(
        'ml-1 flex h-full w-(--ai-panel-width) shrink-0 flex-col overflow-hidden rounded-(--workspace-panel-radius) border border-border bg-background transition-[width,opacity] duration-200 ease-out',
        !open && 'pointer-events-none border-transparent opacity-0',
        isResizing && 'transition-opacity',
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
          {isTerminalPage ? (
            <ControlHintTooltip title={t('ai.tools.getScreen')} side="top">
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={() => void runToolGetScreen()}
                disabled={activeTerminalKey == null || isToolRunning !== null}
                aria-label={t('ai.tools.getScreen')}
              >
                <SquareTerminal className={cn('size-3.5', isToolRunning === 'getScreen' && 'animate-pulse')} />
              </Button>
            </ControlHintTooltip>
          ) : null}
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
            <ControlHintTooltip title={t('ai.provider.title')} side="top">
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={onOpenSettings}
                aria-label={t('ai.provider.title')}
              >
                <Settings className="size-3.5" />
              </Button>
            </ControlHintTooltip>
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

      {/* 工具执行结果（最近一次，可带入对话） */}
      {toolResult ? (
        <div className="mx-2 mt-2 shrink-0 rounded-md border border-border bg-surface/60">
          <div className="flex items-center justify-between border-b border-border/50 px-2 py-1">
            <span className="text-ui-xs font-medium text-foreground-subtle select-none">
              {t('ai.tools.result')}
            </span>
            <div className="flex items-center gap-1">
              <button
                type="button"
                onClick={() => {
                  setToolResult(null);
                  setInput(`分析这段终端输出：\n\n\${toolResult.slice(0, 2000)}`);
                }}
                className="text-ui-xs text-foreground-subtle hover:text-foreground"
              >
                {t('ai.tools.analyze')}
              </button>
              <button
                type="button"
                onClick={() => setToolResult(null)}
                aria-label={t('common.close')}
                className="text-foreground-subtlest hover:text-foreground"
              >
                <X className="size-3" />
              </button>
            </div>
          </div>
          <pre className="max-h-32 overflow-auto px-2 py-1 font-mono text-ui-xs text-foreground-subtle whitespace-pre-wrap break-all">
            {toolResult}
          </pre>
        </div>
      ) : null}

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
          items.map((item, index) =>
            item.toolCall ? (
              <ToolCallCard key={index} toolCall={item.toolCall} />
            ) : (
            <div
              key={index}
              className={
                item.role === 'user'
                  ? 'self-end max-w-[88%] rounded-xl rounded-br-sm bg-primary px-2.5 py-1.5 text-ui-sm text-primary-foreground whitespace-pre-wrap break-words'
                  : 'self-start max-w-full rounded-xl rounded-bl-sm border border-border bg-surface px-3 py-2 text-ui-sm'
              }
            >
              {item.role === 'assistant' && item.content ? (
                <MarkdownContent content={item.content} />
              ) : (
                item.content ||
                (item.streaming ? (
                  <Loader2 className="size-4 animate-spin text-foreground-subtlest" />
                ) : null)
              )}
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
            ),
          )
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
        {isTerminalPage ? (
          <ControlHintTooltip title={t('ai.tools.runCommand')} side="top">
            <Button
              variant="outline"
              size="icon-md"
              disabled={isToolRunning !== null || !input.trim() || activeTerminalKey == null}
              onClick={() => void runToolRunCommand()}
              aria-label={t('ai.tools.runCommand')}
              className="shrink-0"
            >
              <Play className={cn('size-3.5', isToolRunning === 'runCommand' && 'animate-pulse')} />
            </Button>
          </ControlHintTooltip>
        ) : null}
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


/** 系统提示词：声明 AI 可用的终端工具能力，用户可通过面板快捷按钮代为执行 */
function buildSystemPreamble(isTerminalPage: boolean, activeTerminalKey: number | null): string {
  const base =
    '你是 VeloTerm 桌面终端应用的 AI 助手，擅长 Linux/SSH/运维问题排查。回答简洁、直接给可执行的命令或步骤。';
  if (!isTerminalPage || activeTerminalKey == null) {
    return base;
  }
  return [
    base,
    '',
    '## 可用终端工具（你可自主调用，多轮执行直到解决问题）',
    '- `terminal_get_screen(lines?, visible_only?)`: 读取当前终端屏幕与滚动缓冲',
    '- `terminal_send(input, press_enter?)`: 向终端发送输入并执行命令',
    '- `terminal_read_new(wait_ms?)`: 等待并读取终端新输出',
    '',
    '排查流程：先 get_screen 看现场 → 需要验证时 send 执行诊断命令 → read_new 或再次 get_screen 读结果 → 继续分析，直到定位问题给出结论。',
    '安全注意：不要主动执行 rm -rf、dd、mkfs、shutdown 等破坏性命令；重启服务等敏感操作先说明影响再执行。',
  ].join('\n');
}
