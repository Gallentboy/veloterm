import { getDbHostStore, getDbKv, setDbHostStore, setDbKv } from '@/services/dbStorage';
import { JS_MAX_SAFE_INTEGER } from '@/utils/safeUsize';
import type { GroupNode, HostNode, HostStoreData } from './types';

const COLLAPSED_KEY = 'veloterm:groups:collapsed-ids';

/** 数字自增主键：取当前序列最大值 +1（对应后端 `usize`，到达 `JS_MAX_SAFE_INTEGER` 自动从 0 循环找空位） */
export function nextId(records: Array<{ id: number }>): number {
  const max = records.reduce((acc, record) => Math.max(acc, record.id), 0);
  if (max < JS_MAX_SAFE_INTEGER) {
    return max + 1;
  }
  const used = new Set(records.map((r) => r.id));
  for (let candidate = 0; candidate <= JS_MAX_SAFE_INTEGER; candidate += 1) {
    if (!used.has(candidate)) {
      return candidate;
    }
  }
  return 0;
}

export function loadHostStore(): HostStoreData {
  return getDbHostStore();
}

export function saveHostStore(data: HostStoreData): void {
  setDbHostStore(data);
}

export function loadCollapsedIds(): number[] {
  try {
    const raw = getDbKv(COLLAPSED_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.filter((v): v is number => typeof v === 'number') : [];
  } catch {
    return [];
  }
}

export function saveCollapsedIds(ids: number[]): void {
  setDbKv(COLLAPSED_KEY, JSON.stringify(ids));
}

/** 直接子分组（保持插入顺序） */
export function childGroupsOf(groups: GroupNode[], parentId: number | null): GroupNode[] {
  return groups.filter((group) => group.parentId === parentId);
}

/** id 及其全部后代分组 id（删除子树用） */
export function subtreeIds(groups: GroupNode[], id: number): number[] {
  const result = [id];
  const frontier = [id];
  while (frontier.length > 0) {
    const current = frontier.pop()!;
    for (const group of groups) {
      if (group.parentId === current) {
        result.push(group.id);
        frontier.push(group.id);
      }
    }
  }
  return result;
}

/** 分组选项：带层级路径（如 后端 / 生产 / 华东）；excludeSubtreeOf 排除自身子树防环 */
export function groupPathOptions(
  groups: GroupNode[],
  excludeSubtreeOf?: number | null,
): Array<{ id: number; path: string }> {
  const excluded = excludeSubtreeOf != null ? new Set(subtreeIds(groups, excludeSubtreeOf)) : null;
  const byId = new Map(groups.map((group) => [group.id, group]));
  const pathOf = (group: GroupNode): string => {
    const parts: string[] = [group.name];
    let cursor = group.parentId != null ? byId.get(group.parentId) : undefined;
    while (cursor) {
      parts.unshift(cursor.name);
      cursor = cursor.parentId != null ? byId.get(cursor.parentId) : undefined;
    }
    return parts.join(' / ');
  };
  return groups
    .filter((group) => !excluded?.has(group.id))
    .map((group) => ({ id: group.id, path: pathOf(group) }));
}

/**
 * 找出以 id 为跳板机（含多级后代依赖）的全部主机 id，用于在编辑主机时排除自身及依赖自身的主机防环。
 */
export function dependentHostIds(hosts: HostNode[], id: number): number[] {
  const result = [id];
  const frontier = [id];
  while (frontier.length > 0) {
    const current = frontier.pop()!;
    for (const host of hosts) {
      if (host.jumpHostId === current && !result.includes(host.id)) {
        result.push(host.id);
        frontier.push(host.id);
      }
    }
  }
  return result;
}

/**
 * 可选跳板机列表：排除当前编辑的主机自身及其下游跳板依赖链，杜绝跳板环路。
 */
export function jumpHostOptions(
  hosts: HostNode[],
  excludeHostId?: number | null,
): Array<{ id: number; label: string }> {
  const excluded = excludeHostId != null ? new Set(dependentHostIds(hosts, excludeHostId)) : null;
  return hosts
    .filter((host) => !excluded?.has(host.id))
    .map((host) => ({
      id: host.id,
      label: `${host.name} (${host.username || 'root'}@${host.host}:${host.port})`,
    }));
}

/**
 * 解析目标主机的多级跳板机链路（对齐 `warpgate-protocol-ssh::resolve_chain_ids`）：
 * 返回按连接先后顺序排列的前置跳板机数组 `[hop1, hop2, ...]`（不包含目标主机本身）。
 */
export function resolveJumpHostChain(hosts: HostNode[], targetHost: HostNode): HostNode[] {
  const byId = new Map(hosts.map((host) => [host.id, host]));
  const chain: HostNode[] = [];
  const visited = new Set<number>([targetHost.id]);
  let currentJumpId = targetHost.jumpHostId ?? null;

  while (currentJumpId != null) {
    if (visited.has(currentJumpId)) {
      // 检测到跳板环路时安全截断
      break;
    }
    const jumpHost = byId.get(currentJumpId);
    if (!jumpHost) {
      break;
    }
    visited.add(jumpHost.id);
    chain.unshift(jumpHost);
    currentJumpId = jumpHost.jumpHostId ?? null;
  }

  return chain;
}
