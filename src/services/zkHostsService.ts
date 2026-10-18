/**
 * ZK 主机持久化服务：侧栏「ZK 主机」页 CRUD，ZK 客户端页从这里取
 * 已保存主机下拉（选中即自动连接 + 自动 digest 认证）。
 */

import { invoke } from '@tauri-apps/api/core';

export interface ZkCredentialRecord {
  id: number;
  username: string;
  password: string;
  createdAt: number;
  updatedAt: number;
}

export interface ZkHostRecord {
  id: number;
  /** 逗号分隔的多端点，如 "10.0.0.1:2181,10.0.0.2:2181" */
  name: string;
  endpoints: string;
  /** 引用中继配置库；null=直连 */
  relayId: number | null;
  /** 连接后自动 digest 认证的凭证；null=不自动 */
  defaultCredentialId: number | null;
  /** 多凭证，同主机内 username 唯一（保存时集合全量同步：请求里缺失的会被删） */
  credentials: ZkCredentialRecord[];
  createdAt: number;
  updatedAt: number;
}

export function zkHostList(): Promise<ZkHostRecord[]> {
  return invoke('zk_host_list');
}

/** id=0 新增、id>0 更新；返回落库后的记录（含新分配的凭证 id）。 */
export function zkHostSave(record: ZkHostRecord): Promise<ZkHostRecord> {
  return invoke('zk_host_save', { record });
}

export function zkHostDelete(id: number): Promise<void> {
  return invoke('zk_host_delete', { id });
}
