import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { cn } from '@/lib/utils';
import {
  fetchResourceSnapshot,
  type ResourceSnapshot,
} from '@/services/hostResourceService';
import { terminalSessionRegistry } from '@/terminal/terminalSessionRegistry';

/** 轮询间隔：≥2s 才能让 CPU/网络差分有意义的采样窗（采集本身含远端 shell 执行） */
const RESOURCE_POLL_INTERVAL_MS = 2000;

export interface TerminalResourceBarProps {
  sessionId: number;
  /** 断开态：停止轮询，保留最后一帧数据（重连后自动恢复） */
  disconnected: boolean;
}

function formatBytes(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit >= 3 ? 2 : 1;
  return `${value.toFixed(digits)} ${units[unit]}`;
}

function formatRate(bytesPerSec: number | null): string {
  return bytesPerSec == null ? '--' : `${formatBytes(bytesPerSec)}/s`;
}

function formatLatency(ms: number | null): string {
  if (ms == null) return '--';
  if (ms < 1) return '<1 ms';
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

function formatPercent(value: number | null, digits = 0): string {
  return value == null ? '--' : `${value.toFixed(digits)}%`;
}

function formatTransferRate(rx: number | null, tx: number | null): string {
  return `↓ ${formatRate(rx)} ↑ ${formatRate(tx)}`;
}

function Pill({
  label,
  value,
  detail,
  className,
  hint,
}: {
  label: string;
  value: string;
  detail?: string;
  className?: string;
  /** 悬停提示（自定义 tooltip，向上弹出；不用原生 title——样式/延迟不可控且会双重提示） */
  hint?: string;
}) {
  const pill = (
    <div
      className={cn(
        'flex h-5 shrink-0 items-center gap-1.5 rounded-md border border-border/30 bg-surface/60 px-1.5',
        className,
      )}
    >
      <span className="text-ui-xs leading-none font-medium uppercase tracking-wider text-foreground-subtlest">
        {label}
      </span>
      <span className="font-mono text-ui-xs leading-none text-foreground">{value}</span>
      {detail ? (
        <span className="hidden font-mono text-ui-xs leading-none text-foreground-subtle @[760px]:inline">
          {detail}
        </span>
      ) : null}
    </div>
  );
  if (!hint) {
    return pill;
  }
  return (
    <ControlHintTooltip title={hint} side="top">
      {pill}
    </ControlHintTooltip>
  );
}

/**
 * 终端底部远程主机资源监控条（winssh 药丸风格，语义 token 实现）：
 * - 轮询：仅标签页模式的当前可见 SSH 会话由 TerminalSession 条件渲染本组件，
 *   2s setTimeout 链拉取 `ssh_resource_snapshot`（慢链路不重叠）；
 * - 响应式：容器查询按宽度逐级展开——最窄只留 延迟/CPU/内存/磁盘 四枚
 *   百分比药丸，随宽度依次展开 网络速率 → SWAP → GB 明细 → 发行版徽标；
 * - 降级：非 Linux 主机仅显示延迟 + 「仅 Linux」提示；轮询失败保留最后一帧。
 */
export function TerminalResourceBar({ sessionId, disconnected }: TerminalResourceBarProps) {
  const { t } = useTranslation();
  const [snapshot, setSnapshot] = useState<ResourceSnapshot | null>(null);

  useEffect(() => {
    if (disconnected) return;
    let cancelled = false;
    let timer = 0;

    const tick = async () => {
      // terminalId 在 transport.create 异步完成后才可用（重连后也会变化），逐拍解析
      const terminalId = terminalSessionRegistry.getTerminalId(sessionId);
      if (terminalId == null) {
        timer = window.setTimeout(tick, RESOURCE_POLL_INTERVAL_MS);
        return;
      }
      try {
        const next = await fetchResourceSnapshot(terminalId);
        if (!cancelled) setSnapshot(next);
      } catch {
        // 会话未就绪/非 SSH/断开：保留最后一帧静默重试（首次则维持占位展示）
      }
      if (!cancelled) {
        timer = window.setTimeout(tick, RESOURCE_POLL_INTERVAL_MS);
      }
    };

    void tick();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [sessionId, disconnected]);

  const isLinux = snapshot?.platform === 'linux';

  return (
    <div className="@container flex h-[28px] shrink-0 select-none items-center gap-1 overflow-hidden border-t border-border/50 bg-background px-2">
      <Pill
        label={t('terminal.resourceMonitor.latency')}
        value={formatLatency(snapshot?.rttMs ?? null)}
        hint={t('terminal.resourceMonitor.latencyHint')}
      />
      {!isLinux ? (
        <span className="flex h-5 items-center truncate rounded-md border border-dashed border-border/30 bg-surface/60 px-1.5 text-ui-xs leading-none text-foreground-subtle">
          {snapshot
            ? t('terminal.resourceMonitor.linuxOnly')
            : t('terminal.resourceMonitor.loading')}
        </span>
      ) : (
        <>
          <Pill
            label={t('terminal.resourceMonitor.cpu')}
            value={formatPercent(snapshot?.cpu ?? null, 1)}
            hint={t('terminal.resourceMonitor.cpuHint')}
          />
          {snapshot?.memory ? (
            <Pill
              label={t('terminal.resourceMonitor.memory')}
              value={formatPercent(snapshot.memory.usagePercent)}
              detail={`${formatBytes(snapshot.memory.usedBytes)} / ${formatBytes(snapshot.memory.totalBytes)}`}
              hint={t('terminal.resourceMonitor.memoryHint')}
            />
          ) : null}
          <div className="hidden @[460px]:flex">
            <Pill
              label={t('terminal.resourceMonitor.network')}
              value={formatTransferRate(
                snapshot?.network?.rxBytesPerSec ?? null,
                snapshot?.network?.txBytesPerSec ?? null,
              )}
              hint={t('terminal.resourceMonitor.networkHint')}
            />
          </div>
          {snapshot?.disk ? (
            <Pill
              label={t('terminal.resourceMonitor.disk')}
              value={formatPercent(snapshot.disk.usagePercent)}
              detail={`${formatBytes(snapshot.disk.usedBytes)} / ${formatBytes(snapshot.disk.totalBytes)}`}
              hint={t('terminal.resourceMonitor.diskHint')}
            />
          ) : null}
          {snapshot?.swap ? (
            <div className="hidden @[620px]:flex">
              <Pill
                label={t('terminal.resourceMonitor.swap')}
                value={formatPercent(snapshot.swap.usagePercent)}
                hint={t('terminal.resourceMonitor.swapHint')}
              />
            </div>
          ) : null}
          {snapshot?.distro ? (
            <span className="ml-auto hidden shrink-0 @[900px]:inline-flex">
              <ControlHintTooltip title={t('terminal.resourceMonitor.distroHint')} side="top">
                <span className="flex h-5 items-center truncate rounded-md border border-border/30 bg-surface/60 px-1.5 font-mono text-ui-xs leading-none text-foreground-subtle">
                  {snapshot.distro}
                </span>
              </ControlHintTooltip>
            </span>
          ) : null}
        </>
      )}
    </div>
  );
}
