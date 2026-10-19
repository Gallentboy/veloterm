import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { GroupNode, HostNode, HostStoreData, LegacyHostStoreData } from '@/hosts/types';

interface BootstrapStatePayload {
  kv: Record<string, string>;
  hostStore: HostStoreData;
}

const LEGACY_KV_KEYS = [
  'veloterm-theme',
  'veloterm-locale',
  'veloterm:settings:log-level',
  'veloterm:workspace-shell:sidebar-width-px',
  'veloterm:groups:collapsed-ids',
  'veloterm:terminal:panel-state:v1',
] as const;

const LEGACY_HOSTS_V2_KEY = 'veloterm:hosts:v2';
const LEGACY_HOSTS_V1_KEY = 'veloterm:hosts:v1';
const LEGACY_SESSIONS_FALLBACK_KEY = 'veloterm:sessions:fallback:v1';

const memoryKv = new Map<string, string>();
/** 主机库变更订阅（React 侧状态副本同步：useHosts 的 setStore） */
type HostStoreListener = (data: HostStoreData) => void;
const hostStoreListeners = new Set<HostStoreListener>();

export function subscribeDbHostStore(listener: HostStoreListener): () => void {
  hostStoreListeners.add(listener);
  return () => {
    hostStoreListeners.delete(listener);
  };
}

/** 写入内存主机库并广播给订阅者（React 状态副本身份保持一致） */
function applyHostStore(data: HostStoreData): void {
  memoryHostStore = data;
  for (const listener of hostStoreListeners) {
    listener(data);
  }
}
let memoryHostStore: HostStoreData = {
  version: 2,
  groups: [],
  hosts: [],
};
let bootstrapped = false;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function isValidHostStoreV2(value: unknown): value is HostStoreData {
  return (
    isRecord(value) &&
    value.version === 2 &&
    Array.isArray(value.groups) &&
    Array.isArray(value.hosts)
  );
}

function migrateLegacyHostsV1(legacy: LegacyHostStoreData): HostStoreData {
  const idMap = new Map<string, number>();
  let groupSeq = 0;
  let hostSeq = 0;
  const groups: GroupNode[] = [];
  const hosts: HostNode[] = [];

  for (const node of legacy.nodes) {
    if (node.kind === 'folder') {
      idMap.set(node.id, ++groupSeq);
    }
  }
  for (const node of legacy.nodes) {
    if (node.kind === 'folder') {
      groups.push({
        id: idMap.get(node.id)!,
        parentId: node.parentId != null ? (idMap.get(node.parentId) ?? null) : null,
        name: node.name,
        createdAt: node.createdAt,
        updatedAt: node.updatedAt,
      });
    } else {
      const parentGroupId =
        node.parentId != null ? (idMap.get(node.parentId) ?? null) : null;
      const { kind, parentId, ...host } = node;
      void kind;
      void parentId;
      hosts.push({
        ...host,
        id: ++hostSeq,
        groupIds: parentGroupId != null ? [parentGroupId] : [],
      });
    }
  }
  return { version: 2, groups, hosts };
}

function extractLegacyHostStoreFromLocalStorage(): HostStoreData | null {
  if (typeof window === 'undefined') return null;
  try {
    const rawV2 = window.localStorage.getItem(LEGACY_HOSTS_V2_KEY);
    if (rawV2) {
      const parsed: unknown = JSON.parse(rawV2);
      if (isValidHostStoreV2(parsed) && (parsed.groups.length > 0 || parsed.hosts.length > 0)) {
        return parsed;
      }
    }
    const rawV1 = window.localStorage.getItem(LEGACY_HOSTS_V1_KEY);
    if (rawV1) {
      const parsed: unknown = JSON.parse(rawV1);
      if (isRecord(parsed) && parsed.version === 1 && Array.isArray(parsed.nodes)) {
        const migrated = migrateLegacyHostsV1(parsed as unknown as LegacyHostStoreData);
        if (migrated.groups.length > 0 || migrated.hosts.length > 0) {
          return migrated;
        }
      }
    }
  } catch {
    // 忽略旧数据解析异常
  }
  return null;
}

/**
 * 启动时从后端 SQLCipher (`veloterm.db`) 水合全量键值配置与主机分组数据；
 * 若检测到旧版 `localStorage` 残留数据，自动迁移进 `veloterm.db` 并清除明文 `localStorage`。
 */
