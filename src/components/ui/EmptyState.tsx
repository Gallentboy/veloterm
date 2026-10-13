import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

export interface EmptyStateAction {
  label: string;
  icon?: LucideIcon;
  shortcut?: string;
  variant?: 'default' | 'outline' | 'secondary' | 'ghost';
  disabled?: boolean;
  onClick?: () => void;
}

export interface EmptyStateProps {
  icon: LucideIcon;
  title: string;
  description: string;
  primaryAction?: EmptyStateAction;
  secondaryAction?: EmptyStateAction;
  children?: ReactNode;
  className?: string;
}

/**
 * 通用空白引导状态组件（ZCode EmptyState 统一视觉规范）：
 * - 图标容器：56x56 圆角卡片，surface 浅底与细边框
 * - 标题与副标题：字号阶梯清晰，文本居中
 * - 操作按钮组：统一高度 h-8、圆角 rounded-lg，支持快捷键徽标，主副操作风格统一
 */
export function EmptyState({
  icon: Icon,
  title,
  description,
  primaryAction,
  secondaryAction,
  children,
  className,
}: EmptyStateProps) {
  const PrimaryIcon = primaryAction?.icon;
  const SecondaryIcon = secondaryAction?.icon;

  return (
    <div
      className={cn(
        'flex h-full min-h-0 flex-1 flex-col items-center justify-center p-6 text-center select-none bg-background',
        className,
      )}
    >
      <div className="flex size-14 items-center justify-center rounded-2xl border border-border/60 bg-surface/50 shadow-xs">
        <Icon className="size-7 text-foreground-subtle" strokeWidth={1.5} />
      </div>

      <h3 className="mt-4 text-base font-medium tracking-tight text-foreground">
        {title}
      </h3>
      <p className="mt-1.5 max-w-sm text-ui-xs text-foreground-subtle leading-relaxed">
        {description}
      </p>

      {(primaryAction || secondaryAction || children) && (
        <div className="mt-6 flex flex-wrap items-center justify-center gap-3">
          {primaryAction && (
            <Button
              type="button"
              variant={primaryAction.variant ?? 'default'}
              disabled={primaryAction.disabled}
              onClick={primaryAction.onClick}
              className="h-8 gap-2 rounded-lg px-3.5 text-ui-sm font-medium shadow-xs transition-all active:scale-[0.98]"
            >
              {PrimaryIcon && <PrimaryIcon className="size-4 shrink-0" />}
              <span>{primaryAction.label}</span>
              {primaryAction.shortcut && (
                <kbd className="ml-0.5 rounded bg-primary-foreground/20 px-1.5 py-0.5 font-mono text-[10px] font-medium text-primary-foreground select-none">
                  {primaryAction.shortcut}
                </kbd>
              )}
            </Button>
          )}

          {secondaryAction && (
            <Button
              type="button"
              variant={secondaryAction.variant ?? 'outline'}
              disabled={secondaryAction.disabled}
              onClick={secondaryAction.onClick}
              className="h-8 gap-1.5 rounded-lg px-3 text-ui-sm font-medium border-border/80 bg-surface/40 hover:bg-hover active:bg-hover/80 text-foreground shadow-2xs transition-all"
            >
              {SecondaryIcon && (
                <SecondaryIcon className="size-4 shrink-0 text-foreground-subtle" />
              )}
              <span>{secondaryAction.label}</span>
            </Button>
          )}

          {children}
        </div>
      )}
    </div>
  );
}
