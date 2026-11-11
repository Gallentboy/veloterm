/**
 * 本地 Maven 仓库体检与垃圾清理 Tab：
 * 1. 扫描并批量清理 *.lastUpdated、*.part、_remote.repositories、*.tmp 等残存损坏文件与空目录；
 * 2. 体积透视（Top Group 磁盘占用）与 >= 5 个冗余历史废弃版本一键安全去重清理。
 */
import {
  AlertCircle,
  Archive,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  FolderOpen,
  PieChart,
  RotateCcw,
  ShieldCheck,
  Sparkles,
  Trash2,
} from 'lucide-react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ConfirmDialog } from '@/components/ui/ConfirmDialog';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { pickDownloadDir } from '@/services/sftpService';
import {
  mavenAnalyzeRepoSpace,
  mavenCleanBrokenCache,
  mavenCleanHistoricalVersions,
  mavenScanBrokenCache,
  type BrokenCacheItem,
  type MavenBrokenCacheScanResult,
  type MavenCleanResult,
  type RedundantArtifactGroup,
  type RepoSpaceAnalysisResult,
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

type CleanSubTab = 'broken' | 'space';

export function RepoCleanTab({ defaultRepoPath }: { defaultRepoPath?: string | null }) {
  const { t } = useTranslation();
  const [subTab, setSubTab] = useState<CleanSubTab>('broken');
  const [repoPath, setRepoPath] = useState(defaultRepoPath || '');

  // 1. 垃圾损坏标记文件清理
  const [scanning, setScanning] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [scanResult, setScanResult] = useState<MavenBrokenCacheScanResult | null>(null);
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [cleanSummary, setCleanSummary] = useState<MavenCleanResult | null>(null);
  const [confirmCleanBrokenOpen, setConfirmCleanBrokenOpen] = useState(false);

  // 2. 体积透视与多版本去重
  const [minVersions, setMinVersions] = useState('5');
  const [analyzingSpace, setAnalyzingSpace] = useState(false);
  const [cleaningVersions, setCleaningVersions] = useState(false);
  const [spaceResult, setSpaceResult] = useState<RepoSpaceAnalysisResult | null>(null);
  const [selectedVersionDirs, setSelectedVersionDirs] = useState<Set<string>>(new Set());
  const [expandedArtifacts, setExpandedArtifacts] = useState<Set<string>>(new Set());
  const [spaceCleanSummary, setSpaceCleanSummary] = useState<MavenCleanResult | null>(null);
  const [confirmCleanHistoricalOpen, setConfirmCleanHistoricalOpen] = useState(false);

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

  // ---- 垃圾文件扫描与清理 ----
  const handleScan = async () => {
    setScanning(true);
    setError(null);
    setCleanSummary(null);
    try {
      const res = await mavenScanBrokenCache(repoPath.trim() || null);
      setScanResult(res);
      setSelectedPaths(new Set(res.files.map((f) => f.path)));
    } catch (err) {
      setError(String(err));
    } finally {
      setScanning(false);
    }
  };

  const handleCleanClick = () => {
    if (selectedPaths.size === 0) return;
    setConfirmCleanBrokenOpen(true);
  };

  const executeCleanBroken = async () => {
    setCleaning(true);
    setError(null);
    try {
      const res = await mavenCleanBrokenCache(
        Array.from(selectedPaths),
        repoPath.trim() || null,
      );
      setCleanSummary(res);
      setConfirmCleanBrokenOpen(false);
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

  // ---- 体积透视与多版本去重 ----
  const handleAnalyzeSpace = async () => {
    setAnalyzingSpace(true);
    setError(null);
    setSpaceCleanSummary(null);
    try {
      const mv = parseInt(minVersions.trim(), 10) || 5;
      const res = await mavenAnalyzeRepoSpace(repoPath.trim() || null, mv);
      setSpaceResult(res);

      // 默认勾选每个冗余构件中除最新版本之外的所有历史版本
      const initialSelected = new Set<string>();
      const initialExpanded = new Set<string>();
      for (const group of res.redundantArtifacts) {
        const gaKey = `${group.group}:${group.artifact}`;
        initialExpanded.add(gaKey);
        for (const v of group.versions) {
          if (!v.isLatest) {
            initialSelected.add(v.dirPath);
          }
        }
      }
      setSelectedVersionDirs(initialSelected);
      setExpandedArtifacts(initialExpanded);
    } catch (err) {
      setError(String(err));
    } finally {
      setAnalyzingSpace(false);
    }
  };

  const handleCleanHistoricalClick = () => {
    if (selectedVersionDirs.size === 0) return;
    setConfirmCleanHistoricalOpen(true);
  };

  const executeCleanHistorical = async () => {
    setCleaningVersions(true);
    setError(null);
    try {
      const res = await mavenCleanHistoricalVersions(
        Array.from(selectedVersionDirs),
        repoPath.trim() || null,
      );
      setSpaceCleanSummary(res);
      setConfirmCleanHistoricalOpen(false);
      // 刷新体积透视
      const mv = parseInt(minVersions.trim(), 10) || 5;
      const refreshed = await mavenAnalyzeRepoSpace(repoPath.trim() || null, mv);
      setSpaceResult(refreshed);
      const nextSelected = new Set<string>();
      for (const g of refreshed.redundantArtifacts) {
        for (const v of g.versions) {
          if (!v.isLatest) nextSelected.add(v.dirPath);
        }
      }
      setSelectedVersionDirs(nextSelected);
    } catch (err) {
      setError(String(err));
    } finally {
      setCleaningVersions(false);
    }
  };

  const toggleArtifactExpand = (key: string) => {
    setExpandedArtifacts((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const toggleSelectGroupAllHistorical = (group: RedundantArtifactGroup) => {
    const historicalDirs = group.versions.filter((v) => !v.isLatest).map((v) => v.dirPath);
    const allSelected = historicalDirs.every((d) => selectedVersionDirs.has(d));
    setSelectedVersionDirs((prev) => {
      const next = new Set(prev);
      if (allSelected) {
        for (const d of historicalDirs) next.delete(d);
      } else {
        for (const d of historicalDirs) next.add(d);
      }
      return next;
    });
  };

  const toggleVersionDir = (dirPath: string) => {
    setSelectedVersionDirs((prev) => {
      const next = new Set(prev);
      if (next.has(dirPath)) next.delete(dirPath);
      else next.add(dirPath);
      return next;
    });
  };

  const totalRedundantSelectedBytes = useMemo(() => {
    if (!spaceResult) return 0;
    let sum = 0;
    for (const g of spaceResult.redundantArtifacts) {
      for (const v of g.versions) {
        if (selectedVersionDirs.has(v.dirPath)) {
          sum += v.totalBytes;
        }
      }
    }
    return sum;
  }, [spaceResult, selectedVersionDirs]);

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 overflow-hidden">
      {/* 顶部仓库路径配置与子功能切换 */}
      <div className="shrink-0 rounded-xl border border-border/60 bg-surface/30 p-3">
        <div className="flex flex-col gap-2.5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <h2 className="text-ui-sm font-semibold text-foreground flex items-center gap-1.5">
                <Sparkles className="size-4 text-primary" />
                {t('maven.cleanRepoTitle')}
              </h2>
              <p className="mt-0.5 text-ui-xs text-foreground-subtle">
                {t('maven.cleanRepoDesc')}
              </p>
            </div>

            {/* 子 Tab 切换 */}
            <div className="flex items-center gap-1 rounded-lg border border-border/60 bg-surface/50 p-0.5">
              <Button
                variant={subTab === 'broken' ? 'default' : 'ghost'}
                size="xs"
                onClick={() => setSubTab('broken')}
                className="gap-1.5"
              >
                <Sparkles className="size-3.5" />
                {t('maven.subTabBroken')}
              </Button>
              <Button
                variant={subTab === 'space' ? 'default' : 'ghost'}
                size="xs"
                onClick={() => setSubTab('space')}
                className="gap-1.5"
              >
                <PieChart className="size-3.5" />
                {t('maven.subTabSpace')}
              </Button>
            </div>
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

      {/* 提示横幅 */}
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

      {spaceCleanSummary ? (
        <div className="flex shrink-0 items-center gap-2 rounded-lg border border-terminal-green/30 bg-terminal-green/10 px-3 py-2 text-ui-xs text-terminal-green">
          <CheckCircle2 className="size-4 shrink-0" />
          <span className="font-medium">
            已成功清理 {spaceCleanSummary.deletedFiles} 个历史文件与目录，释放磁盘空间 {formatBytes(spaceCleanSummary.freedBytes)}！
          </span>
        </div>
      ) : null}

      {/* 1. 垃圾文件清理模式 */}
      {subTab === 'broken' ? (
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-border/60 bg-background">
          <div className="flex shrink-0 items-center justify-between border-b border-border/50 bg-surface/30 px-3 py-2">
            <span className="text-ui-xs font-semibold text-foreground">
              {t('maven.subTabBroken')}
            </span>
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
                <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-border/50 bg-surface/20 px-3 py-2 text-ui-xs">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-semibold text-foreground">
                      共发现 {scanResult.totalFiles} 个垃圾文件 ({formatBytes(scanResult.totalBytes)})
                    </span>
                    <div className="flex items-center gap-1.5 font-mono text-[11px]">
                      {scanResult.lastUpdatedCount > 0 ? (
                        <Button
                          variant="outline"
                          size="xs"
                          onClick={() => toggleSelectKind('lastUpdated')}
                          className="border-destructive/30 bg-destructive/10 text-destructive hover:bg-destructive/20 h-5 px-1.5"
                        >
                          {t('maven.lastUpdatedDesc')}: {scanResult.lastUpdatedCount}
                        </Button>
                      ) : null}
                      {scanResult.partCount > 0 ? (
                        <Button
                          variant="outline"
                          size="xs"
                          onClick={() => toggleSelectKind('part')}
                          className="border-warning/30 bg-warning/10 text-warning hover:bg-warning/20 h-5 px-1.5"
                        >
                          {t('maven.partDesc')}: {scanResult.partCount}
                        </Button>
                      ) : null}
                      {scanResult.remoteRepoCount > 0 ? (
                        <Button
                          variant="outline"
                          size="xs"
                          onClick={() => toggleSelectKind('remoteRepo')}
                          className="border-primary/30 bg-primary/10 text-primary hover:bg-primary/20 h-5 px-1.5"
                        >
                          {t('maven.remoteRepoDesc')}: {scanResult.remoteRepoCount}
                        </Button>
                      ) : null}
                      {scanResult.tmpCount > 0 ? (
                        <Button
                          variant="outline"
                          size="xs"
                          onClick={() => toggleSelectKind('tmp')}
                          className="border-border/60 bg-surface/60 text-foreground-subtle hover:bg-hover h-5 px-1.5"
                        >
                          {t('maven.tmpDesc')}: {scanResult.tmpCount}
                        </Button>
                      ) : null}
                    </div>
                  </div>

                  <div className="flex items-center gap-2">
                    <Button
                      variant="ghost"
                      size="xs"
                      onClick={toggleSelectAll}
                      className="font-medium text-foreground-subtle hover:text-foreground"
                    >
                      {selectedPaths.size === scanResult.files.length
                        ? t('maven.unselectAll')
                        : t('maven.selectAll')}
                    </Button>
                    <Button
                      variant="destructive"
                      size="sm"
                      disabled={selectedPaths.size === 0 || cleaning}
                      onClick={handleCleanClick}
                      className="gap-1.5"
                    >
                      {cleaning ? <Spinner className="size-3.5" /> : <Trash2 className="size-3.5" />}
                      {cleaning ? t('maven.cleaning') : `${t('maven.cleanAllBroken')} (${selectedPaths.size})`}
                    </Button>
                  </div>
                </div>

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
              <p className="text-ui-sm">点击右上角「开始扫描垃圾」对本地仓库进行全面体检</p>
            </div>
          )}
        </div>
      ) : (
        /* 2. 体积透视与多版本去重模式 */
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-border/60 bg-background">
          <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-border/50 bg-surface/30 px-3 py-2">
            <div className="flex items-center gap-2">
              <span className="text-ui-xs font-semibold text-foreground">
                {t('maven.subTabSpace')}
              </span>
              <div className="flex items-center gap-1 text-[11px] text-foreground-subtle">
                <span>{t('maven.minVersionsLabel')}:</span>
                <input
                  value={minVersions}
                  onChange={(e) => setMinVersions(e.target.value.replace(/\D/g, ''))}
                  placeholder="5"
                  className="h-6 w-12 rounded border border-border/60 bg-input px-1.5 text-center font-mono text-[11px] text-foreground outline-none focus:border-border-hover"
                />
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Button
                size="sm"
                disabled={analyzingSpace || cleaningVersions}
                onClick={() => void handleAnalyzeSpace()}
                className="gap-1.5"
              >
                {analyzingSpace ? <Spinner className="size-3.5" /> : <PieChart className="size-3.5" />}
                {analyzingSpace ? t('maven.analyzing') : t('maven.analyzeSpaceBtn')}
              </Button>
            </div>
          </div>

          {spaceResult ? (
            <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
              {/* 顶部概览指标与操作条 */}
              <div className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-border/40 bg-surface/15 px-3 py-2 text-ui-xs">
                <div className="flex items-center gap-4">
                  <div>
                    <span className="text-foreground-subtler">{t('maven.totalRepoSpace')}: </span>
                    <span className="font-mono font-semibold text-foreground">
                      {formatBytes(spaceResult.totalRepoBytes)}
                    </span>
                  </div>
                  <div>
                    <span className="text-foreground-subtler">冗余构件数: </span>
                    <span className="font-mono font-semibold text-warning">
                      {spaceResult.redundantArtifacts.length} 个
                    </span>
                  </div>
                </div>

                {spaceResult.redundantArtifacts.length > 0 ? (
                  <div className="flex items-center gap-2">
                    <span className="text-[11px] text-foreground-subtle">
                      已选 {selectedVersionDirs.size} 个历史版本 (可释放 {formatBytes(totalRedundantSelectedBytes)})
                    </span>
                    <Button
                      variant="destructive"
                      size="sm"
                      disabled={selectedVersionDirs.size === 0 || cleaningVersions}
                      onClick={handleCleanHistoricalClick}
                      className="gap-1.5"
                    >
                      {cleaningVersions ? <Spinner className="size-3.5" /> : <Trash2 className="size-3.5" />}
                      {cleaningVersions ? t('maven.cleaning') : t('maven.cleanHistoricalBtn')}
                    </Button>
                  </div>
                ) : null}
              </div>

              {/* 左右分栏：左侧 Top Group 占用透视，右侧冗余多版本详情 */}
              <div className="flex min-h-0 flex-1 divide-x divide-border/40">
                {/* 左栏：Top Group 体积排行 */}
                <div className="flex w-72 shrink-0 flex-col overflow-hidden bg-surface/5">
                  <div className="border-b border-border/30 px-3 py-1.5 text-ui-xs font-medium text-foreground-subtle">
                    {t('maven.topGroupsTitle')}
                  </div>
                  <div className="scrollbar-slim min-h-0 flex-1 overflow-y-auto divide-y divide-border/20 p-2">
                    {spaceResult.topGroups.map((g) => {
                      const pct = spaceResult.totalRepoBytes > 0
                        ? Math.min(100, Math.round((g.totalBytes / spaceResult.totalRepoBytes) * 100))
                        : 0;
                      return (
                        <div key={g.group} className="py-2 text-[11px]">
                          <div className="flex items-center justify-between gap-1">
                            <span className="truncate font-mono font-medium text-foreground">
                              {g.group}
                            </span>
                            <span className="font-mono text-foreground-subtle tabular-nums shrink-0">
                              {formatBytes(g.totalBytes)}
                            </span>
                          </div>
                          <div className="mt-1 flex items-center gap-2 text-foreground-subtlest">
                            <div className="h-1 flex-1 overflow-hidden rounded-full bg-surface">
                              <div
                                className="h-full rounded-full bg-primary/70"
                                style={{ width: `${pct}%` }}
                              />
                            </div>
                            <span className="font-mono text-[10px] w-7 text-right">{pct}%</span>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>

                {/* 右栏：多版本冗余列表 */}
                <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
                  <div className="border-b border-border/30 px-3 py-1.5 text-ui-xs font-medium text-foreground-subtle">
                    {t('maven.redundantTitle', { min: minVersions })}
                  </div>
                  <div className="scrollbar-slim min-h-0 flex-1 overflow-y-auto divide-y divide-border/20 p-2 font-mono text-[11px]">
                    {spaceResult.redundantArtifacts.length === 0 ? (
                      <div className="flex h-full flex-col items-center justify-center gap-2 text-terminal-green select-none p-6">
                        <ShieldCheck className="size-10 opacity-70" />
                        <p>{t('maven.noRedundantArtifacts')}</p>
                      </div>
                    ) : (
                      spaceResult.redundantArtifacts.map((group) => {
                        const gaKey = `${group.group}:${group.artifact}`;
                        const isExpanded = expandedArtifacts.has(gaKey);
                        const historicalDirs = group.versions.filter((v) => !v.isLatest).map((v) => v.dirPath);
                        const allHistoricalSelected =
                          historicalDirs.length > 0 &&
                          historicalDirs.every((d) => selectedVersionDirs.has(d));

                        return (
                          <div key={gaKey} className="py-2">
                            {/* 构件标题行 */}
                            <div className="flex items-center gap-2 rounded px-1.5 py-1 transition-colors hover:bg-surface/50">
                              <Button
                                variant="ghost"
                                size="icon-xs"
                                onClick={() => toggleArtifactExpand(gaKey)}
                                className="size-4 p-0 text-foreground-subtlest hover:text-foreground"
                              >
                                {isExpanded ? (
                                  <ChevronDown className="size-3.5" />
                                ) : (
                                  <ChevronRight className="size-3.5" />
                                )}
                              </Button>
                              <input
                                type="checkbox"
                                checked={allHistoricalSelected}
                                onChange={() => toggleSelectGroupAllHistorical(group)}
                                className="size-3.5 accent-[var(--color-primary)] shrink-0"
                              />
                              <div className="min-w-0 flex-1">
                                <span className="font-medium text-foreground">
                                  {group.artifact}
                                </span>
                                <span className="ml-1.5 text-foreground-subtlest text-[10px]">
                                  ({group.group})
                                </span>
                              </div>
                              <span className="rounded bg-warning/10 px-1.5 py-0.5 text-[10px] text-warning border border-warning/20">
                                {group.versions.length} 个版本
                              </span>
                              <span className="text-foreground-subtle tabular-nums font-semibold">
                                {formatBytes(group.totalBytes)}
                              </span>
                            </div>

                            {/* 展开版本子列表 */}
                            {isExpanded ? (
                              <div className="mt-1 ml-6 space-y-1 border-l-2 border-border/40 pl-3">
                                {group.versions.map((v) => {
                                  const isChecked = selectedVersionDirs.has(v.dirPath);
                                  return (
                                    <div
                                      key={v.version}
                                      className="flex items-center justify-between gap-2 py-0.5"
                                    >
                                      <div className="flex items-center gap-2 min-w-0 flex-1">
                                        {v.isLatest ? (
                                          <span className="size-3.5 shrink-0" />
                                        ) : (
                                          <input
                                            type="checkbox"
                                            checked={isChecked}
                                            onChange={() => toggleVersionDir(v.dirPath)}
                                            className="size-3.5 accent-[var(--color-primary)] shrink-0"
                                          />
                                        )}
                                        <span
                                          className={cn(
                                            'truncate',
                                            v.isLatest
                                              ? 'font-semibold text-terminal-green'
                                              : isChecked
                                                ? 'text-foreground'
                                                : 'text-foreground-subtle',
                                          )}
                                        >
                                          {v.version}
                                        </span>
                                        {v.isLatest ? (
                                          <span className="rounded bg-terminal-green/15 px-1.5 py-0.2 text-[10px] font-sans font-medium text-terminal-green border border-terminal-green/30">
                                            🛡️ {t('maven.keepLatestHint')}
                                          </span>
                                        ) : null}
                                      </div>
                                      <span className="text-foreground-subtlest tabular-nums text-[10px]">
                                        {formatBytes(v.totalBytes)} ({v.fileCount} 文件)
                                      </span>
                                    </div>
                                  );
                                })}
                              </div>
                            ) : null}
                          </div>
                        );
                      })
                    )}
                  </div>
                </div>
              </div>
            </div>
          ) : (
            <div className="flex flex-1 flex-col items-center justify-center gap-2 text-foreground-subtlest select-none p-6">
              <Archive className="size-8 opacity-40" />
              <p className="text-ui-sm">点击右上角「分析仓库体积与多版本」透视大目录与冗余版本</p>
            </div>
          )}
        </div>
      )}

      {/* 危险操作二次确认弹窗 */}
      <ConfirmDialog
        open={confirmCleanBrokenOpen}
        title={t('maven.cleanAllBroken')}
        description={t('maven.cleanConfirm', { count: selectedPaths.size })}
        confirmLabel={t('maven.cleanAllBroken')}
        busy={cleaning}
        onConfirm={() => void executeCleanBroken()}
        onCancel={() => setConfirmCleanBrokenOpen(false)}
      />
      <ConfirmDialog
        open={confirmCleanHistoricalOpen}
        title={t('maven.cleanHistoricalBtn')}
        description={t('maven.confirmCleanHistorical', {
          count: selectedVersionDirs.size,
          bytes: formatBytes(totalRedundantSelectedBytes),
        })}
        confirmLabel={t('maven.cleanHistoricalBtn')}
        busy={cleaningVersions}
        onConfirm={() => void executeCleanHistorical()}
        onCancel={() => setConfirmCleanHistoricalOpen(false)}
      />
    </div>
  );
}
