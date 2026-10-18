import { Boxes, Command, Database, FileCode, FolderSync, FolderTree, House, Network, Server, Settings, Terminal, Waypoints } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { SidebarMenuButton } from '@/components/SidebarMenuButton';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import {
  getSidebarMenuConfig,
  normalizeSidebarOrder,
  SIDEBAR_VISIBILITY_EVENT,
} from '@/services/sidebarVisibility';

export type Page = 'home' | 'terminal' | 'editor' | 'files' | 'hosts' | 'groups' | 'commands' | 'zk' | 'zkhosts' | 'relays' | 'grpc' | 'settings';

/** 侧栏导航项（settings 固定展示，不参与可见性配置）。设置页的菜单展示
 *  区块与此清单同源（同一导出），勾选状态经 kv + 事件实时联动。 */
export const SIDEBAR_NAV_ITEMS: Array<{ page: Page; icon: LucideIcon; navKey: string }> = [
  { page: 'home', icon: House, navKey: 'nav.home' },
  { page: 'terminal', icon: Terminal, navKey: 'nav.terminal' },
  { page: 'editor', icon: FileCode, navKey: 'nav.editor' },
  { page: 'files', icon: FolderSync, navKey: 'nav.files' },
  { page: 'hosts', icon: Server, navKey: 'nav.hosts' },
  { page: 'relays', icon: Waypoints, navKey: 'nav.relays' },
  { page: 'grpc', icon: Network, navKey: 'nav.grpc' },
  { page: 'groups', icon: FolderTree, navKey: 'nav.groups' },
  { page: 'commands', icon: Command, navKey: 'nav.commands' },
  { page: 'zkhosts', icon: Boxes, navKey: 'nav.zkHosts' },
  { page: 'zk', icon: Database, navKey: 'nav.zk' },
];

interface WorkspaceSidebarProps {
  activePage: Page;
  onPageChange: (page: Page) => void;
  className?: string;
}

/** 左侧导航栏（结构移植自 ZCode WorkspaceSidebar）：顶部拖拽条 + 导航 + 右下角设置入口。
 *  菜单项可在设置页配置隐藏（默认全部展示，实时联动）。 */
export function WorkspaceSidebar({ activePage, onPageChange, className }: WorkspaceSidebarProps) {
  const { t } = useTranslation();
  const [menuConfig, setMenuConfig] = useState(() => getSidebarMenuConfig());

  useEffect(() => {
    const onChanged = () => setMenuConfig(getSidebarMenuConfig());
    window.addEventListener(SIDEBAR_VISIBILITY_EVENT, onChanged);
    return () => window.removeEventListener(SIDEBAR_VISIBILITY_EVENT, onChanged);
  }, []);

  // 渲染顺序 = 配置顺序（缺失项/新菜单按内置顺序补齐尾），隐藏项过滤
  const orderedItems = normalizeSidebarOrder(
    menuConfig,
    SIDEBAR_NAV_ITEMS.map(({ page }) => page),
  )
    .map((page) => SIDEBAR_NAV_ITEMS.find((item) => item.page === page))
    .filter((item): item is (typeof SIDEBAR_NAV_ITEMS)[number] => item != null)
    .filter(({ page }) => !menuConfig.hidden.includes(page));

  return (
    <aside className={cn('flex h-full flex-col select-none overflow-hidden', className)}>
      {/* 顶部拖拽条：macOS 原生红绿灯悬浮在这条区域上（h-12 与 Header 同高） */}
      <div className="h-12 shrink-0" data-tauri-drag-region />

      {/* 导航容器：ZCode 同款 gap-1 / px-2 / py-3；顺序与可见性由设置页配置（实时联动） */}
      <nav className="flex flex-col gap-1 px-2 py-3">
        {orderedItems.map(({ page, icon, navKey }) => (
          <SidebarMenuButton
            key={page}
            icon={icon}
            label={t(navKey)}
            active={activePage === page}
            onClick={() => onPageChange(page)}
          />
        ))}
      </nav>

      <div className="min-h-0 flex-1" data-tauri-drag-region />

      {/* 底部：设置入口靠右下角（ZCode 侧栏底部布局，无分隔线）；tooltip 右对齐避免被侧栏裁剪 */}
      <footer className="flex shrink-0 items-center justify-end p-2 pr-3">
        <ControlHintTooltip title={t('header.settings')} side="top" align="end">
          <Button
            variant="ghost"
            size="icon-md"
            aria-label={t('header.settings')}
            aria-current={activePage === 'settings' ? 'page' : undefined}
            onClick={() => onPageChange('settings')}
            className={cn(activePage === 'settings' && 'bg-hover')}
          >
            <Settings />
          </Button>
        </ControlHintTooltip>
      </footer>
    </aside>
  );
}
