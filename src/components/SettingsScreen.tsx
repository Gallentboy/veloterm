import { Monitor, Moon, Sun, Trash2 } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import type { ComponentType } from 'react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/hosts/formFields';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { SettingOptionGroup, SettingsSection, SettingRow } from '@/components/settings';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/ConfirmDialog';
import { useAppVersion } from '@/hooks/useAppVersion';
import { AiProviderDialog } from '@/components/ai/AiProviderDialog';
import { deleteAiProvider, listAiProviders, type AiProvider } from '@/services/aiService';
import { SUPPORTED_LOCALES, type Locale } from '@/i18n';
import { cn } from '@/lib/utils';
import { useThemeContext, type Theme } from '@/theme/ThemeProvider';
import { SIDEBAR_NAV_ITEMS } from '@/components/WorkspaceSidebar';
import {
  getSidebarMenuConfig,
  normalizeSidebarOrder,
  setSidebarMenuConfig,
  type SidebarMenuConfig,
  SIDEBAR_VISIBILITY_EVENT,
} from '@/services/sidebarVisibility';
import { Check } from 'lucide-react';
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
  const [aiProviders, setAiProviders] = useState<AiProvider[]>([]);
  const [aiDialog, setAiDialog] = useState<{ mode: 'create' } | { mode: 'edit'; node: AiProvider } | null>(null);

  useEffect(() => {
    void listAiProviders().then(setAiProviders).catch(() => setAiProviders([]));
  }, []);
  const currentLocale: Locale = i18n.language.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en-US';
  const [logLevel, setLogLevelState] = useState<LogLevel>('info');
  const [logFilePath, setLogFilePath] = useState<string | null>(null);
  const [knownHosts, setKnownHosts] = useState<KnownHostEntry[]>([]);
  // 删除确认弹窗目标（工程约定：删除必须弹窗确认，严禁两次点击式或无确认直接删）
  const [pendingRemoveHost, setPendingRemoveHost] = useState<KnownHostEntry | null>(null);
  const [confirmClearAllOpen, setConfirmClearAllOpen] = useState(false);
  // 菜单展示：顺序 + 隐藏实时同步（本页拖拽/勾选与其他入口变更都经事件联动）
  const [menuConfig, setMenuConfig] = useState<SidebarMenuConfig>(() => getSidebarMenuConfig());
  useEffect(() => {
    const onChanged = () => setMenuConfig(getSidebarMenuConfig());
    window.addEventListener(SIDEBAR_VISIBILITY_EVENT, onChanged);
    return () => window.removeEventListener(SIDEBAR_VISIBILITY_EVENT, onChanged);
  }, []);
  const orderedMenuItems = normalizeSidebarOrder(
    menuConfig,
    SIDEBAR_NAV_ITEMS.map(({ page }) => page),
  )
    .map((page) => SIDEBAR_NAV_ITEMS.find((item) => item.page === page))
    .filter((item): item is (typeof SIDEBAR_NAV_ITEMS)[number] => item != null);
  const toggleMenuVisible = (page: string, visible: boolean) => {
    const hidden = new Set(menuConfig.hidden);
    if (visible) hidden.delete(page);
    else hidden.add(page);
    setSidebarMenuConfig({ order: orderedMenuItems.map(({ page: p }) => p), hidden: [...hidden] });
  };
  /** 拖拽排序：把 source 移到 target 当前位置（实时写入并广播，侧栏即时重排） */
  const reorderMenu = (source: string, target: string) => {
    const current: string[] = orderedMenuItems.map(({ page }) => page);
    const from = current.indexOf(source);
    const to = current.indexOf(target);
    if (from < 0 || to < 0 || from === to) return;
    current.splice(to, 0, ...current.splice(from, 1));
    setSidebarMenuConfig({ order: current, hidden: menuConfig.hidden });
  };

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

  const handleRemoveKnownHost = (entry: KnownHostEntry) => {
    setPendingRemoveHost(entry);
  };

  const handleClearAllKnownHosts = () => {
    setConfirmClearAllOpen(true);
  };

  const logLevelDescription = logFilePath
    ? `${t('settings.logLevelDesc')} (${t('settings.logFilePath', { path: logFilePath })})`
    : t('settings.logLevelDesc');

  return (
    <div
      // @container：双列切换按主内容区实际宽度判定（左侧栏可折叠/拖宽，
      // 媒体查询跟窗口宽度走、感知不到真实可用宽度，终端资源条同先例）
      className="@container flex h-full justify-center overflow-y-auto px-5 py-8 select-none"
      onClick={() => {
        setPendingRemoveHost(null);
        setConfirmClearAllOpen(false);
      }}
    >
      {/* 响应式网格：窄时单列（封顶 max-w-2xl 居中），≥820px 双列（封顶
          max-w-5xl，超宽屏两侧留白不摊薄）；高内容分组跨双列 */}
      <div className="grid h-fit w-full max-w-2xl grid-cols-1 gap-6 @[820px]:max-w-5xl @[820px]:grid-cols-2">
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
            <div className="w-32 shrink-0">
              <Select
                aria-label={t('settings.logLevel')}
                value={logLevel}
                onChange={(value) => void handleLogLevelChange(value as LogLevel)}
                options={LOG_LEVEL_OPTIONS}
              />
            </div>
          </SettingRow>
        </SettingsSection>

        <SettingsSection>
          <SettingRow title={t('ai.provider.title')} desc={t('settings.aiDesc')}>
            <div className="flex items-center gap-1.5">
              {aiProviders.length > 0 && (
                <span className="text-ui-xs text-foreground-subtlest select-none">
                  {aiProviders.map((p) => p.name).slice(0, 2).join(' · ')}
                  {aiProviders.length > 2 ? ` +${aiProviders.length - 2}` : ''}
                </span>
              )}
              <Button
                variant="outline"
                size="sm"
                onClick={() => setAiDialog({ mode: 'create' })}
              >
                {t('ai.provider.add')}
              </Button>
            </div>
          </SettingRow>
          {aiProviders.map((p) => (
            <SettingRow
              key={p.id}
              title={`${p.name}${p.isDefault ? ' · 默认' : ''}`}
              desc={`${p.providerType} · ${p.model} · ${t('ai.provider.keysCount', { count: p.apiKeys.length })}`}
            >
              <div className="flex items-center gap-1">
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setAiDialog({ mode: 'edit', node: p })}
                >
                  {t('common.edit')}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    void deleteAiProvider(p.id).then(() =>
                      listAiProviders().then(setAiProviders).catch(() => {}),
                    );
                  }}
                >
                  <Trash2 className="size-3.5 text-destructive" />
                </Button>
              </div>
            </SettingRow>
          ))}
        </SettingsSection>

        {/* 菜单展示：独立分组（宫格需要横向空间，跨双列；列数按列宽分级） */}
        <SettingsSection className="@[820px]:col-span-2">
          <div className="flex flex-col gap-3 px-4 py-3">
            <div className="flex flex-col gap-1">
              <span className="text-ui-base text-foreground select-none">{t('settings.menuVisibility')}</span>
              <span className="text-ui-sm text-foreground-subtle select-none">
                {t('settings.menuVisibilityDesc')}
              </span>
            </div>
            <MenuGrid
              items={orderedMenuItems.map(({ page, icon, navKey }) => ({
                page,
                icon,
                label: navKey,
              }))}
              hidden={menuConfig.hidden}
              onToggle={toggleMenuVisible}
              onReorder={reorderMenu}
            />
          </div>
        </SettingsSection>

        <SettingsSection className="@[820px]:col-span-2">
          <SettingRow title={t('settings.knownHosts')} desc={t('settings.knownHostsDesc')}>
            {knownHosts.length > 0 ? (
              <Button
                variant="outline"
                onClick={(e) => {
                  e.stopPropagation();
                  handleClearAllKnownHosts();
                }}
              >
                {t('settings.knownHostsClearAll')}
              </Button>
            ) : null}
          </SettingRow>

          {knownHosts.length > 0 ? (
            <div className="flex max-h-60 flex-col overflow-y-auto border-t border-border/50">
              {knownHosts.map((entry) => {
                return (
                  <div
                    key={entry.id}
                    onClick={(e) => e.stopPropagation()}
                    className={cn(
                      'group flex items-center justify-between gap-3 px-4 py-2 transition-colors hover:bg-surface-hover',
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
                    <ControlHintTooltip title={t('settings.knownHostsRemove')} side="left">
                      <button
                        type="button"
                        aria-label={t('settings.knownHostsRemove')}
                        onClick={() => handleRemoveKnownHost(entry)}
                        // 行 hover 才显示按钮；门控必须在按钮上——
                        // ControlHintTooltip 的 className 作用于弹出面板而非包裹层
                        className={cn(
                          'inline-flex size-6 shrink-0 items-center justify-center rounded-md transition-colors',
                          'text-foreground-subtle opacity-0 group-hover:opacity-100 focus-visible:opacity-100 hover:bg-hover hover:text-destructive',
                        )}
                      >
                        <Trash2 className="size-3.5" />
                      </button>
                    </ControlHintTooltip>
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

        <SettingsSection className="@[820px]:col-span-2">
          <SettingRow title={t('settings.about')} desc="VeloTerm">
            <span className="shrink-0 font-mono text-ui-base text-foreground-subtle">{version}</span>
          </SettingRow>
        </SettingsSection>
      </div>

      {aiDialog ? (
        <AiProviderDialog
          node={aiDialog.mode === 'edit' ? aiDialog.node : null}
          onClose={() => setAiDialog(null)}
          onSave={() => {
            setAiDialog(null);
            void listAiProviders().then(setAiProviders).catch(() => {});
          }}
        />
      ) : null}

      {/* 已知主机单条移除确认（工程约定：删除必须弹窗确认） */}
      <ConfirmDialog
        open={pendingRemoveHost !== null}
        title={t('common.confirmDeleteTitle')}
        description={t('settings.knownHostsRemoveDesc', {
          host: pendingRemoveHost ? `${pendingRemoveHost.host}:${pendingRemoveHost.port}` : '',
        })}
        detail={pendingRemoveHost?.key_base64}
        onConfirm={() => {
          const target = pendingRemoveHost;
          setPendingRemoveHost(null);
          if (target) {
            void removeKnownHost(target.id).then((removed) => {
              if (removed) {
                setKnownHosts((current) => current.filter((item) => item.id !== target.id));
              }
            });
          }
        }}
        onCancel={() => setPendingRemoveHost(null)}
      />

      {/* 清空全部已知主机确认 */}
      <ConfirmDialog
        open={confirmClearAllOpen}
        title={t('common.confirmDeleteTitle')}
        description={t('settings.knownHostsClearDesc', { count: knownHosts.length })}
        onConfirm={() => {
          setConfirmClearAllOpen(false);
          void clearKnownHosts().then(() => setKnownHosts([]));
        }}
        onCancel={() => setConfirmClearAllOpen(false)}
      />
    </div>
  );
}

/** 菜单宫格：卡片点击切换显示/隐藏（右上角小勾标记展示中的项），拖拽卡片到
 *  另一张上换位排序。排序用指针事件手写（pointerdown/move/up + 坐标命中）：
 *  wry 原生接管 DnD 后，页内 HTML5 拖拽的 drop 分发存在 dragend 与原生事件的
 *  时序竞争（实测拖了不换位），且默认 drop 光标带「+」号——纯 UI 重排不走
 *  该管线，指针事件行为完全确定。 */
function MenuGrid({
  items,
  hidden,
  onToggle,
  onReorder,
}: {
  items: Array<{ page: string; icon: LucideIcon | ComponentType<{ className?: string }>; label: string }>;
  hidden: string[];
  onToggle: (page: string, visible: boolean) => void;
  onReorder: (source: string, target: string) => void;
}) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  // 拖拽会话：source = 拖动的卡片，target = 当前悬停的卡片（换位高亮）
  const [drag, setDrag] = useState<{ source: string; target: string | null } | null>(null);
  const dragRef = useRef(drag);
  dragRef.current = drag;

  // 拖拽进行中：body 挂全局类，样式表用 !important 压过一切元素级 cursor
  useEffect(() => {
    if (!drag) return;
    document.body.classList.add('veloterm-pointer-dragging');
    return () => {
      document.body.classList.remove('veloterm-pointer-dragging');
    };
  }, [drag]);

  const hitCard = (x: number, y: number): string | null => {
    const container = containerRef.current;
    if (!container) return null;
    for (const el of Array.from(container.querySelectorAll<HTMLElement>('[data-menu-page]'))) {
      const rect = el.getBoundingClientRect();
      if (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom) {
        return el.dataset.menuPage ?? null;
      }
    }
    return null;
  };

  const handlePointerDown = (page: string, event: React.PointerEvent) => {
    if (event.button !== 0) return;
    const startX = event.clientX;
    const startY = event.clientY;
    let started = false;
    const onMove = (move: PointerEvent) => {
      if (!started) {
        if (Math.hypot(move.clientX - startX, move.clientY - startY) <= 5) return;
        started = true;
      }
      const target = hitCard(move.clientX, move.clientY);
      setDrag((current) =>
        current?.source === page && current.target === target ? current : { source: page, target },
      );
    };
    const onUp = () => {
      window.removeEventListener('pointermove', onMove);
      const current = dragRef.current;
      // 未发生拖动 = 点击：切换显示/隐藏；拖动且悬停有效 = 换位
      if (!started) {
        onToggle(page, hidden.includes(page));
      } else if (current?.source === page && current.target && current.target !== page) {
        onReorder(page, current.target);
      }
      setDrag(null);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp, { once: true });
  };

  return (
    <div
      ref={containerRef}
      className="grid grid-cols-2 gap-2 @[480px]:grid-cols-3 @[760px]:grid-cols-4"
    >
      {items.map(({ page, icon: Icon, label }) => {
        const visible = !hidden.includes(page);
        const isDragSource = drag?.source === page;
        const isDropTarget = drag?.target === page && drag.target !== drag.source;
        return (
          <div
            key={page}
            data-menu-page={page}
            onPointerDown={(event) => handlePointerDown(page, event)}
            role="checkbox"
            aria-checked={visible}
            className={cn(
              'relative flex cursor-grab select-none touch-none items-center gap-2 rounded-lg border px-3 py-2 transition-colors',
              visible
                ? 'border-border bg-surface/40 text-foreground hover:border-border-hover'
                : 'border-border/40 text-foreground-subtlest opacity-60 hover:opacity-80',
              isDragSource && 'opacity-40',
              isDropTarget && 'border-primary',
            )}
          >
            <Icon className="size-4 shrink-0" />
            <span className="min-w-0 truncate text-ui-sm">{t(label)}</span>
            {visible ? (
              <span className="absolute right-1.5 top-1.5 flex size-4 items-center justify-center rounded-full bg-primary text-primary-foreground">
                <Check className="size-2.5" />
              </span>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
