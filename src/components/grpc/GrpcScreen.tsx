import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Loader2, Network, Play, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { FormField, PasswordInput, TextArea, TextInput } from '@/components/hosts/formFields';
import { writeClipboardText } from '@/lib/clipboard';
import { grpcCall, type GrpcHeaderPair } from '@/services/grpcService';
import { cn } from '@/lib/utils';

/**
 * gRPC 调试页：一元调用（服务名 + 方法名 + JSON 入参 → JSON 出参），
 * schema 经服务器反射获取。TLS（CA/客户端证书/SNI）与认证（Bearer/metadata）
 * 全部可配。表单状态组件内留存（页面常驻挂载，切页不丢）。
 */
export function GrpcScreen() {
  const { t } = useTranslation();
  const [target, setTarget] = useState('127.0.0.1:50051');
  const [service, setService] = useState('');
  const [method, setMethod] = useState('');
  const [jsonInput, setJsonInput] = useState('{}');
  const [tlsEnabled, setTlsEnabled] = useState(false);
  const [caPem, setCaPem] = useState('');
  const [serverName, setServerName] = useState('');
  const [clientCertPem, setClientCertPem] = useState('');
  const [clientKeyPem, setClientKeyPem] = useState('');
  const [bearerToken, setBearerToken] = useState('');
  const [headers, setHeaders] = useState<GrpcHeaderPair[]>([]);
  const [timeoutMs, setTimeoutMs] = useState(15000);
  const [calling, setCalling] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [elapsedMs, setElapsedMs] = useState<number | null>(null);
  const [copied, setCopied] = useState(false);

  const mtlsIncomplete =
    tlsEnabled && (clientCertPem.trim().length > 0) !== (clientKeyPem.trim().length > 0);

  const handleCall = useCallback(async () => {
    if (calling) return;
    setError(null);
    setCopied(false);
    if (!target.trim() || !service.trim() || !method.trim()) {
      setError(t('grpc.validationRequired'));
      return;
    }
    if (mtlsIncomplete) {
      setError(t('grpc.mtlsPairRequired'));
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
        tls: tlsEnabled
          ? {
              caPem: caPem.trim() || null,
              serverName: serverName.trim() || null,
              clientCertPem: clientCertPem.trim() || null,
              clientKeyPem: clientKeyPem.trim() || null,
            }
          : null,
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
  }, [calling, target, service, method, tlsEnabled, caPem, serverName, clientCertPem, clientKeyPem, jsonInput, bearerToken, headers, timeoutMs, mtlsIncomplete, t]);

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
    <div className="flex h-full flex-col overflow-hidden">
      {/* 工具栏 */}
      <div className="flex shrink-0 items-center gap-2 border-b border-border/50 px-4 py-2">
        <Network className="size-4 text-primary" />
        <span className="text-ui-sm font-medium text-foreground select-none">{t('grpc.title')}</span>
        <span className="text-ui-xs text-foreground-subtlest select-none">{t('grpc.subtitle')}</span>
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

          {/* TLS 区块 */}
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
              <div className="flex flex-col gap-3">
                <FormField label={t('grpc.tlsServerName')}>
                  <TextInput
                    value={serverName}
                    onChange={(e) => setServerName(e.target.value)}
                    placeholder="livebos-server"
                    className="font-mono"
                    spellCheck={false}
                  />
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
                {mtlsIncomplete ? (
                  <p className="text-ui-xs text-destructive">{t('grpc.mtlsPairRequired')}</p>
                ) : null}
              </div>
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
    </div>
  );
}
