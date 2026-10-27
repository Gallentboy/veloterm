/**
 * 批量执行页：左侧勾选多台主机（分组树 + 搜索 + 环境快筛），右侧输入
 * 命令并行执行，结果卡片聚合（退出码徽章非零红、点击弹窗看全文输出）、
 * 跨输出搜索、只看失败、CSV 导出。含 prod/staging 主机时复用终端同款
 * SafetyGuardModal 拦截（level !== safe 即弹窗，Critical 需输 CONFIRM）。
 * 执行通道是后端的独立临时连接（不占终端会话；跳板/中继自动生效）。
 */

import {
  ChevronRight,
  CircleCheck,
  CircleX,
  Copy,
  Download,
  ListChecks,
  Loader2,
  Play,
  Search,
  Square,
} from 'lucide-react';
import { useCallback, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { SafetyGuardModal } from '@/components/terminal/SafetyGuardModal';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { SelectMenu } from '@/components/ui/SelectMenu';
import { useHostsStore } from '@/hosts/HostsProvider';
import type { GroupNode, HostEnvironmentLevel, HostNode } from '@/hosts/types';
import { cn } from '@/lib/utils';
import { useCommands } from '@/services/commandStore';
import {
  fleetExecCancel,
  fleetExecRun,
  type FleetExecEvent,
  type FleetHostResult,
} from '@/services/fleetExecService';
import {
  inspectCommandSafety,
  type SafetyInspectResult,
} from '@/services/safetyGuardService';
import { invoke } from '@tauri-apps/api/core';

/** 环境徽章配色（终端页 PROD/STAGE 警示同源语义） */
const ENV_BADGE_CLASS: Record<string, string> = {
  dev: 'text-foreground-subtle',
  test: 'text-terminal-blue',
  staging: 'text-terminal-yellow',
  prod: 'text-terminal-red font-semibold',
};

const ENV_ORDER: HostEnvironmentLevel[] = ['dev', 'test', 'staging', 'prod'];

let runSeq = 0;

export function BatchExecScreen() {
  const { t } = useTranslation();
  const { hosts, groups, childGroups, hostsInGroup, ungroupedHosts } = useHostsStore();
  const commands = useCommands();

  // ── 选择与输入 ──
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
  const [search, setSearch] = useState('');
  const [envFilter, setEnvFilter] = useState<string>('');
  const [commandText, setCommandText] = useState('');
  const [libValue, setLibValue] = useState('');
  const [timeoutText, setTimeoutText] = useState('30');
  const [concurrencyText, setConcurrencyText] = useState('8');

  // ── 运行态 ──
  const [running, setRunning] = useState(false);
  const runIdRef = useRef<string | null>(null);
  const [results, setResults] = useState<Record<number, FleetHostResult>>({});
  const [totalCount, setTotalCount] = useState(0);
  const [doneCount, setDoneCount] = useState(0);
  const [batchCancelled, setBatchCancelled] = useState(false);

  // ── 展示态 ──
  const [outputSearch, setOutputSearch] = useState('');
  const [onlyFailed, setOnlyFailed] = useState(false);
  const [viewingHostId, setViewingHostId] = useState<number | null>(null);
  const [safetyModal, setSafetyModal] = useState<{
    inspectResult: SafetyInspectResult;
    envLevel: HostEnvironmentLevel;
  } | null>(null);

  const hostById = useMemo(() => {
    const map = new Map<number, HostNode>();
    for (const host of hosts) map.set(host.id, host);
    return map;
  }, [hosts]);

  /** 搜索 + 环境筛选后的可见主机 id 集（树渲染与全选都基于它） */
  const visibleHostIds = useMemo(() => {
    const q = search.trim().toLowerCase();
    const ids = new Set<number>();
    for (const host of hosts) {
      if (envFilter && (host.envLevel ?? 'dev') !== envFilter) continue;
      if (
        q &&
        !host.name.toLowerCase().includes(q) &&
        !host.host.toLowerCase().includes(q)
      )
        continue;
      ids.add(host.id);
    }
    return ids;
  }, [hosts, search, envFilter]);

  const toggleHost = useCallback((id: number) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }, []);

  /** 分组子树内全部可见主机（递归） */
  const groupHostIds = useCallback(
    (group: GroupNode): number[] => {
      const own = hostsInGroup(group.id)
        .map((h) => h.id)
        .filter((id) => visibleHostIds.has(id));
      const child = childGroups(group.id).flatMap(groupHostIds);
      return [...own, ...child];
    },
    [childGroups, hostsInGroup, visibleHostIds],
  );

  const toggleGroup = useCallback(
    (group: GroupNode) => {
      const ids = groupHostIds(group);
      const allSelected = ids.every((id) => selectedIds.has(id));
      setSelectedIds((current) => {
        const next = new Set(current);
        for (const id of ids) {
          if (allSelected) {
            next.delete(id);
          } else {
            next.add(id);
          }
        }
        return next;
      });
    },
    [groupHostIds, selectedIds],
  );

  const canRun =
    !running && commandText.trim().length > 0 && selectedIds.size > 0;

  // ── 执行 ──
  const applyEvent = useCallback((event: FleetExecEvent) => {
    if (event.type === 'hostStart') {
      setResults((prev) => ({
        ...prev,
        [event.hostId]: {
          hostId: event.hostId,
          status: 'running',
          exitCode: null,
          elapsedMs: null,
          output: '',
          truncated: false,
        },
      }));
    } else if (event.type === 'hostDone') {
      setDoneCount((n) => n + 1);
      setResults((prev) => ({
        ...prev,
        [event.hostId]: {
          hostId: event.hostId,
          status: 'done',
          exitCode: event.exitCode,
          elapsedMs: event.elapsedMs,
          output: event.output,
          truncated: event.truncated,
        },
      }));
    } else if (event.type === 'hostError') {
      setDoneCount((n) => n + 1);
      setResults((prev) => ({
        ...prev,
        [event.hostId]: {
          hostId: event.hostId,
          status: 'error',
          exitCode: null,
          elapsedMs: event.elapsedMs,
          output: '',
          truncated: false,
          errorMessage: event.message,
        },
      }));
    } else if (event.type === 'done') {
      if (event.cancelled) setBatchCancelled(true);
    }
  }, []);

  const startRun = useCallback(async () => {
    const ids = [...selectedIds];
    const runId = `fleet-${Date.now()}-${++runSeq}`;
    runIdRef.current = runId;
    setRunning(true);
    setResults({});
    setTotalCount(ids.length);
    setDoneCount(0);
    setBatchCancelled(false);
    try {
      await fleetExecRun(runId, ids, commandText, applyEvent, {
        timeoutSecs: Math.max(1, Number.parseInt(timeoutText, 10) || 30),
        concurrency: Math.min(32, Math.max(1, Number.parseInt(concurrencyText, 10) || 8)),
      });
    } catch (err) {
      console.error('[BatchExec] run failed:', err);
    } finally {
      setRunning(false);
      runIdRef.current = null;
    }
  }, [selectedIds, commandText, timeoutText, concurrencyText, applyEvent]);

  const handleRun = useCallback(async () => {
    // SafetyGuard：选中含 prod/staging 时按最高环境检查（终端流同阈值：
    // level !== safe 即弹窗确认，Critical 需输 CONFIRM）
    const envs = [...selectedIds].map(
      (id) => hostById.get(id)?.envLevel ?? 'dev',
    );
    const highest = envs.includes('prod')
      ? 'prod'
      : envs.includes('staging')
        ? 'staging'
        : 'dev';
    if (highest === 'prod' || highest === 'staging') {
      const inspect = await inspectCommandSafety(commandText, highest);
      if (inspect.level !== 'safe') {
        setSafetyModal({ inspectResult: inspect, envLevel: highest });
        return;
      }
    }
    void startRun();
  }, [selectedIds, hostById, commandText, startRun]);

  const handleCancel = useCallback(() => {
    if (runIdRef.current) void fleetExecCancel(runIdRef.current);
  }, []);

  // ── 结果展示 ──
  const resultRows = useMemo(() => {
    const q = outputSearch.trim().toLowerCase();
    return Object.values(results)
      .sort((a, b) => a.hostId - b.hostId)
      .filter((r) => {
        if (onlyFailed && r.status === 'done' && r.exitCode === 0) return false;
        if (!q) return true;
        const host = hostById.get(r.hostId);
        const haystack = `${host?.name ?? ''} ${host?.host ?? ''} ${r.output} ${r.errorMessage ?? ''}`.toLowerCase();
        return haystack.includes(q);
      });
  }, [results, outputSearch, onlyFailed, hostById]);

  const failedCount = useMemo(
    () =>
      Object.values(results).filter(
        (r) => r.status === 'error' || (r.status === 'done' && r.exitCode !== 0),
      ).length,
    [results],
  );

  const viewingResult = viewingHostId != null ? results[viewingHostId] : null;
  const viewingHost = viewingHostId != null ? hostById.get(viewingHostId) : null;

  /** CSV 导出：前端拼串走 export_text_file（rfd 保存对话框，castExport 同惯例） */
  const handleExportCsv = useCallback(async () => {
    const esc = (v: string) => `"${v.replace(/"/g, '""')}"`;
    const lines = [esc('name') + ',' + esc('host') + ',' + esc('env') + ',' + esc('status') + ',' + esc('exitCode') + ',' + esc('elapsedMs') + ',' + esc('output')];
    for (const r of Object.values(results)) {
      const host = hostById.get(r.hostId);
      const status =
        r.status === 'done' ? String(r.exitCode ?? '') : r.status === 'error' ? 'error' : 'running';
      lines.push(
        [
          esc(host?.name ?? String(r.hostId)),
          esc(host?.host ?? ''),
          esc((host?.envLevel ?? 'dev')),
          esc(status === 'done' && r.exitCode === 0 ? 'ok' : status),
          esc(status),
          esc(String(r.elapsedMs ?? '')),
          esc(r.errorMessage ? `[error] ${r.errorMessage}` : r.output),
        ].join(','),
      );
    }
    const stamp = new Date().toISOString().replace(/[:T]/g, '-').slice(0, 19);
    try {
      await invoke('export_text_file', {
        suggestedName: `batch-exec-${stamp}.csv`,
        ext: 'csv',
        filterLabel: 'CSV',
        content: lines.join('\r\n'),
      });
    } catch (err) {
      console.error('[BatchExec] export failed:', err);
    }
  }, [results, hostById]);

  // ── 渲染 ──
  const renderHostRow = (host: HostNode) => {
    if (!visibleHostIds.has(host.id)) return null;
    const checked = selectedIds.has(host.id);
    return (
      <label
        key={host.id}
        className="flex cursor-pointer select-none items-center gap-2 rounded-md px-1.5 py-1 hover:bg-hover"
      >
        <input
          type="checkbox"
          checked={checked}
          onChange={() => toggleHost(host.id)}
          className="size-3.5 accent-[var(--color-primary)]"
        />
        <span className="min-w-0 flex-1 truncate text-ui-sm text-foreground">
          {host.name}
        </span>
        <span className={cn('shrink-0 text-ui-xs uppercase', ENV_BADGE_CLASS[host.envLevel ?? 'dev'])}>
          {host.envLevel ?? 'dev'}
        </span>
      </label>
    );
  };

  const renderGroup = (group: GroupNode, depth: number) => {
    const ids = groupHostIds(group);
    if (ids.length === 0) return null;
    const allSelected = ids.every((id) => selectedIds.has(id));
    return (
      <div key={group.id} style={{ paddingLeft: depth * 12 }}>
        <label className="flex cursor-pointer select-none items-center gap-1.5 rounded-md px-1.5 py-1 hover:bg-hover">
          <ChevronRight className="size-3 shrink-0 text-foreground-subtlest" />
          <input
            type="checkbox"
            checked={allSelected}
            ref={(el) => {
              // 三态：部分选中显示 indeterminate
              if (el) el.indeterminate = !allSelected && ids.some((id) => selectedIds.has(id));
            }}
            onChange={() => toggleGroup(group)}
            className="size-3.5 accent-[var(--color-primary)]"
          />
          <span className="min-w-0 flex-1 truncate text-ui-sm font-medium text-foreground-subtle">
            {group.name}
          </span>
          <span className="shrink-0 text-ui-xs text-foreground-subtlest">{ids.length}</span>
        </label>
        {hostsInGroup(group.id).map(renderHostRow)}
        {childGroups(group.id).map((child) => renderGroup(child, depth + 1))}
      </div>
    );
  };

  return (
    <div className="flex h-full min-h-0">
      {/* 左栏：主机选择 */}
      <div className="flex w-72 shrink-0 flex-col border-r border-border/60">
        <div className="flex flex-col gap-1.5 border-b border-border/60 p-2">
          <div className="relative">
            <Search className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-foreground-subtlest" />
            <input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder={t('batchExec.hostSearchPlaceholder', '搜索主机名 / 地址')}
              className="h-7 w-full rounded-md border border-border/70 bg-input pl-7 pr-2 text-ui-xs text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover"
            />
          </div>
          <div className="flex flex-wrap gap-1">
            {['', ...ENV_ORDER].map((env) => (
              <button
                key={env || 'all'}
                type="button"
                onClick={() => setEnvFilter(env)}
                className={cn(
                  'rounded border px-1.5 py-0.5 text-ui-xs transition-colors',
                  envFilter === env
                    ? 'border-primary/60 bg-primary/10 text-foreground'
                    : 'border-border/70 text-foreground-subtle hover:bg-hover hover:text-foreground',
                )}
              >
                {env === '' ? t('batchExec.envAll', '全部') : env}
              </button>
            ))}
          </div>
          <div className="flex items-center justify-between text-ui-xs text-foreground-subtlest">
            <span>
              {t('batchExec.selectedCount', '已选 {{n}} 台', { n: selectedIds.size })}
            </span>
            <span className="flex gap-2">
              <button
                type="button"
                className="hover:text-foreground"
                onClick={() => setSelectedIds(new Set(visibleHostIds))}
              >
                {t('batchExec.selectAll', '全选')}
              </button>
              <button
                type="button"
                className="hover:text-foreground"
                onClick={() => setSelectedIds(new Set())}
              >
                {t('batchExec.clearAll', '清空')}
              </button>
            </span>
          </div>
        </div>
        <div className="scrollbar-slim min-h-0 flex-1 overflow-y-auto p-1.5">
          {groups
            .filter((g) => g.parentId === null)
            .map((g) => renderGroup(g, 0))}
          {ungroupedHosts.map(renderHostRow)}
          {visibleHostIds.size === 0 ? (
            <p className="p-3 text-center text-ui-xs text-foreground-subtlest">
              {t('batchExec.noHosts', '没有匹配的主机')}
            </p>
          ) : null}
        </div>
      </div>

      {/* 右栏：命令 + 结果 */}
      <div className="flex min-w-0 flex-1 flex-col">
        {/* 命令区 */}
        <div className="flex flex-col gap-1.5 border-b border-border/60 p-3">
          <div className="flex items-center gap-2">
            <SelectMenu
              value={libValue}
              onChange={(v) => {
                setLibValue(v);
                const record = commands.find((c) => String(c.id) === v);
                if (record) setCommandText(record.command);
              }}
              options={commands.map((c) => ({
                value: String(c.id),
                label: c.title,
                description: c.command,
              }))}
              searchable
              searchPlaceholder={t('batchExec.libSearch', '搜索命令库')}
              emptyLabel={t('batchExec.libEmpty', '无匹配命令')}
              placeholder={t('batchExec.libPlaceholder', '从命令库选择…')}
              className="w-64"
              disabled={running}
            />
            <div className="flex items-center gap-1 text-ui-xs text-foreground-subtle">
              <span>{t('batchExec.timeoutLabel', '超时')}</span>
              <input
                value={timeoutText}
                onChange={(e) => setTimeoutText(e.target.value)}
                disabled={running}
                className="h-6 w-12 rounded border border-border/70 bg-input px-1.5 text-center font-mono text-ui-xs text-foreground outline-none focus:border-border-hover"
              />
              <span>s</span>
            </div>
            <div className="flex items-center gap-1 text-ui-xs text-foreground-subtle">
              <span>{t('batchExec.concurrencyLabel', '并发')}</span>
              <input
                value={concurrencyText}
                onChange={(e) => setConcurrencyText(e.target.value)}
                disabled={running}
                className="h-6 w-12 rounded border border-border/70 bg-input px-1.5 text-center font-mono text-ui-xs text-foreground outline-none focus:border-border-hover"
              />
            </div>
            <div className="flex-1" />
            {running ? (
              <Button variant="destructive" size="sm" onClick={handleCancel}>
                <Square className="size-3.5" />
                {t('batchExec.cancel', '取消')}
              </Button>
            ) : (
              <Button size="sm" disabled={!canRun} onClick={handleRun}>
                <Play className="size-3.5" />
                {t('batchExec.run', '执行')}（{selectedIds.size}）
              </Button>
            )}
          </div>
          <textarea
            value={commandText}
            onChange={(e) => setCommandText(e.target.value)}
            disabled={running}
            spellCheck={false}
            placeholder={t('batchExec.commandPlaceholder', 'hostname && uptime')}
            rows={2}
            className="scrollbar-slim w-full resize-y rounded-md border border-border/60 bg-input px-2 py-1.5 font-mono text-ui-sm text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover"
          />
        </div>

        {/* 结果区 */}
        <div className="flex min-h-0 flex-1 flex-col">
          <div className="flex items-center gap-2 border-b border-border/60 px-3 py-1.5">
            {running ? (
              <span className="flex items-center gap-1.5 text-ui-xs text-foreground-subtle">
                <Loader2 className="size-3.5 animate-spin" />
                {doneCount}/{totalCount}
              </span>
            ) : (
              <span className="text-ui-xs text-foreground-subtle">
                {Object.keys(results).length > 0
                  ? t('batchExec.summary', '共 {{total}} 台 · 成功 {{ok}} · 失败 {{fail}}', {
                      total: totalCount,
                      ok: totalCount - failedCount,
                      fail: failedCount,
                    }) +
                    (batchCancelled ? ` · ${t('batchExec.cancelledTag', '已取消')}` : '')
                  : t('batchExec.noResultsYet', '尚未执行')}
              </span>
            )}
            <div className="flex-1" />
            <div className="relative">
              <Search className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-foreground-subtlest" />
              <input
                value={outputSearch}
                onChange={(e) => setOutputSearch(e.target.value)}
                placeholder={t('batchExec.outputSearchPlaceholder', '跨输出搜索')}
                className="h-7 w-56 rounded-md border border-border/70 bg-input pl-7 pr-2 text-ui-xs text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover"
              />
            </div>
            <label className="flex cursor-pointer select-none items-center gap-1 text-ui-xs text-foreground-subtle">
              <input
                type="checkbox"
                checked={onlyFailed}
                onChange={(e) => setOnlyFailed(e.target.checked)}
                className="size-3.5 accent-[var(--color-primary)]"
              />
              {t('batchExec.onlyFailed', '只看失败')}
            </label>
            <Button
              variant="ghost"
              size="sm"
              disabled={Object.keys(results).length === 0}
              onClick={handleExportCsv}
            >
              <Download className="size-3.5" />
              CSV
            </Button>
          </div>

          <div className="scrollbar-slim min-h-0 flex-1 overflow-y-auto p-2">
            {resultRows.length === 0 ? (
              <div className="flex h-full items-center justify-center text-foreground-subtlest">
                <ListChecks className="mr-2 size-5" />
                <span className="text-ui-sm">
                  {t('batchExec.emptyHint', '勾选左侧主机，输入命令后点「执行」')}
                </span>
              </div>
            ) : (
              resultRows.map((r) => {
                const host = hostById.get(r.hostId);
                const failed = r.status === 'error' || (r.status === 'done' && r.exitCode !== 0);
                return (
                  <button
                    key={r.hostId}
                    type="button"
                    onClick={() => setViewingHostId(r.hostId)}
                    className="flex w-full items-center gap-2 rounded-md border border-border/60 px-2.5 py-1.5 text-left hover:bg-hover"
                  >
                    <span className="shrink-0">
                      {r.status === 'running' ? (
                        <Loader2 className="size-4 animate-spin text-foreground-subtle" />
                      ) : r.status === 'error' ? (
                        <CircleX className="size-4 text-destructive" />
                      ) : r.exitCode === 0 ? (
                        <CircleCheck className="size-4 text-terminal-green" />
                      ) : (
                        <CircleX className="size-4 text-destructive" />
                      )}
                    </span>
                    <span className="w-40 shrink-0 truncate text-ui-sm text-foreground">
                      {host?.name ?? r.hostId}
                    </span>
                    <span className={cn('w-14 shrink-0 text-ui-xs uppercase', ENV_BADGE_CLASS[host?.envLevel ?? 'dev'])}>
                      {host?.envLevel ?? 'dev'}
                    </span>
                    <span
                      className={cn(
                        'w-20 shrink-0 font-mono text-ui-xs',
                        failed ? 'text-destructive' : 'text-foreground-subtle',
                      )}
                    >
                      {r.status === 'running'
                        ? t('batchExec.running', '执行中')
                        : r.status === 'error'
                          ? 'ERROR'
                          : `exit ${r.exitCode ?? '?'}`}
                    </span>
                    <span className="w-16 shrink-0 font-mono text-ui-xs text-foreground-subtlest">
                      {r.elapsedMs != null ? `${(r.elapsedMs / 1000).toFixed(1)}s` : ''}
                    </span>
                    <span
                      className={cn(
                        'min-w-0 flex-1 truncate font-mono text-ui-xs',
                        failed ? 'text-destructive' : 'text-foreground-subtle',
                      )}
                    >
                      {r.errorMessage ??
                        (r.output.length > 0 ? r.output.split('\n')[0] : '')}
                    </span>
                  </button>
                );
              })
            )}
          </div>
        </div>
      </div>

      {/* 输出全文弹窗 */}
      {viewingResult && viewingHost ? (
        <Modal
          title={`${viewingHost.name} · ${viewingResult.status === 'error' ? 'ERROR' : `exit ${viewingResult.exitCode ?? '?'}`}`}
          onClose={() => setViewingHostId(null)}
          className="max-w-3xl"
          containerSelector="main"
          footer={
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void navigator.clipboard.writeText(viewingResult.errorMessage ?? viewingResult.output)}
            >
              <Copy className="size-3.5" />
              {t('batchExec.copyOutput', '复制输出')}
            </Button>
          }
        >
          <pre className="scrollbar-slim max-h-[60vh] overflow-auto whitespace-pre-wrap break-all font-mono text-ui-xs text-foreground">
            {viewingResult.errorMessage ??
              (viewingResult.output.length > 0
                ? viewingResult.output
                : t('batchExec.noOutput', '（无输出）'))}
          </pre>
        </Modal>
      ) : null}

      {/* prod/staging 高危命令拦截（终端同款） */}
      {safetyModal ? (
        <SafetyGuardModal
          isOpen
          command={commandText}
          inspectResult={safetyModal.inspectResult}
          hostName=""
          envLevel={safetyModal.envLevel}
          onConfirm={() => {
            setSafetyModal(null);
            void startRun();
          }}
          onCancel={() => setSafetyModal(null)}
        />
      ) : null}
    </div>
  );
}
