import { ChevronRight, SquareTerminal } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { PRIMARY_MODIFIER_LABEL } from '@/lib/keyboard';
import { cn } from '@/lib/utils';

function ShortcutHint({
  keys,
  label,
  onClick,
}: {
  keys: string[];
  label: string;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center justify-between rounded-md px-3 py-2.5 text-left text-ui-base text-foreground-subtle transition-colors hover:bg-surface"
    >
      <span>{label}</span>
      <span className="flex items-center gap-1">
        {keys.map((key) => (
          <kbd
            key={key}
            className="rounded border border-border bg-surface px-1.5 py-0.5 font-mono text-ui-sm text-foreground"
          >
            {key}
          </kbd>
        ))}
      </span>
    </button>
  );
}

/** 主页欢迎内容（空状态样式参考 ZCode ChatEmptyState / WelcomeScreen）。 */
export function WelcomeScreen({
  onNewTerminal,
  onOpenSettings,
}: {
  onNewTerminal?: () => void;
  onOpenSettings?: () => void;
}) {
  const { t } = useTranslation();

  return (
    <div className="flex h-full flex-col items-center justify-center gap-7 px-8 py-10 select-none">
      <div className="flex size-16 items-center justify-center rounded-2xl border border-border bg-surface">
        <SquareTerminal className="size-8 text-foreground" strokeWidth={1.5} />
      </div>

      <div className="flex flex-col items-center gap-2 text-center">
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
            keys={[PRIMARY_MODIFIER_LABEL, ',']}
            label={t('welcome.openSettings')}
            onClick={onOpenSettings}
          />
        </div>
      </div>

      <p className="flex items-center gap-1.5 text-ui-sm text-foreground-subtlest">
        <ChevronRight className="size-3.5" />
        {t('welcome.pending')}
      </p>
    </div>
  );
}
