import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Box, Gauge, Monitor, Share2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { onSshEvent, type LifecycleEvent } from '@/services/sshLifecycle';
import { chainStateFromEvent, resolveChain, type ChainState } from '@/hosts/topology';
import type { HostNode } from '@/hosts/types';
import { cn } from '@/lib/utils';

const NODE_ICONS = { local: Monitor, relay: Share2, jump: Box, target: Gauge } as const;

function stateBadge(state: ChainState, t: (key: string) => string) {
  switch (state) {
    case 'active':
      return { text: t('topology.stateActive'), cls: 'text-primary' };
    case 'connecting':
      return { text: t('topology.stateConnecting'), cls: 'text-primary animate-pulse' };
    case 'error':
      return { text: t('topology.stateError'), cls: 'text-destructive' };
    default:
      return { text: t('topology.stateIdle'), cls: 'text-foreground-subtle' };
  }
}

/** 全局连接拓扑：所有主机的连接链一屏看全（每行一条链，实时状态汇总）。
 *  在线主机置顶，其余按名称排；点击行尾按钮可回跳单主机详情（由父层传入回调）。 */
export function GlobalTopologyDialog({
  hosts,
  onClose,
  onInspect,
}: {
  hosts: HostNode[];
  onClose: () => void;
  onInspect?: (host: HostNode) => void;
}) {
  const { t } = useTranslation();
  const chains = useMemo(
    () => hosts.map((host) => ({ host, nodes: resolveChain(host, hosts) })),
    [hosts],
  );

  const [latestByHost, setLatestByHost] = useState<Record<number, LifecycleEvent>>({});

  useEffect(() => {
    let dispose: (() => void) | null = null;
    let disposed = false;
    void onSshEvent((event) => {
      if (event.hostId != null) {
        const hostId = event.hostId;
        setLatestByHost((current) => ({ ...current, [hostId]: event }));
      }
    }).then((fn) => {
      if (disposed) fn();
      else dispose = fn;
    });
    return () => {
      disposed = true;
      dispose?.();
    };
  }, []);

  const sorted = useMemo(() => {
    const rank = (state: ChainState) => (state === 'active' ? 0 : state === 'connecting' ? 1 : state === 'error' ? 2 : 3);
    return [...chains].sort((a, b) => {
      const sa = chainStateFromEvent(latestByHost[a.host.id] ?? null);
      const sb = chainStateFromEvent(latestByHost[b.host.id] ?? null);
      const diff = rank(sa) - rank(sb);
      return diff !== 0 ? diff : a.host.name.localeCompare(b.host.name);
    });
  }, [chains, latestByHost]);

  return (
    <Modal
      title={t('topology.globalTitle')}
      onClose={onClose}
      className="w-[860px]"
      footer={
        <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={onClose}>
          {t('common.close')}
        </Button>
      }
    >
      <div className="flex max-h-[70vh] flex-col gap-2 overflow-y-auto pr-1">
        {sorted.length === 0 ? (
          <p className="text-ui-sm text-foreground-subtlest">{t('topology.globalEmpty')}</p>
        ) : null}
        {sorted.map(({ host, nodes }) => {
          const state = chainStateFromEvent(latestByHost[host.id] ?? null);
          const badge = stateBadge(state, t);
          return (
            <div key={host.id} className="flex items-center gap-2 rounded-lg border border-border px-2.5 py-2">
              <div className="flex min-w-0 flex-1 items-center overflow-x-auto">
                {nodes.map((node, index) => {
                  const Icon = NODE_ICONS[node.kind];
                  const isTarget = node.kind === 'target';
                  return (
                    <div key={`${node.kind}-${node.label}-${index}`} className="flex shrink-0 items-center">
                      {index > 0 ? (
                        <div
                          className={cn(
                            'topology-edge w-5 shrink-0',
                            state === 'active'
                              ? 'text-primary topology-edge-active'
                              : state === 'connecting'
                                ? 'text-primary/50'
                                : state === 'error' && isTarget
                                  ? 'text-destructive'
                                  : 'text-border-hover',
                          )}
                        />
                      ) : null}
                      <div className="flex max-w-[168px] flex-col gap-0.5 px-1.5">
                        <div className="flex items-center gap-1">
                          <Icon className="size-3 shrink-0 text-foreground-subtle" />
                          <span
                            className={cn(
                              'truncate text-ui-sm',
                              isTarget ? 'text-foreground' : 'text-foreground-subtle',
                            )}
                            title={node.label}
                          >
                            {node.label}
                          </span>
                        </div>
                        <span className="truncate font-mono text-ui-xs text-foreground-subtlest" title={node.sub}>
                          {node.kind === 'local'
                            ? t('topology.localNode')
                            : node.kind === 'relay'
                              ? t('topology.relayNode')
                              : node.kind === 'jump'
                                ? t('topology.jumpNode')
                                : node.sub}
                        </span>
                      </div>
                    </div>
                  );
                })}
              </div>
              <span className={cn('w-14 shrink-0 text-right text-ui-xs', badge.cls)}>{badge.text}</span>
              {onInspect ? (
                <Button
                  variant="ghost"
                  size="lg"
                  className="h-7 shrink-0 px-2 text-ui-base"
                  onClick={() => onInspect(host)}
                >
                  {t('topology.inspect')}
                </Button>
              ) : null}
            </div>
          );
        })}
      </div>
    </Modal>
  );
}
