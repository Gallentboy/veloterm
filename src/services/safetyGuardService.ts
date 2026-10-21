import { invoke } from '@tauri-apps/api/core';
import type { HostEnvironmentLevel } from '../hosts/types';

export type DangerLevel = 'safe' | 'warning' | 'high' | 'critical';

export interface SafetyInspectResult {
  level: DangerLevel;
  matchedRule?: string | null;
  ruleTitle?: string | null;
  explanation?: string | null;
  offendingCommand?: string | null;
  requireVerificationCode: boolean;
}

/**
 * 调用 Rust 安全分析引擎 inspect_command_safety。
 * 仅在 staging/prod 环境下进行高危检测；dev/test 快速返回 safe。
 * `isClean`: 是否来自 OSC 133 语义化坐标切片（若为 true 则在后端跳过 Prompt 剥离直飞 AST）。
 */
export async function inspectCommandSafety(
  command: string,
  hostEnv: HostEnvironmentLevel = 'dev',
  isClean: boolean = false,
): Promise<SafetyInspectResult> {
  const env = (hostEnv || 'dev').toLowerCase();
  if (env !== 'prod' && env !== 'staging') {
    return {
      level: 'safe',
      requireVerificationCode: false,
    };
  }

  try {
    return await invoke<SafetyInspectResult>('inspect_command_safety', {
      command,
      hostEnv: env,
      isClean,
    });
  } catch (err) {
    console.error('[SafetyGuard] inspect_command_safety failed:', err);
    // 降级策略：分析接口异常时不阻塞用户正常执行
    return {
      level: 'safe',
      requireVerificationCode: false,
    };
  }
}

/**
 * 终端行输入跟踪器（SafetyInputTracker）。
 * 跟踪用户键盘敲入、退格、清行控制符，在用户键入回车时提取待分析的整行命令。
 */
export class SafetyInputTracker {
  private buffer = '';

  /**
   * 喂入 xterm 的 onData 数据片段。
   * 返回是否命中了回车提交点（isEnter = true 时，提供提取的完整命令文本）。
   */
  feed(data: string): { isEnter: boolean; commandToInspect?: string } {
    // 1. 回车符检测 (\r 或 \n)
    if (data.includes('\r') || data.includes('\n')) {
      // 提取回车前的内容
      const line = this.buffer + data.replace(/[\r\n]+$/, '');
      this.buffer = ''; // 提交后清空
      return {
        isEnter: true,
        commandToInspect: line.trim(),
      };
    }

    // 2. 控制字符：退格 Backspace (\x7f 或 \x08)
    if (data === '\x7f' || data === '\x08') {
      this.buffer = this.buffer.slice(0, -1);
      return { isEnter: false };
    }

    // 3. 控制字符：Ctrl+C (\x03) 或 Ctrl+U (\x15)
    if (data === '\x03' || data === '\x15') {
      this.buffer = '';
      return { isEnter: false };
    }

    // 4. 普通可打印字符或转义序列（忽略方向键等控制序列 \x1b...）
    if (!data.startsWith('\x1b')) {
      this.buffer += data;
    }

    return { isEnter: false };
  }

  /** 手动清空当前输入缓存 */
  clear(): void {
    this.buffer = '';
  }

  /** 获取当前未提交的行内容（只读） */
  current(): string {
    return this.buffer;
  }
}

export interface Osc133TrackerState {
  hasOsc133: boolean;
  commandStartX: number;
  commandStartY: number;
  inCommand: boolean;
}

export interface ExtractedTerminalCommand {
  command: string;
  fromOsc133: boolean;
}

/**
 * 直接从 xterm.js 的当前屏幕缓冲区读取光标所在行（含换行拼接），
 * 提取当前行的真实终端文本，直接送往 Rust 后端进行 AST 语法树安全分析。
 *
 * 双模式引擎：
 * 1. 语义化模式 (OSC 133): 若检测到远端 Shell 的 `OSC 133;B`（Prompt 结束 / 命令行开始），
 *    从记录的绝对坐标 (commandStartX, commandStartY) 物理切片提取，精度 100%，直飞语法树！
 * 2. 启发式模式 (Fallback): 若未注入 OSC 133，向上寻找折行拼接整行，交由 Rust 端逆向剥离 Prompt。
 */
export function extractCommandLineFromTerminal(
  term: {
    buffer: {
      active: {
        cursorX: number;
        cursorY: number;
        baseY: number;
        getLine: (y: number) => { isWrapped?: boolean; translateToString: (trimRight?: boolean, startColumn?: number, endColumn?: number) => string } | undefined;
      };
    };
  },
  oscTracker?: Osc133TrackerState | null,
): ExtractedTerminalCommand {
  try {
    const buffer = term.buffer.active;
    const cursorY = buffer.cursorY + buffer.baseY;

    // 1. 若有活跃的 OSC 133;B 坐标，直接进行精准像素切片
    if (oscTracker?.hasOsc133 && oscTracker.commandStartY <= cursorY) {
      let fullLine = '';
      for (let y = oscTracker.commandStartY; y <= cursorY; y++) {
        const line = buffer.getLine(y);
        if (line) {
          if (y === oscTracker.commandStartY) {
            fullLine += line.translateToString(true, oscTracker.commandStartX);
          } else {
            fullLine += line.translateToString(true);
          }
        }
      }
      return { command: fullLine.trim(), fromOsc133: true };
    }

    // 2. Fallback 通道：向上寻找起始行（支持超长命令被 xterm 自动折行 isWrapped）
    let startY = cursorY;
    while (startY > 0 && buffer.getLine(startY)?.isWrapped) {
      startY--;
    }

    let fullLine = '';
    for (let y = startY; y <= cursorY; y++) {
      const line = buffer.getLine(y);
      if (line) {
        fullLine += line.translateToString(true);
      }
    }

    return { command: fullLine.trim(), fromOsc133: false };
  } catch (err) {
    console.warn('[SafetyGuard] extractCommandLineFromTerminal failed:', err);
    return { command: '', fromOsc133: false };
  }
}

/**
 * 前端兼容性辅助函数，Prompt 剥离工作已完全收敛至 Rust 后端
 */
export function cleanPrompt(text: string): string {
  return text.trim();
}

