import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Loader2, Network, Pencil, Play, Plus, Settings2, Trash2, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { SelectMenu } from '@/components/ui/SelectMenu';
import { FormField, PasswordInput, TextArea, TextInput } from '@/components/hosts/formFields';
import { writeClipboardText } from '@/lib/clipboard';
import {
  grpcCall,
  grpcTlsDelete,
  grpcTlsList,
  grpcTlsSave,
  type GrpcCallSeed,
  type GrpcHeaderPair,
  type GrpcTlsProfile,
} from '@/services/grpcService';
import { cn } from '@/lib/utils';

/** 会话 tab：seed 是建 tab 时携带的自动填充种子（ZK 节点的一键调用入口），
 *  GrpcSessionView 挂载时消费一次 */
interface GrpcTab {
  key: number;
  customTitle: string | null;
  baseTitle: string | null;
  seed: GrpcCallSeed | null;
}

/** gRPC 调试页（多 tab）：一元调用（服务名 + 方法名 + JSON 入参 → JSON 出参），
 * schema 经服务器反射获取；TLS 走配置库（校验域名为 key，只能下拉选择）。
 * 会话视图常驻隐藏（切页/切 tab 状态保留）；tab 双击重命名；每次外部入口
 * （ZK 节点数据一键调用）都新开 tab。 */
export function GrpcScreen() {
  const { t } = useTranslation();
  const [tabs, setTabs] = useState<GrpcTab[]>([{ key: 1, customTitle: null, baseTitle: null, seed: null }]);
  const [activeKey, setActiveKey] = useState(1);
  const nextKeyRef = useRef(2);
  const [renamingKey, setRenamingKey] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState('');

  const addTab = useCallback((seed: GrpcCallSeed | null = null) => {
    const key = nextKeyRef.current;
    nextKeyRef.current += 1;
    setTabs((current) => [
      ...current,
      { key, customTitle: null, baseTitle: seed?.target ?? null, seed },
    ]);
    setActiveKey(key);
  }, []);

  const closeTab = useCallback((key: number) => {
    setTabs((current) => {
      const idx = current.findIndex((tab) => tab.key === key);
      const next = current.filter((tab) => tab.key !== key);
      if (key === activeKey) {
        const neighbor = next[Math.min(idx, next.length - 1)];
        setActiveKey(neighbor ? neighbor.key : 0);
      }
      return next;
    });
  }, [activeKey]);

  const commitRename = useCallback(() => {
    if (renamingKey == null) return;
    const key = renamingKey;
    const value = renameValue.trim();
    setTabs((current) =>
      current.map((tab) =>
        tab.key === key ? { ...tab, customTitle: value.length > 0 ? value : null } : tab,
      ),
    );
    setRenamingKey(null);
  }, [renamingKey, renameValue]);

  // ZK 页「gRPC 调用」入口（AppShell dispatch + setPage）：每次点击都新开一个 tab 自动填充
  useEffect(() => {
    const onOpenCall = (event: Event) => {
      const seed = (event as CustomEvent<GrpcCallSeed>).detail;
      if (seed && seed.target) {
        addTab(seed);
      }
    };
    window.addEventListener('veloterm:grpc-open-call', onOpenCall);
    return () => window.removeEventListener('veloterm:grpc-open-call', onOpenCall);
  }, [addTab]);

  return (
    <div className="flex h-full flex-col overflow-hidden">
      {/* tab 栏 */}
      <div className="flex h-9 shrink-0 items-center gap-1 border-b border-border/50 px-2">
        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
          {tabs.map((tab) => (
            <div
              key={tab.key}
              className={cn(
                'group/tab flex h-7 shrink-0 items-center gap-1 rounded-lg border px-2 transition-colors',
                tab.key === activeKey
                  ? 'border-border-hover bg-surface'
                  : 'border-transparent text-foreground-subtle hover:bg-surface-hover',
              )}
              onClick={() => setActiveKey(tab.key)}
              onDoubleClick={() => {
                setRenamingKey(tab.key);
                setRenameValue(tab.customTitle ?? tab.baseTitle ?? '');
              }}
            >
              <Network className="size-3.5 shrink-0" />
              {renamingKey === tab.key ? (
                <input
                  autoFocus
                  value={renameValue}
                  onChange={(e) => setRenameValue(e.target.value)}
                  onBlur={commitRename}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') commitRename();
                    if (e.key === 'Escape') setRenamingKey(null);
                  }}
                  onClick={(e) => e.stopPropagation()}
                  className="w-36 rounded border border-border bg-background px-1 text-ui-xs text-foreground outline-none"
                />
              ) : (
                <span className="max-w-52 truncate text-ui-sm select-none">
                  {tab.customTitle ?? tab.baseTitle ?? t('grpc.newTab')}
                </span>
              )}
              <button
                type="button"
                className="shrink-0 rounded p-0.5 text-foreground-subtlest opacity-0 transition-opacity group-hover/tab:opacity-100 hover:text-foreground"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(tab.key);
                }}
              >
                <X className="size-3" />
              </button>
            </div>
          ))}
          <button
            type="button"
            className="flex size-7 shrink-0 items-center justify-center rounded-lg text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
            onClick={() => addTab()}
            title={t('grpc.newTab')}
          >
            <Plus className="size-4" />
          </button>
        </div>
      </div>

      {/* 会话视图（常驻隐藏，状态保留） */}
      {tabs.length === 0 ? (
        <div className="flex flex-1 items-center justify-center gap-2 select-none">
          <span className="text-ui-sm text-foreground-subtlest">{t('grpc.noTabs')}</span>
          <Button variant="outline" size="lg" className="h-7 px-2.5 text-ui-xs" onClick={() => addTab()}>
            {t('grpc.newTab')}
          </Button>
        </div>
      ) : (
        tabs.map((tab) => (
          <div key={tab.key} className={cn('flex min-h-0 flex-1 flex-col', tab.key !== activeKey && 'hidden')}>
            <GrpcSessionView initialSeed={tab.seed} />
          </div>
        ))
      )}
    </div>
  );
}

