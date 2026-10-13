import { useSyncExternalStore } from 'react';
import {
  copyRemoteFile,
  downloadRemoteFile,
  uploadLocalFile,
  type TransferProgress,
} from '@/services/sftpService';

/**
 * 全局 SFTP 传输队列：并行调度（可配并发数）、失败自动重试（可配次数 + 手动重试）、
 * 可选限速（0 = 不限速）。跨会话/主机共享一个队列，设置持久化到 localStorage。
 * 支持上传 / 下载 / 远端到远端复制（copy：源端 sessionId+remotePath，目标端
 * targetSessionId+targetRemotePath，由 Rust 后端流式中转，不经本地磁盘）。
 */

export type TransferDirection = 'upload' | 'download' | 'copy';

export type TransferTaskStatus = 'queued' | 'active' | 'done' | 'error' | 'canceled';

export interface TransferTask {
  id: number;
  direction: TransferDirection;
  /** copy 方向时为源端会话；upload/download 方向为操作会话 */
  sessionId: string;
  /** copy 方向时为源端路径 */
  remotePath: string;
  /** 本地路径（copy 方向远端到远端直传，无本地路径） */
  localPath?: string;
  /** 复制目标端会话（仅 direction = 'copy'） */
  targetSessionId?: string;
  /** 复制目标端路径（仅 direction = 'copy'） */
  targetRemotePath?: string;
  fileName: string;
  totalBytes: number;
  transferred: number;
  speedBps: number;
  percent: number;
  status: TransferTaskStatus;
  error: string | null;
  /** 已尝试次数（含首次） */
  attempt: number;
  startedAt: number | null;
  finishedAt: number | null;
  /** 上传完成后回调（如刷新目录） */
  onFinished?: (task: TransferTask) => void;
  cancel: (() => void) | null;
}

export interface TransferQueueSettings {
  /** 最大并行传输数 */
  maxConcurrent: number;
  /** 失败自动重试次数（0 = 不自动重试） */
  maxAutoRetry: number;
  /** 全局限速 bytes/s（0 = 不限速） */
  speedLimitBps: number;
}

const DEFAULT_SETTINGS: TransferQueueSettings = {
  maxConcurrent: 3,
  maxAutoRetry: 2,
  speedLimitBps: 0,
};

const SETTINGS_KEY = 'veloterm:transfer:settings';
const AUTO_RETRY_DELAY_MS = 1200;

interface EnqueueParams {
  direction: TransferDirection;
  /** copy 方向时为源端会话；upload/download 方向为操作会话 */
  sessionId: string;
  /** copy 方向时为源端路径 */
  remotePath: string;
  /** 本地路径（copy 方向可省略） */
  localPath?: string;
  /** 复制目标端会话（仅 direction = 'copy'） */
  targetSessionId?: string;
  /** 复制目标端路径（仅 direction = 'copy'） */
  targetRemotePath?: string;
  fileName: string;
  onFinished?: (task: TransferTask) => void;
}

class TransferQueue {
  private tasks = new Map<number, TransferTask>();
  private order: number[] = [];
  private settings: TransferQueueSettings = DEFAULT_SETTINGS;
  private nextId = 1;
  private listeners = new Set<() => void>();
  private snapshot: TransferTask[] = [];
  private retryTimers = new Map<number, number>();

  constructor() {
    try {
      const raw = localStorage.getItem(SETTINGS_KEY);
      if (raw) {
        this.settings = { ...DEFAULT_SETTINGS, ...JSON.parse(raw) };
      }
    } catch {
      // 读取失败用默认值
    }
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): TransferTask[] => this.snapshot;

  getSettings = (): TransferQueueSettings => this.settings;

