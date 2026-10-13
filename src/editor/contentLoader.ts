import { invoke, isTauri } from '@tauri-apps/api/core';
import { hashContent } from '@/editor/types';

/**
 * 远端文件内容读写封装（Rust 端 `sftp_read_file` / `sftp_write_file` 由他人实现）：
 * - 仅 Tauri 环境真实 invoke（`isTauri()` 即 `window.__TAURI_INTERNALS__` 存在）；
 * - 浏览器开发环境 `loadRemoteContent` 返回 null（调用方按拉取失败处理）。
 */

/** `sftp_read_file` 约定返回结构（超过 5MB 或非文本时 Rust 端 reject） */
export interface RemoteContentPayload {
  content: string;
  size: number;
}

export async function loadRemoteContent(
  hostId: number,
  remotePath: string,
): Promise<RemoteContentPayload | null> {
  if (typeof window === 'undefined' || !isTauri()) {
    return null;
  }
  return invoke<RemoteContentPayload>('sftp_read_file', { hostId, remotePath });
}

export async function saveRemoteContent(
  hostId: number,
  remotePath: string,
  content: string,
): Promise<void> {
  if (typeof window === 'undefined' || !isTauri()) {
    throw new Error('saveRemoteContent is only available in the Tauri environment');
  }
  await invoke<void>('sftp_write_file', { hostId, remotePath, content });
}

/** baseHash 口径统一走 types.ts 的 hashContent（SHA-256 hex） */
export function computeBaseHash(content: string): Promise<string> {
  return hashContent(content);
}
