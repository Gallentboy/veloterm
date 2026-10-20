/**
 * 主机密码历史服务：自动改密/手输更新前留档的旧密码（后端每主机最多保留 20 条）。
 * 只读 + 清空，不做写入（写入由后端在更新主机密码时完成）。
 */

import { invoke } from '@tauri-apps/api/core';

/** 来源：kbd-int-change（认证内自动改密）/ user-input（用户手输密码通过认证） */
export type HostPasswordHistorySource = 'kbd-int-change' | 'user-input';

export interface HostPasswordHistoryEntry {
  id: number;
  hostId: number;
  /** 被替换掉的旧密码明文（库本身是 SQLCipher 加密的） */
  password: string;
  /** 被替换时刻（unix 毫秒） */
  replacedAt: number;
  /** 来源标记（后端为 string，已知取值见 HostPasswordHistorySource） */
  source: string;
}

/** 查某主机的密码历史（按替换时间倒序，最多 20 条） */
export function hostPasswordHistoryList(hostId: number): Promise<HostPasswordHistoryEntry[]> {
  return invoke('host_password_history_list', { hostId });
}

/** 清空密码历史：hostId 为 null 时清空全部主机，返回删除条数 */
export function hostPasswordHistoryClear(hostId: number | null): Promise<number> {
  return invoke('host_password_history_clear', { hostId });
}