/** 单个 gRPC 调用会话：表单（目标/服务/方法/TLS 配置库/认证）+ 结果，
 *  全部状态内部留存；initialSeed 在挂载时消费一次（自动填充 target/service）。 */
function GrpcSessionView({ initialSeed }: { initialSeed?: GrpcCallSeed | null }) {
  const { t } = useTranslation();
  const [target, setTarget] = useState(initialSeed?.target ?? '127.0.0.1:50051');
  const [service, setService] = useState(initialSeed?.service ?? '');
  const [method, setMethod] = useState('');
  const [jsonInput, setJsonInput] = useState('{}');
  const [tlsEnabled, setTlsEnabled] = useState(false);
  const [tlsProfiles, setTlsProfiles] = useState<GrpcTlsProfile[]>([]);
  const [tlsPickedId, setTlsPickedId] = useState<number | null>(null);
  const [tlsManaging, setTlsManaging] = useState(false);
  const [bearerToken, setBearerToken] = useState('');
  const [headers, setHeaders] = useState<GrpcHeaderPair[]>([]);
  const [timeoutMs, setTimeoutMs] = useState(15000);
  const [calling, setCalling] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [elapsedMs, setElapsedMs] = useState<number | null>(null);
  const [copied, setCopied] = useState(false);

  const refreshProfiles = useCallback(() => {
    void grpcTlsList()
      .then(setTlsProfiles)
      .catch(() => {
        // 库不可用时保持空列表，只能明文调用
      });
  }, []);

  useEffect(() => {
    refreshProfiles();
  }, [refreshProfiles]);

  const pickedProfile = tlsProfiles.find((p) => p.id === tlsPickedId) ?? null;

  const handleCall = useCallback(async () => {
    if (calling) return;
    setError(null);
    setCopied(false);
    if (!target.trim() || !service.trim() || !method.trim()) {
      setError(t('grpc.validationRequired'));
      return;
    }
    if (tlsEnabled && tlsPickedId == null) {
      setError(t('grpc.tlsPickRequired'));
      return;
    }
    setCalling(true);
    const startedAt = performance.now();
    try {
      const extraHeaders = headers
        .map((h) => ({ name: h.name.trim(), value: h.value.trim() }))
        .filter((h) => h.name.length > 0);
      const res = await grpcCall({
        target: target.trim(),
        tlsProfileId: tlsEnabled ? tlsPickedId : null,
        service: service.trim(),
        method: method.trim(),
        jsonInput,
        bearerToken: bearerToken.trim() || null,
        headers: extraHeaders,
        timeoutMs: Number.isFinite(timeoutMs) && timeoutMs > 0 ? Math.round(timeoutMs) : null,
      });
      setResult(res.jsonOutput);
      setElapsedMs(Math.round(performance.now() - startedAt));
    } catch (e) {
      setResult(null);
      setElapsedMs(null);
      setError(String(e));
    } finally {
      setCalling(false);
    }
  }, [calling, target, service, method, tlsEnabled, tlsPickedId, jsonInput, bearerToken, headers, timeoutMs, t]);

  const prettyResult = useCallback(() => {
    if (result == null) return '';
    try {
      return JSON.stringify(JSON.parse(result), null, 2);
    } catch {
      return result;
    }
  }, [result]);

  const handleCopy = useCallback(async () => {
    if (result == null) return;
    await writeClipboardText(prettyResult());
    setCopied(true);
  }, [result, prettyResult]);

  const handleClear = useCallback(() => {
    setResult(null);
    setError(null);
    setElapsedMs(null);
    setCopied(false);
  }, []);

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      {/* 工具栏 */}
      <div className="flex shrink-0 items-center gap-2 border-b border-border/50 px-4 py-2">
        <span className="text-ui-sm font-medium text-foreground select-none">{t('grpc.title')}</span>
        {initialSeed?.tls ? (
          <span className="text-ui-xs text-foreground-subtlest select-none">{t('grpc.seedTlsHint')}</span>
        ) : null}
        <div className="flex-1" />
        <Button
          size="lg"
          className="h-7 px-3 text-ui-xs"
          disabled={calling}
          onClick={() => void handleCall()}
        >
          {calling ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
          {calling ? t('grpc.calling') : t('grpc.call')}
        </Button>
        <Button variant="outline" size="lg" className="h-7 px-2.5 text-ui-xs" onClick={handleClear}>
          <Trash2 className="size-3.5" />
          {t('grpc.clearResult')}
        </Button>
      </div>

      {/* 主体：左表单 / 右结果 */}
      <div className="flex min-h-0 flex-1">
        <div className="flex w-[46%] min-w-[420px] flex-col gap-3.5 overflow-y-auto border-r border-border/50 p-4">
          <div className="grid grid-cols-2 gap-3">
            <FormField label={t('grpc.target')}>
              <TextInput
                value={target}
                onChange={(e) => setTarget(e.target.value)}
                placeholder="10.0.0.1:50051"
                className="font-mono"
                spellCheck={false}
              />
            </FormField>
            <FormField label={t('grpc.timeoutMs')}>
              <TextInput
                type="number"
                value={String(timeoutMs)}
                min={1000}
                onChange={(e) => setTimeoutMs(Number(e.target.value))}
              />
            </FormField>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <FormField label={t('grpc.service')}>
              <TextInput
                value={service}
                onChange={(e) => setService(e.target.value)}
                placeholder="com.example.UserService"
                className="font-mono"
                spellCheck={false}
              />
            </FormField>
            <FormField label={t('grpc.method')}>
              <TextInput
                value={method}
                onChange={(e) => setMethod(e.target.value)}
                placeholder="GetUser"
                className="font-mono"
                spellCheck={false}
              />
            </FormField>
          </div>
          <FormField label={t('grpc.jsonInput')}>
            <TextArea
              value={jsonInput}
              onChange={(e) => setJsonInput(e.target.value)}
              placeholder='{"id": 1}'
              spellCheck={false}
            />
          </FormField>

          {/* TLS 区块：只能从配置库下拉选择，要用先保存 */}
          <div className="flex flex-col gap-2.5 rounded-lg border border-border/60 p-3">
            <label className="flex items-center gap-1.5 text-ui-sm text-foreground select-none">
              <input
                type="checkbox"
                checked={tlsEnabled}
                onChange={(e) => setTlsEnabled(e.target.checked)}
                className="accent-foreground"
              />
              {t('grpc.tlsTitle')}
            </label>
            {tlsEnabled ? (
              <div className="flex items-center gap-2">
                {tlsProfiles.length > 0 ? (
                  <div className="w-56">
                    <SelectMenu
                      aria-label={t('grpc.tlsPicker')}
                      value={tlsPickedId != null ? String(tlsPickedId) : ''}
                      onChange={(next) => {
                        const profile = tlsProfiles.find((p) => String(p.id) === next);
                        setTlsPickedId(profile ? profile.id : null);
                      }}
                      options={[
                        { value: '', label: t('grpc.tlsPickPlaceholder') },
                        ...tlsProfiles.map((p) => ({
                          value: String(p.id),
                          label: p.serverName,
                          description: [
                            p.caPem ? 'CA' : t('grpc.tlsSystemRoots'),
                            p.clientCertPem ? t('grpc.tlsClientCertBadge') : null,
                          ]
                            .filter(Boolean)
                            .join(' · '),
                        })),
                      ]}
                    />
                  </div>
                ) : (
                  <span className="text-ui-xs text-foreground-subtle">{t('grpc.tlsEmptyHint')}</span>
                )}
                <Button
                  variant="outline"
                  size="lg"
                  className="h-7 px-2.5 text-ui-xs"
                  onClick={() => setTlsManaging(true)}
                >
                  <Settings2 className="size-3.5" />
                  {t('grpc.tlsManage')}
                </Button>
              </div>
            ) : null}
            {pickedProfile ? (
              <span className="text-ui-xs text-foreground-subtlest select-none">
                {t('grpc.tlsUsing', { name: pickedProfile.serverName })}
              </span>
            ) : null}
          </div>

          {/* 认证区块 */}
          <div className="flex flex-col gap-2.5 rounded-lg border border-border/60 p-3">
            <span className="text-ui-sm text-foreground-subtle select-none">{t('grpc.authTitle')}</span>
            <FormField label={t('grpc.bearerToken')}>
              <PasswordInput
                value={bearerToken}
                onChange={(e) => setBearerToken(e.target.value)}
                autoComplete="off"
              />
            </FormField>
            <div className="flex flex-col gap-2">
              <span className="text-ui-xs text-foreground-subtlest select-none">{t('grpc.headersTitle')}</span>
              {headers.map((h, i) => (
                <div key={i} className="flex items-center gap-2">
                  <TextInput
                    value={h.name}
                    onChange={(e) =>
                      setHeaders((cur) => cur.map((x, idx) => (idx === i ? { ...x, name: e.target.value } : x)))
                    }
                    placeholder="x-tenant-id"
                    className="font-mono"
                    spellCheck={false}
                  />
                  <TextInput
                    value={h.value}
                    onChange={(e) =>
                      setHeaders((cur) => cur.map((x, idx) => (idx === i ? { ...x, value: e.target.value } : x)))
                    }
                    className="font-mono"
                    spellCheck={false}
                  />
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={t('grpc.headerRemove')}
                    onClick={() => setHeaders((cur) => cur.filter((_, idx) => idx !== i))}
                  >
                    <Trash2 className="size-3.5" />
                  </Button>
                </div>
              ))}
              <Button
                variant="outline"
                size="lg"
                className="h-7 self-start px-2.5 text-ui-xs"
                onClick={() => setHeaders((cur) => [...cur, { name: '', value: '' }])}
              >
                {t('grpc.headerAdd')}
              </Button>
            </div>
          </div>
        </div>

        {/* 结果区 */}
        <div className="flex min-w-0 flex-1 flex-col p-4">
          {error ? (
            <div className="flex shrink-0 items-start gap-2 rounded-md border border-destructive/20 bg-destructive/10 px-3 py-2 text-ui-xs text-destructive">
              <span className="whitespace-pre-wrap break-all">{error}</span>
            </div>
          ) : null}
          {result != null ? (
            <>
              <div className="mb-2 flex shrink-0 items-center gap-2">
                <span className="text-ui-xs text-foreground-subtle select-none">
                  {t('grpc.elapsed', { ms: elapsedMs ?? 0 })}
                </span>
                <div className="flex-1" />
                <Button variant="outline" size="lg" className="h-7 px-2.5 text-ui-xs" onClick={() => void handleCopy()}>
                  {copied ? t('grpc.copied') : t('grpc.copy')}
                </Button>
              </div>
              <pre
                className={cn(
                  'min-h-0 flex-1 overflow-auto rounded-md border border-border/60 bg-input/40 p-3',
                  'font-mono text-ui-sm leading-relaxed text-foreground',
                )}
              >
                {prettyResult()}
              </pre>
            </>
          ) : error == null ? (
            <div className="flex flex-1 items-center justify-center">
              <span className="text-ui-sm text-foreground-subtlest select-none">{t('grpc.emptyResult')}</span>
            </div>
          ) : null}
        </div>
      </div>

      {/* TLS 配置管理 */}
      {tlsManaging ? (
        <TlsManageModal
          profiles={tlsProfiles}
          onClose={() => setTlsManaging(false)}
          onChanged={(next) => {
            setTlsProfiles(next);
            if (tlsPickedId != null && !next.some((p) => p.id === tlsPickedId)) {
              setTlsPickedId(null);
            }
          }}
        />
      ) : null}
    </div>
  );
}

