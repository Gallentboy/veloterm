import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Loader2, Network, Pencil, Play, Plus, SearchCheck, Settings2, Table2, Trash2, TriangleAlert, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { SelectMenu } from '@/components/ui/SelectMenu';
import { FormField, PasswordInput, TextArea, TextInput } from '@/components/hosts/formFields';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { writeClipboardText } from '@/lib/clipboard';
import {
  grpcCall,
  grpcList,
  grpcMethodSchema,
  grpcTlsDelete,
  grpcTlsList,
  grpcTlsSave,
  type GrpcCallSeed,
  type GrpcHeaderPair,
  type GrpcMethodSummary,
  type GrpcTlsProfile,
} from '@/services/grpcService';
import { cn } from '@/lib/utils';

/** 常见 gRPC 状态码的中文语义（键为去空格小写后的 code 名）。 */
const GRPC_CODE_LABELS: Record<string, string> = {
  internalerror: '服务端内部错误',
  internal: '服务端内部错误',
  permissiondenied: '权限不足',
  unauthenticated: '未认证或凭据无效',
  unimplemented: '服务端未实现该方法',
  unavailable: '服务不可用（连接中断）',
  deadlineexceeded: '调用超时',
  notfound: '未找到目标资源',
  invalidargument: '入参无效',
  resourceexhausted: '资源耗尽或被限流',
  failedprecondition: '前置条件不满足',
  outofrange: '参数越界',
  alreadyexists: '资源已存在',
  aborted: '操作被中止',
  dataloss: '数据损坏',
  cancelled: '调用被取消',
  unknownerror: '未知错误',
  unknown: '未知错误',
};

