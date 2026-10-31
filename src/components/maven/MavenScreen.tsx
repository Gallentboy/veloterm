/**
 * Maven 仓库助手（mvn-dr 迁移的可视化宿主）：下载 / 搜索 / 上传 三 Tab。
 *
 * - 下载：GAV / pom.xml / GAV 清单三模式 → 仅解析（交互式依赖树）或完整下载
 *   （实时进度仪表：进度条 + 在途/队列/速度统计 + 文件流 + 取消）。
 * - 搜索：关键词（Nexus/中央仓）/ g:a 版本探测 / g:a:v 存在性三形态。
 * - 上传：目录扫描计划预览 → 并发 PUT 进度。
 *
 * 事件流为 Channel 三行式（mavenService）；后端命令层见 src-tauri/src/maven/mod.rs。
 */
import {
  AlertCircle,
  CheckCircle2,
  CircleStop,
  Download,
  FolderOpen,
  Package,
  Play,
  RefreshCw,
  Search,
  Settings2,
  Upload,
  XCircle,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { SelectMenu } from '@/components/ui/SelectMenu';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { MavenTree } from '@/components/maven/MavenTree';
import {
  mavenCancelRun,
  mavenEnvInfo,
  mavenGetRun,
  mavenPutPlan,
  mavenPutRun,
  mavenResolve,
  mavenSearch,
  nextMavenRunId,
  type MavenEnvInfo,
  type MavenEvent,
  type MavenGetSummary,
  type MavenResolveResult,
  type ProbeCell,
  type PutSummary,
  type SearchOutcome,
} from '@/services/mavenService';
import { pickDownloadDir, pickUploadFiles } from '@/services/sftpService';
import { cn } from '@/lib/utils';

// ---------------- 通用小组件 ----------------

/** 三态探测徽章：1=✓ / 0=✗ / 2=? */
function TriCell({ cell }: { cell: ProbeCell }) {
  const { t } = useTranslation();
  if (cell.state === 1) {
    return (
      <span className="inline-flex items-center gap-0.5 font-mono text-[11px] text-terminal-green">
        {cell.name}:✓
      </span>
    );
  }
  if (cell.state === 2) {
    return (
      <span
        className="inline-flex items-center gap-0.5 font-mono text-[11px] text-terminal-yellow"
        title={t('maven.probeUnknown')}
      >
        {cell.name}:?
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-0.5 font-mono text-[11px] text-foreground-subtlest">
      {cell.name}:✗
    </span>
  );
}

/** 实时统计条（Stats 事件驱动） */
function StatsBar({
  stats,
  className,
}: {
  stats: { done: number; total: number; active: number; queued: number; failed: number; bytes: number } | null;
  className?: string;
}) {
  const { t } = useTranslation();
  if (!stats) return null;
  const pct = stats.total > 0 ? Math.min(100, Math.round((stats.done / stats.total) * 100)) : 0;
  return (
    <div className={cn('flex flex-col gap-1.5', className)}>
      <div className="flex items-center gap-2 text-ui-xs text-foreground-subtle select-none">
        <span className="font-mono tabular-nums">
          {stats.done}/{stats.total}
        </span>
        <div className="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-surface">
          <div
            className={cn(
              'h-full rounded-full transition-[width] duration-300',
              stats.failed > 0 ? 'bg-warning' : 'bg-terminal-green',
            )}
            style={{ width: `${pct}%` }}
          />
        </div>
        <span className="font-mono tabular-nums">{pct}%</span>
      </div>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5 font-mono text-[11px] text-foreground-subtlest select-none">
        <span>
          {t('maven.statActive')} {stats.active}
        </span>
        <span>
          {t('maven.statQueued')} {stats.queued}
        </span>
        <span>
          {t('maven.statFailed')} <span className={stats.failed > 0 ? 'text-warning' : ''}>{stats.failed}</span>
        </span>
        <span>
          {t('maven.statBytes')} {formatBytes(stats.bytes)}
        </span>
      </div>
    </div>
  );
}

function formatBytes(n: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let v = n;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u += 1;
  }
  return u === 0 ? `${n} B` : `${v.toFixed(1)} ${units[u]}`;
}

function formatMs(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  const m = Math.floor(ms / 60000);
  const s = Math.floor((ms % 60000) / 1000);
  return `${m}m${String(s).padStart(2, '0')}s`;
}

/** 输入框行：标签 + 控件（表单区紧凑布局） */
function Field({
  label,
  children,
  hint,
  className,
}: {
  label: React.ReactNode;
  children: React.ReactNode;
  hint?: string;
  className?: string;
}) {
  return (
    <label className={cn('flex min-w-0 flex-col gap-1', className)}>
      <span className="text-ui-xs font-medium text-foreground-subtle select-none">{label}</span>
      {children}
      {hint ? <span className="text-[11px] leading-snug text-foreground-subtlest">{hint}</span> : null}
    </label>
  );
}

const INPUT_CLS =
  'h-8 w-full min-w-0 rounded-md border border-border/60 bg-input px-2.5 font-mono text-ui-xs text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover';

// ---------------- 下载 Tab ----------------

type DownloadInputMode = 'gav' | 'pom' | 'list';

function DownloadTab() {
  const { t } = useTranslation();
  const [mode, setMode] = useState<DownloadInputMode>('gav');
  const [gav, setGav] = useState('');
  const [pomPath, setPomPath] = useState('');
  const [gavList, setGavList] = useState('');
  const [output, setOutput] = useState('');
  const [reposText, setReposText] = useState('');
  const [definesText, setDefinesText] = useState('');
  const [updateSnapshots, setUpdateSnapshots] = useState(false);
  const [allClassifiers, setAllClassifiers] = useState(false);
  const [sources, setSources] = useState(false);
  const [noTransitive, setNoTransitive] = useState(false);
  const [checksum, setChecksum] = useState<'off' | 'warn' | 'strict'>('warn');
  const [layout, setLayout] = useState<'m2' | 'flat'>('m2');
  const [threads, setThreads] = useState<string>('');
  const [proxy, setProxy] = useState('');
  const [showAdvanced, setShowAdvanced] = useState(false);

  const [resolving, setResolving] = useState(false);
  const [resolveResult, setResolveResult] = useState<MavenResolveResult | null>(null);
  const [running, setRunning] = useState(false);
  const runIdRef = useRef<string | null>(null);
  const [stats, setStats] = useState<MavenEvent extends never ? never : { done: number; total: number; active: number; queued: number; failed: number; bytes: number } | null>(null);
  const [fileFeed, setFileFeed] = useState<
    Array<{ rel: string; ok: boolean; size?: string; speed?: string; took?: string; error?: string }>
  >([]);
  const [logs, setLogs] = useState<Array<{ level: number; message: string }>>([]);
  const [summary, setSummary] = useState<MavenGetSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const logRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    logRef.current?.scrollTo({ top: logRef.current.scrollHeight });
  }, [logs]);

  const buildRequest = useCallback(() => {
    const repos = reposText
      .split('\n')
      .map((l) => l.trim())
      .filter((l) => l.length > 0 && !l.startsWith('#'));
    const define = definesText
      .split('\n')
      .map((l) => l.trim())
      .filter((l) => l.length > 0 && !l.startsWith('#'));
    return {
      gav: mode === 'gav' ? gav.trim() || null : null,
      pomPath: mode === 'pom' ? pomPath.trim() || null : null,
      gavList: mode === 'list' ? gavList.trim() || null : null,
      output: output.trim() || null,
      layout,
      repos: repos.length > 0 ? repos : null,
      define: define.length > 0 ? define : null,
      updateSnapshots,
      allClassifiers,
      sources,
      noTransitive,
      checksum,
      threads: threads.trim() ? Number(threads.trim()) : null,
      proxy: proxy.trim() || null,
    };
  }, [
    mode, gav, pomPath, gavList, output, layout, reposText, definesText,
    updateSnapshots, allClassifiers, sources, noTransitive, checksum, threads, proxy,
  ]);

  const validate = useCallback((): string | null => {
    if (!output.trim()) return t('maven.errOutputRequired');
    if (mode === 'gav' && !gav.trim()) return t('maven.errGavRequired');
    if (mode === 'pom' && !pomPath.trim()) return t('maven.errPomRequired');
    if (mode === 'list' && !gavList.trim()) return t('maven.errListRequired');
    return null;
  }, [mode, gav, pomPath, gavList, output, t]);

  const handleEvent = useCallback((e: MavenEvent) => {
    if (e.type === 'stats') {
      setStats({ done: e.done, total: e.total, active: e.active, queued: e.queued, failed: e.failed, bytes: e.bytes });
    } else if (e.type === 'log') {
      setLogs((prev) => [...prev.slice(-400), { level: e.level, message: e.message }]);
    } else if (e.type === 'downloadStart') {
      setFileFeed((prev) => [...prev.slice(-200), { rel: e.rel, ok: true }]);
    } else if (e.type === 'downloadDone') {
      setFileFeed((prev) => {
        const next = [...prev];
        for (let i = next.length - 1; i >= 0; i -= 1) {
          if (next[i]!.rel === e.rel) {
            next[i] = { ...next[i]!, ok: true, size: e.size, speed: e.speed, took: e.took };
            break;
          }
        }
        return next;
      });
    }
  }, []);

  const handleResolve = async () => {
    const invalid = validate();
    if (invalid) {
      setError(invalid);
      return;
    }
    setError(null);
    setResolveResult(null);
    setResolving(true);
    try {
      const result = await mavenResolve(buildRequest());
      setResolveResult(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setResolving(false);
    }
  };

  const handleRun = async () => {
    const invalid = validate();
    if (invalid) {
      setError(invalid);
      return;
    }
    setError(null);
    setSummary(null);
    setFileFeed([]);
    setLogs([]);
    setStats(null);
    const runId = nextMavenRunId();
    runIdRef.current = runId;
    setRunning(true);
    try {
      const s = await mavenGetRun(runId, buildRequest(), handleEvent);
      setSummary(s);
      if (s.resolvedTotal > 0 && !resolveResult) {
        // 下载完成后也补一份树（方便复盘），失败不阻塞
        void mavenResolve(buildRequest()).then(setResolveResult).catch(() => undefined);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
      runIdRef.current = null;
    }
  };

  const handleCancel = async () => {
    if (runIdRef.current) {
      await mavenCancelRun(runIdRef.current);
    }
  };

  const pickPom = async () => {
    const files = await pickUploadFiles();
    if (files && files.length > 0) setPomPath(files[0]!);
  };
  const pickOutput = async () => {
    const dir = await pickDownloadDir();
    if (dir) setOutput(dir);
  };

  const busy = resolving || running;

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2.5">
      {/* 输入卡 */}
      <div className="shrink-0 rounded-xl border border-border/60 bg-surface/30 p-3">
        <div className="flex flex-wrap items-center gap-2">
          <div className="flex items-center gap-0.5 rounded-lg border border-border/60 bg-surface p-0.5">
            {(
              [
                ['gav', t('maven.modeGav')],
                ['pom', t('maven.modePom')],
                ['list', t('maven.modeList')],
              ] as const
            ).map(([m, label]) => (
              <button
                key={m}
                type="button"
                onClick={() => setMode(m)}
                className={cn(
                  'rounded-md px-2.5 py-1 text-ui-xs font-medium transition-colors',
                  mode === m ? 'bg-selected text-foreground font-semibold' : 'text-foreground-subtle hover:text-foreground',
                )}
              >
                {label}
              </button>
            ))}
          </div>
          {mode === 'gav' ? (
            <input
              value={gav}
              onChange={(e) => setGav(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !busy) void handleRun();
              }}
              placeholder="org.apache.commons:commons-lang3:3.14.0"
              className={cn(INPUT_CLS, 'flex-1')}
            />
          ) : null}
          {mode === 'pom' ? (
            <div className="flex min-w-0 flex-1 items-center gap-1.5">
              <input
                value={pomPath}
                onChange={(e) => setPomPath(e.target.value)}
                placeholder="/path/to/pom.xml"
                className={cn(INPUT_CLS, 'flex-1')}
              />
              <Button variant="outline" size="sm" onClick={() => void pickPom()}>
                <FolderOpen className="size-3.5" />
              </Button>
            </div>
          ) : null}
          <Button
            variant="outline"
            size="sm"
            onClick={() => setShowAdvanced((v) => !v)}
            aria-expanded={showAdvanced}
          >
            <Settings2 className="size-3.5" />
            {t('maven.advanced')}
          </Button>
          <div className="flex-1" />
          <Button variant="outline" size="sm" disabled={busy} onClick={() => void handleResolve()}>
            {resolving ? <Spinner className="size-3.5" /> : <Search className="size-3.5" />}
            {t('maven.resolveOnly')}
          </Button>
          {running ? (
            <Button variant="secondary" size="sm" onClick={() => void handleCancel()}>
              <CircleStop className="size-3.5" />
              {t('maven.cancel')}
            </Button>
          ) : (
            <Button size="sm" disabled={busy} onClick={() => void handleRun()}>
              <Play className="size-3.5" />
              {t('maven.startDownload')}
            </Button>
          )}
        </div>
        {mode === 'list' ? (
          <textarea
            value={gavList}
            onChange={(e) => setGavList(e.target.value)}
            placeholder={'org.slf4j:slf4j-api:2.0.13\ncom.google.guava:guava:33.0.0-jre\n# 注释行'}
            rows={5}
            className="scrollbar-slim mt-2 w-full resize-y rounded-md border border-border/60 bg-input px-2.5 py-1.5 font-mono text-ui-xs text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover"
          />
        ) : null}
        {showAdvanced ? (
          <div className="mt-2.5 grid grid-cols-2 gap-2.5 border-t border-border/40 pt-2.5 @lg:grid-cols-4 @2xl:grid-cols-6">
            <Field label={t('maven.checksum')}>
              <SelectMenu
                value={checksum}
                onChange={(v) => setChecksum(v as 'off' | 'warn' | 'strict')}
                options={[
                  { value: 'warn', label: 'warn' },
                  { value: 'off', label: 'off' },
                  { value: 'strict', label: 'strict' },
                ]}
                aria-label={t('maven.checksum')}
                triggerClassName="cursor-pointer"
              />
            </Field>
            <Field label={t('maven.threads')} hint={t('maven.threadsHint')}>
              <input
                value={threads}
                onChange={(e) => setThreads(e.target.value.replace(/\D/g, ''))}
                placeholder="16"
                className={INPUT_CLS}
              />
            </Field>
            <Field label={t('maven.proxy')}>
              <input
                value={proxy}
                onChange={(e) => setProxy(e.target.value)}
                placeholder="http://127.0.0.1:7890"
                className={INPUT_CLS}
              />
            </Field>
            <Field label={t('maven.repos')} className="col-span-2" hint={t('maven.reposHint')}>
              <textarea
                value={reposText}
                onChange={(e) => setReposText(e.target.value)}
                placeholder={'corp=https://nexus.example.com/repository/maven-public/\nhttps://repo.jboss.org/nexus/content/groups/public/'}
                rows={3}
                className="scrollbar-slim w-full resize-y rounded-md border border-border/60 bg-input px-2.5 py-1.5 font-mono text-ui-xs text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover"
              />
            </Field>
            <Field label={t('maven.defines')} className="col-span-2" hint={t('maven.definesHint')}>
              <textarea
                value={definesText}
                onChange={(e) => setDefinesText(e.target.value)}
                placeholder={'my.prop=value\nrevision=1.2.3'}
                rows={3}
                className="scrollbar-slim w-full resize-y rounded-md border border-border/60 bg-input px-2.5 py-1.5 font-mono text-ui-xs text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover"
              />
            </Field>
            <div className="col-span-2 flex flex-col justify-center gap-1.5 @xl:col-span-3">
              {(
                [
                  [updateSnapshots, setUpdateSnapshots, t('maven.updateSnapshots'), t('maven.updateSnapshotsHint')],
                  [allClassifiers, setAllClassifiers, t('maven.allClassifiers'), t('maven.allClassifiersHint')],
                  [sources, setSources, t('maven.sources'), t('maven.sourcesHint')],
                  [noTransitive, setNoTransitive, t('maven.noTransitive'), t('maven.noTransitiveHint')],
                ] as const
              ).map(([value, setter, label, hint]) => (
                <label
                  key={label}
                  className="flex cursor-pointer items-center gap-1.5 text-ui-xs text-foreground-subtle select-none hover:text-foreground"
                  title={hint}
                >
                  <input
                    type="checkbox"
                    checked={value}
                    onChange={(e) => setter(e.target.checked)}
                    className="size-3.5 accent-[var(--color-primary)]"
                  />
                  {label}
                </label>
              ))}
            </div>
          </div>
        ) : null}
      </div>

      {error ? (
        <div className="flex shrink-0 items-start gap-1.5 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-ui-xs text-destructive">
          <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
          <span className="min-w-0 whitespace-pre-wrap break-all">{error}</span>
        </div>
      ) : null}
      {resolveResult && resolveResult.failures.length > 0 ? (
        <div className="flex shrink-0 items-center gap-1.5 rounded-lg border border-warning/30 bg-warning/5 px-3 py-1.5 text-ui-xs text-warning">
          <AlertCircle className="size-3.5 shrink-0" />
          <span>
            {resolveResult.failures.length} {t('maven.resolveFailures')}
          </span>
          <span
            className="min-w-0 flex-1 truncate font-mono text-[11px] opacity-75"
            title={resolveResult.failures.map((f) => `${f.item}: ${f.error}`).join('\n')}
          >
            {resolveResult.failures.map((f) => f.item).join(', ')}
          </span>
        </div>
      ) : null}
      <div className="flex min-h-0 flex-1 gap-2.5">
        <div className="flex min-w-0 flex-1 flex-col overflow-hidden rounded-xl border border-border/60 bg-background">
          {resolveResult ? (
            resolveResult.nodes.length > 0 ? (
              <MavenTree nodes={resolveResult.nodes} className="min-h-0 flex-1" />
            ) : (
              <div className="flex flex-1 items-center justify-center text-ui-sm text-foreground-subtlest">
                {t('maven.noNodes')}
              </div>
            )
          ) : (
            <div className="flex flex-1 flex-col items-center justify-center gap-2 text-foreground-subtlest select-none">
              <Package className="size-8 opacity-40" />
              <span className="text-ui-sm">{t('maven.treeEmptyHint')}</span>
            </div>
          )}
        </div>

        <div className="flex w-[44%] min-w-72 shrink-0 flex-col gap-2.5">
          {/* 输出设置：目录（必选，无默认值）+ 存储布局 */}
          <div className="shrink-0 rounded-xl border border-border/60 bg-surface/30 p-3">
            <div className="flex flex-col gap-2">
              <Field
                label={
                  <span>
                    <span className="mr-0.5 text-destructive">*</span>
                    {t('maven.outputDir')}
                  </span>
                }
              >
                <div className="flex items-center gap-1.5">
                  <input
                    value={output}
                    onChange={(e) => setOutput(e.target.value)}
                    placeholder={t('maven.outputDirPlaceholder')}
                    className={cn(INPUT_CLS, 'flex-1', !output.trim() && 'border-destructive/50')}
                  />
                  <Button variant="outline" size="sm" onClick={() => void pickOutput()}>
                    <FolderOpen className="size-3.5" />
                  </Button>
                </div>
              </Field>
              <Field label={t('maven.layout')}>
                <SelectMenu
                  value={layout}
                  onChange={(v) => setLayout(v as 'm2' | 'flat')}
                  options={[
                    { value: 'm2', label: 'm2' },
                    { value: 'flat', label: 'flat' },
                  ]}
                  aria-label={t('maven.layout')}
                  triggerClassName="cursor-pointer"
                />
              </Field>
            </div>
          </div>
          {/* 进度仪表 */}
          <div className="shrink-0 rounded-xl border border-border/60 bg-surface/30 p-3">
            <div className="mb-2 flex items-center gap-2">
              {running ? (
                <Spinner className="size-3.5 text-terminal-green" />
              ) : (
                <Download className="size-3.5 text-foreground-subtle" />
              )}
              <span className="text-ui-xs font-medium text-foreground">
                {running ? t('maven.downloading') : t('maven.progress')}
              </span>
              <span className="flex-1" />
              {running ? (
                <span className="animate-pulse font-mono text-[11px] text-terminal-green">{t('maven.live')}</span>
              ) : null}
            </div>
            {stats || running ? (
              <StatsBar stats={stats} />
            ) : (
              <p className="text-ui-xs text-foreground-subtlest">{t('maven.progressHint')}</p>
            )}
            {summary ? (
              <div className="mt-2.5 space-y-1 border-t border-border/40 pt-2 text-ui-xs text-foreground-subtle">
                {summary.cancelled ? (
                  <p className="text-warning">⚠ {t('maven.cancelled')}</p>
                ) : (
                  <p className="flex items-center gap-1 text-terminal-green">
                    <CheckCircle2 className="size-3.5" />
                    {t('maven.doneSummary', {
                      ok: summary.downloaded,
                      skip: summary.skipped,
                      ms: formatMs(summary.elapsedMs),
                    })}
                    {summary.failed.length > 0 ? (
                      <span className="text-warning">· {summary.failed.length} {t('maven.failedCount')}</span>
                    ) : null}
                  </p>
                )}
                <p className="truncate font-mono text-[11px] text-foreground-subtlest" title={summary.outputDir}>
                  → {summary.outputDir}
                </p>
                {summary.reportPath ? (
                  <p className="truncate font-mono text-[11px] text-foreground-subtlest" title={summary.reportPath}>
                    {t('maven.reportAt')}: {summary.reportPath}
                  </p>
                ) : null}
              </div>
            ) : null}
          </div>

          {/* 文件流 */}
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-border/60 bg-background">
            <div className="flex shrink-0 items-center gap-2 border-b border-border/40 px-2.5 py-1 text-ui-xs text-foreground-subtle select-none">
              <span className="font-medium text-foreground">{t('maven.fileFeed')}</span>
              <span className="flex-1" />
              {fileFeed.length > 0 ? (
                <button
                  type="button"
                  onClick={() => setFileFeed([])}
                  className="rounded px-1.5 py-0.5 hover:bg-hover hover:text-foreground"
                >
                  {t('maven.clear')}
                </button>
              ) : null}
            </div>
            <div className="scrollbar-slim min-h-0 flex-1 overflow-y-auto px-2 py-1">
              {fileFeed.length === 0 ? (
                <div className="flex h-full items-center justify-center text-ui-xs text-foreground-subtlest select-none">
                  {t('maven.fileFeedHint')}
                </div>
              ) : (
                fileFeed.map((f, i) => (
                  <div key={`${f.rel}-${i}`} className="flex items-center gap-1.5 py-px font-mono text-[11px]">
                    {f.size ? (
                      <CheckCircle2 className="size-3 shrink-0 text-terminal-green" />
                    ) : (
                      <Spinner className="size-3 shrink-0" />
                    )}
                    <span className="min-w-0 flex-1 truncate text-foreground-subtle" title={f.rel}>
                      {f.rel}
                    </span>
                    {f.size ? (
                      <span className="shrink-0 text-foreground-subtlest">
                        {f.size} · {f.took}
                      </span>
                    ) : null}
                  </div>
                ))
              )}
            </div>
          </div>
        </div>
      </div>

      {/* 日志 */}
      <div className="flex h-28 shrink-0 flex-col overflow-hidden rounded-xl border border-border/60 bg-background">
        <div className="flex shrink-0 items-center gap-2 border-b border-border/40 px-2.5 py-1 text-ui-xs text-foreground-subtle select-none">
          <span className="font-medium text-foreground">{t('maven.logs')}</span>
          <span className="flex-1" />
          {logs.length > 0 ? (
            <button
              type="button"
              onClick={() => setLogs([])}
              className="rounded px-1.5 py-0.5 hover:bg-hover hover:text-foreground"
            >
              {t('maven.clear')}
            </button>
          ) : null}
        </div>
        <div ref={logRef} className="scrollbar-slim min-h-0 flex-1 overflow-y-auto px-2.5 py-1 font-mono text-[11px] leading-relaxed">
          {logs.length === 0 ? (
            <span className="text-foreground-subtlest select-none">{t('maven.logsHint')}</span>
          ) : (
            logs.map((l, i) => (
              <div
                key={i}
                className={cn(
                  'whitespace-pre-wrap break-all',
                  l.level >= 2 ? 'text-foreground-subtlest' : 'text-foreground-subtle',
                )}
              >
                {l.message}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}

// ---------------- 搜索 Tab ----------------

function SearchTab() {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');
  const [repo, setRepo] = useState('');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [checkJar, setCheckJar] = useState(false);
  const [classifiers, setClassifiers] = useState('');
  const [versions, setVersions] = useState(false);
  const [searching, setSearching] = useState(false);
  const [outcome, setOutcome] = useState<SearchOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    const q = query.trim();
    if (!q) {
      setError(t('maven.errQueryRequired'));
      return;
    }
    setError(null);
    setOutcome(null);
    setSearching(true);
    try {
      const result = await mavenSearch({
        query: q,
        repo: repo.trim() || null,
        username: username.trim() || null,
        password: password || null,
        versions,
        checkJar,
        classifiers:
          classifiers
            .split(/[,，;；]/)
            .map((c) => c.trim())
            .filter((c) => c.length > 0) ?? null,
        rows: 50,
      });
      setOutcome(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setSearching(false);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2.5">
      <div className="shrink-0 rounded-xl border border-border/60 bg-surface/30 p-3">
        <div className="flex flex-wrap items-center gap-2">
          <Search className="size-4 shrink-0 text-foreground-subtle" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !searching) void run();
            }}
            placeholder={t('maven.searchPlaceholder')}
            className={cn(INPUT_CLS, 'flex-1')}
          />
          <Button size="sm" disabled={searching} onClick={() => void run()}>
            {searching ? <Spinner className="size-3.5" /> : <Search className="size-3.5" />}
            {t('maven.search')}
          </Button>
        </div>
        <div className="mt-2.5 grid grid-cols-2 gap-2.5 @lg:grid-cols-5">
          <Field label={t('maven.searchRepo')} hint={t('maven.searchRepoHint')} className="col-span-2">
            <input
              value={repo}
              onChange={(e) => setRepo(e.target.value)}
              placeholder="https://nexus.example.com/repository/maven-public/"
              className={INPUT_CLS}
            />
          </Field>
          <Field label={t('maven.username')}>
            <input value={username} onChange={(e) => setUsername(e.target.value)} className={INPUT_CLS} />
          </Field>
          <Field label={t('maven.password')}>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className={INPUT_CLS}
            />
          </Field>
          <Field label={t('maven.classifiers')} hint={t('maven.classifiersHint')}>
            <input
              value={classifiers}
              onChange={(e) => setClassifiers(e.target.value)}
              placeholder="linux-x86_64,sources"
              className={INPUT_CLS}
            />
          </Field>
        </div>
        <div className="mt-2 flex flex-wrap items-center gap-4">
          <label className="flex cursor-pointer items-center gap-1.5 text-ui-xs text-foreground-subtle select-none hover:text-foreground" title={t('maven.checkJarHint')}>
            <input
              type="checkbox"
              checked={checkJar}
              onChange={(e) => setCheckJar(e.target.checked)}
              className="size-3.5 accent-[var(--color-primary)]"
            />
            {t('maven.checkJar')}
          </label>
          <label className="flex cursor-pointer items-center gap-1.5 text-ui-xs text-foreground-subtle select-none hover:text-foreground" title={t('maven.versionsHint')}>
            <input
              type="checkbox"
              checked={versions}
              onChange={(e) => setVersions(e.target.checked)}
              className="size-3.5 accent-[var(--color-primary)]"
            />
            {t('maven.versions')}
          </label>
          <span className="text-[11px] text-foreground-subtlest">{t('maven.searchModeHint')}</span>
        </div>
      </div>

      {error ? (
        <div className="flex shrink-0 items-start gap-1.5 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-ui-xs text-destructive">
          <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
          <span className="min-w-0 whitespace-pre-wrap break-all">{error}</span>
        </div>
      ) : null}

      <div className="scrollbar-slim min-h-0 flex-1 overflow-auto rounded-xl border border-border/60 bg-background">
        {!outcome ? (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-foreground-subtlest select-none">
            <Search className="size-8 opacity-40" />
            <span className="text-ui-sm">{t('maven.searchEmptyHint')}</span>
          </div>
        ) : outcome.type === 'hits' ? (
          <table className="w-full border-collapse text-ui-xs">
            <thead className="sticky top-0 z-10 bg-surface/95 backdrop-blur-xs">
              <tr className="border-b border-border/60 text-left font-semibold text-foreground-subtle">
                <th className="px-3 py-2">GROUP</th>
                <th className="px-3 py-2">ARTIFACT</th>
                <th className="px-3 py-2">VERSION</th>
                <th className="px-3 py-2">INFO</th>
              </tr>
            </thead>
            <tbody>
              {outcome.hits.map((h) => (
                <tr key={`${h.group}:${h.artifact}`} className="border-b border-border/30 hover:bg-hover/50">
                  <td className="px-3 py-1.5 font-mono text-foreground">{h.group}</td>
                  <td className="px-3 py-1.5 font-mono text-foreground">{h.artifact}</td>
                  <td className="px-3 py-1.5 font-mono text-terminal-green">{h.latest}</td>
                  <td className="px-3 py-1.5 font-mono text-foreground-subtle">{h.extra}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : outcome.type === 'versions' ? (
          <table className="w-full border-collapse text-ui-xs">
            <thead className="sticky top-0 z-10 bg-surface/95 backdrop-blur-xs">
              <tr className="border-b border-border/60 text-left font-semibold text-foreground-subtle">
                <th className="px-3 py-2">{t('maven.colVersion')}</th>
                <th className="px-3 py-2">POM</th>
                <th className="px-3 py-2">{t('maven.colExtras')}</th>
              </tr>
            </thead>
            <tbody>
              {outcome.versions.map((v) => (
                <tr key={v.version} className="border-b border-border/30 hover:bg-hover/50">
                  <td
                    className={cn(
                      'px-3 py-1.5 font-mono',
                      v.pomOk ? 'text-foreground' : 'text-foreground-subtlest line-through',
                    )}
                  >
                    {v.version}
                  </td>
                  <td className="px-3 py-1.5 font-mono">
                    {v.pomOk ? (
                      <span className="text-terminal-green">✓</span>
                    ) : (
                      <span className="text-foreground-subtlest">✗</span>
                    )}
                  </td>
                  <td className="flex flex-wrap items-center gap-2 px-3 py-1.5">
                    {v.extras.map((c) => (
                      <TriCell key={c.name} cell={c} />
                    ))}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <div className="flex flex-col gap-3 p-4">
            <div
              className={cn(
                'flex items-center gap-2 rounded-lg border px-3 py-2.5 text-ui-sm',
                outcome.exists
                  ? 'border-terminal-green/30 bg-terminal-green/5 text-terminal-green'
                  : 'border-destructive/30 bg-destructive/5 text-destructive',
              )}
            >
              {outcome.exists ? (
                <CheckCircle2 className="size-4 shrink-0" />
              ) : (
                <XCircle className="size-4 shrink-0" />
              )}
              <span className="font-mono">
                {outcome.group}:{outcome.artifact}:{outcome.version}
              </span>
              <span>{outcome.exists ? t('maven.existsYes') : t('maven.existsNo')}</span>
              <span className="ml-auto flex gap-2">
                {outcome.extras.map((c) => (
                  <TriCell key={c.name} cell={c} />
                ))}
              </span>
            </div>
            {outcome.suggestions.length > 0 ? (
              <div>
                <p className="mb-1.5 text-ui-xs font-medium text-foreground-subtle">
                  {t('maven.suggestions')}
                </p>
                <div className="flex flex-col gap-1">
                  {outcome.suggestions.map((s) => (
                    <div key={s.version} className="flex items-center gap-2 rounded-md border border-border/40 bg-surface/30 px-2.5 py-1.5 font-mono text-ui-xs">
                      <span className="text-foreground">{s.version}</span>
                      <span className="rounded bg-surface px-1 text-[10px] text-foreground-subtle border border-border/40">
                        {s.tag}
                      </span>
                      <span className="ml-auto flex gap-2">
                        {s.extras.map((c) => (
                          <TriCell key={c.name} cell={c} />
                        ))}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------- 上传 Tab ----------------

function UploadTab() {
  const { t } = useTranslation();
  const [dir, setDir] = useState('');
  const [repo, setRepo] = useState('');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [checkRepo, setCheckRepo] = useState('');
  const [checkUsername, setCheckUsername] = useState('');
  const [checkPassword, setCheckPassword] = useState('');
  const [planning, setPlanning] = useState(false);
  const [plan, setPlan] = useState<{ items: Array<{ local: string; rel: string }>; skipped: number } | null>(null);
  const [running, setRunning] = useState(false);
  const runIdRef = useRef<string | null>(null);
  const [stats, setStats] = useState<{ done: number; total: number; active: number; queued: number; failed: number; bytes: number } | null>(null);
  const [summary, setSummary] = useState<PutSummary | null>(null);
  const [error, setError] = useState<string | null>(null);

  const pickDir = async () => {
    const d = await pickDownloadDir();
    if (d) setDir(d);
  };

  const handlePlan = async () => {
    if (!dir.trim()) {
      setError(t('maven.errDirRequired'));
      return;
    }
    setError(null);
    setPlan(null);
    setPlanning(true);
    try {
      const p = await mavenPutPlan(dir.trim());
      setPlan(p);
    } catch (e) {
      setError(String(e));
    } finally {
      setPlanning(false);
    }
  };

  const handleEvent = useCallback((e: MavenEvent) => {
    if (e.type === 'stats') {
      setStats({ done: e.done, total: e.total, active: e.active, queued: e.queued, failed: e.failed, bytes: e.bytes });
    }
  }, []);

  const handleRun = async () => {
    if (!dir.trim() || !repo.trim()) {
      setError(t('maven.errUploadRequired'));
      return;
    }
    setError(null);
    setSummary(null);
    setStats(null);
    const runId = nextMavenRunId();
    runIdRef.current = runId;
    setRunning(true);
    try {
      const s = await mavenPutRun(
        runId,
        {
          dir: dir.trim(),
          repo: repo.trim(),
          username: username.trim() || null,
          password: password || null,
          checkRepo: checkRepo.trim() || null,
          checkUsername: checkUsername.trim() || null,
          checkPassword: checkPassword || null,
        },
        handleEvent,
      );
      setSummary(s);
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
      runIdRef.current = null;
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2.5">
      <div className="shrink-0 rounded-xl border border-border/60 bg-surface/30 p-3">
        <div className="grid grid-cols-2 gap-2.5 @lg:grid-cols-4">
          <Field label={t('maven.uploadDir')} hint={t('maven.uploadDirHint')} className="col-span-2">
            <div className="flex items-center gap-1.5">
              <input
                value={dir}
                onChange={(e) => setDir(e.target.value)}
                placeholder="/data/m2-repo 或 Gradle cache 目录"
                className={cn(INPUT_CLS, 'flex-1')}
              />
              <Button variant="outline" size="sm" onClick={() => void pickDir()}>
                <FolderOpen className="size-3.5" />
              </Button>
            </div>
          </Field>
          <Field label={t('maven.uploadRepo')} className="col-span-2">
            <input
              value={repo}
              onChange={(e) => setRepo(e.target.value)}
              placeholder="https://nexus.example.com/repository/maven-releases/"
              className={INPUT_CLS}
            />
          </Field>
          <Field label={t('maven.username')}>
            <input value={username} onChange={(e) => setUsername(e.target.value)} className={INPUT_CLS} />
          </Field>
          <Field label={t('maven.password')}>
            <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} className={INPUT_CLS} />
          </Field>
          <Field label={t('maven.checkRepo')} hint={t('maven.checkRepoHint')} className="col-span-2">
            <input
              value={checkRepo}
              onChange={(e) => setCheckRepo(e.target.value)}
              className={INPUT_CLS}
            />
          </Field>
        </div>
        {(checkUsername || checkPassword || true) && checkRepo.trim() ? (
          <div className="mt-2.5 grid grid-cols-2 gap-2.5 border-t border-border/40 pt-2.5 @lg:grid-cols-4">
            <Field label={t('maven.checkUsername')} hint={t('maven.checkCredsHint')}>
              <input value={checkUsername} onChange={(e) => setCheckUsername(e.target.value)} className={INPUT_CLS} />
            </Field>
            <Field label={t('maven.checkPassword')}>
              <input
                type="password"
                value={checkPassword}
                onChange={(e) => setCheckPassword(e.target.value)}
                className={INPUT_CLS}
              />
            </Field>
          </div>
        ) : null}
        <div className="mt-2.5 flex items-center gap-2">
          <Button variant="outline" size="sm" disabled={planning || running} onClick={() => void handlePlan()}>
            {planning ? <Spinner className="size-3.5" /> : <Search className="size-3.5" />}
            {t('maven.scanPlan')}
          </Button>
          <div className="flex-1" />
          {running ? (
            <Button variant="secondary" size="sm" onClick={() => runIdRef.current && void mavenCancelRun(runIdRef.current)}>
              <CircleStop className="size-3.5" />
              {t('maven.cancel')}
            </Button>
          ) : (
            <Button size="sm" disabled={!dir.trim() || !repo.trim()} onClick={() => void handleRun()}>
              <Upload className="size-3.5" />
              {t('maven.startUpload')}
            </Button>
          )}
        </div>
      </div>

      {error ? (
        <div className="flex shrink-0 items-start gap-1.5 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-ui-xs text-destructive">
          <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
          <span className="min-w-0 whitespace-pre-wrap break-all">{error}</span>
        </div>
      ) : null}

      {stats || running ? (
        <div className="shrink-0 rounded-xl border border-border/60 bg-surface/30 p-3">
          <StatsBar stats={stats} />
        </div>
      ) : null}

      {summary ? (
        <div className="shrink-0 space-y-1 rounded-xl border border-border/60 bg-surface/30 p-3 text-ui-xs text-foreground-subtle">
          {summary.cancelled ? <p className="text-warning">⚠ {t('maven.cancelled')}</p> : null}
          <p className="flex items-center gap-1 text-terminal-green">
            <CheckCircle2 className="size-3.5" />
            {t('maven.uploadDoneSummary', {
              ok: summary.uploaded,
              skip: summary.skipped,
              ms: formatMs(summary.elapsedMs),
              bytes: formatBytes(summary.bytes),
            })}
            {summary.failed.length > 0 ? (
              <span className="text-warning">· {summary.failed.length} {t('maven.failedCount')}</span>
            ) : null}
          </p>
          {summary.failed.slice(0, 5).map((f) => (
            <p key={f.item} className="truncate font-mono text-[11px] text-warning" title={`${f.item}: ${f.error}`}>
              ✗ {f.item} — {f.error}
            </p>
          ))}
        </div>
      ) : null}

      <div className="scrollbar-slim min-h-0 flex-1 overflow-auto rounded-xl border border-border/60 bg-background">
        {plan ? (
          <table className="w-full border-collapse text-ui-xs">
            <thead className="sticky top-0 z-10 bg-surface/95 backdrop-blur-xs">
              <tr className="border-b border-border/60 text-left font-semibold text-foreground-subtle">
                <th className="px-3 py-2">#</th>
                <th className="px-3 py-2">{t('maven.colLocal')}</th>
                <th className="px-3 py-2">{t('maven.colRemote')}</th>
              </tr>
            </thead>
            <tbody>
              {plan.items.map((it, i) => (
                <tr key={it.rel} className="border-b border-border/30 hover:bg-hover/50">
                  <td className="px-3 py-1.5 font-mono text-foreground-subtlest">{i + 1}</td>
                  <td className="px-3 py-1.5 font-mono text-foreground-subtle" title={it.local}>
                    <span className="line-clamp-1">{it.local}</span>
                  </td>
                  <td className="px-3 py-1.5 font-mono text-foreground" title={it.rel}>
                    <span className="line-clamp-1">{it.rel}</span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-foreground-subtlest select-none">
            <Upload className="size-8 opacity-40" />
            <span className="text-ui-sm">{t('maven.uploadEmptyHint')}</span>
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------- 主屏 ----------------

export function MavenScreen() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<'download' | 'search' | 'upload'>('download');
  const [env, setEnv] = useState<MavenEnvInfo | null>(null);

  const loadEnv = useCallback(() => {
    void mavenEnvInfo().then(setEnv).catch(() => setEnv(null));
  }, []);
  useEffect(loadEnv, [loadEnv]);

  const tabs = useMemo(
    () =>
      [
        ['download', <Download key="d" className="size-3.5" />, t('maven.tabDownload')],
        ['search', <Search key="s" className="size-3.5" />, t('maven.tabSearch')],
        ['upload', <Upload key="u" className="size-3.5" />, t('maven.tabUpload')],
      ] as const,
    [t],
  );

  return (
    <section data-testid="maven-screen" className="flex h-full min-h-0 flex-col overflow-hidden bg-background">
      {/* 标题栏 + Tab */}
      <div className="flex shrink-0 flex-wrap items-center gap-2.5 border-b border-border/50 px-4 py-2.5">
        <div className="flex size-8 items-center justify-center rounded-lg border border-border/50 bg-surface/50">
          <Package className="size-4 text-foreground-subtle" />
        </div>
        <h1 className="text-ui-base font-semibold text-foreground">{t('maven.title')}</h1>
        <span className="hidden truncate font-mono text-[11px] text-foreground-subtlest md:inline" title={env?.localRepository}>
          {env ? `${t('maven.localRepo')}: ${env.localRepository}` : ''}
        </span>
        <div className="flex-1" />
        <div className="flex items-center gap-0.5 rounded-lg border border-border/60 bg-surface/50 p-0.5">
          {tabs.map(([id, icon, label]) => (
            <button
              key={id}
              type="button"
              onClick={() => setTab(id)}
              className={cn(
                'flex items-center gap-1.5 rounded-md px-2.5 py-1 text-ui-xs font-medium transition-colors',
                tab === id
                  ? 'bg-selected text-foreground font-semibold'
                  : 'text-foreground-subtle hover:text-foreground',
              )}
            >
              {icon}
              {label}
            </button>
          ))}
        </div>
        <Button variant="ghost" size="icon-sm" aria-label={t('maven.refreshEnv')} onClick={loadEnv}>
          <RefreshCw className="size-3.5" />
        </Button>
      </div>

      {/* 内容区 */}
      <div className="flex min-h-0 flex-1 flex-col p-3">
        {tab === 'download' ? <DownloadTab /> : tab === 'search' ? <SearchTab /> : <UploadTab />}
      </div>
    </section>
  );
}
