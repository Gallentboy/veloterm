/**
 * 会话快照：断线重连时保留滚动缓冲。
 *
 * 背景：重连 = `terminalSessionRegistry.release`（销毁旧 xterm 实例）+
 * `reconnectNonce` bump 强制 TerminalSession remount，新 xterm 的
 * scrollback 是空的。这里在旧实例 dispose 前用 @xterm/addon-serialize
 * 序列化缓冲（含 ANSI 颜色），按 sessionId 暂存，新挂载时取回写入、
 * 追加 `-- Reconnected --` 分隔线后接新会话输出。
 *
 * 容量约束：
 * - 单份快照截尾 256KB（在行边界切割，保最近的输出）
 * - 全局最多暂存 16 份（FIFO 淘汰）——用户关闭标签页（非重连）也会走
 *   dispose 存一份，取走或淘汰前一直占内存，必须有上限
 * - 快照是一次性资产：取走即删，同一会话不会恢复两次
 */
import { SerializeAddon } from '@xterm/addon-serialize';
import type { Terminal } from '@xterm/xterm';

const MAX_SNAPSHOT_CHARS = 256 * 1024;
const MAX_ENTRIES = 16;

const snapshots = new Map<number, string>();

/**
 * 在旧 xterm 实例 dispose 前调用：serialize 缓冲并暂存。
 * 任何失败（addon 异常等）只静默放弃快照，绝不阻断销毁流程。
 */
export function saveSessionSnapshot(sessionId: number, term: Terminal): void {
  try {
    const addon = new SerializeAddon();
    term.loadAddon(addon);
    const text = addon.serialize();
    if (!text) return;
    snapshots.delete(sessionId); // 重插到 Map 尾部，维持插入序淘汰
    if (text.length <= MAX_SNAPSHOT_CHARS) {
      snapshots.set(sessionId, text);
    } else {
      // 截尾保最近：从头丢弃到第一个行边界，避免恢复出半行
      const tail = text.slice(text.length - MAX_SNAPSHOT_CHARS);
      const firstBreak = tail.indexOf('\r\n');
      snapshots.set(sessionId, firstBreak >= 0 ? tail.slice(firstBreak + 2) : tail);
    }
    while (snapshots.size > MAX_ENTRIES) {
      const oldest = snapshots.keys().next().value;
      if (oldest === undefined) break;
      snapshots.delete(oldest);
    }
  } catch (error) {
    console.warn('[sessionSnapshot] serialize failed:', error);
  }
}

/**
 * 新挂载时取回快照（取走即删）。无快照（首次挂载/已被淘汰）返回 null。
 */
export function takeSessionSnapshot(sessionId: number): string | null {
  const text = snapshots.get(sessionId) ?? null;
  snapshots.delete(sessionId);
  return text;
}

/**
 * 恢复快照 + 重连分隔线。返回 true 表示已写入恢复内容。
 * 直接 term.write（不过 highlightOutput——快照内含此前已注入的 ANSI，
 * 二次扫描会因 HAS_ANSI_FG 跳过，但没必要再走一遍）。
 */
export function restoreSessionSnapshot(sessionId: number, term: Terminal): boolean {
  const snapshot = takeSessionSnapshot(sessionId);
  if (!snapshot) return false;
  const time = new Date().toTimeString().slice(0, 8);
  term.write(snapshot);
  term.write(`\r\n\x1b[90m-- Reconnected (${time}) --\x1b[0m\r\n`);
  return true;
}
