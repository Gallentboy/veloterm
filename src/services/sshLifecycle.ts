import { listen, type UnlistenFn } from '@tauri-apps/api/event';

/** SSH / SFTP 生命周期事件种类 */
export type LifecycleKind =
  | 'connecting'
  | 'connected'
  | 'shell-ready'
  | 'failed'
  | 'disconnected';

/** 断开原因 */
export type DisconnectReason = 'network' | 'server' | 'user' | 'timeout';

/** 生命周期事件负载（与 Rust `LifecycleEvent` 结构一一对应） */
export interface LifecycleEvent {
  kind: LifecycleKind;
  sessionId: number;
  hostId: number | null;
  host: string;
  port: number;
  username: string | null;
  error: string | null;
  reason: DisconnectReason | null;
  timestamp: number;
}

/** 订阅回调 */
export type LifecycleCallback = (event: LifecycleEvent) => void;

/**
 * SSH / SFTP 连接生命周期钩子（统一订阅封装）：
 * - `onSshEvent(cb)` / `onSftpEvent(cb)` 订阅某命名空间的全部事件；
 * - `onSshKind(kind, cb)` 等便捷方法按事件种类过滤；
 * - 返回 `unlisten`，调用即取消订阅。
 *
 * 用途：状态指示灯（connected）、就绪后自动执行命令（shell-ready）、
 * 断线通知/自动重连（disconnected + reason=network）、连接失败弹 AI 诊断（failed）。
 */

const SSH_EVENT_NAMES: Record<LifecycleKind, string> = {
  connecting: 'ssh://connecting',
  connected: 'ssh://connected',
  'shell-ready': 'ssh://shell-ready',
  failed: 'ssh://failed',
  disconnected: 'ssh://disconnected',
};

const SFTP_EVENT_NAMES: Record<LifecycleKind, string> = {
  connecting: 'sftp://connecting',
  connected: 'sftp://connected',
  'shell-ready': 'sftp://shell-ready',
  failed: 'sftp://failed',
  disconnected: 'sftp://disconnected',
};

/** 订阅 SSH 终端连接的全部生命周期事件 */
export async function onSshEvent(callback: LifecycleCallback): Promise<UnlistenFn> {
  const unlisteners: UnlistenFn[] = [];
  for (const eventName of Object.values(SSH_EVENT_NAMES)) {
    unlisteners.push(await listen<LifecycleEvent>(eventName, (e) => callback(e.payload)));
  }
  return () => unlisteners.forEach((fn) => fn());
}

/** 订阅 SFTP 连接的全部生命周期事件 */
export async function onSftpEvent(callback: LifecycleCallback): Promise<UnlistenFn> {
  const unlisteners: UnlistenFn[] = [];
  for (const eventName of Object.values(SFTP_EVENT_NAMES)) {
    unlisteners.push(await listen<LifecycleEvent>(eventName, (e) => callback(e.payload)));
  }
  return () => unlisteners.forEach((fn) => fn());
}

/** 订阅 SSH 终端的指定种类事件 */
export async function onSshKind(
  kind: LifecycleKind,
  callback: LifecycleCallback,
): Promise<UnlistenFn> {
  return listen<LifecycleEvent>(SSH_EVENT_NAMES[kind], (e) => callback(e.payload));
}

/** 订阅 SFTP 的指定种类事件 */
export async function onSftpKind(
  kind: LifecycleKind,
  callback: LifecycleCallback,
): Promise<UnlistenFn> {
  return listen<LifecycleEvent>(SFTP_EVENT_NAMES[kind], (e) => callback(e.payload));
}

// ── 常用便捷方法 ──

/** SSH Shell 就绪（首屏输出到达，xterm 可交互） */
export async function onShellReady(callback: LifecycleCallback): Promise<UnlistenFn> {
  return onSshKind('shell-ready', callback);
}

/** SSH 连接断开（含原因：network / server / user / timeout） */
export async function onSshDisconnected(callback: LifecycleCallback): Promise<UnlistenFn> {
  return onSshKind('disconnected', callback);
}

/** SSH 连接失败（含错误信息） */
export async function onSshFailed(callback: LifecycleCallback): Promise<UnlistenFn> {
  return onSshKind('failed', callback);
}
