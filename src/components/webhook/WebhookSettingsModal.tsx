import { Globe, Lock, Server, Shield } from 'lucide-react';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { Spinner } from '@/components/ui/spinner';
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

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
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
      className="max-w-lg"
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
              onClick={handleSave}
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
      <form onSubmit={handleSave} className="space-y-4 py-1">
        {/* 开关 */}
        <div className="flex items-center justify-between p-3 rounded-lg border border-border/70 bg-surface/40">
          <div className="space-y-0.5">
            <label className="text-ui-sm font-medium text-foreground cursor-pointer" htmlFor="wh-enabled-toggle">
              {t('webhook.enableService')}
            </label>
            <p className="text-ui-xs text-foreground-subtle">
              {t('webhook.enableServiceDesc')}
            </p>
          </div>
          <input
            id="wh-enabled-toggle"
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
            className="size-4.5 rounded border-border bg-surface accent-terminal-blue cursor-pointer"
          />
        </div>

        {/* 监听网络与端口 */}
        <div className="grid grid-cols-2 gap-3">
          <div className="space-y-1.5">
            <label className="flex items-center gap-1.5 text-ui-xs font-medium text-foreground">
              <Globe className="size-3.5 text-foreground-subtle" />
              <span>{t('webhook.hostIp')}</span>
            </label>
            <select
              value={bindIp}
              onChange={(e) => setBindIp(e.target.value)}
              className="w-full h-8 rounded-md border border-border/80 bg-surface px-2.5 text-ui-xs text-foreground focus:outline-none focus:ring-1 focus:ring-border"
            >
              <option value="127.0.0.1">{t('webhook.hostIpLocal')}</option>
              <option value="0.0.0.0">{t('webhook.hostIpAll')}</option>
            </select>
          </div>

          <div className="space-y-1.5">
            <label className="flex items-center gap-1.5 text-ui-xs font-medium text-foreground">
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
              className="w-full h-8 rounded-md border border-border/80 bg-surface px-2.5 text-ui-xs text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-border"
              required
            />
          </div>
        </div>

        {/* 全局 Secret */}
        <div className="space-y-1.5">
          <label className="flex items-center gap-1.5 text-ui-xs font-medium text-foreground">
            <Lock className="size-3.5 text-foreground-subtle" />
            <span>{t('webhook.globalSecret')}</span>
          </label>
          <input
            type="password"
            value={globalSecret}
            onChange={(e) => setGlobalSecret(e.target.value)}
            placeholder="X-Velo-Token: secret_key"
            className="w-full h-8 rounded-md border border-border/80 bg-surface px-2.5 text-ui-xs text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-border"
          />
          <p className="text-[11px] text-foreground-subtler">
            {t('webhook.globalSecretDesc')}
          </p>
        </div>

        {/* 请求体限制 */}
        <div className="space-y-1.5">
          <label className="flex items-center gap-1.5 text-ui-xs font-medium text-foreground">
            <Shield className="size-3.5 text-foreground-subtle" />
            <span>{t('webhook.maxBodySize')}</span>
          </label>
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={1}
              max={50}
              value={maxBodyMb}
              onChange={(e) => setMaxBodyMb(Number(e.target.value))}
              className="w-24 h-8 rounded-md border border-border/80 bg-surface px-2.5 text-ui-xs text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-border"
            />
            <span className="text-ui-xs text-foreground-subtle">MB</span>
          </div>
        </div>
      </form>
    </Modal>
  );
}
