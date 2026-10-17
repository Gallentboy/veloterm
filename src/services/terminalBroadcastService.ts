import { terminalSessionRegistry } from '@/terminal/terminalSessionRegistry';
import { encodeTerminalInput } from '@/terminal/transport';
import type { ITerminalTransport } from '@/terminal/types';

export interface BroadcastResult {
  sessionCount: number;
  successCount: number;
  failedSessionIds: number[];
}

/**
 * 并发扇出写字节到指定终端会话列表的底层实现：
 * 查询每个 session 对应的底层 terminalId 并行推送，返回成功与失败统计。
 * excludeSessionId 用于输入镜像时排除发起方自身，避免回环。
 */
async function fanoutToSessions(
  transport: ITerminalTransport,
  sessionIds: number[],
  bytes: Uint8Array,
  excludeSessionId: number | null,
  logTag: string,
): Promise<BroadcastResult> {
  const targets: { sessionId: number; terminalId: number }[] = [];
  const failedSessionIds: number[] = [];

  for (const sid of sessionIds) {
    if (sid === excludeSessionId) continue;
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
        console.warn(`[${logTag}] write to session ${target.sessionId} failed:`, err);
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
 * 终端联动·输入镜像：把某个终端里用户键入的原始字节（onData/onBinary 产物，
 * 不追加任何回车/换行）实时转发给联动集合中的其余终端。
 *
 * 镜像只发输入、不注入输出：每个终端的回显由各自 pty 完成，接收方的
 * onData 不会被程序化写入触发，天然无回环。
 */
export function mirrorInputToSessions(
  transport: ITerminalTransport,
  sessionIds: number[],
  sourceSessionId: number,
  data: string | Uint8Array,
): Promise<BroadcastResult> {
  return fanoutToSessions(
    transport,
    sessionIds,
    encodeTerminalInput(data),
    sourceSessionId,
    'mirrorInputToSessions',
  );
}

/**
 * 并发扇出向指定的终端会话列表广播原始控制字符（如 Ctrl+C = 0x03 SIGINT, Ctrl+D = 0x04 EOF）：
 * 用于在多终端联动执行长时间命令（如 top / ping / tail -f）时，一键协同中断所有终端。
 */
export function broadcastSignalToSessions(
  transport: ITerminalTransport,
  sessionIds: number[],
  signalByte: number = 0x03,
): Promise<BroadcastResult> {
  return fanoutToSessions(
    transport,
    sessionIds,
    new Uint8Array([signalByte]),
    null,
    'broadcastSignalToSessions',
  );
}

/**
 * 终端联动·清屏镜像：把发起方终端的「清除」（右键菜单 clear）同步到联动集合
 * 中的其余终端。
 *
 * 清屏是纯前端缓冲操作（xterm `term.clear()` 清视口+回滚），没有等价的输入
 * 字节可走输入镜像管道——直接经注册表对组内其余 xterm 实例调用 clear()，
 * 与本机菜单行为完全一致：不向 pty 写任何字节、不进 shell 历史。门控与
 * 输入镜像相同（组内多于一台且发起方在组内）。
 */
export function mirrorClearToSessions(
  sessionIds: number[],
  sourceSessionId: number,
): void {
  if (sessionIds.length <= 1 || !sessionIds.includes(sourceSessionId)) return;
  for (const sid of sessionIds) {
    if (sid === sourceSessionId) continue;
    try {
      terminalSessionRegistry.get(sid)?.term.clear();
    } catch (err) {
      console.warn(`[mirrorClearToSessions] clear session ${sid} failed:`, err);
    }
  }
}
