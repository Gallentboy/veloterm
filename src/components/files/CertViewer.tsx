import {
  AlertCircle,
  AlertTriangle,
  CheckCircle2,
  Check,
  Copy,
  Globe,
  KeyRound,
  ShieldCheck,
  ShieldAlert,
  Calendar,
  Layers,
} from 'lucide-react';
import { memo, useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { cn } from '@/lib/utils';

export interface CertificateInfo {
  subjectCn: string;
  subjectFull: string;
  issuerCn: string;
  issuerFull: string;
  notBefore: string;
  notAfter: string;
  notAfterTimestamp: number;
  daysRemaining: number;
  status: 'valid' | 'expiring_soon' | 'expired';
  sans: string[];
  serialNumber: string;
  signatureAlgorithm: string;
  publicKeyInfo: string;
  sha256Fingerprint: string;
  sha1Fingerprint: string;
  isSelfSigned: boolean;
  isCa: boolean;
}

export interface CertParseResult {
  certificates: CertificateInfo[];
  rawPem: string;
}

export interface CertViewerProps {
  content: string;
  className?: string;
}

export const CertViewer = memo(function CertViewer({ content, className }: CertViewerProps) {
  const { t } = useTranslation();

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [data, setData] = useState<CertParseResult | null>(null);
  const [activeIndex, setActiveIndex] = useState(0);

  // 一键复制辅助状态
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const copyText = useCallback(async (text: string, key: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedKey(key);
      setTimeout(() => setCopiedKey(null), 1500);
    } catch (err) {
      console.warn('Copy failed', err);
    }
  }, []);

  useEffect(() => {
    setLoading(true);
    setError(null);
    invoke<CertParseResult>('parse_certificate', { content })
      .then((res) => {
        setData(res);
        setActiveIndex(0);
      })
      .catch((err) => {
        setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        setLoading(false);
      });
  }, [content]);

  if (loading) {
    return (
      <div className={cn('flex h-full w-full items-center justify-center bg-popover text-ui-sm text-foreground-subtle', className)}>
        <span>正在解析证书信息...</span>
      </div>
    );
  }

  if (error || !data || data.certificates.length === 0) {
    return (
      <div className={cn('flex h-full w-full flex-col items-center justify-center gap-2.5 bg-popover p-6 text-center', className)}>
        <AlertCircle className="size-6 text-warning" />
        <span className="text-ui-sm font-semibold text-foreground">无法以 X.509 结构解析此证书</span>
        <p className="max-w-md font-mono text-ui-xs text-foreground-subtle break-all">
          {error || '没有找到合法的证书数据'}
        </p>
      </div>
    );
  }

  const currentCert = data.certificates[activeIndex] || data.certificates[0];

  const statusConfig = {
    valid: {
      colorClass: 'text-terminal-green bg-terminal-green/10 border-terminal-green/30',
      icon: CheckCircle2,
      label: t('files.certValid', { days: currentCert.daysRemaining }),
    },
    expiring_soon: {
      colorClass: 'text-terminal-yellow bg-terminal-yellow/10 border-terminal-yellow/30',
      icon: AlertTriangle,
      label: t('files.certExpiring', { days: currentCert.daysRemaining }),
    },
    expired: {
      colorClass: 'text-terminal-red bg-terminal-red/10 border-terminal-red/30',
      icon: ShieldAlert,
      label: t('files.certExpired', { days: Math.abs(currentCert.daysRemaining) }),
    },
  }[currentCert.status] || {
    colorClass: 'text-foreground-subtle bg-surface border-border',
    icon: ShieldCheck,
    label: currentCert.status,
  };

  const StatusIcon = statusConfig.icon;

  return (
    <div className={cn('flex h-full w-full flex-col overflow-y-auto bg-popover p-5 select-text font-mono text-ui-xs', className)}>
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-4">
        {/* 1. 证书链导航 Tabs（如果是多级证书） */}
        {data.certificates.length > 1 ? (
          <div className="flex items-center gap-1.5 overflow-x-auto rounded-xl border border-border/70 bg-surface/40 p-1 select-none">
            <div className="flex shrink-0 items-center gap-1 px-2 text-foreground-subtle">
              <Layers className="size-3.5" />
              <span className="font-sans text-ui-xs font-medium">证书链:</span>
            </div>
            {data.certificates.map((cert, idx) => {
              const active = idx === activeIndex;
              return (
                <button
                  key={idx}
                  type="button"
                  onClick={() => setActiveIndex(idx)}
                  className={cn(
                    'flex shrink-0 items-center gap-1.5 rounded-lg px-2.5 py-1 text-ui-xs transition-colors',
                    active
                      ? 'bg-selected text-foreground font-semibold shadow-xs'
                      : 'text-foreground-subtle hover:bg-hover hover:text-foreground',
                  )}
                >
                  <span className="size-4 shrink-0 rounded-full bg-surface text-center leading-4 text-[10px]">
                    {idx + 1}
                  </span>
                  <span className="max-w-[180px] truncate">{cert.subjectCn}</span>
                  {cert.isCa ? (
                    <span className="rounded bg-surface px-1 text-[9px] text-foreground-subtlest uppercase">
                      CA
                    </span>
                  ) : null}
                </button>
              );
            })}
          </div>
        ) : null}

        {/* 2. 核心状态头部卡片 */}
        <div className="flex flex-col gap-3 rounded-2xl border border-border/80 bg-surface/30 p-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex min-w-0 items-center gap-3">
            <div className={cn('flex size-10 shrink-0 items-center justify-center rounded-xl border', statusConfig.colorClass)}>
              <StatusIcon className="size-5" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="truncate text-ui-base font-bold text-foreground">
                  {currentCert.subjectCn}
                </span>
                {currentCert.isSelfSigned ? (
                  <span className="shrink-0 rounded-full bg-terminal-yellow/15 px-2 py-0.5 text-[10px] font-semibold text-terminal-yellow">
                    {t('files.selfSigned')}
                  </span>
                ) : null}
              </div>
              <p className="truncate text-ui-xs text-foreground-subtle" title={currentCert.subjectFull}>
                {currentCert.subjectFull}
              </p>
            </div>
          </div>

          <div className={cn('flex shrink-0 items-center gap-1.5 rounded-xl border px-3 py-1.5 font-sans font-medium', statusConfig.colorClass)}>
            <span>{statusConfig.label}</span>
          </div>
        </div>

        {/* 3. 详细属性网格 */}
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
          {/* 有效期卡片 */}
          <div className="flex flex-col gap-2 rounded-xl border border-border/70 bg-surface/20 p-3.5">
            <div className="flex items-center gap-1.5 font-sans text-ui-xs font-semibold text-foreground select-none">
              <Calendar className="size-3.5 text-terminal-cyan" />
              <span>{t('files.certValidity')}</span>
            </div>
            <div className="space-y-1 text-ui-xs">
              <div className="flex justify-between">
                <span className="text-foreground-subtle">生效起始:</span>
                <span className="text-foreground">{currentCert.notBefore}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-foreground-subtle">到期截止:</span>
                <span className="font-semibold text-foreground">{currentCert.notAfter}</span>
              </div>
            </div>
          </div>

          {/* 颁发者卡片 */}
          <div className="flex flex-col gap-2 rounded-xl border border-border/70 bg-surface/20 p-3.5">
            <div className="flex items-center gap-1.5 font-sans text-ui-xs font-semibold text-foreground select-none">
              <ShieldCheck className="size-3.5 text-terminal-blue" />
              <span>{t('files.certIssuer')}</span>
            </div>
            <div className="min-w-0">
              <div className="text-foreground font-semibold truncate">{currentCert.issuerCn}</div>
              <div className="text-foreground-subtle truncate text-[11px]" title={currentCert.issuerFull}>
                {currentCert.issuerFull}
              </div>
            </div>
          </div>
        </div>

        {/* 4. 主题备用名称 (SANs / 覆盖域名) */}
        {currentCert.sans.length > 0 ? (
          <div className="flex flex-col gap-2 rounded-xl border border-border/70 bg-surface/20 p-3.5">
            <div className="flex items-center justify-between font-sans text-ui-xs font-semibold text-foreground select-none">
              <div className="flex items-center gap-1.5">
                <Globe className="size-3.5 text-terminal-green" />
                <span>涵盖域名与 IP ({currentCert.sans.length})</span>
              </div>
              <button
                type="button"
                onClick={() => void copyText(currentCert.sans.join('\n'), 'sans')}
                className="flex items-center gap-1 text-[11px] font-normal text-foreground-subtle hover:text-foreground"
              >
                {copiedKey === 'sans' ? <Check className="size-3 text-terminal-green" /> : <Copy className="size-3" />}
                <span>{copiedKey === 'sans' ? t('files.copied') : '复制全部域名'}</span>
              </button>
            </div>
            <div className="flex flex-wrap gap-1.5 pt-1">
              {currentCert.sans.map((san, idx) => (
                <span
                  key={idx}
                  onClick={() => void copyText(san, `san-${idx}`)}
                  className="cursor-pointer rounded-md border border-border/60 bg-surface px-2 py-0.5 text-ui-xs text-foreground transition-colors hover:border-border-hover hover:bg-hover select-none"
                  title="点击复制域名"
                >
                  {san}
                </span>
              ))}
            </div>
          </div>
        ) : null}

        {/* 5. 技术细节 & 指纹 */}
        <div className="flex flex-col gap-2.5 rounded-xl border border-border/70 bg-surface/20 p-3.5">
          <div className="flex items-center gap-1.5 font-sans text-ui-xs font-semibold text-foreground select-none">
            <KeyRound className="size-3.5 text-terminal-yellow" />
            <span>{t('files.certDetails')}</span>
          </div>

          <div className="divide-y divide-border/40 text-ui-xs">
            <div className="flex items-center justify-between py-1.5">
              <span className="text-foreground-subtle">序列号:</span>
              <span className="text-foreground">{currentCert.serialNumber}</span>
            </div>
            <div className="flex items-center justify-between py-1.5">
              <span className="text-foreground-subtle">签名算法:</span>
              <span className="text-foreground">{currentCert.signatureAlgorithm}</span>
            </div>
            <div className="flex items-center justify-between py-1.5">
              <span className="text-foreground-subtle">公钥算法:</span>
              <span className="text-foreground">{currentCert.publicKeyInfo}</span>
            </div>
            <div className="flex items-center justify-between py-1.5 gap-2">
              <span className="text-foreground-subtle shrink-0">SHA-256 指纹:</span>
              <div className="flex items-center gap-1 min-w-0">
                <span className="truncate text-foreground font-mono text-[11px]" title={currentCert.sha256Fingerprint}>
                  {currentCert.sha256Fingerprint}
                </span>
                <button
                  type="button"
                  onClick={() => void copyText(currentCert.sha256Fingerprint, 'sha256')}
                  className="shrink-0 p-1 text-foreground-subtle hover:text-foreground"
                  title="复制 SHA-256"
                >
                  {copiedKey === 'sha256' ? <Check className="size-3 text-terminal-green" /> : <Copy className="size-3" />}
                </button>
              </div>
            </div>
            <div className="flex items-center justify-between py-1.5 gap-2">
              <span className="text-foreground-subtle shrink-0">SHA-1 指纹:</span>
              <div className="flex items-center gap-1 min-w-0">
                <span className="truncate text-foreground font-mono text-[11px]" title={currentCert.sha1Fingerprint}>
                  {currentCert.sha1Fingerprint}
                </span>
                <button
                  type="button"
                  onClick={() => void copyText(currentCert.sha1Fingerprint, 'sha1')}
                  className="shrink-0 p-1 text-foreground-subtle hover:text-foreground"
                  title="复制 SHA-1"
                >
                  {copiedKey === 'sha1' ? <Check className="size-3 text-terminal-green" /> : <Copy className="size-3" />}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
});
