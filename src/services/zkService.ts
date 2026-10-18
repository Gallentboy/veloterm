/**
 * ZooKeeper 管理服务：连接（直连 / 经 veloterm_relay 中继桥接）、
 * znode 树浏览、数据读写、ACL 读写、digest 认证。
 */

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface ZkNodeStat {
  czxid: number;
  mzxid: number;
  ctime: number;
  mtime: number;
  version: number;
  cversion: number;
  aversion: number;
  ephemeralOwner: number;
  dataLength: number;
  numChildren: number;
}

export interface ZkAclRule {
  scheme: string;
  id: string;
  /** 位集：R1 W2 C4 D8 A16（ALL=31） */
  perms: number;
}

export interface ZkChildren {
  children: string[];
  stat: ZkNodeStat;
}

export interface ZkNode {
  /** utf8 有效时为原文；否则为 Base64（encoding 标识） */
  data: string;
  encoding: 'utf8' | 'base64';
  stat: ZkNodeStat;
  acls: ZkAclRule[];
}

export interface ZkWatchMsg {
  sessionId: string;
  path: string;
  kind: 'children' | 'data';
}

export const ACL_BITS = { read: 1, write: 2, create: 4, delete: 8, admin: 16 } as const;
export const ACL_ALL = 31;

export function zkConnect(
  endpoints: string,
  relayId: number | null,
): Promise<{ sessionId: string }> {
  return invoke('zk_connect', { endpoints, relayId });
}

export function zkDisconnect(sessionId: string): Promise<void> {
  return invoke('zk_disconnect', { sessionId });
}

export function zkList(sessionId: string, path: string): Promise<ZkChildren> {
  return invoke('zk_list', { sessionId, path });
}

export function zkGet(sessionId: string, path: string): Promise<ZkNode> {
  return invoke('zk_get', { sessionId, path });
}

export function zkSetData(
  sessionId: string,
  path: string,
  data: string,
  encoding: 'utf8' | 'base64',
  version: number,
): Promise<ZkNodeStat> {
  return invoke('zk_set_data', { sessionId, path, data, encoding, version });
}

export function listenZkWatch(
  handler: (msg: ZkWatchMsg) => void,
): Promise<() => void> {
  return listen<ZkWatchMsg>('zk-watch', (e) => handler(e.payload));
}

export function zkCreate(
  sessionId: string,
  parent: string,
  name: string,
  data: string,
  ephemeral: boolean,
  sequential: boolean,
): Promise<{ createdPath: string }> {
  return invoke('zk_create', { sessionId, parent, name, data, ephemeral, sequential });
}

export function zkDelete(sessionId: string, path: string): Promise<void> {
  return invoke('zk_delete', { sessionId, path });
}

export function zkSetAcl(
  sessionId: string,
  path: string,
  acls: ZkAclRule[],
  aversion: number,
): Promise<ZkNodeStat> {
  return invoke('zk_set_acl', { sessionId, path, acls, aversion });
}

export function zkDigestId(user: string, password: string): Promise<string> {
  return invoke('zk_digest_id', { user, password });
}

export function zkAuth(sessionId: string, user: string, password: string): Promise<void> {
  return invoke('zk_auth', { sessionId, user, password });
}
