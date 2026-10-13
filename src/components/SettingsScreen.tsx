import { ChevronDown, Monitor, Moon, Sun, Trash2 } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/hosts/formFields';
import { SettingOptionGroup, SettingsSection, SettingRow } from '@/components/settings';
import { Button } from '@/components/ui/button';
import { useAppVersion } from '@/hooks/useAppVersion';
import { SUPPORTED_LOCALES, type Locale } from '@/i18n';
import { cn } from '@/lib/utils';
import { useThemeContext, type Theme } from '@/theme/ThemeProvider';
import {
  clearKnownHosts,
  listKnownHosts,
  removeKnownHost,
  type KnownHostEntry,
} from '@/utils/knownHosts';
import {
  getCurrentLogLevel,
  getLogFilePath,
  logger,
  setLogLevel,
  type LogLevel,
} from '@/utils/logger';

const THEME_OPTIONS: Array<{ value: Theme; icon: LucideIcon; labelKey: string }> = [
  { value: 'system', icon: Monitor, labelKey: 'theme.system' },
  { value: 'light', icon: Sun, labelKey: 'theme.light' },
  { value: 'dark', icon: Moon, labelKey: 'theme.dark' },
];

const LOCALE_LABELS: Record<Locale, string> = {
  'zh-CN': '简体中文',
  'en-US': 'English',
};

const LOG_LEVEL_OPTIONS: Array<{ value: LogLevel; label: string }> = [
  { value: 'trace', label: 'Trace' },
  { value: 'debug', label: 'Debug' },
  { value: 'info', label: 'Info' },
  { value: 'warn', label: 'Warn' },
  { value: 'error', label: 'Error' },
];

