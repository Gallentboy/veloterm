import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  childGroupsOf,
  loadCollapsedIds,
  loadHostStore,
  nextId,
  saveCollapsedIds,
  saveHostStore,
  subtreeIds,
} from './storage';
import type { GroupInput, GroupNode, HostInput, HostNode, HostStoreData } from './types';

function touch(): number {
  return Date.now();
}

/**
 * 主机库状态钩子：分组树 + 主机（多对多）CRUD，直接持久化至后端 SQLCipher (`veloterm.db`)。
 * id 为数字自增（分组、主机各自独立序列），在 setState 更新器内取
 * nextId(current)，连点创建也不会撞号；结构与后端关系表 (`groups`, `hosts`, `host_groups`, `sessions`) 一一对应。
 * 删除分组会连带删除子分组，但只把主机移出分组（主机保留、变为未分组）。
 */
export function useHosts() {
  const [store, setStore] = useState<HostStoreData>(() => loadHostStore());
  const [collapsedIds, setCollapsedIds] = useState<number[]>(() => loadCollapsedIds());

  useEffect(() => {
    saveHostStore(store);
  }, [store]);

  useEffect(() => {
    saveCollapsedIds(collapsedIds);
  }, [collapsedIds]);

  // ===== 分组 =====
  const createGroup = useCallback((input: GroupInput) => {
    setStore((current) => ({
      ...current,
      groups: [
        ...current.groups,
        { id: nextId(current.groups), ...input, createdAt: touch(), updatedAt: touch() },
      ],
    }));
  }, []);

  const updateGroup = useCallback((id: number, patch: Partial<Omit<GroupNode, 'id'>>) => {
    setStore((current) => ({
      ...current,
      groups: current.groups.map((group) =>
        group.id === id ? { ...group, ...patch, updatedAt: touch() } : group,
      ),
    }));
  }, []);

  const removeGroup = useCallback((id: number) => {
    setStore((current) => {
      const removing = new Set(subtreeIds(current.groups, id));
      return {
        ...current,
        groups: current.groups.filter((group) => !removing.has(group.id)),
        // 主机只解除关联，不删除
        hosts: current.hosts.map((host) => ({
          ...host,
          groupIds: host.groupIds.filter((gid) => !removing.has(gid)),
          updatedAt: touch(),
        })),
      };
    });
    setCollapsedIds((current) => current.filter((cid) => cid !== id));
  }, []);

  // ===== 主机 =====
  const createHost = useCallback((input: HostInput) => {
    setStore((current) => ({
      ...current,
      hosts: [
        ...current.hosts,
        { id: nextId(current.hosts), ...input, createdAt: touch(), updatedAt: touch() },
      ],
    }));
  }, []);

  const updateHost = useCallback((id: number, patch: Partial<Omit<HostNode, 'id'>>) => {
    setStore((current) => ({
      ...current,
      hosts: current.hosts.map((host) =>
        host.id === id ? { ...host, ...patch, updatedAt: touch() } : host,
      ),
    }));
  }, []);

  const removeHost = useCallback((id: number) => {
    setStore((current) => ({
      ...current,
      hosts: current.hosts
        .filter((host) => host.id !== id)
        .map((host) =>
          host.jumpHostId === id ? { ...host, jumpHostId: null, updatedAt: touch() } : host,
        ),
    }));
  }, []);

  // ===== 展开态（分组树跨主机页/分组页共享） =====
  const isExpanded = useCallback((id: number) => !collapsedIds.includes(id), [collapsedIds]);

  const toggleExpanded = useCallback((id: number) => {
    setCollapsedIds((current) =>
      current.includes(id) ? current.filter((cid) => cid !== id) : [...current, id],
    );
  }, []);

  const childGroups = useCallback(
    (parentId: number | null) => childGroupsOf(store.groups, parentId),
    [store.groups],
  );

  const hostsInGroup = useCallback(
    (groupId: number) => store.hosts.filter((host) => host.groupIds.includes(groupId)),
    [store.hosts],
  );

  const ungroupedHosts = useMemo(
    () => store.hosts.filter((host) => host.groupIds.length === 0),
    [store.hosts],
  );

  return {
    groups: store.groups,
    hosts: store.hosts,
    childGroups,
    hostsInGroup,
    ungroupedHosts,
    createGroup,
    updateGroup,
    removeGroup,
    createHost,
    updateHost,
    removeHost,
    isExpanded,
    toggleExpanded,
  } as const;
}
