import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import { terminalSessionRegistry } from './terminalSessionRegistry';
import type {
  ITerminalByteChannel,
  ITerminalTransport,
  TerminalChannelPayload,
  TerminalCreateParams,
  TerminalCreateResult,
  TerminalDisposeParams,
  TerminalResizeParams,
  TerminalWriteParams,
} from './types';

const sharedTextEncoder = new TextEncoder();

/**
 * 将 Rust `Channel<Vec<u8>>` 推送过来的任意字节负载（`number[]` / `ArrayBuffer` / `Uint8Array`）
 * 统一归一化为 `Uint8Array`，供 `xterm.write(bytes)` 直接消费，避免主线程 UTF-8 截断乱码。
 */
export function normalizeChannelBytes(payload: TerminalChannelPayload): Uint8Array {
  if (payload instanceof Uint8Array) {
    return payload;
  }
  if (payload instanceof ArrayBuffer) {
    return new Uint8Array(payload);
  }
  return Uint8Array.from(payload);
}

/**
 * 将 xterm `onData(string)` 的 UTF-8 文本编码为 `Uint8Array`（对应后端 `Vec<u8>`）。
 */
export function encodeTerminalInput(data: string | Uint8Array): Uint8Array {
  if (data instanceof Uint8Array) {
    return data;
  }
  return sharedTextEncoder.encode(data);
}

/**
 * 将 xterm `onBinary(string)` 的二进制字符串转为 `Uint8Array`。
 */
export function encodeBinaryString(binary: string): Uint8Array {
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i) & 0xff;
  }
  return bytes;
}

/**
 * 创建绑定 `Uint8Array` 回调的 `Channel<Vec<u8>>` 实例：
 * - 在 Tauri 环境中直接实例化 `@tauri-apps/api/core` 的 `Channel<TerminalChannelPayload>`；
 * - 在纯浏览器预览或 Mock 模式下提供兼容的 `onmessage` / `sendMockBytes` 通道。
 */
export function createTerminalByteChannel(
  onBytes: (bytes: Uint8Array) => void,
): ITerminalByteChannel {
  const handlePayload = (payload: TerminalChannelPayload) => {
    const bytes = normalizeChannelBytes(payload);
    if (bytes.byteLength > 0) {
      onBytes(bytes);
    }
  };

  if (typeof window !== 'undefined' && isTauri()) {
    const channel = new Channel<TerminalChannelPayload>(handlePayload);
    return Object.assign(channel, {
      sendMockBytes(payload: TerminalChannelPayload) {
        channel.onmessage(payload);
      },
    });
  }

  const fallbackChannel: ITerminalByteChannel = {
    onmessage: handlePayload,
    sendMockBytes(payload: TerminalChannelPayload) {
      fallbackChannel.onmessage(payload);
    },
  };
  return fallbackChannel;
}

/**
 * 计算单个 Unicode 字符在终端中的显示列宽（CJK 宽字符占 2 列，其余占 1 列），
 * 确保 Mock Echo 终端在退格删除中文字符时擦除正确的列数。
 */
function getCharCellWidth(char: string): number {
  const code = char.codePointAt(0);
  if (!code) return 1;
  if (
    code >= 0x1100 &&
    (code <= 0x115f ||
      code === 0x2329 ||
      code === 0x232a ||
      (code >= 0x2e80 && code <= 0xa4cf && code !== 0x303f) ||
      (code >= 0xac00 && code <= 0xd7a3) ||
      (code >= 0xf900 && code <= 0xfaff) ||
      (code >= 0xfe10 && code <= 0xfe19) ||
      (code >= 0xfe30 && code <= 0xfe6f) ||
      (code >= 0xff00 && code <= 0xff60) ||
      (code >= 0xffe0 && code <= 0xffe6) ||
      (code >= 0x20000 && code <= 0x2fffd) ||
      (code >= 0x30000 && code <= 0x3fffd))
  ) {
    return 2;
  }
  return 1;
}

