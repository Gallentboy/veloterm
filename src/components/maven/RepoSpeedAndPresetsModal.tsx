/**
 * Maven 镜像源测速与 Settings.xml 预设快速切换弹窗：
 * 并发发起 HEAD 测速，实时展示延时色块（绿色 <100ms、黄色 <300ms、红色 超时），
 * 支持快速切换并一键应用至当前下载配置。
 */
import {
  Check,
  CheckCircle2,
  ExternalLink,
  Radio,
  Sparkles,
  Zap,
} from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { Spinner } from '@/components/ui/spinner';
import {
  mavenPingRepos,
  type MavenRepoPingResult,
} from '@/services/mavenService';
import { cn } from '@/lib/utils';

export interface RepoPreset {
  id: string;
  name: string;
  desc: string;
  url: string;
  region: 'CN' | 'GLOBAL';
}

export const POPULAR_PRESETS: RepoPreset[] = [
  {
    id: 'aliyun',
    name: '阿里云公共镜像 (Aliyun Public)',
    desc: '国内访问最广泛的 Maven 镜像，同步 Central、Spring、Google 等仓库',
    url: 'https://maven.aliyun.com/repository/public',
    region: 'CN',
  },
  {
    id: 'tencent',
    name: '腾讯云镜像 (Tencent Cloud)',
    desc: '腾讯云内外部高速 Maven 镜像，高带宽多节点覆盖',
    url: 'https://mirrors.cloud.tencent.com/nexus/repository/maven-public/',
    region: 'CN',
  },
  {
    id: 'huawei',
    name: '华为云镜像 (Huawei Cloud)',
    desc: '华为开源镜像站，提供高可用 Maven 依赖下载服务',
    url: 'https://repo.huaweicloud.com/repository/maven/',
    region: 'CN',
  },
  {
    id: '163',
    name: '网易开源镜像 (163 Mirrors)',
    desc: '网易国内教育网与公网骨干加速镜像',
    url: 'http://mirrors.163.com/maven/repository/maven-central/',
    region: 'CN',
  },
  {
    id: 'central',
    name: '官方中央仓库 (Maven Central)',
    desc: '全球官方单一事实来源（海外及专线环境推荐直连）',
    url: 'https://repo.maven.apache.org/maven2',
    region: 'GLOBAL',
  },
];

interface RepoSpeedAndPresetsModalProps {
  isOpen?: boolean;
  onClose: () => void;
  currentReposText: string;
  onApplyRepo: (repoUrl: string) => void;
}

