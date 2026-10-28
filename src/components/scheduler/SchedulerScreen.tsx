/**
 * 定时任务页面：
 * 1. 任务列表（名称/cron/目标/状态）+ 新建/编辑表单 + 单任务日志
 * 2. 全局执行历史表（时间/任务/耗时/状态/退出码）+ 条件查询/筛选 + 详情查看与复制
 * 响应式布局：铺满右侧主内容区，大屏自适应约 80% 宽度居中，较小屏幕 100% 铺满。
 * 事件驱动：listenTaskEvent 实时刷新状态与历史。
 */

import {
  Bell,
  Braces,
  CalendarClock,
  Check,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  ChevronDown,
  ChevronUp,
  CircleSlash,
  Clock,
  Copy,
  History,
  ListTodo,
  Loader2,
  Play,
  Plus,
  RefreshCw,
  Sliders,
  Sparkles,
  SquarePen,
  Terminal,
  Trash2,
  X,
} from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { Button } from '@/components/ui/button';
import { SelectMenu } from '@/components/ui/SelectMenu';
import { Spinner } from '@/components/ui/spinner';
import {
  listenTaskEvent,
  schedulerDescribeCron,
  schedulerHistoryClear,
  schedulerPreviewNext,
  schedulerRunNow,
  schedulerTaskCreate,
  schedulerTaskDelete,
  schedulerTaskHistory,
  schedulerTaskList,
  schedulerTaskLogs,
  schedulerTaskToggle,
  schedulerTaskUpdate,
  type ScheduledTask,
  type TaskHistoryItem,
  type TaskLog,
} from '@/services/schedulerService';
import { cn } from '@/lib/utils';

/** cron 表达式格式适配：标准 5 字段自动补前导 0 秒位 */
function normalizeCron(expr: string): string {
  const parts = expr.trim().split(/\s+/);
  if (parts.length === 5) return `0 ${parts.join(' ')}`;
  return expr.trim();
}

function isValidCron(expr: string): boolean {
  const parts = expr.trim().split(/\s+/);
  return (parts.length === 5 || parts.length === 6) && parts.every((p) => p.length > 0);
}

