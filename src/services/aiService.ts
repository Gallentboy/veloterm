import { Channel, invoke } from '@tauri-apps/api/core';

/** AI 供应商配置（与 Rust AiProviderRecord 对齐，camelCase） */
export interface AiProvider {
  id: number;
  name: string;
  /** 'openai' | 'anthropic' | 'gemini' */
  providerType: 'openai' | 'anthropic' | 'gemini' | string;
  baseUrl: string | null;
  model: string;
  /** 多 Key 轮询池 */
  apiKeys: string[];
  isDefault: boolean;
  createdAt: number;
  updatedAt: number;
}

/** Channel 推送的流式事件（与 Rust AiChunk 对齐） */
export interface AiChunk {
  delta: string | null;
  keyIndex: number | null;
  error: string | null;
  done: boolean;
}

export interface AiChatMessage {
  role: 'user' | 'assistant';
  content: string;
}

export function listAiProviders(): Promise<AiProvider[]> {
  return invoke<AiProvider[]>('ai_provider_list');
}

export function saveAiProvider(record: AiProvider): Promise<void> {
  return invoke<void>('ai_provider_save', { record });
}

export function deleteAiProvider(id: number): Promise<void> {
  return invoke<void>('ai_provider_delete', { id });
}

/**
 * 统一流式问答：增量文本经 onChunk 回调推送，多 Key 由后端轮询。
 * 返回的 Promise 在流结束时 resolve（错误时 reject）。
 */
export function aiChat(
  options: {
    providerId?: number | null;
    history: AiChatMessage[];
    preamble?: string | null;
  },
  onChunk: (chunk: AiChunk) => void,
): Promise<void> {
  const channel = new Channel<AiChunk>();
  channel.onmessage = onChunk;
  return invoke<void>('ai_chat', {
    providerId: options.providerId ?? null,
    history: options.history,
    preamble: options.preamble ?? null,
    onChunk: channel,
  });
}

/** 供应商连通性测试（流式返回首个增量或错误） */
export function testAiProvider(
  record: AiProvider,
  onChunk: (chunk: AiChunk) => void,
): Promise<void> {
  const channel = new Channel<AiChunk>();
  channel.onmessage = onChunk;
  return invoke<void>('ai_provider_test', { record, onChunk: channel });
}

/** 列出供应商可用模型 */
export function listAiModels(record: AiProvider): Promise<string[]> {
  return invoke<string[]>('ai_list_models', { record });
}

/** 智能体模式 Channel 事件（与 Rust AgentChunk 对齐） */
export interface AgentChunk {
  delta: string | null;
  keyIndex: number | null;
  error: string | null;
  done: boolean;
  /** 非空 = AI 请求执行终端工具，前端执行后须回传 ai_tool_result */
  toolCall: { callId: string; name: string; arguments: unknown } | null;
}

/** 智能体模式对话：AI 自主调用终端工具（读屏/执行/读输出），多轮循环 */
export function aiAgentChat(
  options: {
    providerId?: number | null;
    history: AiChatMessage[];
    preamble?: string | null;
  },
  onChunk: (chunk: AgentChunk) => void,
): Promise<void> {
  const channel = new Channel<AgentChunk>();
  channel.onmessage = onChunk;
  return invoke<void>('ai_agent_chat', {
    providerId: options.providerId ?? null,
    history: options.history,
    preamble: options.preamble ?? null,
    onChunk: channel,
  });
}

/** 工具执行结果回传（唤醒 Rust 侧挂起的 oneshot） */
export function reportAiToolResult(callId: string, result: string): Promise<boolean> {
  return invoke<boolean>('ai_tool_result', { callId, result });
}
