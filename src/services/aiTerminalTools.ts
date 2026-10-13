/**
 * AI 终端工具桥：让 AI 问答能感知/操作终端。
 *
 * 三个工具（对齐 AI 系统提示词中的能力描述）：
 * 1. `getActiveTerminalText`   读取当前激活终端的可见屏幕 + scrollback（截断）
 * 2. `sendToTerminal`          向当前激活终端写入内容（等价键盘输入，支持 \r 回车、\x03 Ctrl-C）
 * 3. `readTerminalOutput`      延时读取终端新输出（send 后等 N ms 收集增量，捕获命令回显）
 *
 * 数据源是 `terminalSessionRegistry`（xterm 实例常驻），写路径复用 TerminalSession
 * 同款 transport（自适应 Tauri/Mock），不直接操纵 DOM。
 */

import type { Terminal } from '@xterm/xterm';
import { encodeTerminalInput } from '@/terminal/transport';
import { terminalSessionRegistry } from '@/terminal/terminalSessionRegistry';

/** 等待一会话的 terminalId 就绪（transport.create 是异步的） */
function awaitTerminalId(key: number, timeoutMs = 4000): Promise<number | null> {
  return new Promise((resolve) => {
    const started = Date.now();
    const timer = window.setInterval(() => {
      const id = terminalSessionRegistry.getTerminalId(key);
      if (id != null || Date.now() - started > timeoutMs) {
        window.clearInterval(timer);
        resolve(id);
      }
    }, 50);
  });
}

function readBuffer(term: Terminal, opts: { lines: number; includeScrollback: boolean }): string {
  const buffer = term.buffer.active;
  const total = buffer.length;
  const viewportRows = term.rows;
  const start = opts.includeScrollback
    ? Math.max(0, total - opts.lines)
    : Math.max(0, total - viewportRows);
  const end = Math.min(total, start + opts.lines);
  const out: string[] = [];
  for (let i = start; i < end; i++) {
    const line = buffer.getLine(i);
    if (line) {
      out.push(line.translateToString(true));
    }
  }
  return out.join('\n');
}

/** 工具 1：获取当前激活终端内容（可见屏 + scrollback 截断）。 */
export async function getActiveTerminalText(
  sessionKey: number,
  options: { lines?: number; visibleOnly?: boolean } = {},
): Promise<string> {
  const entry = terminalSessionRegistry.get(sessionKey);
  if (!entry) {
    return '[没有活跃的终端会话]';
  }
  const lines = Math.max(1, Math.min(options.lines ?? 200, 2000));
  const text = readBuffer(entry.term, {
    lines,
    includeScrollback: !options.visibleOnly,
  });
  if (!text.trim()) {
    return '[终端缓冲区为空]';
  }
  const truncated =
    text.length > 24000 ? `${text.slice(-24000)}\n[…内容过长，仅保留最后 24000 字符]` : text;
  return truncated;
}

/** 工具 2：向当前激活终端发送输入（\r 回车、\x03 Ctrl-C 等控制字符原样传递）。 */
export async function sendToTerminal(
  sessionKey: number,
  input: string,
): Promise<{ ok: boolean; error?: string }> {
  const terminalId = await awaitTerminalId(sessionKey);
  if (terminalId == null) {
    return { ok: false, error: '终端后端会话尚未就绪' };
  }
  const { defaultTerminalTransport: transport } = await import('@/terminal/transport');
  try {
    const bytes = encodeTerminalInput(input);
    await transport.write({ id: terminalId, data: bytes });
    return { ok: true };
  } catch (error) {
    return { ok: false, error: String(error) };
  }
}

/** 工具 3：发送后延时读取终端新输出（默认等 2s，捕获命令执行回显）。 */
export async function readTerminalOutput(
  sessionKey: number,
  options: { waitForMs?: number } = {},
): Promise<string> {
  const entry = terminalSessionRegistry.get(sessionKey);
  if (!entry) {
    return '[没有活跃的终端会话]';
  }
  const before = entry.term.buffer.active.length;
  const waitForMs = Math.max(200, Math.min(options.waitForMs ?? 2000, 10000));
  await new Promise((resolve) => setTimeout(resolve, waitForMs));
  const buffer = entry.term.buffer.active;
  const total = buffer.length;
  // 只取增量行；若缓冲被清屏重置（total < before）则全读可见区
  const start = total >= before ? before : Math.max(0, total - entry.term.rows);
  const out: string[] = [];
  for (let i = start; i < total; i++) {
    const line = buffer.getLine(i);
    if (line) {
      out.push(line.translateToString(true));
    }
  }
  const text = out.join('\n').trimEnd();
  return text || '[等待期间没有新输出]';
}

/** 供系统提示词注入的当前激活会话描述 */
export function describeActiveSession(sessionKey: number): string {
  const entry = terminalSessionRegistry.get(sessionKey);
  if (!entry) {
    return '';
  }
  return `当前激活终端（cwd: ${entry.cwd || '未知'}）`;
}


/**
 * 智能体工具执行入口：按 Rust 推来的工具名分发执行，返回文本结果。
 * 工具 schema 与 Rust 侧 terminal_tool_definitions 一一对应。
 */
export async function executeAgentTool(
  sessionKey: number,
  name: string,
  args: Record<string, unknown>,
): Promise<string> {
  switch (name) {
    case 'terminal_get_screen': {
      const lines = typeof args.lines === 'number' ? args.lines : 120;
      const visibleOnly = args.visibleOnly === true;
      return getActiveTerminalText(sessionKey, { lines, visibleOnly });
    }
    case 'terminal_send': {
      const input = typeof args.input === 'string' ? args.input : '';
      const pressEnter = args.pressEnter !== false;
      if (!input) {
        return '[input 参数为空]';
      }
      const result = await sendToTerminal(sessionKey, pressEnter ? `${input}\r` : input);
      return result.ok ? `[已发送 ${input.length} 字节]` : `[发送失败: ${result.error}]`;
    }
    case 'terminal_read_new': {
      const raw = (args.wait_ms ?? args.waitMs) as number | undefined;
      const waitMs = typeof raw === 'number' ? raw : 2000;
      return readTerminalOutput(sessionKey, { waitForMs: waitMs });
    }
    default:
      return `[未知工具: ${name}]`;
  }
}