function formatDuration(ms?: number | null): string {
  if (ms == null) return '-';
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

function formatTimestamp(ts: number): string {
  if (!ts) return '-';
  const d = new Date(ts * 1000);
  return d.toLocaleString();
}

export function SchedulerScreen() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<'tasks' | 'history'>('tasks');
  const [tasks, setTasks] = useState<ScheduledTask[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<ScheduledTask | 'new' | null>(null);
  const [logTaskId, setLogTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<TaskLog[]>([]);
  const [error, setError] = useState<string | null>(null);

  // 执行历史状态
  const [historyItems, setHistoryItems] = useState<TaskHistoryItem[]>([]);
  const [historyTotal, setHistoryTotal] = useState(0);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [filterTaskId, setFilterTaskId] = useState<string>('all');
  const [filterStatus, setFilterStatus] = useState<string>('all');
  const [page, setPage] = useState(0);
  const pageSize = 20;

  // 详情弹窗
  const [detailItem, setDetailItem] = useState<TaskHistoryItem | null>(null);

  const tasksRef = useRef(tasks);
  tasksRef.current = tasks;

  const reloadTasks = useCallback(async () => {
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

  const reloadHistory = useCallback(async () => {
    setHistoryLoading(true);
    try {
      const resp = await schedulerTaskHistory({
        taskId: filterTaskId === 'all' ? undefined : filterTaskId,
        status: filterStatus === 'all' ? undefined : filterStatus,
        limit: pageSize,
        offset: page * pageSize,
      });
      setHistoryItems(resp.items);
      setHistoryTotal(resp.total);
    } catch (e) {
      setError(String(e));
    } finally {
      setHistoryLoading(false);
    }
  }, [filterTaskId, filterStatus, page]);

  // 初次加载与事件监听
  useEffect(() => {
    void reloadTasks();
    let unlisten: (() => void) | null = null;
    void listenTaskEvent(() => {
      void reloadTasks();
      void reloadHistory();
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [reloadTasks, reloadHistory]);

  // 历史参数变动时拉取历史
  useEffect(() => {
    if (activeTab === 'history') {
      void reloadHistory();
    }
  }, [activeTab, reloadHistory]);

  // 任务内联日志拉取
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
      void reloadTasks();
      void reloadHistory();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleToggle = async (task: ScheduledTask) => {
    try {
      await schedulerTaskToggle(task.id, !task.enabled);
      void reloadTasks();
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

  const handleClearHistory = async () => {
    if (!window.confirm(t('scheduler.historyClearConfirm'))) return;
    try {
      await schedulerHistoryClear(filterTaskId === 'all' ? undefined : filterTaskId);
      setPage(0);
      void reloadHistory();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleGoTaskHistory = (taskId: string) => {
    setFilterTaskId(taskId);
    setFilterStatus('all');
    setPage(0);
    setActiveTab('history');
  };

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-foreground-subtle select-none">
        <Spinner className="size-4" />
        {t('scheduler.loading')}
      </div>
    );
  }

  const totalPages = Math.ceil(historyTotal / pageSize);

  return (
    <div className="scrollbar-slim h-full overflow-y-auto p-4 md:p-6 lg:p-8">
      {/* 响应式主体容器：大屏占 80% 居中，小屏 100% 铺满 */}
      <div className="w-full xl:w-[80%] lg:w-[85%] md:w-[92%] mx-auto flex flex-col gap-5">
        {/* 顶部标题与 Tab 栏 */}
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/50 pb-4">
          <div className="flex items-center gap-3">
            <div className="flex size-9 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <CalendarClock className="size-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-ui-lg font-semibold text-foreground">{t('scheduler.title')}</h1>
                <span className="rounded-full bg-surface px-2 py-0.5 text-ui-xs text-foreground-subtle">
                  {t('scheduler.summary', { total: tasks.length })}
                </span>
              </div>
            </div>
          </div>

          {/* 视图 Tab 切换与主操作区 */}
          <div className="flex items-center gap-2.5">
            <div className="flex items-center rounded-lg border border-border/60 bg-surface/50 p-0.5">
              <button
                type="button"
                onClick={() => setActiveTab('tasks')}
                className={cn(
                  'flex items-center gap-1.5 rounded-md px-3 py-1.5 text-ui-xs font-medium transition-colors',
                  activeTab === 'tasks'
                    ? 'bg-background text-foreground shadow-sm'
                    : 'text-foreground-subtle hover:text-foreground',
                )}
              >
                <ListTodo className="size-3.5" />
                {t('scheduler.tabTasks')}
              </button>
              <button
                type="button"
                onClick={() => setActiveTab('history')}
                className={cn(
                  'flex items-center gap-1.5 rounded-md px-3 py-1.5 text-ui-xs font-medium transition-colors',
                  activeTab === 'history'
                    ? 'bg-background text-foreground shadow-sm'
                    : 'text-foreground-subtle hover:text-foreground',
                )}
              >
                <History className="size-3.5" />
                {t('scheduler.tabHistory')}
              </button>
            </div>

            <Button
              variant="outline"
              size="icon-sm"
              onClick={() => {
                void reloadTasks();
                if (activeTab === 'history') void reloadHistory();
              }}
              title={t('scheduler.refresh')}
            >
              <RefreshCw className={cn('size-4', historyLoading && 'animate-spin')} />
            </Button>

            {activeTab === 'tasks' ? (
              <Button
                size="sm"
                onClick={() => setEditing('new')}
                className="flex items-center gap-1.5"
              >
                <Plus className="size-4" />
                {t('scheduler.newTask')}
              </Button>
            ) : (
              <Button
                variant="outline"
                size="sm"
                onClick={() => void handleClearHistory()}
                className="flex items-center gap-1.5 text-destructive hover:text-destructive"
              >
                <Trash2 className="size-3.5" />
                {t('scheduler.historyClear')}
              </Button>
            )}
          </div>
        </div>

        {error ? (
          <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-ui-sm text-destructive">
            {error}
          </div>
        ) : null}

        {/* 视图一：任务列表 */}
        {activeTab === 'tasks' && (
          <div className="flex flex-col gap-4">
            {tasks.length === 0 ? (
              <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border/80 py-16 text-foreground-subtlest select-none">
                <CalendarClock className="size-10 stroke-[1.5]" />
                <p className="text-ui-sm">{t('scheduler.empty')}</p>
                <Button size="sm" variant="outline" onClick={() => setEditing('new')}>
                  <Plus className="size-4" />
                  {t('scheduler.newTask')}
                </Button>
              </div>
            ) : (
              <div className="flex flex-col gap-2.5">
                {tasks.map((task) => (
                  <div
                    key={task.id}
                    className={cn(
                      'flex items-center gap-3.5 rounded-xl border px-4 py-3 transition-colors',
                      task.enabled
                        ? 'border-border/60 bg-surface/30 hover:border-border'
                        : 'border-border/40 bg-surface/10 opacity-60',
                      logTaskId === task.id && 'border-primary/50 ring-1 ring-primary/20',
                    )}
                  >
                    {/* 类型图标 */}
                    <div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-surface border border-border/40">
                      {task.taskType === 'js_script' ? (
                        <Braces className="size-4 text-terminal-yellow" />
                      ) : task.taskType === 'ssh_command' ? (
                        <Terminal className="size-4 text-terminal-green" />
                      ) : (
                        <Clock className="size-4 text-terminal-cyan" />
                      )}
                    </div>

                    {/* 任务核心信息 */}
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="truncate text-ui-sm font-semibold text-foreground">
                          {task.name}
                        </span>
                        {task.runAt ? (
                          <span className="shrink-0 rounded bg-terminal-yellow/10 px-2 py-0.5 font-mono text-ui-xs text-terminal-yellow border border-terminal-yellow/20">
                            {t('scheduler.badgeOnce')} · {formatTimestamp(task.runAt)}
                          </span>
                        ) : (
                          <span className="shrink-0 rounded bg-surface/80 px-2 py-0.5 font-mono text-ui-xs text-foreground-subtle border border-border/40">
                            {task.cronExpr}
                          </span>
                        )}
                        {task.notifyOn && task.notifyOn !== 'none' ? (
                          <span
                            className="shrink-0 rounded bg-terminal-cyan/10 px-1.5 py-0.5 font-mono text-[10px] text-terminal-cyan border border-terminal-cyan/20 flex items-center gap-0.5"
                            title={task.webhookUrl ? `Webhook: ${task.webhookUrl}` : t('scheduler.badgeNotify')}
                          >
                            <Bell className="size-2.5" />
                            {t('scheduler.badgeNotify')}
                          </span>
                        ) : null}
                        {task.maxRetries && task.maxRetries > 0 ? (
                          <span
                            className="shrink-0 rounded bg-surface/80 px-1.5 py-0.5 font-mono text-[10px] text-foreground-subtle border border-border/40"
                            title={t('scheduler.badgeRetry', { count: task.maxRetries })}
                          >
                            {t('scheduler.badgeRetry', { count: task.maxRetries })}
                          </span>
                        ) : null}
                        {task.envVars ? (
                          <span
                            className="shrink-0 rounded bg-primary/10 px-1.5 py-0.5 font-mono text-[10px] text-primary border border-primary/20"
                            title={t('scheduler.envBadge')}
                          >
                            ENV
                          </span>
                        ) : null}
                        {task.lastStatus ? (
                          <span
                            className={cn(
                              'shrink-0 rounded px-2 py-0.5 text-ui-xs font-medium',
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
                      <div className="mt-1 flex items-center gap-2 truncate font-mono text-ui-xs text-foreground-subtlest">
                        <span className="font-semibold text-foreground-subtle uppercase">
                          {task.taskType === 'js_script'
                            ? 'JS'
                            : task.taskType === 'ssh_command'
                              ? `SSH (host #${task.hostId ?? '?'})`
                              : 'LOCAL'}
                        </span>
                        <span>·</span>
                        <span className="truncate">{task.command}</span>
                      </div>
                    </div>

                    {/* 操作动作 */}
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
                        <SquarePen className="size-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={() => handleGoTaskHistory(task.id)}
                        title={t('scheduler.viewTaskHistory')}
                      >
                        <History className="size-3.5 text-foreground-subtle" />
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

            {/* 单任务内联快速日志面板 */}
            {logTaskId ? (
              <div className="rounded-xl border border-border/70 bg-surface/30 p-4 shadow-sm">
                <div className="mb-3 flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <CalendarClock className="size-4 text-primary" />
                    <span className="text-ui-sm font-semibold text-foreground">
                      {t('scheduler.logTitle', {
                        name: tasksRef.current.find((t2) => t2.id === logTaskId)?.name ?? logTaskId,
                      })}
                    </span>
                  </div>
                  <button
                    type="button"
                    onClick={() => setLogTaskId(null)}
                    className="rounded p-1 text-foreground-subtlest hover:text-foreground"
                  >
                    <X className="size-4" />
                  </button>
                </div>

                {logs.length === 0 ? (
                  <p className="py-6 text-center text-ui-sm text-foreground-subtlest">
                    {t('scheduler.noLogs')}
                  </p>
                ) : (
                  <div className="flex flex-col gap-2.5">
                    {logs.map((log) => (
                      <div
                        key={log.id}
                        className="rounded-lg border border-border/50 overflow-hidden bg-background"
                      >
                        <div className="flex items-center gap-2 border-b border-border/30 bg-surface/40 px-3 py-1.5 text-ui-xs">
                          <span className="text-foreground-subtle font-medium">
                            {formatTimestamp(log.startedAt)}
                          </span>
                          {log.exitCode != null ? (
                            <span
                              className={cn(
                                'rounded px-1.5 py-px font-mono font-semibold',
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
                          <span className="flex-1" />
                          {log.finishedAt != null && log.startedAt != null ? (
                            <span className="text-foreground-subtlest font-mono">
                              {((log.finishedAt - log.startedAt) * 1000).toFixed(0)}ms
                            </span>
                          ) : null}
                        </div>
                        {log.stdoutTail || log.stderrTail ? (
                          <div className="max-h-48 overflow-y-auto bg-background px-3.5 py-2.5 scrollbar-slim font-mono text-[11px] leading-relaxed border-t border-border/30">
                            {log.stdoutTail ? (
                              <pre className="whitespace-pre-wrap break-all text-foreground">
                                {log.stdoutTail}
                              </pre>
                            ) : null}
                            {log.stderrTail ? (
                              <pre
                                className={cn(
                                  'whitespace-pre-wrap break-all text-destructive',
                                  log.stdoutTail && 'mt-1.5',
                                )}
                              >
                                {log.stderrTail}
                              </pre>
                            ) : null}
                          </div>
                        ) : (
                          <div className="bg-background px-3.5 py-2 text-[11px] text-foreground-subtlest font-mono select-none border-t border-border/30">
                            {t('scheduler.noOutput')}
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            ) : null}
          </div>
        )}

        {/* 视图二：执行历史表与查询 */}
        {activeTab === 'history' && (
          <div className="flex flex-col gap-4">
            {/* 筛选与查询栏 */}
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border/60 bg-surface/30 p-3.5">
              <div className="flex flex-wrap items-center gap-3">
                {/* 任务筛选（项目自绘 SelectMenu——原生 select 弹层系统绘制，观感割裂不可控） */}
                <div className="flex items-center gap-1.5">
                  <span className="text-ui-xs text-foreground-subtle font-medium">
                    {t('scheduler.colTask')}:
                  </span>
                  <SelectMenu
                    value={filterTaskId}
                    options={[
                      { value: 'all', label: t('scheduler.historyFilterTask') },
                      ...tasks.map((task) => ({ value: task.id, label: task.name })),
                    ]}
                    onChange={(v) => {
                      setFilterTaskId(v);
                      setPage(0);
                    }}
                    className="w-48"
                    triggerClassName="h-8 rounded-md border-border/60 bg-background px-2.5 text-ui-xs hover:border-border"
                  />
                </div>

                {/* 状态筛选 */}
                <div className="flex items-center gap-1.5">
                  <span className="text-ui-xs text-foreground-subtle font-medium">
                    {t('scheduler.colStatus')}:
                  </span>
                  <SelectMenu
                    value={filterStatus}
                    options={[
                      { value: 'all', label: t('scheduler.historyFilterStatus') },
                      { value: 'success', label: t('scheduler.statusSuccess') },
                      { value: 'failed', label: t('scheduler.statusFailed') },
                      { value: 'running', label: t('scheduler.statusRunning') },
                    ]}
                    onChange={(v) => {
                      setFilterStatus(v);
                      setPage(0);
                    }}
                    className="w-32"
                    triggerClassName="h-8 rounded-md border-border/60 bg-background px-2.5 text-ui-xs hover:border-border"
                  />
                </div>
              </div>

              {/* 统计数量 */}
              <div className="text-ui-xs text-foreground-subtle">
                {t('scheduler.historySummary', { total: historyTotal })}
              </div>
            </div>

            {/* 历史记录表格 */}
            <div className="rounded-xl border border-border/60 bg-background overflow-hidden shadow-sm">
              <div className="overflow-x-auto">
                <table className="w-full text-left text-ui-xs">
                  <thead>
                    <tr className="border-b border-border/60 bg-surface/50 text-foreground-subtle select-none">
                      <th className="px-4 py-3 font-medium">{t('scheduler.colTask')}</th>
                      <th className="px-4 py-3 font-medium">{t('scheduler.commandLabel')}</th>
                      <th className="px-4 py-3 font-medium">{t('scheduler.colTime')}</th>
                      <th className="px-4 py-3 font-medium">{t('scheduler.colDuration')}</th>
                      <th className="px-4 py-3 font-medium">{t('scheduler.colStatus')}</th>
                      <th className="px-4 py-3 text-right font-medium">{t('scheduler.colAction')}</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border/40">
                    {historyItems.length === 0 ? (
                      <tr>
                        <td colSpan={6} className="py-12 text-center text-foreground-subtlest">
                          <History className="mx-auto size-7 mb-2 opacity-50" />
                          <p>{t('scheduler.historyEmpty')}</p>
                        </td>
                      </tr>
                    ) : (
                      historyItems.map((item) => (
                        <tr key={item.id} className="hover:bg-hover/50 transition-colors">
                          <td className="px-4 py-3">
                            <div className="font-semibold text-foreground text-ui-sm">
                              {item.taskName}
                            </div>
                            <div className="text-[11px] text-foreground-subtlest font-mono">
                              {item.taskType === 'js_script'
                                ? 'JS'
                                : item.taskType === 'ssh_command'
                                  ? 'SSH'
                                  : 'LOCAL'}
                            </div>
                          </td>
                          <td className="px-4 py-3 max-w-[260px]">
                            <div className="truncate font-mono text-foreground-subtle" title={item.command ?? ''}>
                              {item.command?.replace(/\r?\n+/g, ' ') || '-'}
                            </div>
                          </td>
                          <td className="px-4 py-3 font-mono text-foreground-subtle whitespace-nowrap">
                            {formatTimestamp(item.startedAt)}
                          </td>
                          <td className="px-4 py-3 font-mono text-foreground-subtle whitespace-nowrap">
                            {formatDuration(item.durationMs)}
                          </td>
                          <td className="px-4 py-3 whitespace-nowrap">
                            {item.status === 'success' ? (
                              <span className="inline-flex items-center gap-1 rounded bg-terminal-green/10 px-2 py-0.5 font-medium text-terminal-green">
                                <Check className="size-3" />
                                {t('scheduler.statusSuccess')}
                                <span className="font-mono text-[10px] opacity-80">(0)</span>
                              </span>
                            ) : item.status === 'failed' ? (
                              <span className="inline-flex items-center gap-1 rounded bg-destructive/10 px-2 py-0.5 font-medium text-destructive">
                                <X className="size-3" />
                                {t('scheduler.statusFailed')}
                                {item.exitCode != null ? (
                                  <span className="font-mono text-[10px]">({item.exitCode})</span>
                                ) : null}
                              </span>
                            ) : (
                              <span className="inline-flex items-center gap-1 rounded bg-primary/10 px-2 py-0.5 font-medium text-primary">
                                <Loader2 className="size-3 animate-spin" />
                                {t('scheduler.statusRunning')}
                              </span>
                            )}
                          </td>
                          <td className="px-4 py-3 text-right whitespace-nowrap">
                            <Button
                              variant="ghost"
                              size="sm"
                              className="h-7 px-2.5 text-xs text-primary hover:text-primary"
                              onClick={() => setDetailItem(item)}
                            >
                              {t('scheduler.viewDetail')}
                            </Button>
                          </td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
              </div>

              {/* 分页控制栏 */}
              {totalPages > 1 && (
                <div className="flex items-center justify-between border-t border-border/50 px-4 py-2.5 bg-surface/30">
                  <span className="text-ui-xs text-foreground-subtle">
                    {page + 1} / {totalPages}
                  </span>
                  <div className="flex items-center gap-1.5">
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={page <= 0}
                      onClick={() => setPage((p) => Math.max(0, p - 1))}
                      className="h-7 px-2"
                    >
                      <ChevronLeft className="size-3.5" />
                      {t('scheduler.prevPage')}
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={page >= totalPages - 1}
                      onClick={() => setPage((p) => p + 1)}
                      className="h-7 px-2"
                    >
                      {t('scheduler.nextPage')}
                      <ChevronRight className="size-3.5" />
                    </Button>
                  </div>
                </div>
              )}
            </div>
          </div>
        )}

        {/* 新建/编辑任务弹窗 */}
        {editing ? (
          <TaskForm
            initial={editing === 'new' ? null : editing}
            onClose={() => setEditing(null)}
            onSaved={() => {
              setEditing(null);
              void reloadTasks();
              void reloadHistory();
            }}
          />
        ) : null}

        {/* 执行详情与输出查看弹窗 */}
        {detailItem ? (
          <HistoryDetailModal
            item={detailItem}
            onClose={() => setDetailItem(null)}
          />
        ) : null}
      </div>
    </div>
  );
}

// ── 执行历史详情弹窗 ────────────────────────────────────────────────────────

function HistoryDetailModal({
  item,
  onClose,
}: {
  item: TaskHistoryItem;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const [copiedCommand, setCopiedCommand] = useState(false);
  const [expandCommand, setExpandCommand] = useState(false);

  const fullOutput = [item.stdoutTail, item.stderrTail].filter(Boolean).join('\n') || '';

  const handleCopy = async () => {
    if (!fullOutput) return;
    try {
      await navigator.clipboard.writeText(fullOutput);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // 忽略
    }
  };

  const handleCopyCommand = async () => {
    if (!item.command) return;
    try {
      await navigator.clipboard.writeText(item.command);
      setCopiedCommand(true);
      setTimeout(() => setCopiedCommand(false), 2000);
    } catch {
      // 忽略
    }
  };

  const isJs = item.taskType === 'js_script';
  const isCommandMultiLine = Boolean(
    item.command && (item.command.includes('\n') || item.command.length > 80),
  );

  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/40 p-4 backdrop-blur-xs"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
    >
      <div
        className="flex w-[720px] max-w-full max-h-[85vh] flex-col overflow-hidden rounded-xl border border-border bg-popover shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 头部：固定 */}
        <div className="flex items-center justify-between border-b border-border/50 px-5 py-3.5 shrink-0">
          <div className="flex items-center gap-2 min-w-0">
            <History className="size-4 text-primary shrink-0" />
            <h3 className="truncate text-ui-base font-semibold text-foreground">
              {t('scheduler.dialogDetailTitle', { name: item.taskName })}
            </h3>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded p-1 text-foreground-subtle hover:text-foreground cursor-pointer"
          >
            <X className="size-4" />
          </button>
        </div>

        {/* 中间主体区域：独立滚动区，彻底杜绝内容过长撑爆弹窗 */}
        <div className="flex-1 min-h-0 overflow-y-auto scrollbar-slim flex flex-col">
          {/* 元数据网格 */}
          <div className="grid grid-cols-2 md:grid-cols-4 gap-3 bg-surface/30 px-5 py-3 border-b border-border/40 text-ui-xs shrink-0">
            <div>
              <span className="text-foreground-subtlest block mb-0.5">{t('scheduler.startedAtLabel')}</span>
              <span className="font-mono text-foreground">{formatTimestamp(item.startedAt)}</span>
            </div>
            <div>
              <span className="text-foreground-subtlest block mb-0.5">{t('scheduler.colDuration')}</span>
              <span className="font-mono text-foreground">{formatDuration(item.durationMs)}</span>
            </div>
            <div>
              <span className="text-foreground-subtlest block mb-0.5">{t('scheduler.colStatus')}</span>
              <span className="font-medium">
                {item.status === 'success' ? (
                  <span className="text-terminal-green">{t('scheduler.statusSuccess')} (0)</span>
                ) : item.status === 'failed' ? (
                  <span className="text-destructive">
                    {t('scheduler.statusFailed')} ({item.exitCode ?? 'Err'})
                  </span>
                ) : (
                  <span className="text-primary">{t('scheduler.statusRunning')}</span>
                )}
              </span>
            </div>
            <div>
              <span className="text-foreground-subtlest block mb-0.5">{t('scheduler.fieldType')}</span>
              <span className="font-mono text-foreground uppercase">
                {item.taskType === 'js_script' ? 'JS' : item.taskType === 'ssh_command' ? 'SSH' : 'LOCAL'}
              </span>
            </div>
          </div>

          {/* 触发命令 / 脚本内容：做最大高度限制 + 局部滚动 + 展开/收起 + 复制 */}
          {item.command ? (
            <div className="px-5 py-2.5 border-b border-border/40 bg-surface/20 shrink-0">
              <div className="flex items-center justify-between mb-1.5">
                <span className="text-ui-xs text-foreground-subtlest font-medium">
                  {isJs ? t('scheduler.fieldScript') : t('scheduler.commandLabel')}
                </span>
                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={() => void handleCopyCommand()}
                    className="flex items-center gap-1 text-[11px] text-foreground-subtle hover:text-foreground transition-colors cursor-pointer"
                    title={t('scheduler.copyCode')}
                  >
                    {copiedCommand ? (
                      <Check className="size-3 text-terminal-green" />
                    ) : (
                      <Copy className="size-3" />
                    )}
                    <span>{copiedCommand ? t('scheduler.copied') : t('scheduler.copy')}</span>
                  </button>
                  {isCommandMultiLine ? (
                    <button
                      type="button"
                      onClick={() => setExpandCommand(!expandCommand)}
                      className="flex items-center gap-0.5 text-[11px] text-primary hover:text-primary transition-colors cursor-pointer"
                    >
                      {expandCommand ? (
                        <>
                          <ChevronUp className="size-3" />
                          <span>{t('scheduler.collapse')}</span>
                        </>
                      ) : (
                        <>
                          <ChevronDown className="size-3" />
                          <span>{t('scheduler.expand')}</span>
                        </>
                      )}
                    </button>
                  ) : null}
                </div>
              </div>
              <div
                className={cn(
                  'rounded bg-background border border-border/40 p-2.5 font-mono text-[11px] text-foreground-subtle scrollbar-slim',
                  expandCommand ? 'max-h-60 overflow-y-auto' : 'max-h-24 overflow-y-auto',
                )}
              >
                <pre className="whitespace-pre-wrap break-all leading-relaxed">{item.command}</pre>
              </div>
            </div>
          ) : null}

          {/* 输出控制台：背景色与当前主色一致，设置充足高度且可独立滚动 */}
          <div className="flex-1 min-h-[220px] overflow-y-auto bg-background p-4 font-mono text-[12px] leading-relaxed scrollbar-slim">
            {item.stdoutTail ? (
              <div className="mb-2">
                <span className="text-[10px] font-bold text-foreground-subtlest uppercase select-none block mb-1">
                  ── {t('scheduler.stdoutLabel')} ──
                </span>
                <pre className="whitespace-pre-wrap break-all text-foreground">
                  {item.stdoutTail}
                </pre>
              </div>
            ) : null}
            {item.stderrTail ? (
              <div>
                <span className="text-[10px] font-bold text-destructive uppercase select-none block mb-1">
                  ── {t('scheduler.stderrLabel')} ──
                </span>
                <pre className="whitespace-pre-wrap break-all text-destructive">
                  {item.stderrTail}
                </pre>
              </div>
            ) : null}
            {!item.stdoutTail && !item.stderrTail && (
              <div className="text-center text-foreground-subtlest py-10 select-none">
                {t('scheduler.noOutput')}
              </div>
            )}
          </div>
        </div>

        {/* 底部按钮：固定在底部 */}
        <div className="flex items-center justify-between border-t border-border/50 px-5 py-3 shrink-0">
          <Button
            variant="outline"
            size="sm"
            disabled={!fullOutput}
            onClick={() => void handleCopy()}
            className="flex items-center gap-1.5"
          >
            {copied ? <Check className="size-3.5 text-terminal-green" /> : <Copy className="size-3.5" />}
            {copied ? t('scheduler.copied') : t('scheduler.copyOutput')}
          </Button>

          <Button size="sm" onClick={onClose}>
            {t('scheduler.dialogClose')}
          </Button>
        </div>
      </div>
    </div>
  );
}

// ── 任务表单 ────────────────────────────────────────────────────────────────

interface EnvRow {
  id: string;
  key: string;
  value: string;
}

/** 智能解析类 Vercel 的 .env / 键值对文本（支持 KEY=VAL、带引号、export 前缀、制表符分隔、# 注释） */
function parseEnvText(text: string): { key: string; value: string }[] {
  const lines = text.split(/\r?\n/);
  const result: { key: string; value: string }[] = [];
  for (let rawLine of lines) {
    let line = rawLine.trim();
    if (!line || line.startsWith('#')) continue;
    if (line.startsWith('export ')) {
      line = line.slice(7).trim();
    }
    const eqIdx = line.indexOf('=');
    if (eqIdx > 0) {
      let key = line.slice(0, eqIdx).trim();
      while (key.startsWith('$')) {
        key = key.slice(1);
      }
      let val = line.slice(eqIdx + 1).trim();
      if (
        (val.startsWith('"') && val.endsWith('"') && val.length >= 2) ||
        (val.startsWith("'") && val.endsWith("'") && val.length >= 2)
      ) {
        val = val.slice(1, -1);
      }
      if (key) {
        result.push({ key, value: val });
      }
    } else {
      const parts = line.split('\t');
      if (parts.length >= 2 && parts[0].trim()) {
        let key = parts[0].trim();
        while (key.startsWith('$')) {
          key = key.slice(1);
        }
        if (key) {
          result.push({ key, value: parts.slice(1).join('\t').trim() });
        }
      }
    }
  }
  return result;
}

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

  // 调度方式：周期 Cron 还是 一次性任务 (run_at)
  const [scheduleMode, setScheduleMode] = useState<'cron' | 'once'>(
    initial?.runAt ? 'once' : 'cron',
  );
  const [cronExpr, setCronExpr] = useState(initial?.cronExpr ?? '*/5 * * * *');

  const toLocalIso = (d: Date) => {
    const pad = (n: number) => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
  };

  const [runAtStr, setRunAtStr] = useState<string>(() => {
    if (initial?.runAt) {
      return toLocalIso(new Date(initial.runAt * 1000));
    }
    return toLocalIso(new Date(Date.now() + 10 * 60 * 1000));
  });

  const setQuickRunAt = (mins: number) => {
    setRunAtStr(toLocalIso(new Date(Date.now() + mins * 60 * 1000)));
  };

  const setTonightRunAt = () => {
    const d = new Date();
    d.setHours(23, 0, 0, 0);
    if (d.getTime() <= Date.now()) {
      d.setDate(d.getDate() + 1);
    }
    setRunAtStr(toLocalIso(d));
  };

  const setTomorrowRunAt = () => {
    const d = new Date();
    d.setDate(d.getDate() + 1);
    d.setHours(9, 0, 0, 0);
    setRunAtStr(toLocalIso(d));
  };

  // D3 Cron 实时人话翻译与未来触发预测
  const [cronDescription, setCronDescription] = useState<string | null>(null);
  const [nextRuns, setNextRuns] = useState<number[]>([]);

  useEffect(() => {
    if (scheduleMode !== 'cron' || !isValidCron(cronExpr)) {
      setCronDescription(null);
      setNextRuns([]);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      void schedulerDescribeCron(cronExpr)
        .then((desc) => {
          if (!cancelled) setCronDescription(desc);
        })
        .catch(() => {
          if (!cancelled) setCronDescription(null);
        });

      void schedulerPreviewNext(cronExpr, 3)
        .then((runs) => {
          if (!cancelled) setNextRuns(runs);
        })
        .catch(() => {
          if (!cancelled) setNextRuns([]);
        });
    }, 250);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [cronExpr, scheduleMode]);

  const [taskType, setTaskType] = useState(initial?.taskType ?? 'local_command');
  const [command, setCommand] = useState(initial?.command ?? '');
  const [workingDir, setWorkingDir] = useState(initial?.workingDir ?? '');
  const [hostId, setHostId] = useState(initial?.hostId?.toString() ?? '');
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [touched, setTouched] = useState(false);

  // 高级配置（通知、超时重试、日志保留）
  const [notifyOn, setNotifyOn] = useState<'none' | 'failure' | 'always'>(
    (initial?.notifyOn as 'failure' | 'always') ?? 'none',
  );
  const [webhookUrl, setWebhookUrl] = useState(initial?.webhookUrl ?? '');
  const [timeoutSecs, setTimeoutSecs] = useState<string>(
    initial?.timeoutSecs ? String(initial.timeoutSecs) : '600',
  );
  const [maxRetries, setMaxRetries] = useState<string>(
    initial?.maxRetries != null ? String(initial.maxRetries) : '0',
  );
  const [retryBackoffSecs, setRetryBackoffSecs] = useState<string>(
    initial?.retryBackoffSecs ? String(initial.retryBackoffSecs) : '30',
  );
  const [retentionDays, setRetentionDays] = useState<string>(
    initial?.retentionDays != null ? String(initial.retentionDays) : '30',
  );

  const hasAdvanced = Boolean(
    (initial?.notifyOn && initial.notifyOn !== 'none') ||
    initial?.webhookUrl ||
    (initial?.timeoutSecs && initial.timeoutSecs !== 600) ||
    (initial?.maxRetries && initial.maxRetries > 0) ||
    (initial?.retentionDays && initial.retentionDays !== 30),
  );
  const [showAdvanced, setShowAdvanced] = useState(hasAdvanced);

  // 环境变量列表（Vercel 风格交互：单项增删 + 粘贴 .env 智能拆解）
  const [envList, setEnvList] = useState<EnvRow[]>(() => {
    if (!initial?.envVars) return [];
    try {
      const map = JSON.parse(initial.envVars);
      return Object.entries(map).map(([k, v]) => ({
        id: crypto.randomUUID(),
        key: k,
        value: String(v),
      }));
    } catch {
      return [];
    }
  });
  const [showBatchEnv, setShowBatchEnv] = useState(false);
  const [rawEnvText, setRawEnvText] = useState('');

  // 严格模态：拦截 Escape 键（不走 onClose——只能通过按钮关闭）
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
      }
    };
    window.addEventListener('keydown', onKeyDown, true);
    return () => window.removeEventListener('keydown', onKeyDown, true);
  }, []);

  const nameError = !name.trim() ? t('scheduler.errNameRequired') : null;
  const cronError =
    scheduleMode === 'cron' && !isValidCron(cronExpr) ? t('scheduler.cronInvalid') : null;
  const runAtError =
    scheduleMode === 'once' && (!runAtStr || new Date(runAtStr).getTime() <= Date.now())
      ? t('scheduler.errRunAtFuture')
      : null;
  const commandError = !command.trim() ? t('scheduler.errCommandRequired') : null;
  const sshHostError =
    taskType === 'ssh_command' && (!hostId || Number.isNaN(Number(hostId)) || Number(hostId) <= 0)
      ? t('scheduler.errHostIdRequired')
      : null;
  const hasError = Boolean(
    nameError || cronError || runAtError || commandError || sshHostError,
  );
  const canSave = !hasError && !saving;

  const handleSave = async () => {
    setTouched(true);
    if (hasError || saving) return;
    setSaving(true);
    setFormError(null);

    // 序列化环境变量：过滤空 key
    const envObj: Record<string, string> = {};
    for (const item of envList) {
      const k = item.key.trim();
      if (k) {
        envObj[k] = item.value;
      }
    }
    const envVars = Object.keys(envObj).length > 0 ? JSON.stringify(envObj) : null;

    const runAt =
      scheduleMode === 'once' ? Math.floor(new Date(runAtStr).getTime() / 1000) : null;

    const task: ScheduledTask = {
      id: initial?.id ?? crypto.randomUUID(),
      name: name.trim(),
      cronExpr: scheduleMode === 'cron' ? normalizeCron(cronExpr) : '* * * * *',
      taskType,
      command: command.trim(),
      workingDir: workingDir.trim() || null,
      hostId: taskType === 'ssh_command' && hostId ? Number(hostId) : null,
      enabled,
      createdAt: initial?.createdAt ?? 0,
      updatedAt: initial?.updatedAt ?? 0,
      lastRunAt: initial?.lastRunAt ?? null,
      lastStatus: initial?.lastStatus ?? null,
      notifyOn: notifyOn === 'none' ? null : notifyOn,
      webhookUrl: notifyOn === 'none' ? null : (webhookUrl.trim() || null),
      timeoutSecs: timeoutSecs ? Math.max(1, Number(timeoutSecs)) : 600,
      maxRetries: maxRetries ? Math.max(0, Number(maxRetries)) : 0,
      retryBackoffSecs: retryBackoffSecs ? Math.max(1, Number(retryBackoffSecs)) : 30,
      runAt,
      retentionDays: retentionDays ? Math.max(0, Number(retentionDays)) : 30,
      envVars,
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

  // Vercel 核心：在 Key 输入框直接粘贴多行或 KEY=VAL 文本时自动智能拆分
  const handleKeyPaste = (e: React.ClipboardEvent<HTMLInputElement>, index: number) => {
    const text = e.clipboardData.getData('text');
    if (text && (text.includes('\n') || text.includes('='))) {
      e.preventDefault();
      const parsed = parseEnvText(text);
      if (parsed.length > 0) {
        setEnvList((prev) => {
          const next = [...prev];
          const curr = next[index];
          let startIdx = 0;
          let insertAt = index;
          if (curr && !curr.key.trim() && !curr.value) {
            next[index] = { id: curr.id, key: parsed[0].key, value: parsed[0].value };
            startIdx = 1;
            insertAt = index + 1;
          }
          const added = parsed.slice(startIdx).map((p) => ({
            id: crypto.randomUUID(),
            key: p.key,
            value: p.value,
          }));
          next.splice(insertAt, 0, ...added);
          return next;
        });
      }
    }
  };

  const handleBatchImport = () => {
    if (!rawEnvText.trim()) return;
    const parsed = parseEnvText(rawEnvText);
    if (parsed.length > 0) {
      setEnvList((prev) => [
        ...prev.filter((item) => item.key.trim() || item.value.trim()),
        ...parsed.map((p) => ({ id: crypto.randomUUID(), key: p.key, value: p.value })),
      ]);
      setRawEnvText('');
      setShowBatchEnv(false);
    }
  };

  const inputCls =
    'h-8 w-full rounded-md border border-border/60 bg-background px-2.5 text-ui-sm text-foreground outline-none transition-colors focus:border-border placeholder:text-foreground-subtlest';
  const labelCls = 'mb-1 block text-ui-xs font-medium text-foreground-subtle';

  return (
    // 严格模态：遮罩层不关（点外面不关闭），只走保存/取消按钮——
    // 用户反馈编辑中的表单不能被误触关闭丢数据
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/40 p-4 backdrop-blur-xs"
      role="dialog"
      aria-modal="true"
    >
      <div
        className="flex w-[560px] max-w-full max-h-[88vh] flex-col overflow-hidden rounded-xl border border-border bg-popover shadow-2xl"
      >
        <div className="border-b border-border/50 px-5 py-3.5 shrink-0 flex items-center justify-between">
          <span className="text-ui-base font-semibold text-foreground">
            {initial ? t('scheduler.editTitle') : t('scheduler.newTitle')}
          </span>
          <button
            type="button"
            onClick={onClose}
            className="rounded p-1 text-foreground-subtle hover:text-foreground cursor-pointer"
          >
            <X className="size-4" />
          </button>
        </div>
        <div className="flex-1 min-h-0 overflow-y-auto scrollbar-slim flex flex-col gap-3 px-5 py-4">
          <div>
            <label className={labelCls}>{t('scheduler.fieldName')}</label>
            <input
              className={cn(inputCls, touched && nameError && 'border-destructive/50')}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t('scheduler.fieldNamePlaceholder')}
            />
            {touched && nameError ? (
              <p className="mt-1 text-ui-xs text-destructive">{nameError}</p>
            ) : null}
          </div>
          {/* 调度方式切换 */}
          <div>
            <div className="flex items-center justify-between mb-1.5">
              <label className={labelCls}>{t('scheduler.fieldScheduleMode')}</label>
              <div className="flex rounded-md border border-border/60 bg-surface/30 p-0.5">
                <button
                  type="button"
                  onClick={() => setScheduleMode('cron')}
                  className={cn(
                    'rounded px-2 py-0.5 text-[11px] font-medium transition-colors cursor-pointer',
                    scheduleMode === 'cron'
                      ? 'bg-surface text-primary shadow-xs'
                      : 'text-foreground-subtle hover:text-foreground',
                  )}
                >
                  {t('scheduler.modeCron')}
                </button>
                <button
                  type="button"
                  onClick={() => setScheduleMode('once')}
                  className={cn(
                    'rounded px-2 py-0.5 text-[11px] font-medium transition-colors cursor-pointer',
                    scheduleMode === 'once'
                      ? 'bg-surface text-primary shadow-xs'
                      : 'text-foreground-subtle hover:text-foreground',
                  )}
                >
                  {t('scheduler.modeOnce')}
                </button>
              </div>
            </div>

            {scheduleMode === 'cron' ? (
              <div>
                <input
                  className={cn(inputCls, 'font-mono', touched && cronError && 'border-destructive/50')}
                  value={cronExpr}
                  onChange={(e) => setCronExpr(e.target.value)}
                  placeholder="*/5 * * * *"
                />
                {/* D3 Cron 人话翻译与触发预测 */}
                {cronDescription ? (
                  <div className="mt-1.5 flex flex-col gap-1 rounded-md bg-surface/40 p-2 text-ui-xs border border-border/40">
                    <div className="flex items-center gap-1.5 text-terminal-green font-medium">
                      <Sparkles className="size-3.5 shrink-0" />
                      <span>{cronDescription}</span>
                    </div>
                    {nextRuns.length > 0 ? (
                      <div className="flex flex-col gap-0.5 mt-0.5 font-mono text-[11px] text-foreground-subtlest">
                        <span className="text-foreground-subtle font-sans">{t('scheduler.cronNextRuns')}:</span>
                        {nextRuns.map((ts, idx) => (
                          <span key={idx}>▸ {formatTimestamp(ts)}</span>
                        ))}
                      </div>
                    ) : null}
                  </div>
                ) : null}
                {touched && cronError ? (
                  <p className="mt-1 text-ui-xs text-destructive">{cronError}</p>
                ) : null}
              </div>
            ) : (
              /* D4 一次性执行时间 */
              <div className="flex flex-col gap-1.5">
                <input
                  type="datetime-local"
                  className={cn(inputCls, 'font-mono', touched && runAtError && 'border-destructive/50')}
                  value={runAtStr}
                  onChange={(e) => setRunAtStr(e.target.value)}
                />
                <div className="flex flex-wrap gap-1.5 pt-0.5">
                  <button
                    type="button"
                    onClick={() => setQuickRunAt(5)}
                    className="rounded border border-border/50 bg-surface/50 px-2 py-0.5 text-[11px] text-foreground-subtle hover:text-foreground cursor-pointer"
                  >
                    {t('scheduler.quickIn5m')}
                  </button>
                  <button
                    type="button"
                    onClick={() => setQuickRunAt(30)}
                    className="rounded border border-border/50 bg-surface/50 px-2 py-0.5 text-[11px] text-foreground-subtle hover:text-foreground cursor-pointer"
                  >
                    {t('scheduler.quickIn30m')}
                  </button>
                  <button
                    type="button"
                    onClick={() => setQuickRunAt(60)}
                    className="rounded border border-border/50 bg-surface/50 px-2 py-0.5 text-[11px] text-foreground-subtle hover:text-foreground cursor-pointer"
                  >
                    {t('scheduler.quickIn1h')}
                  </button>
                  <button
                    type="button"
                    onClick={setTonightRunAt}
                    className="rounded border border-border/50 bg-surface/50 px-2 py-0.5 text-[11px] text-foreground-subtle hover:text-foreground cursor-pointer"
                  >
                    {t('scheduler.quickTonight')}
                  </button>
                  <button
                    type="button"
                    onClick={setTomorrowRunAt}
                    className="rounded border border-border/50 bg-surface/50 px-2 py-0.5 text-[11px] text-foreground-subtle hover:text-foreground cursor-pointer"
                  >
                    {t('scheduler.quickTomorrow')}
                  </button>
                </div>
                {touched && runAtError ? (
                  <p className="mt-1 text-ui-xs text-destructive">{runAtError}</p>
                ) : null}
              </div>
            )}
          </div>
          <div>
            <label className={labelCls}>{t('scheduler.fieldType')}</label>
            <div className="flex gap-2">
              {(
                [
                  ['local_command', t('scheduler.typeLocal')],
                  ['ssh_command', t('scheduler.typeSsh')],
                  ['js_script', t('scheduler.typeJs')],
                ] as const
              ).map(([value, label]) => (
                <button
                  key={value}
                  type="button"
                  onClick={() => setTaskType(value)}
                  className={cn(
                    'flex-1 rounded-md border px-2.5 py-1.5 text-ui-xs font-medium transition-colors',
                    taskType === value
                      ? 'border-primary/40 bg-primary/10 font-semibold text-primary'
                      : 'border-border/60 text-foreground-subtle hover:bg-hover',
                  )}
                >
                  {label}
                </button>
              ))}
            </div>
          </div>
          <div>
            <label className={labelCls}>
              {taskType === 'js_script' ? t('scheduler.fieldScript') : t('scheduler.fieldCommand')}
            </label>
            <textarea
              className={cn(
                inputCls,
                taskType === 'js_script' ? 'h-36 resize-y py-2 font-mono' : 'h-20 resize-none py-2 font-mono',
                touched && commandError && 'border-destructive/50',
              )}
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              placeholder={
                taskType === 'js_script'
                  ? t('scheduler.fieldScriptPlaceholder')
                  : t('scheduler.fieldCommandPlaceholder')
              }
            />
            {touched && commandError ? (
              <p className="mt-1 text-ui-xs text-destructive">{commandError}</p>
            ) : null}
          </div>

          {/* 环境变量（参考 Vercel 体验：支持行增删 + 任意 Key 框直接粘贴 .env 智能拆解 + 批量模式） */}
          <div className="flex flex-col gap-1.5 pt-1">
            <div className="flex items-center justify-between">
              <label className={labelCls}>{t('scheduler.fieldEnvVars')}</label>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => setShowBatchEnv((v) => !v)}
                  className="text-[11px] text-primary hover:underline cursor-pointer"
                >
                  {showBatchEnv ? t('scheduler.envModeTable') : t('scheduler.envModeBatch')}
                </button>
                <button
                  type="button"
                  onClick={() =>
                    setEnvList((prev) => [...prev, { id: crypto.randomUUID(), key: '', value: '' }])
                  }
                  className="flex items-center gap-1 rounded bg-surface px-1.5 py-0.5 text-[11px] text-foreground-subtle hover:text-foreground cursor-pointer border border-border/50"
                >
                  <Plus className="size-3" />
                  {t('scheduler.addEnv')}
                </button>
              </div>
            </div>

            {showBatchEnv ? (
              <div className="flex flex-col gap-2 rounded-lg border border-border/60 bg-surface/20 p-2.5">
                <div className="text-[11px] text-foreground-subtlest">
                  {t('scheduler.envBatchHint')}
                </div>
                <textarea
                  className={cn(inputCls, 'h-24 resize-none py-1.5 font-mono text-ui-xs')}
                  value={rawEnvText}
                  onChange={(e) => setRawEnvText(e.target.value)}
                  placeholder={'API_KEY="secret-token"\nPORT=8080\nDATABASE_URL=postgres://...'}
                />
                <div className="flex justify-end gap-2">
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-6 px-2 text-ui-xs"
                    onClick={() => setShowBatchEnv(false)}
                  >
                    {t('scheduler.cancel')}
                  </Button>
                  <Button
                    size="sm"
                    className="h-6 px-2 text-ui-xs"
                    onClick={handleBatchImport}
                    disabled={!rawEnvText.trim()}
                  >
                    {t('scheduler.envApplyBatch')}
                  </Button>
                </div>
              </div>
            ) : null}

            {envList.length > 0 ? (
              <div className="flex flex-col gap-1.5 max-h-44 overflow-y-auto pr-1 scrollbar-slim">
                {envList.map((env, idx) => (
                  <div key={env.id} className="flex items-center gap-1.5">
                    <input
                      className={cn(inputCls, 'font-mono text-ui-xs w-4/10 shrink-0')}
                      value={env.key}
                      onChange={(e) => {
                        let val = e.target.value;
                        if (val.startsWith('$')) {
                          val = val.replace(/^\$+/, '');
                        }
                        setEnvList((prev) => {
                          const next = [...prev];
                          next[idx] = { ...next[idx], key: val };
                          return next;
                        });
                      }}
                      onPaste={(e) => handleKeyPaste(e, idx)}
                      placeholder="KEY (无需带 $)"
                    />
                    <input
                      className={cn(inputCls, 'font-mono text-ui-xs flex-1 min-w-0')}
                      value={env.value}
                      onChange={(e) => {
                        const val = e.target.value;
                        setEnvList((prev) => {
                          const next = [...prev];
                          next[idx] = { ...next[idx], value: val };
                          return next;
                        });
                      }}
                      placeholder="VALUE"
                    />
                    <button
                      type="button"
                      onClick={() => setEnvList((prev) => prev.filter((_, i) => i !== idx))}
                      className="size-7 shrink-0 flex items-center justify-center rounded text-foreground-subtlest hover:text-destructive hover:bg-destructive/10 cursor-pointer"
                      title={t('scheduler.removeEnv')}
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  </div>
                ))}
              </div>
            ) : !showBatchEnv ? (
              <div
                onClick={() =>
                  setEnvList([{ id: crypto.randomUUID(), key: '', value: '' }])
                }
                className="rounded-lg border border-dashed border-border/60 py-2.5 text-center text-ui-xs text-foreground-subtlest hover:border-border hover:text-foreground-subtle cursor-pointer transition-colors"
              >
                {t('scheduler.noEnvHint')}
              </div>
            ) : null}
            <p className="mt-0.5 text-[11px] text-foreground-subtlest font-mono select-none">
              {t('scheduler.envKeyHint')}
            </p>
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
          ) : taskType === 'ssh_command' ? (
            <div>
              <label className={labelCls}>{t('scheduler.fieldHostId')}</label>
              <input
                className={cn(inputCls, 'font-mono', touched && sshHostError && 'border-destructive/50')}
                type="number"
                value={hostId}
                onChange={(e) => setHostId(e.target.value)}
                placeholder="1（主机列表的 id）"
              />
              {touched && sshHostError ? (
                <p className="mt-1 text-ui-xs text-destructive">{sshHostError}</p>
              ) : null}
              <p className="mt-1 text-ui-xs text-foreground-subtlest">
                {t('scheduler.fieldHostIdHint')}
              </p>
            </div>
          ) : null}
          {/* 高级配置（通知、超时重试、日志保留） */}
          <div className="pt-1">
            <button
              type="button"
              onClick={() => setShowAdvanced((v) => !v)}
              className="flex items-center gap-1.5 text-ui-xs text-foreground-subtle hover:text-foreground cursor-pointer select-none"
            >
              <Sliders className="size-3.5" />
              <span>{t('scheduler.advancedSettings')}</span>
              {showAdvanced ? (
                <ChevronUp className="size-3" />
              ) : (
                <ChevronDown className="size-3" />
              )}
            </button>

            {showAdvanced ? (
              <div className="mt-2.5 flex flex-col gap-3 rounded-lg border border-border/50 bg-surface/20 p-3">
                {/* D1 失败通知策略 */}
                <div>
                  <label className={labelCls}>{t('scheduler.fieldNotifyOn')}</label>
                  <div className="flex gap-1.5">
                    {(
                      [
                        ['none', t('scheduler.notifyNone')],
                        ['failure', t('scheduler.notifyFailure')],
                        ['always', t('scheduler.notifyAlways')],
                      ] as const
                    ).map(([val, label]) => (
                      <button
                        key={val}
                        type="button"
                        onClick={() => setNotifyOn(val)}
                        className={cn(
                          'flex-1 rounded-md border px-2 py-1 text-ui-xs transition-colors cursor-pointer',
                          notifyOn === val
                            ? 'border-primary/40 bg-primary/10 font-medium text-primary'
                            : 'border-border/60 text-foreground-subtle hover:bg-hover',
                        )}
                      >
                        {label}
                      </button>
                    ))}
                  </div>
                </div>

                {notifyOn !== 'none' ? (
                  <div>
                    <label className={labelCls}>{t('scheduler.fieldWebhookUrl')}</label>
                    <input
                      className={cn(inputCls, 'font-mono text-ui-xs')}
                      value={webhookUrl}
                      onChange={(e) => setWebhookUrl(e.target.value)}
                      placeholder={t('scheduler.fieldWebhookPlaceholder')}
                    />
                  </div>
                ) : null}

                {/* D2 超时与重试 */}
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <label className={labelCls}>{t('scheduler.fieldTimeout')}</label>
                    <input
                      type="number"
                      min="1"
                      className={cn(inputCls, 'font-mono')}
                      value={timeoutSecs}
                      onChange={(e) => setTimeoutSecs(e.target.value)}
                      placeholder="600"
                    />
                  </div>
                  <div>
                    <label className={labelCls}>{t('scheduler.fieldMaxRetries')}</label>
                    <input
                      type="number"
                      min="0"
                      max="10"
                      className={cn(inputCls, 'font-mono')}
                      value={maxRetries}
                      onChange={(e) => setMaxRetries(e.target.value)}
                      placeholder="0"
                    />
                  </div>
                </div>

                {Number(maxRetries) > 0 ? (
                  <div>
                    <label className={labelCls}>{t('scheduler.fieldRetryBackoff')}</label>
                    <input
                      type="number"
                      min="1"
                      className={cn(inputCls, 'font-mono')}
                      value={retryBackoffSecs}
                      onChange={(e) => setRetryBackoffSecs(e.target.value)}
                      placeholder="30"
                    />
                  </div>
                ) : null}

                {/* D5 日志保留天数 */}
                <div>
                  <label className={labelCls}>{t('scheduler.fieldRetentionDays')}</label>
                  <input
                    type="number"
                    min="0"
                    className={cn(inputCls, 'font-mono')}
                    value={retentionDays}
                    onChange={(e) => setRetentionDays(e.target.value)}
                    placeholder="30"
                  />
                </div>
              </div>
            ) : null}
          </div>

          <label className="flex items-center gap-2 text-ui-sm text-foreground-subtle select-none cursor-pointer">
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
        <div className="flex justify-end gap-2 border-t border-border/50 px-5 py-3.5 shrink-0 bg-surface/20">
          <Button variant="ghost" onClick={onClose}>
            {t('scheduler.cancel')}
          </Button>
          <Button
            size="sm"
            disabled={!canSave}
            onClick={() => void handleSave()}
            className={cn('flex items-center gap-1.5', touched && hasError && 'opacity-40')}
            title={touched && hasError ? t('scheduler.errFormInvalid') : undefined}
          >
            {saving ? <Spinner className="size-3.5" /> : null}
            {touched && hasError ? t('scheduler.errFormInvalid') : t('scheduler.save')}
          </Button>
        </div>
      </div>
    </div>
  );
}