export function RepoSpeedAndPresetsModal({
  isOpen,
  onClose,
  currentReposText,
  onApplyRepo,
}: RepoSpeedAndPresetsModalProps) {
  const { t } = useTranslation();
  const [pinging, setPinging] = useState(false);
  const [pingResults, setPingResults] = useState<Map<string, MavenRepoPingResult>>(new Map());
  const [appliedUrl, setAppliedUrl] = useState<string | null>(null);

  // 提取自定义仓库 URL
  const customUrls = currentReposText
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0 && !l.startsWith('#'))
    .map((l) => (l.includes('=') ? l.split('=')[1]?.trim() || l : l))
    .filter((u) => u.startsWith('http://') || u.startsWith('https://'));

  const allUrls = Array.from(new Set([...POPULAR_PRESETS.map((p) => p.url), ...customUrls]));

  const runPing = async () => {
    setPinging(true);
    try {
      const results = await mavenPingRepos(allUrls, 6000);
      const m = new Map<string, MavenRepoPingResult>();
      for (const r of results) {
        m.set(r.url, r);
      }
      setPingResults(m);
    } catch (err) {
      console.error('Ping repos failed', err);
    } finally {
      setPinging(false);
    }
  };

  useEffect(() => {
    if (isOpen) {
      void runPing();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen]);

  if (!isOpen) return null;

  const handleApply = (url: string) => {
    onApplyRepo(url);
    setAppliedUrl(url);
    setTimeout(() => setAppliedUrl(null), 2000);
  };

  return (
    <Modal
      title={t('maven.speedTestModalTitle')}
      onClose={onClose}
      className="w-[780px] max-w-full h-[82vh]"
      footer={
        <div className="flex w-full items-center justify-between">
          <span className="text-[11px] text-foreground-subtler flex items-center gap-1.5">
            <CheckCircle2 className="size-3.5 text-terminal-green" />
            {t('maven.speedTestTip')}
          </span>
          <Button size="sm" onClick={onClose}>
            {t('common.close')}
          </Button>
        </div>
      }
    >
      <div className="flex h-full flex-col gap-3 min-h-0">
        {/* 说明与测速动作栏 */}
        <div className="flex shrink-0 items-center justify-between rounded-lg border border-border/60 bg-surface/40 p-2.5">
          <p className="text-[11px] text-foreground-subtle">
            {t('maven.speedTestModalDesc')}
          </p>
          <Button
            size="sm"
            variant="outline"
            disabled={pinging}
            onClick={() => void runPing()}
            className="gap-1.5 shrink-0"
          >
            {pinging ? <Spinner className="size-3.5" /> : <Zap className="size-3.5 text-primary" />}
            {pinging ? t('maven.pinging') : t('maven.retestAll')}
          </Button>
        </div>

        {/* 列表区 */}
        <div className="scrollbar-slim min-h-0 flex-1 overflow-y-auto space-y-3 rounded-lg border border-border/50 bg-input/20 p-3">
          <div className="text-ui-xs font-semibold text-foreground flex items-center gap-1.5">
            <Radio className="size-3.5 text-primary" />
            <span>{t('maven.presetMirrors')}</span>
          </div>

          <div className="space-y-2">
            {POPULAR_PRESETS.map((preset) => {
              const ping = pingResults.get(preset.url);
              const isApplied = appliedUrl === preset.url;
              return (
                <div
                  key={preset.id}
                  className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 rounded-xl border border-border/60 bg-surface/40 p-3 transition-colors hover:bg-surface/70"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="font-semibold text-ui-xs text-foreground">
                        {preset.name}
                      </span>
                      <span className="rounded bg-surface px-1.5 py-0.2 font-mono text-[10px] text-foreground-subtlest border border-border/40">
                        {preset.region}
                      </span>
                    </div>
                    <p className="mt-0.5 text-[11px] text-foreground-subtle">
                      {preset.desc}
                    </p>
                    <ControlHintTooltip title={preset.url} side="top" align="start" block>
                      <span className="block mt-1 font-mono text-[10px] text-foreground-subtlest truncate">
                        {preset.url}
                      </span>
                    </ControlHintTooltip>
                  </div>

                  <div className="flex items-center gap-3 shrink-0 self-end sm:self-center">
                    {/* 延时指示色块 */}
                    {pinging && !ping ? (
                      <Spinner className="size-3.5 text-foreground-subtle" />
                    ) : ping ? (
                      ping.ok ? (
                        <div
                          className={cn(
                            'flex items-center gap-1.5 rounded-full px-2.5 py-1 font-mono text-[11px] font-semibold border',
                            ping.latencyMs < 100
                              ? 'bg-terminal-green/15 text-terminal-green border-terminal-green/30'
                              : ping.latencyMs < 300
                                ? 'bg-warning/15 text-warning border-warning/30'
                                : 'bg-destructive/15 text-destructive border-destructive/30',
                          )}
                        >
                          <span
                            className={cn(
                              'size-2 rounded-full',
                              ping.latencyMs < 100
                                ? 'bg-terminal-green animate-pulse'
                                : ping.latencyMs < 300
                                  ? 'bg-warning'
                                  : 'bg-destructive',
                            )}
                          />
                          <span>{ping.latencyMs} ms</span>
                        </div>
                      ) : (
                        <span className="rounded bg-destructive/10 px-2 py-0.5 font-mono text-[10px] text-destructive border border-destructive/20 font-medium">
                          {ping.error || '连接失败'}
                        </span>
                      )
                    ) : (
                      <span className="font-mono text-[11px] text-foreground-subtlest">未测速</span>
                    )}

                    {/* 应用按钮 */}
                    <Button
                      size="sm"
                      variant={isApplied ? 'secondary' : 'outline'}
                      onClick={() => handleApply(preset.url)}
                      className="gap-1 text-ui-xs h-7"
                    >
                      {isApplied ? (
                        <Check className="size-3 text-terminal-green" />
                      ) : (
                        <ExternalLink className="size-3" />
                      )}
                      {isApplied ? t('maven.applied') : t('maven.applyAsCurrent')}
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>

          {/* 自定义仓库地址列表测速 */}
          {customUrls.length > 0 ? (
            <div className="pt-3">
              <div className="text-ui-xs font-semibold text-foreground mb-2 flex items-center gap-1.5">
                <Sparkles className="size-3.5 text-primary" />
                <span>{t('maven.customReposInUse')}</span>
              </div>
              <div className="space-y-2">
                {customUrls.map((url) => {
                  const ping = pingResults.get(url);
                  return (
                    <div
                      key={url}
                      className="flex items-center justify-between gap-2 rounded-lg border border-border/50 bg-surface/30 px-3 py-2 text-ui-xs"
                    >
                      <ControlHintTooltip title={url} side="top" align="start" block>
                        <span className="font-mono text-[11px] text-foreground-subtle truncate flex-1">
                          {url}
                        </span>
                      </ControlHintTooltip>
                      <div className="shrink-0">
                        {pinging && !ping ? (
                          <Spinner className="size-3.5 text-foreground-subtle" />
                        ) : ping ? (
                          ping.ok ? (
                            <span
                              className={cn(
                                'font-mono text-[11px] font-semibold',
                                ping.latencyMs < 150 ? 'text-terminal-green' : 'text-warning',
                              )}
                            >
                              {ping.latencyMs} ms
                            </span>
                          ) : (
                            <span className="font-mono text-[10px] text-destructive">
                              {ping.error || '失败'}
                            </span>
                          )
                        ) : (
                          <span className="font-mono text-[11px] text-foreground-subtlest">-</span>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </Modal>
  );
}