export function SettingsScreen() {
  const { t, i18n } = useTranslation();
  const { theme, setTheme } = useThemeContext();
  const { version } = useAppVersion();
  const currentLocale: Locale = i18n.language.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en-US';
  const [logLevel, setLogLevelState] = useState<LogLevel>('info');
  const [logFilePath, setLogFilePath] = useState<string | null>(null);
  const [knownHosts, setKnownHosts] = useState<KnownHostEntry[]>([]);
  const [pendingRemoveHostId, setPendingRemoveHostId] = useState<number | null>(null);
  const [confirmClearAll, setConfirmClearAll] = useState(false);

  useEffect(() => {
    let active = true;
    void Promise.all([getCurrentLogLevel(), getLogFilePath(), listKnownHosts()]).then(
      ([level, filePath, entries]) => {
        if (active) {
          setLogLevelState(level);
          setLogFilePath(filePath);
          setKnownHosts(entries);
        }
      },
    );
    return () => {
      active = false;
    };
  }, []);

  const handleLogLevelChange = async (nextLevel: LogLevel) => {
    setLogLevelState(nextLevel);
    await setLogLevel(nextLevel);
    await logger.debug(`[Frontend Logger] debug verification log after setting level=${nextLevel}`);
    await logger.info(`[Frontend Logger] info verification log after setting level=${nextLevel}`);
  };

  const handleRemoveKnownHost = async (id: number) => {
    if (pendingRemoveHostId !== id) {
      setPendingRemoveHostId(id);
      setConfirmClearAll(false);
      return;
    }
    setPendingRemoveHostId(null);
    const removed = await removeKnownHost(id);
    if (removed) {
      setKnownHosts((current) => current.filter((item) => item.id !== id));
    }
  };

  const handleClearAllKnownHosts = async () => {
    if (!confirmClearAll) {
      setConfirmClearAll(true);
      setPendingRemoveHostId(null);
      return;
    }
    setConfirmClearAll(false);
    await clearKnownHosts();
    setKnownHosts([]);
  };

  const logLevelDescription = logFilePath
    ? `${t('settings.logLevelDesc')} (${t('settings.logFilePath', { path: logFilePath })})`
    : t('settings.logLevelDesc');

  return (
    <div
      className="flex h-full justify-center overflow-y-auto px-5 py-8 select-none"
      onClick={() => {
        setPendingRemoveHostId(null);
        setConfirmClearAll(false);
      }}
    >
      <div className="flex h-fit w-full max-w-xl flex-col gap-6">
        <SettingsSection>
          <SettingRow title={t('theme.label')} desc={t('settings.appearanceDesc')}>
            <SettingOptionGroup
              value={theme}
              onChange={setTheme}
              options={THEME_OPTIONS.map(({ value, icon, labelKey }) => ({
                value,
                icon,
                label: t(labelKey),
              }))}
            />
          </SettingRow>
          <SettingRow title={t('language.label')} desc={t('settings.languageDesc')}>
            <SettingOptionGroup
              value={currentLocale}
              onChange={(locale) => void i18n.changeLanguage(locale)}
              options={SUPPORTED_LOCALES.map((locale) => ({
                value: locale,
                label: LOCALE_LABELS[locale],
              }))}
            />
          </SettingRow>
          <SettingRow title={t('settings.logLevel')} desc={logLevelDescription}>
            <div className="relative w-32 shrink-0">
              <Select
                aria-label={t('settings.logLevel')}
                value={logLevel}
                onChange={(e) => void handleLogLevelChange(e.target.value as LogLevel)}
              >
                {LOG_LEVEL_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </Select>
              <ChevronDown className="pointer-events-none absolute right-2 top-1/2 size-3.5 -translate-y-1/2 text-foreground-subtle" />
            </div>
          </SettingRow>
        </SettingsSection>

        <SettingsSection>
          <SettingRow title={t('settings.knownHosts')} desc={t('settings.knownHostsDesc')}>
            {knownHosts.length > 0 ? (
              <Button
                variant={confirmClearAll ? 'destructive' : 'outline'}
                onClick={(e) => {
                  e.stopPropagation();
                  void handleClearAllKnownHosts();
                }}
              >
                {confirmClearAll ? t('settings.knownHostsConfirmClear') : t('settings.knownHostsClearAll')}
              </Button>
            ) : null}
          </SettingRow>

          {knownHosts.length > 0 ? (
            <div className="flex max-h-60 flex-col overflow-y-auto border-t border-border/50">
              {knownHosts.map((entry) => {
                const isPendingDelete = pendingRemoveHostId === entry.id;
                return (
                  <div
                    key={entry.id}
                    onClick={(e) => e.stopPropagation()}
                    className={cn(
                      'group flex items-center justify-between gap-3 px-4 py-2 transition-colors hover:bg-surface-hover',
                      isPendingDelete && 'bg-destructive/5',
                    )}
                  >
                    <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <div className="flex items-center gap-2">
                        <span className="truncate font-mono text-ui-sm text-foreground">
                          {entry.host}:{entry.port}
                        </span>
                        <span className="shrink-0 rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle">
                          {entry.key_type}
                        </span>
                      </div>
                      <span className="truncate font-mono text-ui-xs text-foreground-subtlest">
                        {entry.key_base64}
                      </span>
                    </div>
                    <button
                      type="button"
                      title={
                        isPendingDelete
                          ? t('settings.knownHostsConfirmRemove')
                          : t('settings.knownHostsRemove')
                      }
                      aria-label={
                        isPendingDelete
                          ? t('settings.knownHostsConfirmRemove')
                          : t('settings.knownHostsRemove')
                      }
                      onClick={() => void handleRemoveKnownHost(entry.id)}
                      className={cn(
                        'inline-flex size-6 shrink-0 items-center justify-center rounded-md transition-colors',
                        isPendingDelete
                          ? 'text-destructive hover:bg-destructive/10'
                          : 'text-foreground-subtle opacity-0 group-hover:opacity-100 hover:bg-hover hover:text-destructive',
                      )}
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="border-t border-border/50 px-4 py-3 text-ui-sm text-foreground-subtlest">
              {t('settings.knownHostsEmpty')}
            </div>
          )}
        </SettingsSection>

        <SettingsSection>
          <SettingRow title={t('settings.about')} desc="VeloTerm">
            <span className="shrink-0 font-mono text-ui-base text-foreground-subtle">{version}</span>
          </SettingRow>
        </SettingsSection>
      </div>
    </div>
  );
}
