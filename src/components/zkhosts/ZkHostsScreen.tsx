import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Boxes, Copy, Pencil, Play, Plus, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { SelectMenu } from '@/components/ui/SelectMenu';
import { FormField, PasswordInput, TextInput } from '@/components/hosts/formFields';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { relayList, type RelayConfig } from '@/services/relayConfigService';
import {
  zkHostDelete,
  zkHostList,
  zkHostSave,
  type ZkHostRecord,
} from '@/services/zkHostsService';
import { cn } from '@/lib/utils';

/** 表单内的凭证草稿：localId 供 React key 与「默认凭证」下拉在新增行（id=0）间稳定引用 */
interface CredentialDraft {
  localId: number;
  id: number;
  username: string;
  password: string;
  createdAt: number;
}

let credentialLocalIdSeq = 0;
function nextCredentialLocalId(): number {
  credentialLocalIdSeq += 1;
  return credentialLocalIdSeq;
}

function toDrafts(host: ZkHostRecord | null): CredentialDraft[] {
  return (host?.credentials ?? []).map((c) => ({
    localId: nextCredentialLocalId(),
    id: c.id,
    username: c.username,
    password: c.password,
    createdAt: c.createdAt,
  }));
}

/** ZK 主机管理页：名称 + endpoints + 中继 + digest 凭证集，新建/编辑/复制/删除；
 * 行内「连接」经 onConnectHost 跳 ZK 客户端页自动开 tab 连接。 */
