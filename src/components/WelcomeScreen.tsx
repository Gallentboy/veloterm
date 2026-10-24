import {
  ChevronRight,
  Database,
  FileSearch,
  Package,
  ShieldCheck,
  ArrowLeftRight,
  SquareTerminal,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { PRIMARY_MODIFIER_LABEL } from '@/lib/keyboard';
import { cn } from '@/lib/utils';

function ShortcutHint({
  keys,
  label,
  onClick,
}: {
  /** 快捷键（可空：无键位的入口右侧给右箭头引导） */
  keys?: string[];
  label: string;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={!onClick}
      className="flex w-full items-center justify-between gap-3 whitespace-nowrap rounded-md px-3 py-2.5 text-left text-ui-base text-foreground-subtle transition-colors hover:bg-surface disabled:cursor-default disabled:opacity-60"
    >
      <span>{label}</span>
      <span className="flex shrink-0 items-center gap-1">
        {keys ? (
          keys.map((key) => (
            <kbd
              key={key}
              className="rounded border border-border bg-surface px-1.5 py-0.5 font-mono text-ui-sm text-foreground"
            >
              {key}
            </kbd>
          ))
        ) : (
          <ChevronRight className="size-3.5 text-foreground-subtlest" />
        )}
      </span>
    </button>
  );
}

/** 功能矩阵卡片（纯展示：图标 + 标题 + 一行要点） */
function FeatureCard({
  icon,
  title,
  desc,
}: {
  icon: React.ReactNode;
  title: string;
  desc: string;
}) {
  return (
    <div className="flex items-start gap-2.5 rounded-lg border border-border bg-surface/40 p-3">
      <span className="mt-0.5 shrink-0 text-foreground-subtle">{icon}</span>
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="text-ui-sm font-medium text-foreground">{title}</span>
        <span className="text-ui-xs leading-relaxed text-foreground-subtle">{desc}</span>
      </span>
    </div>
  );
}

/** 主页欢迎内容（空状态样式参考 ZCode ChatEmptyState / WelcomeScreen）。 */
export function WelcomeScreen({
  onNewTerminal,
  onOpenSettings,
  onNavigateHosts,
  onOpenCommandPalette,
  onOpenManual,
}: {
  onNewTerminal?: () => void;
  onOpenSettings?: () => void;
  /** 去主机页连接服务器（含 SSH 多跳/中继） */
  onNavigateHosts?: () => void;
  onOpenCommandPalette?: () => void;
  /** 打开使用手册（F1 同路径） */
  onOpenManual?: () => void;
}) {
  const { t } = useTranslation();

  return (
    <div className="flex h-full flex-col items-center justify-center gap-7 overflow-y-auto px-8 py-10 scrollbar-slim select-none">
      <div className="flex flex-col items-center gap-2 text-center">
        <div className="mb-1 flex size-16 items-center justify-center rounded-2xl border border-border bg-surface">
          <SquareTerminal className="size-8 text-foreground" strokeWidth={1.5} />
        </div>
        <h1 className="text-ui-xl font-semibold tracking-tight text-foreground">VeloTerm</h1>
        <p className="text-ui-base text-foreground-subtle">{t('welcome.subtitle')}</p>
      </div>

      <div className={cn('w-full max-w-md overflow-hidden rounded-lg border border-border')}>
        <div className="border-b border-border/50 bg-surface px-3.5 py-2 text-ui-sm text-foreground-subtlest">
          {t('welcome.getStarted')}
        </div>
        <div className="flex flex-col p-1">
          <ShortcutHint
            keys={[PRIMARY_MODIFIER_LABEL, 'N']}
            label={t('welcome.newTerminal')}
            onClick={onNewTerminal}
          />
          <ShortcutHint
            label={t('welcome.connectHosts')}
            onClick={onNavigateHosts}
          />
          <ShortcutHint
            keys={[PRIMARY_MODIFIER_LABEL, 'K']}
            label={t('welcome.commandPalette')}
            onClick={onOpenCommandPalette}
          />
          <ShortcutHint keys={['F1']} label={t('welcome.manual')} onClick={onOpenManual} />
          <ShortcutHint
            keys={[PRIMARY_MODIFIER_LABEL, ',']}
            label={t('welcome.openSettings')}
            onClick={onOpenSettings}
          />
        </div>
      </div>

      <div className="flex w-full max-w-2xl flex-col gap-2.5">
        <div className="px-1 text-ui-sm text-foreground-subtlest">{t('welcome.featuresTitle')}</div>
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          <FeatureCard
            icon={<SquareTerminal className="size-4" />}
            title={t('welcome.featTerminal')}
            desc={t('welcome.featTerminalDesc')}
          />
          <FeatureCard
            icon={<ArrowLeftRight className="size-4" />}
            title={t('welcome.featSftp')}
            desc={t('welcome.featSftpDesc')}
          />
          <FeatureCard
            icon={<Database className="size-4" />}
            title={t('welcome.featDb')}
            desc={t('welcome.featDbDesc')}
          />
          <FeatureCard
            icon={<FileSearch className="size-4" />}
            title={t('welcome.featPreview')}
            desc={t('welcome.featPreviewDesc')}
          />
          <FeatureCard
            icon={<ShieldCheck className="size-4" />}
            title={t('welcome.featSafety')}
            desc={t('welcome.featSafetyDesc')}
          />
          <FeatureCard
            icon={<Package className="size-4" />}
            title={t('welcome.featTools')}
            desc={t('welcome.featToolsDesc')}
          />
        </div>
      </div>
    </div>
  );
}
