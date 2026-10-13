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
