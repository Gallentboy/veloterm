import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

/** 转发类型：本地 -L / 远程 -R / 动态 SOCKS5 -D（与 Rust PortForwardKind 对齐） */
export type PortForwardKind = 'local' | 'remote' | 'dynamic';

/** 一条转发规则（与 Rust port_forward.rs 对齐 camelCase） */
export interface PortForwardRule {
  id: number;
  hostId: number;
  kind: PortForwardKind;
  bindHost: string;
  bindPort: number;
  /** dynamic 规则不使用目标（由 SOCKS5 客户端逐连接指定） */
  targetHost: string;
  targetPort: number;
  enabled: boolean;
}

export type RuleStatus = 'stopped' | 'starting' | 'active' | 'error';

export interface RuleRuntime {
  ruleId: number;
  status: RuleStatus;
  lastError: string | null;
  /** remote 规则服务器实际分配端口（bind_port=0 时） */
  effectivePort: number | null;
}

export interface ForwardStateMsg {
  hostId: number;
  rules: RuleRuntime[];
}

export function listPortForwards(hostId: number): Promise<PortForwardRule[]> {
  return invoke<PortForwardRule[]>('port_forward_list', { hostId });
}

export function savePortForward(rule: PortForwardRule): Promise<PortForwardRule> {
  return invoke<PortForwardRule>('port_forward_save', { rule });
}

export function deletePortForward(hostId: number, ruleId: number): Promise<void> {
  return invoke('port_forward_delete', { hostId, ruleId });
}

export function setPortForwardEnabled(
  hostId: number,
  ruleId: number,
  enabled: boolean,
): Promise<void> {
  return invoke('port_forward_set_enabled', { hostId, ruleId, enabled });
}

export function fetchPortForwardStates(hostId: number): Promise<RuleRuntime[]> {
  return invoke<RuleRuntime[]>('port_forward_states', { hostId });
}

/** 订阅规则状态事件（连接/断开/错误时后端广播），返回反订阅 */
export function listenForwardState(
  handler: (msg: ForwardStateMsg) => void,
): Promise<() => void> {
  return listen<ForwardStateMsg>('port-forward-state', ({ payload }) => handler(payload));
}
