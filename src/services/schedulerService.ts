/**
 * 定时任务服务层：后端 scheduler 模块的 CRUD + 日志 + 立即执行 + 事件监听。
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface ScheduledTask {
  id: string;
  name: string;
  cronExpr: string;
  /** 'local_command' | 'ssh_command' | 'js_script' */
  taskType: 'local_command' | 'ssh_command' | 'js_script' | string;
  command: string;
  workingDir?: string | null;
  hostId?: number | null;
  enabled: boolean;
  createdAt: number;
  updatedAt: number;
  lastRunAt?: number | null;
  lastStatus?: string | null;
  // D1 失败通知
  notifyOn?: string | null;
  webhookUrl?: string | null;
  // D2 超时与重试
  timeoutSecs?: number | null;
  maxRetries?: number | null;
  retryBackoffSecs?: number | null;
  // D4 一次性任务
  runAt?: number | null;
  // D5 历史保留
  retentionDays?: number | null;
  // 环境变量配置（JSON 格式字符串，如 {"PORT":"8080"}）
  envVars?: string | null;
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

export interface TaskHistoryQuery {
  taskId?: string | null;
  status?: 'all' | 'success' | 'failed' | 'running' | string | null;
  limit?: number | null;
  offset?: number | null;
}

export interface TaskHistoryItem {
  id: number;
  taskId: string;
  taskName: string;
  taskType?: string | null;
  command?: string | null;
  startedAt: number;
  finishedAt?: number | null;
  durationMs?: number | null;
  exitCode?: number | null;
  status: 'success' | 'failed' | 'running' | string;
  stdoutTail?: string | null;
  stderrTail?: string | null;
}

export interface TaskHistoryResponse {
  items: TaskHistoryItem[];
  total: number;
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

export function schedulerTaskHistory(query?: TaskHistoryQuery): Promise<TaskHistoryResponse> {
  return invoke('scheduler_task_history', { query });
}

export function schedulerHistoryClear(taskId?: string | null): Promise<void> {
  return invoke('scheduler_history_clear', { taskId });
}

export function schedulerRunNow(id: string): Promise<void> {
  return invoke('scheduler_run_now', { id });
}

/** D3: Cron 表达式的未来触发预览（返回接下来 N 次 Unix 秒时间戳） */
export function schedulerPreviewNext(cronExpr: string, count?: number): Promise<number[]> {
  return invoke('scheduler_preview_next', { cronExpr, count });
}

/** D3: Cron 表达式的人话翻译 */
export function schedulerDescribeCron(cronExpr: string): Promise<string> {
  return invoke('scheduler_describe_cron', { cronExpr });
}

export function listenTaskEvent(cb: (event: TaskEvent) => void): Promise<UnlistenFn> {
  return listen<TaskEvent>('scheduler-task-event', (e) => cb(e.payload));
}
