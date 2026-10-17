import { ArrowRight, Plus, Power, Trash2 } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { FormField, Select, TextInput } from '@/components/hosts/formFields';
import { cn } from '@/lib/utils';
import type { HostNode } from '@/hosts/types';
import {
  deletePortForward,
  fetchPortForwardStates,
  listPortForwards,
  listenForwardState,
  savePortForward,
  setPortForwardEnabled,
  type PortForwardKind,
  type PortForwardRule,
  type RuleRuntime,
  type RuleStatus,
} from '@/services/portForwardService';

interface DraftState {
  kind: PortForwardKind;
  bindHost: string;
  bindPort: string;
  targetHost: string;
  targetPort: string;
}

const EMPTY_DRAFT: DraftState = {
  kind: 'local',
  bindHost: '127.0.0.1',
  bindPort: '',
  targetHost: '',
  targetPort: '',
};

function statusDotClass(status: RuleStatus | undefined): string {
  switch (status) {
    case 'active':
      return 'bg-primary';
    case 'starting':
      return 'bg-foreground-subtlest animate-pulse';
    case 'error':
      return 'bg-destructive';
    default:
      return 'bg-border';
  }
}

/**
 * 主机端口转发管理弹层（规则按主机维度持久化，与终端会话生命周期解耦）：
 * - 规则表：类型徽标 / 监听→目标 / 运行状态（经 port-forward-state 事件实时）/
 *   启停 / 删除（弹层内二次确认）；
 * - 新建表单：三种类型（本地 -L / 远程 -R / 动态 SOCKS5 -D），dynamic 无目标字段
 *   （目标由 SOCKS5 客户端逐连接指定），remote 的监听端口可填 0（由服务器分配）。
 */
