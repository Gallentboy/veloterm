import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';

/**
 * 设置页通用套件（排版参考 ZCode SettingsPage）：
 * 字号/颜色/粗细全部内置在这层，新增设置项只填内容，不需要逐个调样式。
 * 体系：页面标题 text-2xl semibold；行标题 text-ui-base；行描述 text-ui-sm subtle；
 * 分组卡片 rounded-lg + border；控件走 SettingOptionGroup。
 */

/** 页面大标题（ZCode: text-2xl font-semibold tracking-tight text-foreground） */
export function SettingsPageTitle({ children }: { children: ReactNode }) {
  return (
    <h1 className="text-2xl font-semibold tracking-tight text-foreground select-none">{children}</h1>
  );
}

/** 设置分组卡片 */
export function SettingsSection({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <section className={cn('overflow-hidden rounded-lg border border-border', className)}>
      {children}
    </section>
  );
}

/** 设置行：左侧标题 + 描述（可选），右侧控件槽；窄窗口自动上下堆叠（响应式内置）；
 *  行间不加分隔线（内容区无分割线，仅 header 到内容有）。 */
export function SettingRow({
  title,
  desc,
  children,
}: {
  title: ReactNode;
  desc?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-start gap-3 px-4 py-3 sm:flex-row sm:items-center sm:justify-between sm:gap-6">
      <div className="flex min-w-0 flex-col gap-1">
        <span className="text-ui-base text-foreground select-none">{title}</span>
        {desc ? <span className="text-ui-sm text-foreground-subtle">{desc}</span> : null}
      </div>
      {children}
    </div>
  );
}

/** 设置行右侧的单选分段控件（当前值高亮，选项带可选图标） */
export function SettingOptionGroup<T extends string>({
  options,
  value,
  onChange,
}: {
  options: Array<{ value: T; label: string; icon?: LucideIcon }>;
  value: T;
  onChange: (value: T) => void;
}) {
  return (
    <div className="flex shrink-0 items-center gap-0.5 rounded-lg bg-surface p-0.5">
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          onClick={() => onChange(option.value)}
          className={cn(
            'flex h-7 items-center justify-center gap-1.5 rounded-md px-2.5 text-ui-sm font-normal transition-colors',
            value === option.value
              ? 'bg-panel text-foreground shadow-sm'
              : 'text-foreground-subtle hover:text-foreground',
          )}
        >
          {option.icon ? <option.icon className="size-3.5" /> : null}
          <span>{option.label}</span>
        </button>
      ))}
    </div>
  );
}
