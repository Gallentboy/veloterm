import { getDbKv, setDbKv } from '@/services/dbStorage';
import { terminalSessionRegistry } from '@/terminal/terminalSessionRegistry';
import { encodeTerminalInput } from '@/terminal/transport';
import type { ITerminalTransport } from '@/terminal/types';

const BROADCAST_HISTORY_STORAGE_KEY = 'veloterm:terminal:broadcast-history';
const MAX_HISTORY_ITEMS = 50;

/**
 * 高危命令规则库：匹配同时向多台终端广播时极具破坏性的指令
 */
const DANGEROUS_PATTERNS: RegExp[] = [
  /\brm\s+(-[a-zA-Z]*r[a-zA-Z]*f*|-f[a-zA-Z]*r)\b/i, // rm -rf, rm -fr
  /\b(reboot|shutdown|poweroff|halt|init\s+0)\b/i,    // 关机与重启
  /\bmkfs(\.[a-zA-Z0-9]+)?\b/i,                       // 格式化文件系统
  /\bdd\s+if=.*of=\/dev\/[a-zA-Z0-9]+/i,              // 裸写底层磁盘设备
  /\bchmod\s+(-[a-zA-Z]*R[a-zA-Z]*\s+)?(777|000)\s+(\/|\/etc|\/var|\/usr)\b/i, // 危险递归改权
  /:(){ :\|:& };:/,                                   // Fork 炸弹
];

/**
 * 嗅探当前输入的广播命令是否包含高危操作
 */
export function isDangerousCommand(command: string): boolean {
  const trimmed = command.trim();
  if (!trimmed) return false;
  return DANGEROUS_PATTERNS.some((pattern) => pattern.test(trimmed));
}

export interface BroadcastResult {
  sessionCount: number;
  successCount: number;
  failedSessionIds: number[];
}

/**
 * 并发扇出执行广播命令到指定的终端会话列表：
 * - 统一补回车符 \r（xterm Enter 键的原始终端输入：Windows ConPTY/cmd 只认 CR，
 *   裸 LF 只回显不提交；Unix 行规程 ICRNL 把 CR 转 NL，两端通用，无需平台分支。
 *   与就绪钩子注入 TerminalSession.tsx 的 `${command}\r` 同一语义）；
 * - 查询每个 session 对应的底层 terminalId 并行推送；
 * - 返回成功与失败状态统计。
 */
export async function broadcastToSessions(
  transport: ITerminalTransport,
  sessionIds: number[],
  command: string,
): Promise<BroadcastResult> {
  const finalCommand = `${command}\r`;
  const bytes = encodeTerminalInput(finalCommand);

  const targets: { sessionId: number; terminalId: number }[] = [];
  const failedSessionIds: number[] = [];

  for (const sid of sessionIds) {
    const tid = terminalSessionRegistry.getTerminalId(sid);
    if (tid !== null) {
      targets.push({ sessionId: sid, terminalId: tid });
    } else {
      failedSessionIds.push(sid);
    }
  }

  const writePromises = targets.map((target) =>
    transport
      .write({ id: target.terminalId, data: bytes })
      .then(() => true)
      .catch((err) => {
        console.warn(`[broadcastToSessions] write to session ${target.sessionId} failed:`, err);
        return false;
      }),
  );

  const results = await Promise.allSettled(writePromises);
  let successCount = 0;

  results.forEach((res, idx) => {
    if (res.status === 'fulfilled' && res.value === true) {
      successCount += 1;
    } else {
      const target = targets[idx];
      if (target && !failedSessionIds.includes(target.sessionId)) {
        failedSessionIds.push(target.sessionId);
      }
    }
  });

  return {
    sessionCount: sessionIds.length,
    successCount,
    failedSessionIds,
  };
}

/**
 * 并发扇出向指定的终端会话列表广播原始控制字符（如 Ctrl+C = 0x03 SIGINT, Ctrl+D = 0x04 EOF）：
 * 用于在多终端群发执行长时间命令（如 top / ping / tail -f）时，一键协同中断所有终端。
 */
export async function broadcastSignalToSessions(
  transport: ITerminalTransport,
  sessionIds: number[],
  signalByte: number = 0x03,
): Promise<BroadcastResult> {
  const bytes = new Uint8Array([signalByte]);
  const targets: { sessionId: number; terminalId: number }[] = [];
  const failedSessionIds: number[] = [];

  for (const sid of sessionIds) {
    const tid = terminalSessionRegistry.getTerminalId(sid);
    if (tid !== null) {
      targets.push({ sessionId: sid, terminalId: tid });
    } else {
      failedSessionIds.push(sid);
    }
  }

  const writePromises = targets.map((target) =>
    transport
      .write({ id: target.terminalId, data: bytes })
      .then(() => true)
      .catch((err) => {
        console.warn(`[broadcastSignalToSessions] write to session ${target.sessionId} failed:`, err);
        return false;
      }),
  );

  const results = await Promise.allSettled(writePromises);
  let successCount = 0;

  results.forEach((res, idx) => {
    if (res.status === 'fulfilled' && res.value === true) {
      successCount += 1;
    } else {
      const target = targets[idx];
      if (target && !failedSessionIds.includes(target.sessionId)) {
        failedSessionIds.push(target.sessionId);
      }
    }
  });

  return {
    sessionCount: sessionIds.length,
    successCount,
    failedSessionIds,
  };
}

/**
 * 读取历史广播命令列表
 */
export function getBroadcastHistory(): string[] {
  if (typeof window === 'undefined') return [];
  const raw = getDbKv(BROADCAST_HISTORY_STORAGE_KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

/**
 * 追加记录广播命令历史
 */
export function appendBroadcastHistory(command: string): void {
  const trimmed = command.trim();
  if (!trimmed || typeof window === 'undefined') return;

  const current = getBroadcastHistory();
  const next = [trimmed, ...current.filter((item) => item !== trimmed)].slice(
    0,
    MAX_HISTORY_ITEMS,
  );
  setDbKv(BROADCAST_HISTORY_STORAGE_KEY, JSON.stringify(next));
}
