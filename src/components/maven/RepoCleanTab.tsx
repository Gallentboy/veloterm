/**
 * 本地 Maven 仓库体检与垃圾清理 Tab：
 * 扫描并批量清理 *.lastUpdated、*.part、_remote.repositories、*.tmp 等临时损坏文件，
 * 并级联清理空目录，彻底解决 Maven 离线报错与缓存锁死。
 */
import {
  AlertCircle,
  CheckCircle2,
  FolderOpen,
  RotateCcw,
  ShieldCheck,
  Sparkles,
  Trash2,
} from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { pickDownloadDir } from '@/services/sftpService';
import {
  mavenCleanBrokenCache,
  mavenScanBrokenCache,
  type BrokenCacheItem,
  type MavenBrokenCacheScanResult,
  type MavenCleanResult,
} from '@/services/mavenService';
import { cn } from '@/lib/utils';

function formatBytes(bytes: number): string {
  if (bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  let v = bytes;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u += 1;
  }
  return `${v.toFixed(1)} ${units[u]}`;
}

export function RepoCleanTab({ defaultRepoPath }: { defaultRepoPath?: string | null }) {
  const { t } = useTranslation();
  const [repoPath, setRepoPath] = useState(defaultRepoPath || '');
  const [scanning, setScanning] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [scanResult, setScanResult] = useState<MavenBrokenCacheScanResult | null>(null);
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [cleanSummary, setCleanSummary] = useState<MavenCleanResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handlePickRepo = async () => {
    try {
      const selected = await pickDownloadDir();
      if (selected) {
        setRepoPath(selected);
      }
    } catch (err) {
      console.error('Pick directory failed', err);
    }
  };

  const handleScan = async () => {
    setScanning(true);
    setError(null);
    setCleanSummary(null);
    try {
      const res = await mavenScanBrokenCache(repoPath.trim() || null);
      setScanResult(res);
      // 默认全选
      setSelectedPaths(new Set(res.files.map((f) => f.path)));
    } catch (err) {
      setError(String(err));
    } finally {
      setScanning(false);
    }
  };

  const handleClean = async () => {
    if (selectedPaths.size === 0) return;
    const count = selectedPaths.size;
    if (!window.confirm(t('maven.cleanConfirm', { count }))) {
      return;
    }

    setCleaning(true);
    setError(null);
    try {
      const res = await mavenCleanBrokenCache(
        Array.from(selectedPaths),
        repoPath.trim() || null,
      );
      setCleanSummary(res);
      // 清理后自动重新扫描以呈现最新状态
      const refreshed = await mavenScanBrokenCache(repoPath.trim() || null);
      setScanResult(refreshed);
      setSelectedPaths(new Set(refreshed.files.map((f) => f.path)));
    } catch (err) {
      setError(String(err));
    } finally {
      setCleaning(false);
    }
  };

  const toggleSelectAll = () => {
    if (!scanResult) return;
    if (selectedPaths.size === scanResult.files.length) {
      setSelectedPaths(new Set());
    } else {
      setSelectedPaths(new Set(scanResult.files.map((f) => f.path)));
    }
  };

  const toggleSelectKind = (kind: string) => {
    if (!scanResult) return;
    const kindPaths = scanResult.files.filter((f) => f.kind === kind).map((f) => f.path);
    const allSelected = kindPaths.every((p) => selectedPaths.has(p));
    setSelectedPaths((prev) => {
      const next = new Set(prev);
      if (allSelected) {
        for (const p of kindPaths) next.delete(p);
      } else {
        for (const p of kindPaths) next.add(p);
      }
      return next;
    });
  };

  const toggleItem = (path: string) => {
    setSelectedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 overflow-hidden">
      {/* 顶部仓库路径配置与横幅 */}
      <div className="shrink-0 rounded-xl border border-border/60 bg-surface/30 p-3">
        <div className="flex flex-col gap-2.5">
          <div className="flex items-center justify-between gap-2">
            <div>
              <h2 className="text-ui-sm font-semibold text-foreground flex items-center gap-1.5">
                <Sparkles className="size-4 text-primary" />
                {t('maven.cleanRepoTitle')}
              </h2>
              <p className="mt-0.5 text-ui-xs text-foreground-subtle">
                {t('maven.cleanRepoDesc')}
              </p>
            </div>
            <Button
              size="sm"
              disabled={scanning || cleaning}
              onClick={() => void handleScan()}
              className="gap-1.5 shrink-0"
            >
              {scanning ? <Spinner className="size-3.5" /> : <Sparkles className="size-3.5" />}
              {scanning ? t('maven.scanning') : t('maven.scanBrokenCache')}
            </Button>
          </div>

          <div className="flex items-center gap-1.5 pt-1">
            <span className="text-ui-xs font-medium text-foreground-subtle shrink-0">
              {t('maven.cleanRepoPath')}:
            </span>
            <input
              value={repoPath}
              onChange={(e) => setRepoPath(e.target.value)}
              placeholder={defaultRepoPath || '~/.m2/repository'}
              className="h-8 flex-1 rounded-md border border-border/60 bg-input px-2.5 font-mono text-ui-xs text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover"
            />
            {defaultRepoPath && repoPath !== defaultRepoPath ? (
              <ControlHintTooltip title={t('common.reset')} side="top">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setRepoPath(defaultRepoPath)}
                >
                  <RotateCcw className="size-3.5" />
                </Button>
              </ControlHintTooltip>
            ) : null}
            <ControlHintTooltip title={t('maven.chooseDir')} side="top">
              <Button variant="outline" size="sm" onClick={() => void handlePickRepo()}>
                <FolderOpen className="size-3.5" />
              </Button>
            </ControlHintTooltip>
          </div>
        </div>
      </div>

      {/* 提示与结果横幅 */}
      {error ? (
        <div className="flex shrink-0 items-start gap-1.5 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-ui-xs text-destructive">
          <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
          <span className="min-w-0 whitespace-pre-wrap break-all">{error}</span>
        </div>
      ) : null}

      {cleanSummary ? (
        <div className="flex shrink-0 items-center gap-2 rounded-lg border border-terminal-green/30 bg-terminal-green/10 px-3 py-2 text-ui-xs text-terminal-green">
          <CheckCircle2 className="size-4 shrink-0" />
          <span className="font-medium">
            {t('maven.cleanSuccess', {
              files: cleanSummary.deletedFiles,
              dirs: cleanSummary.cleanedDirs,
              bytes: formatBytes(cleanSummary.freedBytes),
            })}
          </span>
        </div>
      ) : null}

      {/* 扫描结果卡片与文件列表 */}
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-border/60 bg-background">
        {scanResult ? (
          scanResult.totalFiles === 0 ? (
            <div className="flex flex-1 flex-col items-center justify-center gap-2.5 text-terminal-green select-none p-6">
              <ShieldCheck className="size-12 opacity-80" />
              <p className="text-ui-base font-semibold">{t('maven.noBrokenFiles')}</p>
              <span className="font-mono text-ui-xs text-foreground-subtlest">
                {scanResult.repoPath}
              </span>
            </div>
          ) : (
            <div className="flex h-full min-h-0 flex-col">
              {/* 统计操作栏 */}
              <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-border/50 bg-surface/30 px-3 py-2 text-ui-xs">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-semibold text-foreground">
                    共发现 {scanResult.totalFiles} 个垃圾文件 ({formatBytes(scanResult.totalBytes)})
                  </span>
                  <div className="flex items-center gap-1.5 font-mono text-[11px]">
                    {scanResult.lastUpdatedCount > 0 ? (
                      <button
                        type="button"
                        onClick={() => toggleSelectKind('lastUpdated')}
                        className="rounded border border-destructive/30 bg-destructive/10 px-1.5 py-0.5 text-destructive hover:bg-destructive/20"
                      >
                        {t('maven.lastUpdatedDesc')}: {scanResult.lastUpdatedCount}
                      </button>
                    ) : null}
                    {scanResult.partCount > 0 ? (
                      <button
                        type="button"
                        onClick={() => toggleSelectKind('part')}
                        className="rounded border border-warning/30 bg-warning/10 px-1.5 py-0.5 text-warning hover:bg-warning/20"
                      >
                        {t('maven.partDesc')}: {scanResult.partCount}
                      </button>
                    ) : null}
                    {scanResult.remoteRepoCount > 0 ? (
                      <button
                        type="button"
                        onClick={() => toggleSelectKind('remoteRepo')}
                        className="rounded border border-primary/30 bg-primary/10 px-1.5 py-0.5 text-primary hover:bg-primary/20"
                      >
                        {t('maven.remoteRepoDesc')}: {scanResult.remoteRepoCount}
                      </button>
                    ) : null}
                    {scanResult.tmpCount > 0 ? (
                      <button
                        type="button"
                        onClick={() => toggleSelectKind('tmp')}
                        className="rounded border border-border/60 bg-surface/60 px-1.5 py-0.5 text-foreground-subtle hover:bg-hover"
                      >
                        {t('maven.tmpDesc')}: {scanResult.tmpCount}
                      </button>
                    ) : null}
                  </div>
                </div>

                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={toggleSelectAll}
                    className="rounded px-2 py-0.5 font-medium text-foreground-subtle hover:bg-hover hover:text-foreground"
                  >
                    {selectedPaths.size === scanResult.files.length
                      ? t('maven.unselectAll')
                      : t('maven.selectAll')}
                  </button>
                  <Button
                    variant="destructive"
                    size="sm"
                    disabled={selectedPaths.size === 0 || cleaning}
                    onClick={() => void handleClean()}
                    className="gap-1.5"
                  >
                    {cleaning ? <Spinner className="size-3.5" /> : <Trash2 className="size-3.5" />}
                    {cleaning ? t('maven.cleaning') : `${t('maven.cleanAllBroken')} (${selectedPaths.size})`}
                  </Button>
                </div>
              </div>

              {/* 垃圾文件详细列表 */}
              <div className="scrollbar-slim min-h-0 flex-1 overflow-y-auto divide-y divide-border/30 px-3 py-1 font-mono text-[11px]">
                {scanResult.files.map((file: BrokenCacheItem) => {
                  const isChecked = selectedPaths.has(file.path);
                  return (
                    <label
                      key={file.path}
                      className={cn(
                        'flex cursor-pointer items-center gap-2.5 py-1.5 transition-colors hover:bg-surface/50 select-none px-1 rounded',
                        isChecked && 'bg-surface/30',
                      )}
                    >
                      <input
                        type="checkbox"
                        checked={isChecked}
                        onChange={() => toggleItem(file.path)}
                        className="size-3.5 accent-[var(--color-primary)] shrink-0"
                      />
                      <span
                        className={cn(
                          'rounded px-1.5 py-0.2 text-[10px] font-sans font-medium shrink-0',
                          file.kind === 'lastUpdated'
                            ? 'bg-destructive/15 text-destructive border border-destructive/25'
                            : file.kind === 'part'
                              ? 'bg-warning/15 text-warning border border-warning/25'
                              : file.kind === 'remoteRepo'
                                ? 'bg-primary/15 text-primary border border-primary/25'
                                : 'bg-surface text-foreground-subtle border border-border/40',
                        )}
                      >
                        {file.kind}
                      </span>
                      <div className="min-w-0 flex-1">
                        <ControlHintTooltip title={file.path} side="top" align="start" delayMs={200} block>
                          <span className="block truncate text-foreground-subtle hover:text-foreground">
                            {file.relPath}
                          </span>
                        </ControlHintTooltip>
                      </div>
                      <span className="text-foreground-subtlest shrink-0 tabular-nums">
                        {formatBytes(file.size)}
                      </span>
                    </label>
                  );
                })}
              </div>
            </div>
          )
        ) : (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 text-foreground-subtlest select-none p-6">
            <Sparkles className="size-8 opacity-40" />
            <p className="text-ui-sm">点击上方「开始扫描垃圾」对本地仓库进行全面体检</p>
          </div>
        )}
      </div>
    </div>
  );
}
