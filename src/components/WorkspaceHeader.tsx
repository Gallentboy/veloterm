import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AboutDialog } from '@/components/AboutDialog';
import { WorkspaceHelpMenuButton } from '@/components/WorkspaceHelpMenuButton';
import type { Page } from '@/components/WorkspaceSidebar';
import { isMacOS } from '@/lib/platform';
import { cn } from '@/lib/utils';

interface WorkspaceHeaderProps {
  page: Page;
  isSidebarVisible: boolean;
}

/** 主面板头部（移植自 ZCode WorkspaceHeader）：h-12、整条可拖拽移动窗口，右侧帮助菜单。 */
export function WorkspaceHeader({ page, isSidebarVisible }: WorkspaceHeaderProps) {
  const { t } = useTranslation();
  const [isAboutOpen, setAboutOpen] = useState(false);
  const title =
    page === 'terminal'
      ? t('terminal.title')
      : page === 'editor'
        ? t('editor.title')
        : page === 'files'
        ? t('files.title')
        : page === 'hosts'
          ? t('hosts.title')
          : page === 'groups'
            ? t('groups.title')
            : page === 'settings'
              ? t('settings.title')
              : t('app.name');

  return (
    <header
      data-tauri-drag-region
      className="relative flex h-12 w-full shrink-0 select-none items-center justify-between gap-2 border-b border-border/50 p-2"
    >
      {/* 侧边栏收起后，左上角浮层开关（红绿灯安全区 + 按钮）会悬在内容区上方，标题需让位。 */}
      <div
        data-tauri-drag-region
        className={cn(
          'flex min-w-0 flex-1 items-center overflow-hidden pl-2 transition-[padding] duration-300 ease-out',
          !isSidebarVisible && (isMacOS ? 'pl-36' : 'pl-14'),
        )}
      >
        <span className="truncate text-ui-lg font-semibold text-foreground-subtle">{title}</span>
      </div>

      <div className="flex shrink-0 items-center gap-0.5">
        <WorkspaceHelpMenuButton onShowAbout={() => setAboutOpen(true)} />
      </div>

      <AboutDialog open={isAboutOpen} onClose={() => setAboutOpen(false)} />
    </header>
  );
}
