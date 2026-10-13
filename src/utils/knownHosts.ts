import { invoke, isTauri } from '@tauri-apps/api/core';

export interface KnownHostEntry {
  id: number;
  host: string;
  port: number;
  key_type: string;
  key_base64: string;
}

/**
 * 获取后端已信任的全部 SSH 主机公钥指纹列表（`veloterm.db` `known_hosts` 表）。
 */
export async function listKnownHosts(): Promise<KnownHostEntry[]> {
  if (typeof window !== 'undefined' && isTauri()) {
    try {
      return await invoke<KnownHostEntry[]>('ssh_list_known_hosts');
    } catch {
      return [];
    }
  }
  return [];
}

/**
 * 移除指定 ID (`usize`) 的已信任主机公钥指纹。
 */
export async function removeKnownHost(id: number): Promise<boolean> {
  if (typeof window !== 'undefined' && isTauri()) {
    try {
      return await invoke<boolean>('ssh_remove_known_host', { id });
    } catch {
      return false;
    }
  }
  return false;
}

/**
 * 清空全部已信任的主机公钥指纹记录。
 */
export async function clearKnownHosts(): Promise<number> {
  if (typeof window !== 'undefined' && isTauri()) {
    try {
      return await invoke<number>('ssh_clear_known_hosts');
    } catch {
      return 0;
    }
  }
  return 0;
}