/** TLS 配置管理 Modal：列表（编辑/删除）+ 新建/编辑表单（校验域名为 key）。 */
function TlsManageModal({
  profiles,
  onClose,
  onChanged,
}: {
  profiles: GrpcTlsProfile[];
  onClose: () => void;
  onChanged: (profiles: GrpcTlsProfile[]) => void;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<GrpcTlsProfile | null>(null);
  const [deleting, setDeleting] = useState<GrpcTlsProfile | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const reload = useCallback(() => {
    void grpcTlsList()
      .then(onChanged)
      .catch(() => {
        // 列表刷新失败保持现状
      });
  }, [onChanged]);

  return (
    <Modal
      title={t('grpc.tlsManageTitle')}
      onClose={editing != null ? () => setEditing(null) : deleting != null ? () => setDeleting(null) : onClose}
      className="w-[560px]"
      footer={
        editing == null && deleting == null ? (
          <>
            <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={onClose}>
              {t('common.close')}
            </Button>
            <Button size="lg" className="h-8 px-3 text-ui-base" onClick={() => setEditing(newEmptyProfile())}>
              <Plus className="size-3.5" />
              {t('grpc.tlsNew')}
            </Button>
          </>
        ) : undefined
      }
    >
      {editing != null ? (
        <TlsProfileForm
          initial={editing}
          onSaved={() => {
            setEditing(null);
            reload();
          }}
          onCancel={() => setEditing(null)}
        />
      ) : deleting != null ? (
        <div className="flex flex-col gap-3">
          <p className="text-ui-sm text-foreground">
            {t('grpc.tlsDeleteConfirm', { name: deleting.serverName })}
          </p>
          {actionError ? <p className="text-ui-xs text-destructive">{actionError}</p> : null}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={() => setDeleting(null)}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="destructive"
              size="lg"
              className="h-8 px-3 text-ui-base"
              onClick={() => {
                setActionError(null);
                void grpcTlsDelete(deleting.id)
                  .then(() => {
                    setDeleting(null);
                    reload();
                  })
                  .catch((e) => setActionError(String(e)));
              }}
            >
              {t('common.delete')}
            </Button>
          </div>
        </div>
      ) : (
        <div className="flex max-h-[50vh] flex-col gap-1.5 overflow-y-auto">
          {profiles.length === 0 ? (
            <p className="py-6 text-center text-ui-sm text-foreground-subtle">{t('grpc.tlsEmptyHint')}</p>
          ) : (
            profiles.map((p) => (
              <div
                key={p.id}
                className="flex items-center gap-2 rounded-lg border border-border/60 px-3 py-2"
              >
                <span className="font-mono text-ui-sm font-medium text-foreground">{p.serverName}</span>
                <span className="text-ui-xs text-foreground-subtle">
                  {[p.caPem ? 'CA' : t('grpc.tlsSystemRoots'), p.clientCertPem ? t('grpc.tlsClientCertBadge') : null]
                    .filter(Boolean)
                    .join(' · ')}
                </span>
                <div className="flex-1" />
                <Button variant="ghost" size="icon-sm" aria-label={t('grpc.tlsEdit')} onClick={() => setEditing({ ...p })}>
                  <Pencil className="size-3.5" />
                </Button>
                <Button variant="ghost" size="icon-sm" aria-label={t('grpc.tlsDelete')} onClick={() => setDeleting(p)}>
                  <Trash2 className="size-3.5" />
                </Button>
              </div>
            ))
          )}
        </div>
      )}
    </Modal>
  );
}

function newEmptyProfile(): GrpcTlsProfile {
  return {
    id: 0,
    serverName: '',
    caPem: null,
    clientCertPem: null,
    clientKeyPem: null,
    createdAt: 0,
    updatedAt: 0,
  };
}

/** TLS 配置表单：校验域名（唯一 key，无 placeholder）+ CA + 客户端证书对。 */
function TlsProfileForm({
  initial,
  onSaved,
  onCancel,
}: {
  initial: GrpcTlsProfile;
  onSaved: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const [serverName, setServerName] = useState(initial.serverName);
  const [caPem, setCaPem] = useState(initial.caPem ?? '');
  const [clientCertPem, setClientCertPem] = useState(initial.clientCertPem ?? '');
  const [clientKeyPem, setClientKeyPem] = useState(initial.clientKeyPem ?? '');
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const mtlsIncomplete = clientCertPem.trim().length > 0 !== (clientKeyPem.trim().length > 0);

  const handleSave = useCallback(async () => {
    if (saving) return;
    setError(null);
    if (!serverName.trim()) {
      setError(t('grpc.tlsNameRequired'));
      return;
    }
    if (mtlsIncomplete) {
      setError(t('grpc.mtlsPairRequired'));
      return;
    }
    setSaving(true);
    try {
      await grpcTlsSave({
        ...initial,
        serverName: serverName.trim(),
        caPem: caPem.trim() || null,
        clientCertPem: clientCertPem.trim() || null,
        clientKeyPem: clientKeyPem.trim() || null,
      });
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }, [saving, initial, serverName, caPem, clientCertPem, clientKeyPem, mtlsIncomplete, t, onSaved]);

  return (
    <div className="flex flex-col gap-3">
      <FormField label={t('grpc.tlsServerName')}>
        {/* 校验域名是唯一 key，刻意无 placeholder（用户要求） */}
        <TextInput value={serverName} onChange={(e) => setServerName(e.target.value)} spellCheck={false} />
      </FormField>
      <FormField label={t('grpc.tlsCaPem')}>
        <TextArea
          value={caPem}
          onChange={(e) => setCaPem(e.target.value)}
          placeholder={t('grpc.tlsCaPlaceholder')}
          className="h-20 font-mono text-ui-xs"
          spellCheck={false}
        />
      </FormField>
      <div className="grid grid-cols-2 gap-3">
        <FormField label={t('grpc.tlsClientCert')}>
          <TextArea
            value={clientCertPem}
            onChange={(e) => setClientCertPem(e.target.value)}
            placeholder={t('grpc.tlsClientPlaceholder')}
            className="h-20 font-mono text-ui-xs"
            spellCheck={false}
          />
        </FormField>
        <FormField label={t('grpc.tlsClientKey')}>
          <TextArea
            value={clientKeyPem}
            onChange={(e) => setClientKeyPem(e.target.value)}
            placeholder={t('grpc.tlsClientPlaceholder')}
            className="h-20 font-mono text-ui-xs"
            spellCheck={false}
          />
        </FormField>
      </div>
      {mtlsIncomplete ? <p className="text-ui-xs text-destructive">{t('grpc.mtlsPairRequired')}</p> : null}
      {error ? <p className="text-ui-xs text-destructive">{error}</p> : null}
      <div className="flex justify-end gap-2">
        <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={onCancel}>
          {t('common.cancel')}
        </Button>
        <Button size="lg" className="h-8 px-3 text-ui-base" disabled={saving} onClick={() => void handleSave()}>
          {t('common.save')}
        </Button>
      </div>
    </div>
  );
}
