/**
 * 定时任务页面：任务列表（名称/cron/目标/状态）+ 新建/编辑表单 + 执行日志。
 * 事件推送：listen('scheduler-task-event') 实时刷新列表状态。
 */

import {
  CalendarClock,
  CheckCircle2,
  CircleSlash,
  Clock,
  Loader2,
  Play,
  Plus,
  RefreshCw,
  Terminal,
  Trash2,
  X,
} from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import {
  listenTaskEvent,
  schedulerRunNow,
  schedulerTaskCreate,
  schedulerTaskDelete,
  schedulerTaskList,
  schedulerTaskLogs,
  schedulerTaskToggle,
  schedulerTaskUpdate,
  type ScheduledTask,
  type TaskLog,
} from '@/services/schedulerService';
import { cn } from '@/lib/utils';

/** cron 表达式的简单格式校验（后端 tokio-cron-scheduler 做完整校验） */
function isValidCron(expr: string): boolean {
  const parts = expr.trim().split(/\s+/);
  return parts.length === 5 && parts.every((p) => p.length > 0);
}

export function SchedulerScreen() {
  const { t } = useTranslation();
  const [tasks, setTasks] = useState<ScheduledTask[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<ScheduledTask | 'new' | null>(null);
  const [logTaskId, setLogTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<TaskLog[]>([]);
  const [error, setError] = useState<string | null>(null);
  const tasksRef = useRef(tasks);
  tasksRef.current = tasks;

  const reload = useCallback(async () => {
    try {
      const list = await schedulerTaskList();
      setTasks(list);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
    // 事件驱动刷新：任务执行开始/结束时重拉列表更新 last_status
    let unlisten: (() => void) | null = null;
    void listenTaskEvent(() => {
      void reload();
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [reload]);

  // 日志面板
  useEffect(() => {
    if (!logTaskId) {
      setLogs([]);
      return;
    }
    void schedulerTaskLogs(logTaskId, 20)
      .then(setLogs)
      .catch(() => setLogs([]));
  }, [logTaskId]);

  const handleDelete = async (id: string) => {
    try {
      await schedulerTaskDelete(id);
      if (logTaskId === id) setLogTaskId(null);
      void reload();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleToggle = async (task: ScheduledTask) => {
    try {
      await schedulerTaskToggle(task.id, !task.enabled);
      void reload();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleRunNow = async (id: string) => {
    try {
      await schedulerRunNow(id);
    } catch (e) {
      setError(String(e));
    }
  };

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-foreground-subtle select-none">
        <Spinner className="size-4" />
        {t('scheduler.loading')}
      </div>
    );
  }

  return (
    <div className="scrollbar-slim h-full overflow-y-auto p-6">
      <div className="mx-auto flex max-w-4xl flex-col gap-4">
        {/* 标题行 */}
        <div className="flex items-center gap-3">
          <CalendarClock className="size-5 text-primary" />
          <h1 className="text-ui-lg font-semibold text-foreground">{t('scheduler.title')}</h1>
          <span className="text-ui-sm text-foreground-subtlest">
            {t('scheduler.summary', { total: tasks.length })}
          </span>
          <div className="flex-1" />
          <Button variant="ghost" size="icon-sm" onClick={() => void reload()} title={t('scheduler.refresh')}>
            <RefreshCw className="size-4" />
          </Button>
          <Button
            size="sm"
            onClick={() => setEditing('new')}
            className="flex items-center gap-1.5"
          >
            <Plus className="size-4" />
            {t('scheduler.newTask')}
          </Button>
        </div>

        {error ? (
          <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-ui-sm text-destructive">
            {error}
          </div>
        ) : null}

        {/* 任务列表 */}
        {tasks.length === 0 ? (
          <div className="flex flex-col items-center gap-2 rounded-lg border border-border/60 py-12 text-foreground-subtlest select-none">
            <CalendarClock className="size-8" />
            <p className="text-ui-sm">{t('scheduler.empty')}</p>
            <Button size="sm" variant="outline" onClick={() => setEditing('new')}>
              <Plus className="size-4" />
              {t('scheduler.newTask')}
            </Button>
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            {tasks.map((task) => (
              <div
                key={task.id}
                className={cn(
                  'flex items-center gap-3 rounded-lg border px-3 py-2.5 transition-colors',
                  task.enabled
                    ? 'border-border/60 bg-surface/40'
                    : 'border-border/40 bg-surface/20 opacity-60',
                  logTaskId === task.id && 'border-primary/40',
                )}
              >
                {/* 状态图标 */}
                <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-surface">
                  {task.taskType === 'ssh_command' ? (
                    <Terminal className="size-4 text-terminal-green" />
                  ) : (
                    <Clock className="size-4 text-terminal-cyan" />
                  )}
                </div>
                {/* 信息 */}
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="truncate text-ui-sm font-medium text-foreground">
                      {task.name}
                    </span>
                    <span className="shrink-0 rounded bg-surface px-1.5 py-px font-mono text-ui-xs text-foreground-subtle">
                      {task.cronExpr}
                    </span>
                    {task.lastStatus ? (
                      <span
                        className={cn(
                          'shrink-0 rounded px-1.5 py-px text-ui-xs font-medium',
                          task.lastStatus === 'success'
                            ? 'bg-terminal-green/10 text-terminal-green'
                            : task.lastStatus === 'failed'
                              ? 'bg-destructive/10 text-destructive'
                              : 'bg-primary/10 text-primary',
                        )}
                      >
                        {task.lastStatus === 'success'
                          ? t('scheduler.statusSuccess')
                          : task.lastStatus === 'failed'
                            ? t('scheduler.statusFailed')
                            : t('scheduler.statusRunning')}
                      </span>
                    ) : null}
                  </div>
                  <div className="mt-0.5 truncate font-mono text-ui-xs text-foreground-subtlest">
                    {task.taskType === 'ssh_command' ? 'SSH' : 'LOCAL'} · {task.command}
                  </div>
                </div>
                {/* 操作 */}
                <div className="flex shrink-0 items-center gap-1">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => void handleRunNow(task.id)}
                    title={t('scheduler.runNow')}
                  >
                    <Play className="size-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => void handleToggle(task)}
                    title={task.enabled ? t('scheduler.pause') : t('scheduler.resume')}
                  >
                    {task.enabled ? (
                      <CircleSlash className="size-3.5" />
                    ) : (
                      <CheckCircle2 className="size-3.5" />
                    )}
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => setEditing(task)}
                    title={t('scheduler.edit')}
                  >
                    <Clock className="size-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => setLogTaskId(logTaskId === task.id ? null : task.id)}
                    title={t('scheduler.logs')}
                  >
                    <CalendarClock className="size-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="text-destructive hover:text-destructive"
                    onClick={() => void handleDelete(task.id)}
                    title={t('scheduler.delete')}
                  >
                    <Trash2 className="size-3.5" />
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}

        {/* 日志面板 */}
        {logTaskId ? (
          <div className="rounded-lg border border-border/60 bg-surface/20 p-3">
            <div className="mb-2 flex items-center gap-2">
              <span className="text-ui-sm font-medium text-foreground">
                {t('scheduler.logTitle', {
                  name: tasksRef.current.find((t2) => t2.id === logTaskId)?.name ?? logTaskId,
                })}
              </span>
              <div className="flex-1" />
              <button
                type="button"
                onClick={() => setLogTaskId(null)}
                className="rounded p-1 text-foreground-subtlest hover:text-foreground"
              >
                <X className="size-3.5" />
              </button>
            </div>
            {logs.length === 0 ? (
              <p className="py-4 text-center text-ui-sm text-foreground-subtlest">
                {t('scheduler.noLogs')}
              </p>
            ) : (
              <div className="flex flex-col gap-1.5 font-mono text-ui-xs">
                {logs.map((log) => (
                  <div key={log.id} className="rounded border border-border/40 p-2">
                    <div className="flex items-center gap-2 text-foreground-subtle">
                      <span>
                        {new Date(log.startedAt * 1000).toLocaleString()}
                      </span>
                      {log.exitCode != null ? (
                        <span
                          className={cn(
                            'rounded px-1 font-medium',
                            log.exitCode === 0
                              ? 'bg-terminal-green/10 text-terminal-green'
                              : 'bg-destructive/10 text-destructive',
                          )}
                        >
                          exit {log.exitCode}
                        </span>
                      ) : (
                        <span className="flex items-center gap-1 text-primary">
                          <Loader2 className="size-3 animate-spin" />
                          running
                        </span>
                      )}
                    </div>
                    {log.stdoutTail ? (
                      <pre className="mt-1 max-h-24 overflow-y-auto whitespace-pre-wrap text-foreground-subtle">
                        {log.stdoutTail}
                      </pre>
                    ) : null}
                    {log.stderrTail ? (
                      <pre className="mt-1 max-h-24 overflow-y-auto whitespace-pre-wrap text-destructive">
                        {log.stderrTail}
                      </pre>
                    ) : null}
                  </div>
                ))}
              </div>
            )}
          </div>
        ) : null}

        {/* 新建/编辑弹层 */}
        {editing ? (
          <TaskForm
            initial={editing === 'new' ? null : editing}
            onClose={() => setEditing(null)}
            onSaved={() => {
              setEditing(null);
              void reload();
            }}
          />
        ) : null}
      </div>
    </div>
  );
}

// ── 任务表单 ────────────────────────────────────────────────────────────────

function TaskForm({
  initial,
  onClose,
  onSaved,
}: {
  initial: ScheduledTask | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(initial?.name ?? '');
  const [cronExpr, setCronExpr] = useState(initial?.cronExpr ?? '*/5 * * * *');
  const [taskType, setTaskType] = useState(initial?.taskType ?? 'local_command');
  const [command, setCommand] = useState(initial?.command ?? '');
  const [workingDir, setWorkingDir] = useState(initial?.workingDir ?? '');
  const [hostId, setHostId] = useState(initial?.hostId?.toString() ?? '');
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  const cronValid = isValidCron(cronExpr);
  const canSave = name.trim() && cronValid && command.trim();

  const handleSave = async () => {
    if (!canSave || saving) return;
    setSaving(true);
    setFormError(null);
    const task: ScheduledTask = {
      id: initial?.id ?? crypto.randomUUID(),
      name: name.trim(),
      cronExpr: cronExpr.trim(),
      taskType,
      command: command.trim(),
      workingDir: workingDir.trim() || null,
      hostId: taskType === 'ssh_command' && hostId ? Number(hostId) : null,
      enabled,
      createdAt: initial?.createdAt ?? 0,
      updatedAt: initial?.updatedAt ?? 0,
      lastRunAt: initial?.lastRunAt ?? null,
      lastStatus: initial?.lastStatus ?? null,
    };
    try {
      if (initial) {
        await schedulerTaskUpdate(task);
      } else {
        await schedulerTaskCreate(task);
      }
      onSaved();
    } catch (e) {
      setFormError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const inputCls =
    'h-8 w-full rounded-md border border-border/60 bg-background px-2.5 text-ui-sm text-foreground outline-none transition-colors focus:border-border placeholder:text-foreground-subtlest';
  const labelCls = 'mb-1 block text-ui-xs font-medium text-foreground-subtle';

  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/25 p-6"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
    >
      <div
        className="flex w-[480px] max-w-full flex-col overflow-hidden rounded-xl border border-border bg-popover shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="border-b border-border/50 px-4 py-3">
          <span className="text-ui-base font-medium text-foreground">
            {initial ? t('scheduler.editTitle') : t('scheduler.newTitle')}
          </span>
        </div>
        <div className="flex flex-col gap-3 px-4 py-3">
          <div>
            <label className={labelCls}>{t('scheduler.fieldName')}</label>
            <input
              className={inputCls}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t('scheduler.fieldNamePlaceholder')}
            />
          </div>
          <div>
            <label className={labelCls}>
              {t('scheduler.fieldCron')}{' '}
              <span className="font-mono text-foreground-subtlest">* * * * *</span>
            </label>
            <input
              className={cn(inputCls, 'font-mono', !cronValid && 'border-destructive/50')}
              value={cronExpr}
              onChange={(e) => setCronExpr(e.target.value)}
              placeholder="*/5 * * * *"
            />
            {!cronValid ? (
              <p className="mt-1 text-ui-xs text-destructive">
                {t('scheduler.cronInvalid')}
              </p>
            ) : null}
          </div>
          <div>
            <label className={labelCls}>{t('scheduler.fieldType')}</label>
            <div className="flex gap-2">
              {(
                [
                  ['local_command', t('scheduler.typeLocal')],
                  ['ssh_command', t('scheduler.typeSsh')],
                ] as const
              ).map(([value, label]) => (
                <button
                  key={value}
                  type="button"
                  onClick={() => setTaskType(value)}
                  className={cn(
                    'flex-1 rounded-md border px-3 py-1.5 text-ui-sm transition-colors',
                    taskType === value
                      ? 'border-primary/40 bg-primary/10 font-medium text-primary'
                      : 'border-border/60 text-foreground-subtle hover:bg-hover',
                  )}
                >
                  {label}
                </button>
              ))}
            </div>
          </div>
          <div>
            <label className={labelCls}>{t('scheduler.fieldCommand')}</label>
            <textarea
              className={cn(inputCls, 'h-20 resize-none py-2 font-mono')}
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              placeholder={t('scheduler.fieldCommandPlaceholder')}
            />
          </div>
          {taskType === 'local_command' ? (
            <div>
              <label className={labelCls}>{t('scheduler.fieldWorkDir')}</label>
              <input
                className={inputCls}
                value={workingDir}
                onChange={(e) => setWorkingDir(e.target.value)}
                placeholder="/tmp（可选）"
              />
            </div>
          ) : (
            <div>
              <label className={labelCls}>{t('scheduler.fieldHostId')}</label>
              <input
                className={cn(inputCls, 'font-mono')}
                type="number"
                value={hostId}
                onChange={(e) => setHostId(e.target.value)}
                placeholder="1（主机列表的 id）"
              />
              <p className="mt-1 text-ui-xs text-foreground-subtlest">
                {t('scheduler.fieldHostIdHint')}
              </p>
            </div>
          )}
          <label className="flex items-center gap-2 text-ui-sm text-foreground-subtle select-none">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
              className="accent-primary"
            />
            {t('scheduler.fieldEnabled')}
          </label>
          {formError ? (
            <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-ui-xs text-destructive">
              {formError}
            </div>
          ) : null}
        </div>
        <div className="flex justify-end gap-2 border-t border-border/50 px-4 py-3">
          <Button variant="ghost" onClick={onClose}>
            {t('scheduler.cancel')}
          </Button>
          <Button
            size="sm"
            disabled={!canSave || saving}
            onClick={() => void handleSave()}
            className="flex items-center gap-1.5"
          >
            {saving ? <Spinner className="size-3.5" /> : null}
            {t('scheduler.save')}
          </Button>
        </div>
      </div>
    </div>
  );
}
