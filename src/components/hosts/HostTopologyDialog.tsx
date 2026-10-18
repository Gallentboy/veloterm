import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Box, Gauge, Monitor, Share2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import {
  fetchPortForwardStates,
  listenForwardState,
  listPortForwards,
  setPortForwardEnabled,
  type PortForwardRule,
  type RuleStatus,
} from '@/services/portForwardService';
import { onSshEvent, type LifecycleEvent } from '@/services/sshLifecycle';
import { chainStateFromEvent, resolveChain } from '@/hosts/topology';
import type { HostNode } from '@/hosts/types';
import { cn } from '@/lib/utils';

interface HostTopologyDialogProps {
  host: HostNode;
  hosts: HostNode[];
  onClose: () => void;
}

const NODE_ICONS = { local: Monitor, relay: Share2, jump: Box, target: Gauge } as const;
const NODE_WIDTH = 'w-[148px]';

/** 连接拓扑：主链（本机/中继/跳板/目标）+ 目标下挂的端口转发分支。
 *  实时状态：SSH 链走 lifecycle 事件（整链粒度），转发规则走 port-forward-state 事件。 */
export function HostTopologyDialog({ host, hosts, onClose }: HostTopologyDialogProps) {
  const { t } = useTranslation();
  const chain = useMemo(() => resolveChain(host, hosts), [host, hosts]);

  const [latestEvent, setLatestEvent] = useState<LifecycleEvent | null>(null);
  const [rules, setRules] = useState<PortForwardRule[]>([]);
  const [statuses, setStatuses] = useState<Record<number, RuleStatus>>({});
  const [toggling, setToggling] = useState<number | null>(null);

  // SSH 链实时状态（同主机多会话取最新事件）
  useEffect(() => {
    let dispose: (() => void) | null = null;
    let disposed = false;
    void onSshEvent((event) => {
      if (event.hostId === host.id) {
        setLatestEvent(event);
      }
    }).then((fn) => {
      if (disposed) fn();
      else dispose = fn;
    });
    return () => {
      disposed = true;
      dispose?.();
    };
  }, [host.id]);

  // 转发规则 + 实时状态
  const refreshRules = async () => {
    try {
      const [list, states] = await Promise.all([
        listPortForwards(host.id),
        fetchPortForwardStates(host.id),
      ]);
      setRules(list);
      setStatuses(Object.fromEntries(states.map((state) => [state.ruleId, state.status])));
    } catch {
      // 后端不可达（测试环境）时保持空态
    }
  };

  useEffect(() => {
    void refreshRules();
    let dispose: (() => void) | null = null;
    let disposed = false;
    void listenForwardState((msg) => {
      if (msg.hostId === host.id) {
        setStatuses((current) => ({
          ...current,
          ...Object.fromEntries(msg.rules.map((rule) => [rule.ruleId, rule.status])),
        }));
      }
    }).then((fn) => {
      if (disposed) fn();
      else dispose = fn;
    });
    return () => {
      disposed = true;
      dispose?.();
    };
    // refreshRules 依赖 host.id，eslint exhaustive-deps 以实际闭包为准
  }, [host.id]);

  const chainState = chainStateFromEvent(latestEvent);

  const toggleRule = async (rule: PortForwardRule) => {
    setToggling(rule.id);
    try {
      await setPortForwardEnabled(host.id, rule.id, !rule.enabled);
      rule.enabled = !rule.enabled;
      setRules((current) => [...current]);
      void refreshRules();
    } finally {
      setToggling(null);
    }
  };

  const edgeClass = (index: number) => {
    // 最后一段（进入目标）在整链活跃时流动；连接中最后一段脉冲等待态
    const isLast = index === chain.length - 2;
    if (chainState === 'active') return 'text-primary';
    if (chainState === 'connecting' && isLast) return 'animate-pulse text-primary';
    if (chainState === 'error' && isLast) return 'text-destructive';
    return 'text-border-hover';
  };

  const chainStateBadge = () => {
    switch (chainState) {
      case 'active':
        return { text: t('topology.stateActive'), cls: 'text-primary' };
      case 'connecting':
        return { text: t('topology.stateConnecting'), cls: 'text-primary animate-pulse' };
      case 'error':
        return { text: t('topology.stateError'), cls: 'text-destructive' };
      default:
        return { text: t('topology.stateIdle'), cls: 'text-foreground-subtle' };
    }
  };
  const badge = chainStateBadge();

  const ruleKindBadge = (kind: PortForwardRule['kind']) => {
    switch (kind) {
      case 'local':
        return t('topology.forwardLocal');
      case 'remote':
        return t('topology.forwardRemote');
      default:
        return t('topology.forwardDynamic');
    }
  };

  const ruleStatusDot = (status: RuleStatus | undefined, enabled: boolean) => {
    const effective: RuleStatus = status ?? (enabled ? 'starting' : 'stopped');
    switch (effective) {
      case 'active':
        return { cls: 'bg-primary', title: t('topology.stateActive') };
      case 'starting':
        return { cls: 'bg-primary/50 animate-pulse', title: t('topology.stateConnecting') };
      case 'error':
        return { cls: 'bg-destructive', title: t('topology.stateError') };
      default:
        return { cls: 'bg-border-hover', title: t('topology.stateIdle') };
    }
  };

  return (
    <Modal title={t('topology.title', { name: host.name })} onClose={onClose} className="w-[760px]"
      footer={
        <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={onClose}>
          {t('common.close')}
        </Button>
      }
    >
      <div className="flex flex-col gap-5">
        {/* 主链：本机 →（中继）→ 跳板链 → 目标 */}
        <div className="flex items-center overflow-x-auto pb-1">
          {chain.map((node, index) => {
            const Icon = NODE_ICONS[node.kind];
            const isTarget = node.kind === 'target';
            return (
              <div key={`${node.kind}-${node.label}-${index}`} className="flex items-center">
                {index > 0 ? (
                  <div
                    className={cn('topology-edge w-7 shrink-0', edgeClass(index - 1), {
                      'topology-edge-active': chainState === 'active',
                    })}
                  />
                ) : null}
                <div
                  className={cn(
                    NODE_WIDTH,
                    'flex shrink-0 flex-col gap-1 rounded-lg border px-3 py-2',
                    isTarget && chainState === 'error'
                      ? 'border-destructive/60'
                      : isTarget && chainState === 'active'
                        ? 'border-border-hover bg-surface'
                        : 'border-border',
                  )}
                >
                  <div className="flex items-center gap-1.5">
                    <Icon className="size-3.5 shrink-0 text-foreground-subtle" />
                    <span className="truncate font-mono text-ui-sm text-foreground" title={node.label}>
                      {node.label}
                    </span>
                    {isTarget ? (
                      <span className={cn('ml-auto shrink-0 text-ui-xs', badge.cls)}>{badge.text}</span>
                    ) : null}
                  </div>
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-ui-xs text-foreground-subtlest">
                      {node.kind === 'local'
                        ? t('topology.localNode')
                        : node.kind === 'relay'
                          ? t('topology.relayNode')
                          : node.kind === 'jump'
                            ? t('topology.jumpNode')
                            : t('topology.targetNode')}
                    </span>
                    <span className="ml-auto truncate font-mono text-ui-xs text-foreground-subtle">
                      {node.sub}
                    </span>
                  </div>
                </div>
              </div>
            );
          })}
        </div>

        {/* 端口转发分支（挂在目标主机下） */}
        <div className="flex flex-col gap-2">
          <p className="text-ui-sm text-foreground-subtle select-none">{t('topology.forwardsSection')}</p>
          {rules.length === 0 ? (
            <p className="text-ui-xs text-foreground-subtlest">{t('topology.noForwards')}</p>
          ) : (
            <div className="flex flex-col gap-1.5">
              {rules.map((rule) => {
                const dot = ruleStatusDot(statuses[rule.id], rule.enabled);
                return (
                  <div key={rule.id} className="flex items-center gap-2">
                    <div className={cn('w-6 shrink-0 self-stretch border-b border-border-hover', 'ml-3')} />
                    <button
                      type="button"
                      onClick={() => void toggleRule(rule)}
                      disabled={toggling === rule.id}
                      className={cn(
                        'flex min-w-0 flex-1 items-center gap-2 rounded-lg border px-2.5 py-1.5 text-left transition-colors',
                        rule.enabled
                          ? 'border-border-hover bg-surface hover:bg-surface-hover'
                          : 'border-border hover:bg-surface-hover',
                        toggling === rule.id && 'opacity-60',
                      )}
                      title={t('topology.forwardToggleHint')}
                    >
                      <span className={cn('size-2 shrink-0 rounded-full', dot.cls)} title={dot.title} />
                      <span className="w-8 shrink-0 text-center text-ui-xs text-foreground-subtle">
                        {ruleKindBadge(rule.kind)}
                      </span>
                      <span className="min-w-0 flex-1 truncate font-mono text-ui-sm text-foreground">
                        {rule.kind === 'dynamic'
                          ? `${rule.bindHost}:${rule.bindPort} → SOCKS5`
                          : rule.kind === 'remote'
                            ? `${rule.targetHost}:${rule.targetPort} → :${rule.bindPort}`
                            : `${rule.bindHost}:${rule.bindPort} → ${rule.targetHost}:${rule.targetPort}`}
                      </span>
                    </button>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* 中继详情脚注 */}
        {host.relay ? (
          <p className="text-ui-xs text-foreground-subtlest select-none">
            {t('topology.relayFooter', {
              transport: host.relay.transport.toUpperCase(),
              host: host.relay.host,
              port: host.relay.port,
            })}
          </p>
        ) : null}
      </div>
    </Modal>
  );
}
