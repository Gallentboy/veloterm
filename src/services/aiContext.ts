/**
 * AI 错误上下文：终端/SSH 故障时暂存现场，供「问 AI」一键带入。
 * 模块级单例 + 订阅（与 transferQueue 同模式），不进 React 树。
 */

export interface AiErrorContext {
  /** 概要标题，如「SSH 终端启动失败」 */
  title: string;
  /** 组装好的提问正文（含目标主机与错误详情） */
  detail: string;
  /** 上下文产生时间（ms），浮层按新鲜度决定是否展示 */
  occurredAt: number;
}

let current: AiErrorContext | null = null;
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) {
    listener();
  }
}

export function setAiErrorContext(title: string, detail: string): void {
  current = { title, detail, occurredAt: Date.now() };
  emit();
}

export function clearAiErrorContext(): void {
  if (!current) return;
  current = null;
  emit();
}

export function getAiErrorContext(): AiErrorContext | null {
  return current;
}

export function subscribeAiErrorContext(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** SSH 终端创建失败的标准化提问模板 */
export function buildSshErrorPrompt(options: {
  hostLabel: string;
  errorMessage: string;
}): string {
  return [
    `SSH 终端启动失败，帮我排查。`,
    ``,
    `连接目标：${options.hostLabel}`,
    `错误信息：${options.errorMessage}`,
    ``,
    `请按可能性从高到低给出原因分析，并给出每一步的验证命令。`,
  ].join('\n');
}
