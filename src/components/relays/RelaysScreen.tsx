import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Loader2, Pencil, PlugZap, Server, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { SelectMenu } from '@/components/ui/SelectMenu';
import { FormField, PasswordInput, TextInput } from '@/components/hosts/formFields';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import {
  relayDelete,
  relayList,
  relaySave,
  relayTest,
  type RelayConfig,
} from '@/services/relayConfigService';
import type { RelayTransport } from '@/hosts/types';
import { cn } from '@/lib/utils';

interface RelayFormResult {
  name: string;
  host: string;
  port: number;
  transport: RelayTransport;
  keyId: number;
  secret: string;
  certFingerprint: string | null;
}

/** 行内测试状态：探测中 / 可达（耗时）/ 失败（原因）。 */
type TestState =
  | { kind: 'testing' }
  | { kind: 'ok'; ms: number }
  | { kind: 'fail'; message: string };

/** 中继配置管理页：名称 + 地址/端口/传输/凭证列表，新建/编辑/删除。 */
export function RelaysScreen() {
  const { t } = useTranslation();
  const [records, setRecords] = useState<RelayConfig[]>([]);
  const [editing, setEditing] = useState<RelayConfig | null>(null); // null=列表态；Some=表单态（id=0 新建）
  const [deleting, setDeleting] = useState<RelayConfig | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [testStates, setTestStates] = useState<Record<number, TestState>>({});

  const refresh = async () => {
    try {
      setRecords(await relayList());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  const handleSave = async (result: RelayFormResult) => {
    try {
      await relaySave({
        id: editing?.id ?? 0,
        createdAt: editing?.createdAt ?? 0,
        updatedAt: editing?.updatedAt ?? 0,
        ...result,
      });
      setEditing(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleDelete = async () => {
    if (!deleting) return;
    try {
      await relayDelete(deleting.id);
      setDeleting(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleTest = async (record: RelayConfig) => {
    setTestStates((cur) => ({ ...cur, [record.id]: { kind: 'testing' } }));
    try {
      const ms = await relayTest(record);
      setTestStates((cur) => ({ ...cur, [record.id]: { kind: 'ok', ms } }));
    } catch (e) {
      setTestStates((cur) => ({ ...cur, [record.id]: { kind: 'fail', message: String(e) } }));
    }
  };

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="flex shrink-0 items-center justify-between border-b border-border/50 px-4 py-2">
        <span className="text-ui-sm text-foreground-subtle select-none">
          {t('relays.summary', { total: records.length })}
        </span>
        <Button
          size="lg"
          className="h-7 px-3 text-ui-xs"
          onClick={() => setEditing({ id: 0, name: '', host: '', port: 7300, transport: 'tcp', keyId: 1, secret: '', certFingerprint: null, createdAt: 0, updatedAt: 0 })}
        >
          {t('relays.newRelay')}
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
            <span className="text-ui-sm text-foreground-subtlest">{t('relays.empty')}</span>
            <Button variant="outline" size="lg" onClick={() => setEditing({ id: 0, name: '', host: '', port: 7300, transport: 'tcp', keyId: 1, secret: '', certFingerprint: null, createdAt: 0, updatedAt: 0 })}>
              {t('relays.newRelay')}
            </Button>
          </div>
        ) : (
          records.map((record) => (
            <div
              key={record.id}
              className="group flex items-center gap-3 rounded-lg border border-border px-3 py-2 transition-colors hover:border-border-hover"
            >
              <Server className="size-4 shrink-0 text-foreground-subtle" />
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="truncate text-ui-sm text-foreground">{record.name}</span>
                <span className="truncate font-mono text-ui-xs text-foreground-subtle">
                  {record.host}:{record.port} · {record.transport.toUpperCase()} · key {record.keyId}
                  {record.certFingerprint ? ' · pin' : ''}
                </span>
              </div>
              {(() => {
                const state = testStates[record.id];
                if (!state) return null;
                if (state.kind === 'testing')
                  return (
                    <span className="flex shrink-0 items-center gap-1 text-ui-xs text-foreground-subtle select-none">
                      <Loader2 className="size-3 animate-spin" />
                      {t('relays.testing')}
                    </span>
                  );
                if (state.kind === 'ok')
                  return (
                    <span className="shrink-0 text-ui-xs text-primary select-none">
                      {t('relays.testOk', { ms: state.ms })}
                    </span>
                  );
                return (
                  <span
                    className="max-w-56 shrink-0 truncate text-ui-xs text-destructive select-none"
                    title={state.message}
                  >
                    {t('relays.testFailed')}
                  </span>
                );
              })()}
              <span className="flex shrink-0 items-center gap-0.5">
                <ControlHintTooltip title={t('relays.test')} side="bottom">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="size-7 rounded-lg text-foreground-subtle hover:text-primary"
                    disabled={testStates[record.id]?.kind === 'testing'}
                    onClick={() => void handleTest(record)}
                  >
                    <PlugZap className="size-3.5" />
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
          ))
        )}
      </div>

      {editing ? (
        <RelayFormDialog node={editing.id > 0 ? editing : null} onSave={handleSave} onClose={() => setEditing(null)} />
      ) : null}

      {deleting ? (
        <Modal
          title={t('relays.deleteTitle')}
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
          <p className="text-ui-sm text-foreground">{t('relays.deleteConfirm', { name: deleting.name })}</p>
        </Modal>
      ) : null}
    </div>
  );
}

function RelayFormDialog({
  node,
  onSave,
  onClose,
}: {
  node: RelayConfig | null;
  onSave: (result: RelayFormResult) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(node?.name ?? '');
  const [host, setHost] = useState(node?.host ?? '');
  const [port, setPort] = useState(String(node?.port ?? 7300));
  const [transport, setTransport] = useState<RelayTransport>(node?.transport ?? 'tcp');
  const [keyId, setKeyId] = useState(String(node?.keyId ?? 1));
  const [secret, setSecret] = useState(node?.secret ?? '');
  const [fingerprint, setFingerprint] = useState(node?.certFingerprint ?? '');
  const [error, setError] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: true; ms: number } | { ok: false; message: string } | null>(null);

  const handleSave = () => {
    const portNumber = Number(port);
    const keyIdNumber = Number(keyId);
    if (!name.trim()) return setError(t('relays.nameRequired'));
    if (!host.trim()) return setError(t('relays.hostRequired'));
    if (!Number.isInteger(portNumber) || portNumber < 1 || portNumber > 65535)
      return setError(t('hosts.form.relayPortInvalid'));
    if (!Number.isInteger(keyIdNumber) || keyIdNumber < 0 || keyIdNumber > 255)
      return setError(t('hosts.form.relayKeyIdInvalid'));
    if (!/^[0-9a-fA-F]{64}$/.test(secret.trim()))
      return setError(t('hosts.form.relaySecretInvalid'));
    const fingerprintTrimmed = fingerprint.trim();
    if (fingerprintTrimmed !== '' && !/^[0-9a-fA-F:]{64,79}$/.test(fingerprintTrimmed.replace(/:/g, '').length === 64 ? fingerprintTrimmed : 'x'))
      return setError(t('hosts.form.relayFingerprintInvalid'));
    onSave({
      name: name.trim(),
      host: host.trim(),
      port: portNumber,
      transport,
      keyId: keyIdNumber,
      secret: secret.trim(),
      certFingerprint: fingerprintTrimmed || null,
    });
  };

  /** 用表单当前草稿值探测（未保存也可测；secret/地址不全由后端报错）。 */
  const handleTest = async () => {
    if (testing) return;
    setTesting(true);
    setTestResult(null);
    try {
      const ms = await relayTest({
        id: node?.id ?? 0,
        name: name.trim() || '(draft)',
        host: host.trim(),
        port: Number(port) || 0,
        transport,
        keyId: Number(keyId) || 0,
        secret: secret.trim(),
        certFingerprint: fingerprint.trim() || null,
        createdAt: 0,
        updatedAt: 0,
      });
      setTestResult({ ok: true, ms });
    } catch (e) {
      setTestResult({ ok: false, message: String(e) });
    } finally {
      setTesting(false);
    }
  };

  return (
    <Modal
      title={node ? t('relays.editTitle') : t('relays.newTitle')}
      onClose={onClose}
      className="w-[520px]"
      footer={
        <>
          <Button variant="outline" size="lg" className="h-8 px-3 text-ui-base" disabled={testing} onClick={() => void handleTest()}>
            {testing ? <Loader2 className="size-3.5 animate-spin" /> : <PlugZap className="size-3.5" />}
            {t('relays.test')}
          </Button>
          <div className="flex-1" />
          {testResult ? (
            <span
              className={cn(
                'mr-2 max-w-52 truncate text-ui-xs select-none',
                testResult.ok ? 'text-primary' : 'text-destructive',
              )}
              title={testResult.ok ? undefined : testResult.message}
            >
              {testResult.ok ? t('relays.testOk', { ms: testResult.ms }) : t('relays.testFailed')}
            </span>
          ) : null}
          <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button size="lg" className="h-8 px-3 text-ui-base" onClick={handleSave}>
            {t('common.save')}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5">
        <div className="flex gap-3">
          <div className="min-w-0 flex-1">
            <FormField label={t('relays.name')}>
              <TextInput value={name} onChange={(e) => setName(e.target.value)} placeholder="office-relay" />
            </FormField>
          </div>
          <div className="w-24 shrink-0">
            <FormField label={t('hosts.form.relayPort')}>
              <TextInput type="number" value={port} onChange={(e) => setPort(e.target.value)} min={1} max={65535} />
            </FormField>
          </div>
        </div>
        <FormField label={t('hosts.form.relayHost')}>
          <TextInput value={host} onChange={(e) => setHost(e.target.value)} placeholder="relay.example.com" spellCheck={false} />
        </FormField>
        <div className="flex gap-3">
          <div className="w-36 shrink-0">
            <FormField label={t('zk.relayTransport')}>
              <SelectMenu
                aria-label={t('zk.relayTransport')}
                value={transport}
                onChange={(next) => setTransport(next as RelayTransport)}
                options={[
                  { value: 'tcp', label: 'TCP' },
                  { value: 'ws', label: 'WebSocket' },
                  { value: 'quic', label: 'QUIC' },
                ]}
              />
            </FormField>
          </div>
          <div className="w-24 shrink-0">
            <FormField label={t('hosts.form.relayKeyId')}>
              <TextInput type="number" value={keyId} onChange={(e) => setKeyId(e.target.value)} min={0} max={255} />
            </FormField>
          </div>
          <div className="min-w-0 flex-1">
            <FormField label={t('hosts.form.relaySecret')}>
              <PasswordInput value={secret} onChange={(e) => setSecret(e.target.value)} autoComplete="off" />
            </FormField>
          </div>
        </div>
        <FormField label={`${t('hosts.form.relayFingerprint')}（${t('common.optional')}）`}>
          <TextInput
            value={fingerprint}
            onChange={(e) => setFingerprint(e.target.value)}
            placeholder="AB:CD:EF:…"
            spellCheck={false}
            className="font-mono"
          />
        </FormField>
        {error ? <p className="text-ui-sm text-destructive">{error}</p> : null}
      </div>
    </Modal>
  );
}