interface MockEchoSession {
  id: number;
  sessionId: number;
  cols: number;
  rows: number;
  cwd: string;
  lineBuffer: string;
  history: string[];
  historyIndex: number;
  decoder: TextDecoder;
  onOutput: ITerminalByteChannel;
  onExit?: (exitCode: number) => void;
  streamTimers: Set<number>;
  disposed: boolean;
}

/**
 * 模拟 Echo 终端传输层（用于无后端阶段测试多 Tab 状态保持、尺寸自适应、主题切换与 `Channel<Vec<u8>>` 字节流）。
 * 所有输出均先编码为 `Uint8Array`（模拟 Rust `Vec<u8>`），再经由 `onOutput` Channel 投递给 `xterm.write(Uint8Array)`。
 */
export class MockEchoTerminalTransport implements ITerminalTransport {
  private sessions = new Map<number, MockEchoSession>();
  private nextId = 1;

  async create(params: TerminalCreateParams): Promise<TerminalCreateResult> {
    const id = this.nextId++;
    const cwd = params.cwd ?? '~';
    const session: MockEchoSession = {
      id,
      sessionId: params.sessionId,
      cols: params.cols,
      rows: params.rows,
      cwd,
      lineBuffer: '',
      history: [],
      historyIndex: -1,
      decoder: new TextDecoder('utf-8'),
      onOutput: params.onOutput,
      onExit: params.onExit,
      streamTimers: new Set(),
      disposed: false,
    };

    this.sessions.set(id, session);

    // 异步首帧模拟 Rust 后端连接就绪后通过 Channel<Vec<u8>> 推送欢迎 Banner 与 Prompt
    queueMicrotask(() => {
      if (session.disposed) return;
      this.emitBanner(session);
      this.emitPrompt(session);
    });

    return {
      id,
      shell: '/bin/zsh (echo-mock)',
      cwd,
    };
  }

  async write({ id, data }: TerminalWriteParams): Promise<void> {
    const session = this.sessions.get(id);
    if (!session || session.disposed) return;

    const text = session.decoder.decode(data, { stream: true });
    if (!text) return;

    this.handleInputText(session, text);
  }

  async resize({ id, cols, rows }: TerminalResizeParams): Promise<void> {
    const session = this.sessions.get(id);
    if (!session || session.disposed) return;
    session.cols = cols;
    session.rows = rows;
  }

  async dispose({ id }: TerminalDisposeParams): Promise<void> {
    const session = this.sessions.get(id);
    if (!session) return;
    session.disposed = true;
    for (const timer of session.streamTimers) {
      window.clearTimeout(timer);
    }
    session.streamTimers.clear();
    this.sessions.delete(id);
  }

  private emitBytes(session: MockEchoSession, bytes: Uint8Array): void {
    if (session.disposed) return;
    if (session.onOutput.sendMockBytes) {
      session.onOutput.sendMockBytes(bytes);
    } else {
      session.onOutput.onmessage(bytes);
    }
  }

  private emitText(session: MockEchoSession, text: string): void {
    this.emitBytes(session, sharedTextEncoder.encode(text));
  }

  private emitBanner(session: MockEchoSession): void {
    const lines = [
      `\x1b[1;36mVeloTerm Mock Echo Shell\x1b[0m \x1b[90m[${session.id} · WebGL2 + Channel<Vec<u8>>]\x1b[0m`,
      `\x1b[90m输入任意内容回车即可回显（Echo）；切换左侧导航或顶部 Tab 会话状态自动保活。\x1b[0m`,
      `\x1b[90m内置命令: \x1b[33mhelp\x1b[90m | \x1b[33mwebgl\x1b[90m | \x1b[33mecho <text>\x1b[90m | \x1b[33mcolors\x1b[90m | \x1b[33msize\x1b[90m | \x1b[33mstream\x1b[90m | \x1b[33mclear\x1b[90m | \x1b[33mexit\x1b[0m`,
      '',
    ];
    this.emitText(session, lines.join('\r\n'));
  }