export function ZkHostsScreen({ onConnectHost }: { onConnectHost: (host: ZkHostRecord) => void }) {
  const { t } = useTranslation();
  const [records, setRecords] = useState<ZkHostRecord[]>([]);
  const [relays, setRelays] = useState<RelayConfig[]>([]);
  const [editing, setEditing] = useState<ZkHostRecord | null>(null); // null=列表态；Some=表单态（id=0 新建）
  const [deleting, setDeleting] = useState<ZkHostRecord | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = async () => {
    try {
      setRecords(await zkHostList());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void refresh();
    // 中继库只取一次名称映射（引用被删的中继显示为 #id）
    void relayList().then(setRelays).catch(() => {
      // 库不可用时保持空列表，只能直连
    });
  }, []);

  const relayNameOf = (host: ZkHostRecord): string =>
    host.relayId != null ? (relays.find((r) => r.id === host.relayId)?.name ?? `#${host.relayId}`) : t('zkHosts.direct');

  const handleDuplicate = async (host: ZkHostRecord) => {
    try {
      // 前端克隆：id=0、凭证 id 全 0；默认凭证指向的 id 会变，复制后不自动认证
      await zkHostSave({
        ...host,
        id: 0,
        name: t('zkHosts.copyName', { name: host.name }),
        defaultCredentialId: null,
        credentials: host.credentials.map((c) => ({ ...c, id: 0 })),
        createdAt: 0,
        updatedAt: 0,
      });
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleDelete = async () => {
    if (!deleting) return;
    try {
      await zkHostDelete(deleting.id);
      setDeleting(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const newHost = (): ZkHostRecord => ({
    id: 0,
    name: '',
    endpoints: '',
    relayId: null,
    defaultCredentialId: null,
    credentials: [],
    createdAt: 0,
    updatedAt: 0,
  });

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="flex shrink-0 items-center justify-between border-b border-border/50 px-4 py-2">
        <span className="text-ui-sm text-foreground-subtle select-none">
          {t('zkHosts.summary', { total: records.length })}
        </span>
        <Button size="lg" className="h-7 px-3 text-ui-xs" onClick={() => setEditing(newHost())}>
          {t('zkHosts.newHost')}
        </Button>
      </div>

      {error ? (
        <div className="flex shrink-0 items-center justify-between gap-2 border-b border-destructive/20 bg-destructive/10 px-4 py-1.5 text-ui-xs text-destructive">
          <span className="min-w-0 flex-1 truncate">{error}</span>
          <button type="button" onClick={() => setError(null)} className="opacity-70 hover:opacity-100">✕</button>
        </div>
      ) : null}

      <div className="flex min-h-0 flex-1 flex-col gap-1.5 overflow-y-auto p-3">
        {loading ? (
          <span className="text-ui-sm text-foreground-subtlest">{t('zk.loading')}</span>
        ) : records.length === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 select-none">
            <span className="text-ui-sm text-foreground-subtlest">{t('zkHosts.empty')}</span>
            <Button variant="outline" size="lg" onClick={() => setEditing(newHost())}>
              {t('zkHosts.newHost')}
            </Button>
          </div>
        ) : (
          records.map((record) => {
            const defaultCred =
              record.defaultCredentialId != null
                ? (record.credentials.find((c) => c.id === record.defaultCredentialId) ?? null)
                : null;
            const otherCreds = record.credentials.filter((c) => c !== defaultCred);
            return (
              <div
                key={record.id}
                className="group flex items-center gap-3 rounded-lg border border-border px-3 py-2 transition-colors hover:border-border-hover"
              >
                <Boxes className="size-4 shrink-0 text-foreground-subtle" />
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="truncate text-ui-sm font-medium text-foreground">{record.name}</span>
                  <span className="truncate font-mono text-ui-xs text-foreground-subtle">
                    {record.endpoints} · {relayNameOf(record)}
                  </span>
                </div>
                {/* 凭证摘要：默认凭证 username 高亮，其余灰色；无凭证显示「无认证」 */}
                <span className="hidden max-w-56 shrink-0 truncate text-ui-xs md:block">
                  {record.credentials.length === 0 ? (
                    <span className="text-foreground-subtlest">{t('zkHosts.noAuth')}</span>
                  ) : (
                    <>
                      {defaultCred ? (
                        <span className="font-medium text-foreground">{defaultCred.username}</span>
                      ) : null}
                      {otherCreds.length > 0 ? (
                        <span className="text-foreground-subtlest">
                          {defaultCred ? ' · ' : ''}
                          {otherCreds.map((c) => c.username).join(', ')}
                        </span>
                      ) : null}
                    </>
                  )}
                </span>
                <span className="flex shrink-0 items-center gap-0.5">
                  <ControlHintTooltip title={t('zkHosts.connect')} side="bottom">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="size-7 rounded-lg text-foreground-subtle hover:text-primary"
                      onClick={() => onConnectHost(record)}
                    >
                      <Play className="size-3.5" />
                    </Button>
                  </ControlHintTooltip>
                  <ControlHintTooltip title={t('common.edit')} side="bottom">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="size-7 rounded-lg text-foreground-subtle hover:text-foreground"
                      onClick={() => setEditing(record)}
                    >
                      <Pencil className="size-3.5" />
                    </Button>
                  </ControlHintTooltip>
                  <ControlHintTooltip title={t('zkHosts.duplicate')} side="bottom">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="size-7 rounded-lg text-foreground-subtle hover:text-foreground"
                      onClick={() => void handleDuplicate(record)}
                    >
                      <Copy className="size-3.5" />
                    </Button>
                  </ControlHintTooltip>
                  <ControlHintTooltip title={t('common.delete')} side="bottom">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="size-7 rounded-lg text-foreground-subtle hover:text-destructive"
                      onClick={() => setDeleting(record)}
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </ControlHintTooltip>
                </span>
              </div>
            );
          })
        )}
      </div>

      {editing ? (
        <ZkHostFormDialog
          node={editing.id > 0 ? editing : null}
          relays={relays}
          onSaved={() => {
            setEditing(null);
            void refresh();
          }}
          onClose={() => setEditing(null)}
        />
      ) : null}

      {deleting ? (
        <Modal
          title={t('zkHosts.deleteTitle')}
          onClose={() => setDeleting(null)}
          className="w-[420px]"
          footer={
            <>
              <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={() => setDeleting(null)}>
                {t('common.cancel')}
              </Button>
              <Button variant="destructive" size="lg" className="h-8 px-3 text-ui-base" onClick={() => void handleDelete()}>
                {t('common.delete')}
              </Button>
            </>
          }
        >
          <p className="text-ui-sm text-foreground">{t('zkHosts.deleteConfirm', { name: deleting.name })}</p>
        </Modal>
      ) : null}
    </div>
  );
}

function ZkHostFormDialog({
  node,
  relays,
  onSaved,
  onClose,
}: {
  node: ZkHostRecord | null;
  relays: RelayConfig[];
  onSaved: () => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(node?.name ?? '');
  const [endpoints, setEndpoints] = useState(node?.endpoints ?? '');
  const [relayId, setRelayId] = useState<number | null>(node?.relayId ?? null);
  const [credentials, setCredentials] = useState<CredentialDraft[]>(() => toDrafts(node));
  // 「默认凭证」在新增行（id=0）上没有后端 id 可引用：先用 localId 记住选择，
  // 挂载时把已有记录的 defaultCredentialId 映射到对应草稿行
  const [defaultLocalId, setDefaultLocalId] = useState<number | null>(() => {
    const defId = node?.defaultCredentialId;
    if (defId == null) return null;
    return toDrafts(node).find((c) => c.id === defId)?.localId ?? null;
  });
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);

  // 同主机内 username 重复：即时标错 + 禁用保存
  const duplicateUsernames = useMemo(() => {
    const seen = new Set<string>();
    const dup = new Set<string>();
    for (const c of credentials) {
      const key = c.username.trim();
      if (!key) continue;
      if (seen.has(key)) dup.add(key);
      seen.add(key);
    }
    return [...dup];
  }, [credentials]);

  // 用户名/密码只填了一半的行
  const hasIncomplete = credentials.some((c) => (c.username.trim().length > 0) !== (c.password.length > 0));

  const canSave =
    !saving &&
    name.trim().length > 0 &&
    endpoints.trim().length > 0 &&
    duplicateUsernames.length === 0 &&
    !hasIncomplete;

  const updateCredential = (localId: number, patch: Partial<Pick<CredentialDraft, 'username' | 'password'>>) => {
    setCredentials((current) => current.map((c) => (c.localId === localId ? { ...c, ...patch } : c)));
  };

  const removeCredential = (localId: number) => {
    setCredentials((current) => current.filter((c) => c.localId !== localId));
    setDefaultLocalId((current) => (current === localId ? null : current));
  };

  const addCredential = () => {
    setCredentials((current) => [
      ...current,
      { localId: nextCredentialLocalId(), id: 0, username: '', password: '', createdAt: 0 },
    ]);
  };

  const handleSave = async () => {
    if (!canSave || savingRef.current) return;
    const trimmedName = name.trim();
    const trimmedEndpoints = endpoints.trim();
    if (!trimmedName) return setError(t('zkHosts.nameRequired'));
    if (!trimmedEndpoints) return setError(t('zkHosts.endpointsRequired'));
    if (duplicateUsernames.length > 0)
      return setError(t('zkHosts.credentialDup', { name: duplicateUsernames.join(', ') }));
    if (hasIncomplete) return setError(t('zkHosts.credentialIncomplete'));

    // 全空行（用户名与密码都没填）保存时静默丢弃
    const kept = credentials.filter((c) => c.username.trim().length > 0 || c.password.length > 0);
    const defaultRow =
      defaultLocalId != null ? (kept.find((c) => c.localId === defaultLocalId) ?? null) : null;
    const record: ZkHostRecord = {
      id: node?.id ?? 0,
      name: trimmedName,
      endpoints: trimmedEndpoints,
      relayId,
      defaultCredentialId: defaultRow && defaultRow.id > 0 ? defaultRow.id : null,
      credentials: kept.map((c) => ({
        id: c.id,
        username: c.username.trim(),
        password: c.password,
        createdAt: c.createdAt,
        updatedAt: 0,
      })),
      createdAt: node?.createdAt ?? 0,
      updatedAt: node?.updatedAt ?? 0,
    };
    savingRef.current = true;
    setSaving(true);
    try {
      let saved = await zkHostSave(record);
      // 默认凭证指向新增行（保存前无 id）：按 username（同主机内唯一）在落库
      // 结果里回找新 id，二次保存补上引用
      if (defaultRow && defaultRow.id === 0) {
        const cred = saved.credentials.find((c) => c.username === defaultRow.username.trim());
        if (cred && cred.id > 0) {
          saved = await zkHostSave({ ...saved, defaultCredentialId: cred.id });
        }
      }
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  };

  const credentialOptions = credentials
    .filter((c) => c.username.trim().length > 0 || c.password.length > 0)
    .map((c) => ({ value: String(c.localId), label: c.username.trim() }));

  return (
    <Modal
      title={node ? t('zkHosts.editTitle') : t('zkHosts.newTitle')}
      onClose={onClose}
      className="w-[560px]"
      footer={
        <>
          <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button size="lg" className="h-8 px-3 text-ui-base" disabled={!canSave} onClick={() => void handleSave()}>
            {t('common.save')}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5">
        <div className="flex gap-3">
          <div className="min-w-0 flex-1">
            <FormField label={t('zkHosts.name')}>
              <TextInput value={name} onChange={(e) => setName(e.target.value)} placeholder="prod-zk" />
            </FormField>
          </div>
          <div className="min-w-0 flex-1">
            <FormField label={t('zkHosts.endpoints')}>
              <TextInput
                value={endpoints}
                onChange={(e) => setEndpoints(e.target.value)}
                placeholder="10.0.0.1:2181,10.0.0.2:2181"
                spellCheck={false}
                className="font-mono"
              />
            </FormField>
          </div>
        </div>
        <FormField label={t('zkHosts.relay')}>
          <SelectMenu
            aria-label={t('zkHosts.relay')}
            value={relayId != null ? String(relayId) : ''}
            onChange={(next) => setRelayId(next === '' ? null : Number(next))}
            options={[
              { value: '', label: t('zkHosts.relayNone') },
              ...relays.map((r) => ({
                value: String(r.id),
                label: r.name,
                description: `${r.host}:${r.port} · ${r.transport.toUpperCase()}`,
              })),
            ]}
          />
        </FormField>

        <div className="flex flex-col gap-1.5">
          <span className="text-ui-sm text-foreground-subtle select-none">{t('zkHosts.credentials')}</span>
          <div className="flex flex-col gap-1.5">
            {credentials.map((row) => {
              const isDup = duplicateUsernames.includes(row.username.trim());
              return (
                <div key={row.localId} className="flex items-center gap-2">
                  <div className="w-44 shrink-0">
                    <TextInput
                      value={row.username}
                      onChange={(e) => updateCredential(row.localId, { username: e.target.value })}
                      placeholder={t('zkHosts.credentialUser')}
                      spellCheck={false}
                      autoComplete="off"
                      className={cn(isDup && 'border-destructive focus:border-destructive')}
                    />
                  </div>
                  <div className="min-w-0 flex-1">
                    <PasswordInput
                      value={row.password}
                      onChange={(e) => updateCredential(row.localId, { password: e.target.value })}
                      autoComplete="new-password"
                    />
                  </div>
                  <ControlHintTooltip title={t('common.delete')} side="left">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="size-7 shrink-0 rounded-lg text-foreground-subtle hover:text-destructive"
                      onClick={() => removeCredential(row.localId)}
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </ControlHintTooltip>
                </div>
              );
            })}
          </div>
          {duplicateUsernames.length > 0 ? (
            <p className="text-ui-xs text-destructive">
              {t('zkHosts.credentialDup', { name: duplicateUsernames.join(', ') })}
            </p>
          ) : hasIncomplete ? (
            <p className="text-ui-xs text-destructive">{t('zkHosts.credentialIncomplete')}</p>
          ) : null}
          <Button variant="outline" size="lg" className="h-7 self-start px-2.5 text-ui-xs" onClick={addCredential}>
            <Plus className="size-3.5" />
            {t('zkHosts.addCredential')}
          </Button>
        </div>

        <FormField label={t('zkHosts.defaultCredential')}>
          <SelectMenu
            aria-label={t('zkHosts.defaultCredential')}
            value={defaultLocalId != null ? String(defaultLocalId) : ''}
            onChange={(next) => setDefaultLocalId(next === '' ? null : Number(next))}
            placeholder={t('zkHosts.defaultCredentialNone')}
            options={[
              { value: '', label: t('zkHosts.defaultCredentialNone') },
              ...credentialOptions,
            ]}
          />
        </FormField>

        {error ? <p className="text-ui-sm text-destructive">{error}</p> : null}
      </div>
    </Modal>
  );
}
