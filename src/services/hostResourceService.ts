import { invoke } from '@tauri-apps/api/core';

/** 字节用量（内存 / SWAP / 磁盘共用，与 Rust ByteUsage 对齐 camelCase） */
export interface ByteUsage {
  usedBytes: number;
  totalBytes: number;
  usagePercent: number;
}

/** 网络上下行速率（首拍无基线时为 null） */
export interface NetworkRate {
  rxBytesPerSec: number | null;
  txBytesPerSec: number | null;
}

/** 一次远程资源采集快照（与 Rust terminal/resource_monitor.rs 对齐） */
export interface ResourceSnapshot {
  /** unix 毫秒字符串（仅作展示标记） */
  sampledAt: string;
  /** 采集命令往返耗时（毫秒，含远端 shell 执行） */
  rttMs: number;
  platform: 'linux' | 'darwin' | 'windows' | 'unknown';
  distro: string | null;
  /** CPU 使用率%（客户端差分，首拍 null） */
  cpu: number | null;
  memory: ByteUsage | null;
  swap: ByteUsage | null;
  network: NetworkRate | null;
  disk: ByteUsage | null;
}

/**
 * 拉取某 SSH 终端会话的远程主机资源快照（延迟/CPU/内存/SWAP/网络/磁盘）。
 * 仅 SSH 会话支持；本地 PTY 或会话不存在回 Err——调用方（轮询方）静默降级。
 */
export function fetchResourceSnapshot(terminalId: number): Promise<ResourceSnapshot> {
  return invoke<ResourceSnapshot>('ssh_resource_snapshot', { id: terminalId });
}
