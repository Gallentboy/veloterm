/**
 * 定时任务服务层：后端 scheduler 模块的 CRUD + 日志 + 立即执行 + 事件监听。
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface ScheduledTask {
  id: string;
  name: string;
  cronExpr: string;
  /** 'local_command' | 'ssh_command' */
  taskType: string;
  command: string;
  workingDir?: string | null;
  hostId?: number | null;
  enabled: boolean;
  createdAt: number;
  updatedAt: number;
  lastRunAt?: number | null;
  lastStatus?: string | null;
}

export interface TaskLog {
  id: number;
  taskId: string;
  startedAt: number;
  finishedAt?: number | null;
  exitCode?: number | null;
  stdoutTail?: string | null;
  stderrTail?: string | null;
}

export interface TaskEvent {
  taskId: string;
  taskName: string;
  kind: 'started' | 'success' | 'failed';
  startedAt: number;
  finishedAt?: number | null;
  exitCode?: number | null;
  message?: string | null;
}

export function schedulerTaskList(): Promise<ScheduledTask[]> {
  return invoke('scheduler_task_list');
}

export function schedulerTaskCreate(task: ScheduledTask): Promise<ScheduledTask> {
  return invoke('scheduler_task_create', { task });
}

export function schedulerTaskUpdate(task: ScheduledTask): Promise<ScheduledTask> {
  return invoke('scheduler_task_update', { task });
}

export function schedulerTaskDelete(id: string): Promise<void> {
  return invoke('scheduler_task_delete', { id });
}

export function schedulerTaskToggle(id: string, enabled: boolean): Promise<ScheduledTask> {
  return invoke('scheduler_task_toggle', { id, enabled });
}

export function schedulerTaskLogs(taskId: string, limit?: number): Promise<TaskLog[]> {
  return invoke('scheduler_task_logs', { taskId, limit });
}

export function schedulerRunNow(id: string): Promise<void> {
  return invoke('scheduler_run_now', { id });
}

export function listenTaskEvent(cb: (event: TaskEvent) => void): Promise<UnlistenFn> {
  return listen<TaskEvent>('scheduler-task-event', (e) => cb(e.payload));
}
