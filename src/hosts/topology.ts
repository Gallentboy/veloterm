/**
 * 连接拓扑数据推导（主机 → 主链节点序列）：本机 →（中继）→ 跳板链 → 目标。
 * HostTopologyDialog（单主机详情）与 GlobalTopologyDialog（全局多主机）共用。
 */

import type { HostNode, SshRelayEndpoint } from '@/hosts/types';
import type { LifecycleEvent } from '@/services/sshLifecycle';

export type ChainNodeKind = 'local' | 'relay' | 'jump' | 'target';

export interface ChainNode {
  kind: ChainNodeKind;
  label: string;
  sub: string;
  relay?: SshRelayEndpoint;
  hostId?: number;
}

/** 整链实时状态（来自 SSH lifecycle 事件，按 hostId 过滤取最新） */
export type ChainState = 'idle' | 'connecting' | 'active' | 'error';

/** 由 hosts 数据推导连接链（跳板按 jumpHostId 前插；visited 集合防环与自引用） */
export function resolveChain(host: HostNode, hosts: HostNode[]): ChainNode[] {
  const byId = new Map(hosts.map((item) => [item.id, item]));
  const jumps: ChainNode[] = [];
  const visited = new Set<number>([host.id]);
  let cursor = host.jumpHostId ?? null;
  while (cursor != null && !visited.has(cursor)) {
    visited.add(cursor);
    const hop = byId.get(cursor);
    if (!hop) break;
    jumps.unshift({ kind: 'jump', label: hop.name, sub: `${hop.host}:${hop.port}`, hostId: hop.id });
    cursor = hop.jumpHostId ?? null;
  }

  const nodes: ChainNode[] = [{ kind: 'local', label: 'local', sub: 'veloterm' }];
  if (host.relay) {
    nodes.push({
      kind: 'relay',
      label: host.relay.host,
      sub: `${host.relay.port} · ${host.relay.transport.toUpperCase()}`,
      relay: host.relay,
    });
  }
  nodes.push(...jumps);
  nodes.push({ kind: 'target', label: host.name, sub: `${host.host}:${host.port}`, hostId: host.id });
  return nodes;
}

export function chainStateFromEvent(event: LifecycleEvent | null): ChainState {
  if (!event) return 'idle';
  switch (event.kind) {
    case 'connecting':
      return 'connecting';
    case 'connected':
    case 'shell-ready':
      return 'active';
    case 'failed':
      return 'error';
    case 'disconnected':
      return 'idle';
    default:
      return 'idle';
  }
}
