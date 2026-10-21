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
 */
export async function inspectCommandSafety(
  command: string,
  hostEnv: HostEnvironmentLevel = 'dev',
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

/**
 * 剥除终端行首的 Shell Prompt 提示符。
 * 支持常见 root / user 提示符形式：
 * - "[root@host ~]# rm -rf /"
 * - "user@host:~$ rm -rf /"
 * - "macbook% rm -rf /"
 * - "bash-5.1$ rm -rf /"
 */
export function cleanPrompt(text: string): string {
  const trimmed = text.trim();
  if (!trimmed) return '';

  // 1. 匹配带空格的提示符：以 # 、$ 、% 、> 结尾且后面跟空格
  const promptMatch = trimmed.match(/^.*?[#$%>]\s+(.*)$/s);
  if (promptMatch && promptMatch[1] !== undefined) {
    return promptMatch[1].trim();
  }

  // 2. 匹配没有空格但后面紧跟命令字符的极端提示符 (如 root#rm)
  const promptMatchNoSpace = trimmed.match(/^.*?[#$%>]([a-zA-Z0-9_\-./].*)$/s);
  if (promptMatchNoSpace && promptMatchNoSpace[1]) {
    return promptMatchNoSpace[1].trim();
  }

  return trimmed;
}

/**
 * 直接从 xterm.js 的当前屏幕缓冲区读取光标所在行（含换行拼接），
 * 并剥离 Prompt 提示符，得到用户当前行真正准备执行的命令行。
 *
 * 核心优势：
 * 不论用户是直接键盘敲入、按 ⬆️/⬇️ 调出历史命令、按 Tab 自动补全、
 * 还是用鼠标粘贴，屏幕缓冲区永远是绝对可靠的真实屏幕内容。
 */
export function extractCommandLineFromTerminal(term: {
  buffer: {
    active: {
      cursorY: number;
      baseY: number;
      getLine: (y: number) => { isWrapped?: boolean; translateToString: (trimRight?: boolean) => string } | undefined;
    };
  };
}): string {
  try {
    const buffer = term.buffer.active;
    const cursorY = buffer.cursorY + buffer.baseY;

    // 向上寻找起始行（支持超长命令被 xterm 自动折行 isWrapped）
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

    return cleanPrompt(fullLine);
  } catch (err) {
    console.warn('[SafetyGuard] extractCommandLineFromTerminal failed:', err);
    return '';
  }
}

