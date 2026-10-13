import { invoke, isTauri } from '@tauri-apps/api/core';
import { createSafeUsizeCounter } from '@/utils/safeUsize';
import { getDbKv, setDbKv } from './dbStorage';

/**
 * 对应 Rust 后端 `SessionModel` 结构体（`sessions` 加密表模型）。
 */
export interface SessionModel {
  id: number;
  name: string;
  host: string;
  port: number;
  username: string;
  auth_type: string;
  group_name: string | null;
}

const FALLBACK_STORAGE_KEY = 'veloterm:sessions:fallback:v1';
const nextFallbackSessionId = createSafeUsizeCounter(1);

function readFallbackSessions(): SessionModel[] {
  try {
    const raw = getDbKv(FALLBACK_STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as SessionModel[]) : [];
  } catch {
    return [];
  }
}

function writeFallbackSessions(sessions: SessionModel[]): void {
  setDbKv(FALLBACK_STORAGE_KEY, JSON.stringify(sessions));
}

/**
 * 获取全部已保存的会话（Tauri 环境下读取 SQLCipher 加密库 `veloterm.db`）。
 */
export async function listSessions(): Promise<SessionModel[]> {
  if (typeof window !== 'undefined' && isTauri()) {
    return invoke<SessionModel[]>('list_sessions');
  }
  return readFallbackSessions();
}

/**
 * 新增或更新会话（若 `session.id` 为 0 则由 Rust 后端自动分配 `usize` 主键）。
 */
export async function saveSession(session: SessionModel): Promise<SessionModel> {
  if (typeof window !== 'undefined' && isTauri()) {
    return invoke<SessionModel>('save_session', { session });
  }
  const list = readFallbackSessions();
  const normalizedId = session.id > 0 ? session.id : nextFallbackSessionId();
  const saved: SessionModel = {
    ...session,
    id: normalizedId,
  };
  const idx = list.findIndex((item) => item.id === normalizedId);
  if (idx >= 0) {
    list[idx] = saved;
  } else {
    list.push(saved);
  }
  writeFallbackSessions(list);
  return saved;
}

/**
 * 根据会话 ID (`usize`) 删除会话。
 */
export async function deleteSession(id: number): Promise<void> {
  if (typeof window !== 'undefined' && isTauri()) {
    await invoke<void>('delete_session', { id });
    return;
  }
  const list = readFallbackSessions().filter((item) => item.id !== id);
  writeFallbackSessions(list);
}
