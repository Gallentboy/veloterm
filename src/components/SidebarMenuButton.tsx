import type { LucideIcon } from 'lucide-react';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';

interface SidebarMenuButtonProps {
  icon: LucideIcon;
  label: string;
  active?: boolean;
  onClick?: () => void;
}

/**
 * 侧边栏菜单按钮（通用）：ZCode WorkspaceSidebarItem 同款规格——
 * ghost default 基座（14px 文本/图标、transition-colors）+ 侧栏覆写
 * （h-8、gap-2、rounded-lg、pl-2.5 pr-1、hover 用 surface-hover）。
 * 新增菜单项直接复用，无需逐个调整间距与 hover。
 */
export function SidebarMenuButton({ icon: Icon, label, active, onClick }: SidebarMenuButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        buttonVariants({ variant: 'ghost', size: 'default' }),
        'flex h-8 w-full min-w-0 justify-start gap-2 rounded-lg pl-2.5 pr-1 text-left text-foreground',
        'hover:bg-surface-hover hover:text-foreground',
        active && 'bg-selected',
      )}
    >
      <Icon />
      <span className="truncate">{label}</span>
    </button>
  );
}