  private emitPrompt(session: MockEchoSession): void {
    this.emitText(
      session,
      `\x1b[1;32mveloterm\x1b[0m:\x1b[1;34m${session.cwd}\x1b[0m$ `,
    );
  }

  private replaceCurrentLine(session: MockEchoSession, nextLine: string): void {
    // 擦除当前行已输入的字符
    const currentChars = Array.from(session.lineBuffer);
    let cellsToClear = 0;
    for (const ch of currentChars) {
      cellsToClear += getCharCellWidth(ch);
    }
    if (cellsToClear > 0) {
      this.emitText(
        session,
        '\b'.repeat(cellsToClear) + ' '.repeat(cellsToClear) + '\b'.repeat(cellsToClear),
      );
    }
    session.lineBuffer = nextLine;
    if (nextLine) {
      this.emitText(session, nextLine);
    }
  }

  private handleInputText(session: MockEchoSession, input: string): void {
    let i = 0;
    while (i < input.length) {
      // 处理常见 ANSI 转义序列（上下方向键历史记录、左右键忽略）
      if (input[i] === '\x1b') {
        if (input.startsWith('\x1b[A', i)) {
          // Up Arrow
          if (session.history.length > 0) {
            const nextIdx =
              session.historyIndex < 0
                ? session.history.length - 1
                : Math.max(0, session.historyIndex - 1);
            session.historyIndex = nextIdx;
            this.replaceCurrentLine(session, session.history[nextIdx] ?? '');
          }
          i += 3;
          continue;
        }
        if (input.startsWith('\x1b[B', i)) {
          // Down Arrow
          if (session.historyIndex >= 0) {
            const nextIdx = session.historyIndex + 1;
            if (nextIdx >= session.history.length) {
              session.historyIndex = -1;
              this.replaceCurrentLine(session, '');
            } else {
              session.historyIndex = nextIdx;
              this.replaceCurrentLine(session, session.history[nextIdx] ?? '');
            }
          }
          i += 3;
          continue;
        }
        if (input.startsWith('\x1b[C', i) || input.startsWith('\x1b[D', i)) {
          i += 3;
          continue;
        }
        // 跳过其余未知 CSI 控制序列
        const csiMatch = input.slice(i).match(/^\x1b\[[0-9;]*[A-Za-z~]/);
        if (csiMatch) {
          i += csiMatch[0].length;
          continue;
        }
        i += 1;
        continue;
      }

      const codePoint = input.codePointAt(i)!;
      const ch = String.fromCodePoint(codePoint);
      i += ch.length;

      if (ch === '\r' || ch === '\n') {
        this.emitText(session, '\r\n');
        const commandLine = session.lineBuffer;
        session.lineBuffer = '';
        session.historyIndex = -1;
        if (commandLine.trim().length > 0) {
          session.history.push(commandLine);
        }
        this.executeCommand(session, commandLine);
        continue;
      }

      // Ctrl+C
      if (ch === '\x03') {
        session.lineBuffer = '';
        session.historyIndex = -1;
        this.emitText(session, '^C\r\n');
        this.emitPrompt(session);
        continue;
      }

      // Ctrl+L (清屏)
      if (ch === '\x0c') {
        this.emitText(session, '\x1b[2J\x1b[H');
        this.emitPrompt(session);
        if (session.lineBuffer) {
          this.emitText(session, session.lineBuffer);
        }
        continue;
      }

      // Ctrl+U (清除整行)
      if (ch === '\x15') {
        this.replaceCurrentLine(session, '');
        continue;
      }

      // Backspace (\x7f or \b)
      if (ch === '\x7f' || ch === '\x08') {
        const chars = Array.from(session.lineBuffer);
        const removed = chars.pop();
        if (removed) {
          session.lineBuffer = chars.join('');
          const width = getCharCellWidth(removed);
          this.emitText(
            session,
            '\b'.repeat(width) + ' '.repeat(width) + '\b'.repeat(width),
          );
        }
        continue;
      }

      // 忽略其余不可见控制字符（保留 Tab 转为 2 空格）
      if (ch === '\t') {
        session.lineBuffer += '  ';
        this.emitText(session, '  ');
        continue;
      }

      if (codePoint < 0x20) {
        continue;
      }

      session.lineBuffer += ch;
      this.emitText(session, ch);
    }
  }

  private executeCommand(session: MockEchoSession, rawLine: string): void {
    const trimmed = rawLine.trim();
    if (!trimmed) {
      this.emitPrompt(session);
      return;
    }

    const [cmd, ...rest] = trimmed.split(/\s+/);
    const argText = trimmed.slice(cmd.length).trim();

    switch (cmd.toLowerCase()) {
      case 'help': {
        const helpLines = [
          '\x1b[1m可用测试指令 (Mock Channel<Vec<u8>>):\x1b[0m',
          '  \x1b[33m<任意文本>\x1b[0m     直接原样回显输入内容（验证 UTF-8 字节流）',
          '  \x1b[33mwebgl\x1b[0m          查看当前终端 WebGL2 硬件加速渲染器状态',
          '  \x1b[33mecho <text>\x1b[0m    输出指定文本',
          '  \x1b[33mcolors\x1b[0m         展示 ANSI 16 色调色板（可切换深浅色主题实时对比）',
          '  \x1b[33msize\x1b[0m           查看当前终端 cols × rows 尺寸',
          '  \x1b[33mstream\x1b[0m         模拟分片二进制流推送（Channel<Vec<u8>> 连续帧）',
          '  \x1b[33mclear\x1b[0m          清空屏幕（或使用快捷键 Ctrl+L）',
          '  \x1b[33mexit\x1b[0m           模拟进程退出并触发当前 Tab 关闭逻辑',
        ];
        this.emitText(session, `${helpLines.join('\r\n')}\r\n`);
        this.emitPrompt(session);
        return;
      }

      case 'webgl': {
        const isActive = terminalSessionRegistry.isWebglActive(session.sessionId);
        this.emitText(
          session,
          `\x1b[36mRenderer:\x1b[0m ${
            isActive
              ? '\x1b[32m@xterm/addon-webgl (WebGL2 Hardware Accelerated)\x1b[0m'
              : '\x1b[33mDOM Fallback\x1b[0m'
          }\r\n`,
        );
        this.emitPrompt(session);
        return;
      }

      case 'clear': {
        this.emitText(session, '\x1b[2J\x1b[H');
        this.emitPrompt(session);
        return;
      }

      case 'size': {
        this.emitText(
          session,
          `\x1b[36mcols:\x1b[0m ${session.cols}  \x1b[36mrows:\x1b[0m ${session.rows}  \x1b[90m(session: ${session.id})\x1b[0m\r\n`,
        );
        this.emitPrompt(session);
        return;
      }

      case 'colors':
      case 'ansi': {
        const normalRow = Array.from({ length: 8 }, (_, idx) => `\x1b[4${idx}m   \x1b[0m`).join(' ');
        const brightRow = Array.from({ length: 8 }, (_, idx) => `\x1b[10${idx}m   \x1b[0m`).join(' ');
        const fgSamples = [
          '\x1b[31mRed\x1b[0m',
          '\x1b[32mGreen\x1b[0m',
          '\x1b[33mYellow\x1b[0m',
          '\x1b[34mBlue\x1b[0m',
          '\x1b[35mMagenta\x1b[0m',
          '\x1b[36mCyan\x1b[0m',
        ].join('  ');
        this.emitText(
          session,
          `ANSI Normal : ${normalRow}\r\nANSI Bright : ${brightRow}\r\nForeground  : ${fgSamples}\r\n`,
        );
        this.emitPrompt(session);
        return;
      }

      case 'stream': {
        const total = 5;
        let step = 0;
        this.emitText(session, `\x1b[90m[Channel<Vec<u8>> streaming ${total} chunks...]\x1b[0m\r\n`);
        const tick = () => {
          if (session.disposed) return;
          step += 1;
          const bar = '█'.repeat(step * 4) + '░'.repeat((total - step) * 4);
          this.emitText(
            session,
            `\r\x1b[36mchunk #${step}/${total}\x1b[0m [${bar}] ${step * 20}%`,
          );
          if (step < total) {
            const timer = window.setTimeout(() => {
              session.streamTimers.delete(timer);
              tick();
            }, 180);
            session.streamTimers.add(timer);
          } else {
            this.emitText(session, ' \x1b[32m✓ done\x1b[0m\r\n');
            this.emitPrompt(session);
          }
        };
        tick();
        return;
      }

      case 'exit': {
        this.emitText(session, 'logout\r\n');
        session.disposed = true;
        this.sessions.delete(session.id);
        session.onExit?.(0);
        return;
      }

      case 'echo': {
        this.emitText(session, `${argText || rest.join(' ')}\r\n`);
        this.emitPrompt(session);
        return;
      }

      default: {
        // 默认 Echo 模式：将用户输入编码为 Vec<u8> 字节流回显
        const byteLen = sharedTextEncoder.encode(rawLine).byteLength;
        this.emitText(
          session,
          `\x1b[32m[echo]\x1b[0m ${rawLine} \x1b[90m(${byteLen} bytes)\x1b[0m\r\n`,
        );
        this.emitPrompt(session);
      }
    }
  }
}

/**
 * Tauri Rust 后端 `Channel<Vec<u8>>` 传输实现：
 * 同时承载本地 `portable-pty` 进程与未来的远程 `russh` 会话。
 */
export class TauriChannelTerminalTransport implements ITerminalTransport {
  async create(params: TerminalCreateParams): Promise<TerminalCreateResult> {
    const onExit = new Channel<number>((exitCode) => {
      params.onExit?.(exitCode);
    });

    return invoke<TerminalCreateResult>('terminal_create', {
      request: {
        sessionId: params.sessionId,
        cols: params.cols,
        rows: params.rows,
        cwd: params.cwd,
        hostId: params.hostId,
        target: params.target,
      },
      onOutput: params.onOutput,
      onExit,
    });
  }

