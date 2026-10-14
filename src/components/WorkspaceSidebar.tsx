import { Command, FileCode, FolderSync, FolderTree, House, Server, Settings, Terminal } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { SidebarMenuButton } from '@/components/SidebarMenuButton';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

export type Page = 'home' | 'terminal' | 'editor' | 'files' | 'hosts' | 'groups' | 'commands' | 'settings';

interface WorkspaceSidebarProps {
  activePage: Page;
  onPageChange: (page: Page) => void;
  className?: string;
}

/** 左侧导航栏（结构移植自 ZCode WorkspaceSidebar）：顶部拖拽条 + 导航 + 右下角设置入口。 */
export function WorkspaceSidebar({ activePage, onPageChange, className }: WorkspaceSidebarProps) {
  const { t } = useTranslation();

  return (
    <aside className={cn('flex h-full flex-col select-none overflow-hidden', className)}>
      {/* 顶部拖拽条：macOS 原生红绿灯悬浮在这条区域上（h-12 与 Header 同高） */}
      <div className="h-12 shrink-0" data-tauri-drag-region />

      {/* 导航容器：ZCode 同款 gap-1 / px-2 / py-3 */}
      <nav className="flex flex-col gap-1 px-2 py-3">
        <SidebarMenuButton
          icon={House}
          label={t('nav.home')}
          active={activePage === 'home'}
          onClick={() => onPageChange('home')}
        />
        <SidebarMenuButton
          icon={Terminal}
          label={t('nav.terminal')}
          active={activePage === 'terminal'}
          onClick={() => onPageChange('terminal')}
        />
        <SidebarMenuButton
          icon={FileCode}
          label={t('nav.editor')}
          active={activePage === 'editor'}
          onClick={() => onPageChange('editor')}
        />
        <SidebarMenuButton
          icon={FolderSync}
          label={t('nav.files')}
          active={activePage === 'files'}
          onClick={() => onPageChange('files')}
        />
        <SidebarMenuButton
          icon={Server}
          label={t('nav.hosts')}
          active={activePage === 'hosts'}
          onClick={() => onPageChange('hosts')}
        />
        <SidebarMenuButton
          icon={FolderTree}
          label={t('nav.groups')}
          active={activePage === 'groups'}
          onClick={() => onPageChange('groups')}
        />
        <SidebarMenuButton
          icon={Command}
          label={t('nav.commands')}
          active={activePage === 'commands'}
          onClick={() => onPageChange('commands')}
        />
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