  updateSettings(patch: Partial<TransferQueueSettings>): void {
    this.settings = { ...this.settings, ...patch };
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify(this.settings));
    } catch {
      // 持久化失败不阻断
    }
    this.emit();
    this.schedule();
  }

  enqueue(params: EnqueueParams): number {
    const id = this.nextId++;
    const task: TransferTask = {
      id,
      direction: params.direction,
      sessionId: params.sessionId,
      remotePath: params.remotePath,
      localPath: params.localPath,
      targetSessionId: params.targetSessionId,
      targetRemotePath: params.targetRemotePath,
      fileName: params.fileName,
      totalBytes: 0,
      transferred: 0,
      speedBps: 0,
      percent: 0,
      status: 'queued',
      error: null,
      attempt: 0,
      startedAt: null,
      finishedAt: null,
      onFinished: params.onFinished,
      cancel: null,
    };
    this.tasks.set(id, task);
    this.order.push(id);
    this.emit();
    this.schedule();
    return id;
  }

  cancelTask(id: number): void {
    const task = this.tasks.get(id);
    if (!task) return;
    task.cancel?.();
    if (task.status === 'queued' || task.status === 'active') {
      task.status = 'canceled';
      task.finishedAt = Date.now();
      this.emit();
      this.schedule();
    }
  }

  retryTask(id: number): void {
    const task = this.tasks.get(id);
    if (!task || (task.status !== 'error' && task.status !== 'canceled')) return;
    const timer = this.retryTimers.get(id);
    if (timer) {
      clearTimeout(timer);
      this.retryTimers.delete(id);
    }
    task.status = 'queued';
    task.error = null;
    task.transferred = 0;
    task.percent = 0;
    task.speedBps = 0;
    this.emit();
    this.schedule();
  }

  /** 清除已完成/已取消的任务（保留进行中和失败的） */
  clearFinished(): void {
    for (const [id, task] of this.tasks) {
      if (task.status === 'done' || task.status === 'canceled') {
        this.tasks.delete(id);
        this.order = this.order.filter((oid) => oid !== id);
      }
    }
    this.emit();
  }

  removeTask(id: number): void {
    this.cancelTask(id);
    this.tasks.delete(id);
    this.order = this.order.filter((oid) => oid !== id);
    this.emit();
  }

  private activeCount(): number {
    let count = 0;
    for (const task of this.tasks.values()) {
      if (task.status === 'active') count++;
    }
    return count;
  }

  private schedule(): void {
    const slots = this.settings.maxConcurrent - this.activeCount();
    if (slots <= 0) return;
    let started = 0;
    for (const id of this.order) {
      if (started >= slots) break;
      const task = this.tasks.get(id);
      if (task?.status === 'queued') {
        void this.start(task);
        started++;
      }
    }
  }

  private async start(task: TransferTask): Promise<void> {
    task.status = 'active';
    task.attempt += 1;
    task.startedAt = Date.now();
    task.finishedAt = null;
    task.error = null;
    this.emit();

    const speedLimit = this.settings.speedLimitBps;
    try {
      const handle =
        task.direction === 'copy'
          ? await copyRemoteFile(
              // 远端到远端复制：sessionId/remotePath 为源端，targetSessionId/targetRemotePath 为目标端
              task.sessionId,
              task.remotePath,
              task.targetSessionId ?? '',
              task.targetRemotePath ?? '',
              (progress: TransferProgress) => this.onProgress(task.id, progress),
              speedLimit,
            )
          : task.direction === 'upload'
            ? await uploadLocalFile(
                task.sessionId,
                task.localPath ?? '',
                task.remotePath,
                (progress: TransferProgress) => this.onProgress(task.id, progress),
                speedLimit,
              )
            : await downloadRemoteFile(
                task.sessionId,
                task.remotePath,
                task.localPath ?? '',
                (progress: TransferProgress) => this.onProgress(task.id, progress),
                speedLimit,
              );
      task.cancel = handle.cancel;
      this.emit();
      await handle.done;
      task.status = 'done';
      task.percent = 100;
      task.finishedAt = Date.now();
      task.cancel = null;
      this.emit();
      task.onFinished?.(task);
    } catch (error) {
      task.cancel = null;
      const message = String(error);
      if (message.includes('cancelled by user')) {
        task.status = 'canceled';
        task.finishedAt = Date.now();
      } else if (task.attempt <= this.settings.maxAutoRetry) {
        // 自动重试：退回队列稍后重试
        task.status = 'queued';
        task.error = message;
        const timer = window.setTimeout(() => {
          this.retryTimers.delete(task.id);
          if (this.tasks.get(task.id)?.status === 'queued') {
            this.schedule();
          }
        }, AUTO_RETRY_DELAY_MS);
        this.retryTimers.set(task.id, timer);
      } else {
        task.status = 'error';
        task.error = message;
        task.finishedAt = Date.now();
      }
      this.emit();
    } finally {
      this.schedule();
    }
  }

  private onProgress(id: number, progress: TransferProgress): void {
    const task = this.tasks.get(id);
    if (!task || task.status !== 'active') return;
    task.transferred = Number(progress.transferred);
    task.totalBytes = Number(progress.total);
    task.speedBps = progress.speed_bps;
    task.percent = progress.percent;
    if (progress.is_finished) {
      // 完成包提前到：等 done promise 结束再置 done，这里只更新进度
      task.percent = 100;
    }
    this.emit();
  }

  private emit(): void {
    this.snapshot = this.order
      .map((id) => this.tasks.get(id))
      .filter((task): task is TransferTask => Boolean(task));
    for (const listener of this.listeners) {
      listener();
    }
  }
}

export const transferQueue = new TransferQueue();

/** React 钩子：订阅全局传输队列任务列表 */
export function useTransferTasks(): TransferTask[] {
  return useSyncExternalStore(transferQueue.subscribe, transferQueue.getSnapshot);
}

/** React 钩子：读取传输队列设置（settings 对象仅在 updateSettings 时更换引用，满足快照稳定性） */
export function useTransferSettings(): TransferQueueSettings {
  return useSyncExternalStore(transferQueue.subscribe, transferQueue.getSettings);
}
