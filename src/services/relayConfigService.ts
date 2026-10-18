/**
 * 中继（veloterm_relay）配置库服务：侧栏「中继」页 CRUD，
 * 主机表单与 ZK 页从这里取下拉选项。
 */

import { invoke } from '@tauri-apps/api/core';

import type { RelayTransport } from '@/hosts/types';

export interface RelayConfig {
  id: number;
  name: string;
  host: string;
  port: number;
  transport: RelayTransport;
  keyId: number;
  secret: string;
  certFingerprint?: string | null;
  createdAt: number;
  updatedAt: number;
}

export function relayList(): Promise<RelayConfig[]> {
  return invoke('relay_list');
}

export function relaySave(record: RelayConfig): Promise<RelayConfig> {
  return invoke('relay_save', { record });
}

export function relayDelete(id: number): Promise<void> {
  return invoke('relay_delete', { id });
}