/** 拆解后端错误串：状态码 + 中文语义 + 详情正文；非 gRPC 状态错误整段作详情。 */
function parseGrpcError(error: string): { code: string | null; label: string | null; detail: string } {
  const match = error.match(/^gRPC 调用失败 \[(.+?)\]:\s*([\s\S]*)$/);
  if (!match) {
    return { code: null, label: null, detail: error };
  }
  const code = match[1] ?? null;
  const label = code ? (GRPC_CODE_LABELS[code.replace(/\s+/g, '').toLowerCase()] ?? null) : null;
  return { code, label, detail: match[2] ?? '' };
}

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

      {/* 会话视图（常驻隐藏，状态保留）；新建入口只有 tab 栏的加号 */}
      {tabs.length === 0 ? (
        <div className="flex flex-1 items-center justify-center select-none">
          <span className="text-ui-sm text-foreground-subtlest">{t('grpc.noTabs')}</span>
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
  const [listing, setListing] = useState(false);
  const [listingMethods, setListingMethods] = useState(false);
  const [fetchingSchema, setFetchingSchema] = useState(false);
  const [responseSchema, setResponseSchema] = useState<string | null>(null);
  const [servicesModal, setServicesModal] = useState<string[] | null>(null);
  const [methodsModal, setMethodsModal] = useState<GrpcMethodSummary[] | null>(null);
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

  // 勾选 TLS 后默认选中第一个配置（若库非空且尚未手选；配置列表晚加载到位时同样生效）
  useEffect(() => {
    if (tlsEnabled && tlsPickedId == null && tlsProfiles.length > 0) {
      setTlsPickedId(tlsProfiles[0]?.id ?? null);
    }
  }, [tlsEnabled, tlsPickedId, tlsProfiles]);

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

  const handleListServices = useCallback(async () => {
    if (listing) return;
    const trimmedTarget = target.trim();
    if (!trimmedTarget) {
      setError(t('grpc.validationRequired'));
      return;
    }
    setListing(true);
    setError(null);
    try {
      const result = await grpcList(
        {
          target: trimmedTarget,
          tlsProfileId: tlsEnabled ? tlsPickedId : null,
          timeoutMs: Number.isFinite(timeoutMs) && timeoutMs > 0 ? Math.round(timeoutMs) : null,
        },
        null,
      );
      if (result.kind === 'services') {
        setServicesModal(result.services);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setListing(false);
    }
  }, [listing, target, tlsEnabled, tlsPickedId, timeoutMs, t]);

  const handleListMethods = useCallback(async () => {
    if (listingMethods) return;
    const trimmedTarget = target.trim();
    const trimmedService = service.trim();
    if (!trimmedTarget || !trimmedService) {
      setError(t('grpc.validationRequired'));
      return;
    }
    setListingMethods(true);
    setError(null);
    try {
      const result = await grpcList(
        {
          target: trimmedTarget,
          tlsProfileId: tlsEnabled ? tlsPickedId : null,
          timeoutMs: Number.isFinite(timeoutMs) && timeoutMs > 0 ? Math.round(timeoutMs) : null,
        },
        trimmedService,
      );
      if (result.kind === 'methods') {
        setMethodsModal(result.methods);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setListingMethods(false);
    }
  }, [listingMethods, target, service, tlsEnabled, tlsPickedId, timeoutMs, t]);

  const handleFetchSchema = useCallback(async () => {
    if (fetchingSchema) return;
    const trimmedTarget = target.trim();
    const trimmedService = service.trim();
    const trimmedMethod = method.trim();
    if (!trimmedTarget || !trimmedService || !trimmedMethod) {
      setError(t('grpc.validationRequired'));
      return;
    }
    setFetchingSchema(true);
    setError(null);
    try {
      const schema = await grpcMethodSchema(
        {
          target: trimmedTarget,
          tlsProfileId: tlsEnabled ? tlsPickedId : null,
          timeoutMs: Number.isFinite(timeoutMs) && timeoutMs > 0 ? Math.round(timeoutMs) : null,
        },
        trimmedService,
        trimmedMethod,
      );
      setJsonInput(JSON.stringify(schema.requestSchema, null, 2));
      setResponseSchema(JSON.stringify(schema.responseSchema, null, 2));
    } catch (e) {
      setError(String(e));
    } finally {
      setFetchingSchema(false);
    }
  }, [fetchingSchema, target, service, method, tlsEnabled, tlsPickedId, timeoutMs, t]);

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
    setResponseSchema(null);
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
              <div className="flex gap-2">
                <TextInput
                  value={target}
                  onChange={(e) => setTarget(e.target.value)}
                  placeholder="10.0.0.1:50051"
                  className="font-mono"
                  spellCheck={false}
                />
                <ControlHintTooltip title={t('grpc.listServices')} side="bottom">
                  <Button
                    variant="outline"
                    size="lg"
                    className="h-8 shrink-0 px-2.5"
                    disabled={listing || !target.trim()}
                    onClick={() => void handleListServices()}
                  >
                    {listing ? <Loader2 className="size-3.5 animate-spin" /> : <SearchCheck className="size-3.5" />}
                  </Button>
                </ControlHintTooltip>
              </div>
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
              <div className="flex gap-2">
                <TextInput
                  value={service}
                  onChange={(e) => setService(e.target.value)}
                  placeholder="com.example.UserService"
                  className="font-mono"
                  spellCheck={false}
                />
                <ControlHintTooltip title={t('grpc.listMethods')} side="bottom">
                  <Button
                    variant="outline"
                    size="lg"
                    className="h-8 shrink-0 px-2.5"
                    disabled={listingMethods || !service.trim() || !target.trim()}
                    onClick={() => void handleListMethods()}
                  >
                    {listingMethods ? <Loader2 className="size-3.5 animate-spin" /> : <SearchCheck className="size-3.5" />}
                  </Button>
                </ControlHintTooltip>
              </div>
            </FormField>
            <FormField label={t('grpc.method')}>
              <div className="flex gap-2">
                <TextInput
                  value={method}
                  onChange={(e) => setMethod(e.target.value)}
                  placeholder="GetUser"
                  className="font-mono"
                  spellCheck={false}
                />
                <ControlHintTooltip title={t('grpc.fetchSchema')} side="bottom">
                  <Button
                    variant="outline"
                    size="lg"
                    className="h-8 shrink-0 px-2.5"
                    disabled={fetchingSchema || !method.trim() || !service.trim() || !target.trim()}
                    onClick={() => void handleFetchSchema()}
                  >
                    {fetchingSchema ? <Loader2 className="size-3.5 animate-spin" /> : <Table2 className="size-3.5" />}
                  </Button>
                </ControlHintTooltip>
              </div>
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
          {responseSchema != null ? (
            <div className="flex flex-col gap-1.5">
              <span className="text-ui-sm text-foreground-subtle select-none">
                {t('grpc.responseSchemaTitle')}
              </span>
              <pre className="max-h-40 overflow-auto rounded-md border border-border/60 bg-input/30 p-2 font-mono text-ui-xs leading-relaxed text-foreground-subtle">
                {responseSchema}
              </pre>
            </div>
          ) : null}

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
          {error ? <GrpcErrorPanel error={error} title={t('grpc.callFailed')} /> : null}
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

      {/* 方法列表选择（列出方法弹窗） */}
      {methodsModal != null ? (
        <MethodsPickerModal
          methods={methodsModal}
          onPick={(name) => {
            setMethod(name);
            setMethodsModal(null);
          }}
          onClose={() => setMethodsModal(null)}
        />
      ) : null}

      {/* 服务列表选择（列出服务弹窗） */}
      {servicesModal != null ? (
        <ServicesPickerModal
          services={servicesModal}
          onPick={(name) => {
            setService(name);
            setServicesModal(null);
          }}
          onClose={() => setServicesModal(null)}
        />
      ) : null}

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

/** 调用失败面板：大字标题 + 状态码徽标 + 中文语义 + 详情正文；不展示原始 Debug。 */
function GrpcErrorPanel({ error, title }: { error: string; title: string }) {
  const { code, label, detail } = parseGrpcError(error);
  return (
    <div className="flex shrink-0 flex-col gap-2 rounded-md border border-destructive/25 bg-destructive/10 px-4 py-3">
      <div className="flex items-center gap-2">
        <TriangleAlert className="size-4 shrink-0 text-destructive" />
        <span className="text-ui-base font-medium text-destructive select-none">{title}</span>
        {code ? (
          <span className="rounded border border-destructive/40 bg-destructive/15 px-1.5 py-0.5 font-mono text-ui-xs text-destructive select-none">
            {code}
          </span>
        ) : null}
      </div>
      {label ? <span className="text-ui-sm text-destructive select-none">{label}</span> : null}
      {detail ? (
        <span className="whitespace-pre-wrap break-words text-ui-sm leading-relaxed text-destructive/90">
          {detail}
        </span>
      ) : null}
    </div>
  );
}

/** 服务列表选择弹窗：搜索过滤 + 点击选中回填服务全名。 */
function ServicesPickerModal({
  services,
  onPick,
  onClose,
}: {
  services: string[];
  onPick: (service: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return services;
    return services.filter((s) => s.toLowerCase().includes(q));
  }, [services, query]);

  return (
    <Modal title={t('grpc.servicesTitle')} onClose={onClose} className="w-[520px]">
      <div className="flex flex-col gap-3">
        <TextInput
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t('grpc.servicesSearch')}
          spellCheck={false}
        />
        <div className="flex max-h-[46vh] flex-col gap-1 overflow-y-auto">
          {filtered.length === 0 ? (
            <p className="py-6 text-center text-ui-sm text-foreground-subtle">
              {services.length === 0 ? t('grpc.servicesEmptyResult') : t('grpc.servicesEmpty')}
            </p>
          ) : (
            filtered.map((name) => (
              <button
                key={name}
                type="button"
                className="rounded-lg border border-border/60 px-3 py-1.5 text-left font-mono text-ui-sm text-foreground transition-colors hover:border-border-hover hover:bg-surface-hover"
                onClick={() => onPick(name)}
              >
                {name}
              </button>
            ))
          )}
        </div>
      </div>
    </Modal>
  );
}

/** 方法列表选择弹窗：搜索过滤 + 点击选中回填方法名（行内附入/出参类型与流式标记）。 */
function MethodsPickerModal({
  methods,
  onPick,
  onClose,
}: {
  methods: GrpcMethodSummary[];
  onPick: (method: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return methods;
    return methods.filter(
      (m) =>
        m.name.toLowerCase().includes(q) ||
        m.inputType.toLowerCase().includes(q) ||
        m.outputType.toLowerCase().includes(q),
    );
  }, [methods, query]);

  return (
    <Modal title={t('grpc.methodsTitle')} onClose={onClose} className="w-[560px]">
      <div className="flex flex-col gap-3">
        <TextInput
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t('grpc.methodsSearch')}
          spellCheck={false}
        />
        <div className="flex max-h-[46vh] flex-col gap-1 overflow-y-auto">
          {filtered.length === 0 ? (
            <p className="py-6 text-center text-ui-sm text-foreground-subtle">
              {methods.length === 0 ? t('grpc.methodsEmptyResult') : t('grpc.servicesEmpty')}
            </p>
          ) : (
            filtered.map((m) => (
              <button
                key={m.name}
                type="button"
                className="flex flex-col gap-0.5 rounded-lg border border-border/60 px-3 py-1.5 text-left transition-colors hover:border-border-hover hover:bg-surface-hover"
                onClick={() => onPick(m.name)}
              >
                <span className="font-mono text-ui-sm text-foreground">
                  {m.name}
                  {m.clientStreaming || m.serverStreaming ? (
                    <span className="ml-1.5 rounded border border-primary/50 bg-primary/15 px-1 text-ui-xs text-foreground select-none">
                      {t('grpc.methodStreaming')}
                    </span>
                  ) : null}
                </span>
                <span className="truncate font-mono text-ui-xs text-foreground-subtle" title={`${m.inputType} → ${m.outputType}`}>
                  {m.inputType} → {m.outputType}
                </span>
              </button>
            ))
          )}
        </div>
      </div>
    </Modal>
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
