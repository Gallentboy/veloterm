/**
 * 批量执行（fleet exec）服务：多台 SSH 主机并行跑同一命令，
 * per-host 结果事件经 Tauri Channel 流式回传（dbQuery 同款模式）。
 * 后端见 src-tauri/src/fleet_exec.rs（复用 resolve_host_chain +
 * connect_ssh_chain_headless + sftp_exec_capture_status）。
 */

import { Channel, invoke } from '@tauri-apps/api/core';

/** 一台主机的执行结果（HostDone/HostError 的前端合并形态） */
export interface FleetHostResult {
  hostId: number;
  /** running = 已起跑未完成；done = 拿到退出码；error = 连接/执行失败 */
  status: 'running' | 'done' | 'error';
  exitCode: number | null;
  elapsedMs: number | null;
  output: string;
  truncated: boolean;
  errorMessage?: string;
}

/** 一批的汇总（fleet_exec_run 的返回值） */
export interface FleetExecSummary {
  total: number;
  succeeded: number;
  failed: number;
  cancelled: boolean;
}

export type FleetExecEvent =
  | { type: 'started'; hostCount: number }
  | { type: 'hostStart'; hostId: number }
  | {
      type: 'hostDone';
      hostId: number;
      exitCode: number | null;
      elapsedMs: number;
      output: string;
      truncated: boolean;
    }
  | { type: 'hostError'; hostId: number; message: string; elapsedMs: number }
  | { type: 'done'; cancelled: boolean; elapsedMs: number };

/**
 * 发起批量执行。runId 由前端生成（时间戳 + 序号），取消命令按它寻址；
 * invoke 的 Promise 在整批结束后 resolve（返回汇总），事件先于它流式到达。
 */
export function fleetExecRun(
  runId: string,
  hostIds: number[],
  command: string,
  onEvent: (event: FleetExecEvent) => void,
  options?: { timeoutSecs?: number; concurrency?: number },
): Promise<FleetExecSummary> {
  const channel = new Channel<FleetExecEvent>();
  channel.onmessage = onEvent;
  return invoke('fleet_exec_run', {
    runId,
    hostIds,
    command,
    timeoutSecs: options?.timeoutSecs,
    concurrency: options?.concurrency,
    onEvent: channel,
  });
}

/** 取消一批运行；runId 已结束（不存在）返回 false 而非报错。 */
export function fleetExecCancel(runId: string): Promise<boolean> {
  return invoke('fleet_exec_cancel', { runId });
}
