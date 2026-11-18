import { Globe, Lock, Server, Shield } from 'lucide-react';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { SelectMenu, type SelectMenuOption } from '@/components/ui/SelectMenu';
import { Spinner } from '@/components/ui/spinner';
import { cn } from '@/lib/utils';
import {
  webhookUpdateSettings,
  type WebhookSettings,
} from '@/services/webhookService';

interface WebhookSettingsModalProps {
  settings: WebhookSettings;
  isOpen: boolean;
  onClose: () => void;
  onSaved: () => void;
}

/** 语义化 Switch 开关组件（对齐设计系统规范，彻底禁用浏览器原生 checkbox） */
function Switch({
  checked,
  disabled,
  onChange,
  'aria-label': ariaLabel,
}: {
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
  'aria-label'?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        'relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border transition-colors focus:outline-none focus:ring-1 focus:ring-primary/40',
        checked ? 'border-transparent bg-primary' : 'border-border bg-input',
        disabled && 'cursor-not-allowed opacity-50',
      )}
    >
      <span
        className={cn(
          'inline-block size-4 rounded-full shadow-sm transition-transform duration-150',
          checked ? 'translate-x-4.5 bg-primary-foreground' : 'translate-x-0.5 bg-foreground-subtle',
        )}
      />
    </button>
  );
}

export function WebhookSettingsModal({
  settings: initialSettings,
  isOpen,
  onClose,
  onSaved,
}: WebhookSettingsModalProps) {
  const { t } = useTranslation();

  const [enabled, setEnabled] = useState(initialSettings.enabled);
  const [bindIp, setBindIp] = useState(initialSettings.bindIp || '127.0.0.1');
  const [bindPort, setBindPort] = useState(initialSettings.bindPort || 9876);
  const [globalSecret, setGlobalSecret] = useState(initialSettings.globalSecret || '');
  const [maxBodyMb, setMaxBodyMb] = useState(
    Math.round((initialSettings.maxBodyBytes || 2 * 1024 * 1024) / (1024 * 1024)),
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!isOpen) return null;

  const ipOptions: SelectMenuOption[] = [
    { value: '127.0.0.1', label: t('webhook.hostIpLocal', { defaultValue: '127.0.0.1 (仅本机安全访问)' }) },
    { value: '0.0.0.0', label: t('webhook.hostIpAll', { defaultValue: '0.0.0.0 (允许局域网/外部网络访问)' }) },
  ];

  const handleSave = async (e?: React.FormEvent) => {
    e?.preventDefault();
    setSaving(true);
    setError(null);

    const portNum = Number(bindPort);
    if (isNaN(portNum) || portNum < 1 || portNum > 65535) {
      setError(t('webhook.portInvalid', { defaultValue: '端口必须在 1 ~ 65535 之间' }));
      setSaving(false);
      return;
    }

    const payload: WebhookSettings = {
      enabled,
      bindIp,
      bindPort: portNum,
      globalSecret: globalSecret.trim() ? globalSecret.trim() : null,
      maxBodyBytes: Math.max(1, maxBodyMb) * 1024 * 1024,
      updatedAt: Date.now(),
    };

    try {
      await webhookUpdateSettings(payload);
      onSaved();
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      title={t('webhook.settingsTitle')}
      onClose={onClose}
      className="max-w-xl"
      footer={
        <div className="flex items-center justify-between w-full">
          <div className="text-ui-xs text-destructive truncate mr-2">{error}</div>
          <div className="flex items-center gap-2 shrink-0">
            <Button type="button" variant="ghost" onClick={onClose} disabled={saving}>
              {t('common.cancel', { defaultValue: '取消' })}
            </Button>
            <Button
              type="button"
              variant="default"
              onClick={() => void handleSave()}
              disabled={saving}
              className="gap-1.5"
            >
              {saving && <Spinner className="size-3.5" />}
              <span>{t('webhook.saveSettings')}</span>
            </Button>
          </div>
        </div>
      }
    >
      <form onSubmit={(e) => void handleSave(e)} className="space-y-5 py-2">
        {/* 服务开关卡片 */}
        <div className="flex items-center justify-between p-4 rounded-xl border border-border/80 bg-surface/50 shadow-xs">
          <div className="space-y-1 pr-4">
            <span className="text-ui-sm font-semibold text-foreground block">
              {t('webhook.enableService')}
            </span>
            <p className="text-ui-xs text-foreground-subtle leading-relaxed">
              {t('webhook.enableServiceDesc')}
            </p>
          </div>
          <Switch
            checked={enabled}
            onChange={setEnabled}
            aria-label={t('webhook.enableService')}
          />
        </div>

        {/* 网络与端口 */}
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
          <div className="space-y-2">
            <label className="flex items-center gap-1.5 text-ui-xs font-medium text-foreground-subtle">
              <Globe className="size-3.5 text-foreground-subtle" />
              <span>{t('webhook.hostIp')}</span>
            </label>
            <SelectMenu
              value={bindIp}
              options={ipOptions}
              onChange={setBindIp}
              triggerClassName="h-9 text-ui-sm"
            />
          </div>

          <div className="space-y-2">
            <label className="flex items-center gap-1.5 text-ui-xs font-medium text-foreground-subtle">
              <Server className="size-3.5 text-foreground-subtle" />
              <span>{t('webhook.port')}</span>
            </label>
            <input
              type="number"
              min={1}
              max={65535}
              value={bindPort}
              onChange={(e) => setBindPort(Number(e.target.value))}
              placeholder="9876"
              className="w-full h-9 rounded-md border border-border bg-input px-3 text-ui-sm text-foreground font-mono transition-colors focus:border-border-hover focus:outline-none"
              required
            />
          </div>
        </div>

        {/* 全局 Secret */}
        <div className="space-y-2">
          <label className="flex items-center gap-1.5 text-ui-xs font-medium text-foreground-subtle">
            <Lock className="size-3.5 text-foreground-subtle" />
            <span>{t('webhook.globalSecret')}</span>
          </label>
          <input
            type="password"
            value={globalSecret}
            onChange={(e) => setGlobalSecret(e.target.value)}
            placeholder="X-Velo-Token: secret_key"
            className="w-full h-9 rounded-md border border-border bg-input px-3 text-ui-sm text-foreground font-mono transition-colors focus:border-border-hover focus:outline-none"
          />
          <p className="text-ui-xs text-foreground-subtler leading-relaxed">
            {t('webhook.globalSecretDesc')}
          </p>
        </div>

        {/* 请求体限制 */}
        <div className="space-y-2">
          <label className="flex items-center gap-1.5 text-ui-xs font-medium text-foreground-subtle">
            <Shield className="size-3.5 text-foreground-subtle" />
            <span>{t('webhook.maxBodySize')}</span>
          </label>
          <div className="flex items-center gap-2.5">
            <input
              type="number"
              min={1}
              max={50}
              value={maxBodyMb}
              onChange={(e) => setMaxBodyMb(Number(e.target.value))}
              className="w-28 h-9 rounded-md border border-border bg-input px-3 text-ui-sm text-foreground font-mono transition-colors focus:border-border-hover focus:outline-none"
            />
            <span className="text-ui-sm text-foreground-subtle font-medium">MB</span>
          </div>
        </div>
      </form>
    </Modal>
  );
}
