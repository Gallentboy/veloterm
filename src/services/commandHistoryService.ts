/**
 * 命令历史服务：侧栏「历史」页读取面（分页列表 / 清空 / 采集开关）。
 * 采集本身在后端输出流旁路完成（terminal::command_history），这里只做查询与设置。
 */

import { invoke } from '@tauri-apps/api/core';

export interface CommandHistoryRow {
  id: number;
  /** null 表示本地终端 */
  hostId: number | null;
  sessionId: number;
  command: string;
  exitCode: number;
  /** unix 毫秒 */
  createdAt: number;
}

export interface CommandHistoryListOptions {
  /** 指定主机 id 过滤；null = 全部远程主机；与 localOnly 互斥（localOnly 时传 null） */
  hostId?: number | null;
  onlyFailed?: boolean;
  /** 仅本地终端（host_id IS NULL）的服务端过滤 */
  localOnly?: boolean;
  limit?: number;
  offset?: number;
}

/** 按主机过滤 / 只看失败 / 仅本地，时间倒序分页（后端 limit 钳制 1..1000，默认 200）。 */
export function commandHistoryList(options?: CommandHistoryListOptions): Promise<CommandHistoryRow[]> {
  const { hostId = null, onlyFailed = false, localOnly = false, limit = 200, offset = 0 } = options ?? {};
  return invoke('command_history_list', { hostId, onlyFailed, localOnly, limit, offset });
}

/** 清空历史（指定主机只清该主机，localOnly 只清本地记录，均不传清全部），返回删除条数。 */
export function commandHistoryClear(hostId: number | null, localOnly = false): Promise<number> {
  return invoke('command_history_clear', { hostId, localOnly });
}

/** 读采集开关（剥离永远执行，开关只控制入库）。 */
export function commandHistoryGetEnabled(): Promise<boolean> {
  return invoke('command_history_get_enabled');
}

/** 写采集开关：内存即时生效 + kv 持久化。 */
export function commandHistorySetEnabled(enabled: boolean): Promise<void> {
  return invoke('command_history_set_enabled', { enabled });
}