  async write({ id, data }: TerminalWriteParams): Promise<void> {
    await invoke('terminal_write', {
      id,
      data: Array.from(data),
    });
  }

  async resize({ id, cols, rows }: TerminalResizeParams): Promise<void> {
    await invoke('terminal_resize', {
      id,
      cols,
      rows,
    });
  }

  async dispose({ id }: TerminalDisposeParams): Promise<void> {
    await invoke('terminal_dispose', { id });
  }
}

/**
 * 自适应传输层：
 * - 在 Tauri 桌面环境中自动走 `TauriChannelTerminalTransport`（调用后端 `portable-pty` / 未来的 `russh`）；
 * - 在纯浏览器开发预览（无 Tauri IPC）时自动回退到 `MockEchoTerminalTransport`。
 */
class AdaptiveTerminalTransport implements ITerminalTransport {
  private readonly tauriTransport = new TauriChannelTerminalTransport();
  private readonly mockTransport = new MockEchoTerminalTransport();

  private active(): ITerminalTransport {
    return typeof window !== 'undefined' && isTauri()
      ? this.tauriTransport
      : this.mockTransport;
  }

  create(params: TerminalCreateParams): Promise<TerminalCreateResult> {
    return this.active().create(params);
  }

  write(params: TerminalWriteParams): Promise<void> {
    return this.active().write(params);
  }

  resize(params: TerminalResizeParams): Promise<void> {
    return this.active().resize(params);
  }

  dispose(params: TerminalDisposeParams): Promise<void> {
    return this.active().dispose(params);
  }
}

/** 全局默认终端传输层实例 */
export const defaultTerminalTransport: ITerminalTransport = new AdaptiveTerminalTransport();
