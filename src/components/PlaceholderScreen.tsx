import type { LucideIcon } from 'lucide-react';
import { useTranslation } from 'react-i18next';

interface PlaceholderScreenProps {
  icon: LucideIcon;
  titleKey: string;
  hintKey: string;
}

/** 占位页：功能核心未就绪时的统一空状态（图标 + 标题 + 提示）。 */
export function PlaceholderScreen({ icon: Icon, titleKey, hintKey }: PlaceholderScreenProps) {
  const { t } = useTranslation();
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 select-none">
      <div className="flex size-12 items-center justify-center rounded-xl border border-border bg-surface">
        <Icon className="size-6 text-foreground-subtle" />
      </div>
      <span className="text-ui-base text-foreground">{t(titleKey)}</span>
      <span className="text-ui-sm text-foreground-subtlest">{t(hintKey)}</span>
    </div>
  );
}