export function PortForwardDialog({ host, onClose }: { host: HostNode; onClose: () => void }) {
  const { t } = useTranslation();
  const [rules, setRules] = useState<PortForwardRule[]>([]);
  const [states, setStates] = useState<Map<number, RuleRuntime>>(new Map());
  const [draft, setDraft] = useState<DraftState>(EMPTY_DRAFT);
  const [error, setError] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<PortForwardRule | null>(null);
  const [isSaving, setIsSaving] = useState(false);

  const reload = useCallback(async () => {
    try {
      const [list, runtime] = await Promise.all([
        listPortForwards(host.id),
        fetchPortForwardStates(host.id),
      ]);
      setRules(list);
      setStates(new Map(runtime.map((rt) => [rt.ruleId, rt])));
    } catch (err) {
      setError(String(err));
    }
  }, [host.id]);

  useEffect(() => {
    void reload();
    const unlisten = listenForwardState((msg) => {
      if (msg.hostId !== host.id) return;
      setStates(new Map(msg.rules.map((rt) => [rt.ruleId, rt])));
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, [host.id, reload]);

  const runtimeOf = (ruleId: number) => states.get(ruleId);

  const handleAdd = async () => {
    const bindPort = Number.parseInt(draft.bindPort, 10);
    const targetPort = Number.parseInt(draft.targetPort, 10);
    if (!Number.isInteger(bindPort) || bindPort < 0 || bindPort > 65535) {
      setError(t('portForward.error.bindPort'));
      return;
    }
    if (draft.kind === 'local' && (!draft.targetHost || !Number.isInteger(targetPort))) {
      setError(t('portForward.error.target'));
      return;
    }
    if (draft.kind === 'remote' && bindPort === 0) {
      // 0 = 服务器分配：remote 允许，但 local/dynamic 必须显式
    }
    if (draft.kind !== 'remote' && bindPort === 0) {
      setError(t('portForward.error.bindPort'));
      return;
    }
    setIsSaving(true);
    setError(null);
    try {
      await savePortForward({
        id: 0,
        hostId: host.id,
        kind: draft.kind,
        bindHost: draft.bindHost.trim() || '127.0.0.1',
        bindPort,
        targetHost: draft.kind === 'dynamic' ? '' : draft.targetHost.trim(),
        targetPort: draft.kind === 'dynamic' ? 0 : (targetPort ?? 0),
        enabled: true,
      });
      setDraft(EMPTY_DRAFT);
      await reload();
    } catch (err) {
      setError(String(err));
    } finally {
      setIsSaving(false);
    }
  };

  const handleToggle = async (rule: PortForwardRule) => {
    try {
      await setPortForwardEnabled(host.id, rule.id, !rule.enabled);
      await reload();
    } catch (err) {
      setError(String(err));
    }
  };

  const handleDelete = async () => {
    if (!deleteTarget) return;
    try {
      await deletePortForward(host.id, deleteTarget.id);
      setDeleteTarget(null);
      await reload();
    } catch (err) {
      setError(String(err));
      setDeleteTarget(null);
    }
  };

  return (
    <Modal
      title={t('portForward.title', { name: host.name })}
      onClose={onClose}
      className="w-[520px]"
    >
      {/* 规则表 */}
      {rules.length > 0 ? (
        <div className="mb-4 flex flex-col gap-1">
          {rules.map((rule) => {
            const rt = runtimeOf(rule.id);
            const effectiveBind =
              rule.kind === 'remote' && rt?.effectivePort ? rt.effectivePort : rule.bindPort;
            return (
              <div
                key={rule.id}
                className="flex items-center gap-2 rounded-lg border border-border/50 px-2.5 py-1.5"
              >
                <span
                  className={cn(
                    'size-1.5 shrink-0 rounded-full',
                    statusDotClass(rt?.status),
                  )}
                  title={rt?.lastError ?? t(`portForward.status.${rt?.status ?? 'stopped'}`)}
                />
                <span className="w-12 shrink-0 rounded bg-surface px-1 text-center text-ui-xs text-foreground-subtle select-none">
                  {t(`portForward.kind.${rule.kind}`)}
                </span>
                <span className="min-w-0 flex-1 truncate font-mono text-ui-xs text-foreground">
                  {rule.bindHost}:{effectiveBind}
                  {rule.kind !== 'dynamic' ? (
                    <>
                      {' '}
                      <ArrowRight className="inline size-3 text-foreground-subtlest" />{' '}
                      {rule.targetHost}:{rule.targetPort}
                    </>
                  ) : null}
                </span>
                <Button
                  type="button"
                  size="icon-xs"
                  variant="ghost"
                  onClick={() => void handleToggle(rule)}
                  aria-label={rule.enabled ? t('portForward.disable') : t('portForward.enable')}
                  className={cn(
                    'size-6 rounded-md',
                    rule.enabled
                      ? 'text-primary hover:bg-hover'
                      : 'text-foreground-subtlest hover:text-foreground hover:bg-hover',
                  )}
                >
                  <Power className="size-3.5" />
                </Button>
                <Button
                  type="button"
                  size="icon-xs"
                  variant="ghost"
                  onClick={() => setDeleteTarget(rule)}
                  aria-label={t('common.delete')}
                  className="size-6 rounded-md text-foreground-subtle hover:text-destructive hover:bg-hover"
                >
                  <Trash2 className="size-3.5" />
                </Button>
              </div>
            );
          })}
        </div>
      ) : (
        <p className="mb-4 text-ui-xs text-foreground-subtle select-none">
          {t('portForward.empty')}
        </p>
      )}

      {/* 新建表单 */}
      <div className="flex flex-col gap-2.5 rounded-lg border border-border/50 p-3">
        <span className="text-ui-xs font-medium text-foreground-subtle select-none">
          {t('portForward.addTitle')}
        </span>
        <div className="grid grid-cols-2 gap-2.5">
          <FormField label={t('portForward.kindLabel')}>
            <Select
              value={draft.kind}
              onChange={(value) => setDraft({ ...draft, kind: value as PortForwardKind })}
              options={[
                { value: 'local', label: t('portForward.kind.local') },
                { value: 'remote', label: t('portForward.kind.remote') },
                { value: 'dynamic', label: t('portForward.kind.dynamic') },
              ]}
            />
          </FormField>
          <div className="grid grid-cols-2 gap-2">
            <FormField label={t('portForward.bindPort')}>
              <TextInput
                value={draft.bindPort}
                onChange={(e) => setDraft({ ...draft, bindPort: e.target.value })}
                placeholder={draft.kind === 'remote' ? '0=auto' : '8080'}
                inputMode="numeric"
              />
            </FormField>
            <FormField label={t('portForward.targetPort')}>
              <TextInput
                value={draft.targetPort}
                onChange={(e) => setDraft({ ...draft, targetPort: e.target.value })}
                placeholder={draft.kind === 'dynamic' ? '—' : '3000'}
                inputMode="numeric"
                disabled={draft.kind === 'dynamic'}
              />
            </FormField>
          </div>
        </div>
        <div className="grid grid-cols-2 gap-2.5">
          <FormField label={t('portForward.bindHost')}>
            <TextInput
              value={draft.bindHost}
              onChange={(e) => setDraft({ ...draft, bindHost: e.target.value })}
              placeholder="127.0.0.1"
            />
          </FormField>
          <FormField label={t('portForward.targetHost')}>
            <TextInput
              value={draft.targetHost}
              onChange={(e) => setDraft({ ...draft, targetHost: e.target.value })}
              placeholder={draft.kind === 'dynamic' ? '—' : '10.0.0.2'}
              disabled={draft.kind === 'dynamic'}
            />
          </FormField>
        </div>
        {error ? (
          <p className="text-ui-xs text-destructive">{error}</p>
        ) : null}
        <div className="flex justify-end">
          <Button
            type="button"
            size="lg"
            onClick={() => void handleAdd()}
            disabled={isSaving}
            className="gap-1.5"
          >
            <Plus className="size-3.5" />
            {t('portForward.add')}
          </Button>
        </div>
      </div>

      {/* 删除二次确认（工程约定：删除必须确认，弹层内联复用 ConfirmDialog 形态） */}
      {deleteTarget ? (
        <div className="mt-3 flex items-center justify-between gap-2 rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2">
          <span className="min-w-0 text-ui-xs text-foreground">
            {t('portForward.confirmDelete', {
              rule: `${deleteTarget.bindHost}:${deleteTarget.bindPort}`,
            })}
          </span>
          <span className="flex shrink-0 gap-1.5">
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={() => setDeleteTarget(null)}
            >
              {t('common.cancel')}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="destructive"
              onClick={() => void handleDelete()}
            >
              {t('common.delete')}
            </Button>
          </span>
        </div>
      ) : null}
    </Modal>
  );
}