export async function bootstrapDbStorage(): Promise<void> {
  if (bootstrapped) return;
  bootstrapped = true;

  if (typeof window === 'undefined' || !isTauri()) {
    return;
  }

  try {
    const payload = await invoke<BootstrapStatePayload>('db_bootstrap_state');

    for (const [k, v] of Object.entries(payload.kv ?? {})) {
      memoryKv.set(k, v);
    }

    // 改密回写同步：后端在 kbd-int 认证内改密成功后已更新 hosts 表，
    // 前端内存副本必须同步刷新——否则新建会话仍用旧密码（实测被拒）。
    // 事件只带 hostId（不传密码），重拉 bootstrap 取权威数据。
    void listen<{ hostId: number }>('host-password-updated', () => {
      void invoke<BootstrapStatePayload>('db_bootstrap_state')
        .then((fresh) => {
          if (
            fresh.hostStore &&
            (fresh.hostStore.groups.length > 0 || fresh.hostStore.hosts.length > 0)
          ) {
            applyHostStore(fresh.hostStore);
          }
        })
        .catch((err) => {
          console.error('[dbStorage] refresh after password update failed:', err);
        });
    });

    // 迁移旧版 localStorage 中的键值配置到 SQLCipher `app_kv` 表并移除 localStorage 项
    for (const key of LEGACY_KV_KEYS) {
      try {
        const legacyVal = window.localStorage.getItem(key);
        if (legacyVal != null) {
          if (!memoryKv.has(key)) {
            memoryKv.set(key, legacyVal);
            await invoke('db_set_kv', { key, value: legacyVal });
          }
          window.localStorage.removeItem(key);
        }
      } catch {
        // 忽略单项迁移异常
      }
    }

    if (
      payload.hostStore &&
      (payload.hostStore.groups.length > 0 || payload.hostStore.hosts.length > 0)
    ) {
      applyHostStore(payload.hostStore);
    } else {
      const legacyHostStore = extractLegacyHostStoreFromLocalStorage();
      if (legacyHostStore) {
        memoryHostStore = legacyHostStore;
        await invoke('db_save_host_store', { store: legacyHostStore });
      }
    }

    // 清理旧版明文主机与会话 localStorage 键
    try {
      window.localStorage.removeItem(LEGACY_HOSTS_V2_KEY);
      window.localStorage.removeItem(LEGACY_HOSTS_V1_KEY);
      window.localStorage.removeItem(LEGACY_SESSIONS_FALLBACK_KEY);
    } catch {
      // 忽略清理异常
    }
  } catch (err) {
    console.error('[dbStorage] failed to bootstrap from SQLCipher database:', err);
  }
}

export function getDbKv(key: string): string | null {
  if (memoryKv.has(key)) {
    return memoryKv.get(key) ?? null;
  }
  if (typeof window !== 'undefined' && !isTauri()) {
    try {
      return window.localStorage.getItem(key);
    } catch {
      return null;
    }
  }
  return null;
}

export function setDbKv(key: string, value: string): void {
  memoryKv.set(key, value);
  if (typeof window !== 'undefined' && isTauri()) {
    void invoke('db_set_kv', { key, value }).catch((err) => {
      console.error(`[dbStorage] db_set_kv failed for key=${key}:`, err);
    });
  } else if (typeof window !== 'undefined') {
    try {
      window.localStorage.setItem(key, value);
    } catch {
      // 忽略纯浏览器预览环境的存储异常
    }
  }
}

export function deleteDbKv(key: string): void {
  memoryKv.delete(key);
  if (typeof window !== 'undefined' && isTauri()) {
    void invoke('db_delete_kv', { key }).catch((err) => {
      console.error(`[dbStorage] db_delete_kv failed for key=${key}:`, err);
    });
  } else if (typeof window !== 'undefined') {
    try {
      window.localStorage.removeItem(key);
    } catch {
      // 同上
    }
  }
}

export function getDbHostStore(): HostStoreData {
  return memoryHostStore;
}

export function setDbHostStore(data: HostStoreData): void {
  applyHostStore(data);
  if (typeof window !== 'undefined' && isTauri()) {
    void invoke('db_save_host_store', { store: data }).catch((err) => {
      console.error('[dbStorage] db_save_host_store failed:', err);
    });
  } else if (typeof window !== 'undefined') {
    try {
      window.localStorage.setItem(LEGACY_HOSTS_V2_KEY, JSON.stringify(data));
    } catch {
      // 同上
    }
  }
}
