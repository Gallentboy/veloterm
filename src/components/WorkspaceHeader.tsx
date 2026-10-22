import { PanelRightClose, PanelRightOpen, Sparkles } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { WorkspaceHelpMenuButton } from '@/components/WorkspaceHelpMenuButton';
import type { Page } from '@/components/WorkspaceSidebar';
import { shortcutLabel } from '@/lib/keyboard';
import { isMacOS } from '@/lib/platform';
import { cn } from '@/lib/utils';

interface WorkspaceHeaderProps {
  page: Page;
  isSidebarVisible: boolean;
  onOpenAiChat?: () => void;
  /** AI 右侧面板当前是否展开（镜像左侧栏开关） */
  isAiPanelOpen?: boolean;
  /** 切换 AI 右侧面板展开/收起 */
  onToggleAiPanel?: () => void;
  /**
   * 打开「关于」。macOS 由调用方触发独立窗口（Rust show_about_window）；
   * Windows 由调用方打开主窗内的纯前端弹层（见 AboutOverlay——WebView2
   * 透明分层窗口不稳定，改为主窗内渲染）。
   */
  onShowAbout?: () => void;
  /** 打开「使用说明 / 用户手册」电子书大弹窗 */
  onOpenManual?: () => void;
}

/** 主面板头部（移植自 ZCode WorkspaceHeader）：h-12、整条可拖拽移动窗口，右侧帮助菜单。 */
export function WorkspaceHeader({
  page,
  isSidebarVisible,
  onOpenAiChat,
  isAiPanelOpen,
  onToggleAiPanel,
  onShowAbout,
  onOpenManual,
}: WorkspaceHeaderProps) {
  const { t } = useTranslation();
  const showAbout = () => {
    if (onShowAbout) {
      onShowAbout();
      return;
    }
    // 兜底：未提供回调时走 Rust 侧独立窗口（macOS 菜单栏同路径）
    void invoke('show_about_window').catch(() => undefined);
  };
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
              : page === 'dbhosts'
                ? t('dbHosts.title')
                : page === 'db'
                  ? t('db.title')
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
        {onToggleAiPanel ? (
          <ControlHintTooltip
            title={t('ai.panel.toggle')}
            shortcut={shortcutLabel('I')}
            side="bottom"
          >
            <Button
              variant="ghost"
              size="icon-md"
              aria-label={t('ai.panel.toggle')}
              aria-pressed={isAiPanelOpen ?? false}
              onClick={onToggleAiPanel}
              className={cn(isAiPanelOpen && 'bg-hover')}
            >
              {isAiPanelOpen ? (
                <PanelRightClose className="size-4" />
              ) : (
                <PanelRightOpen className="size-4" />
              )}
            </Button>
          </ControlHintTooltip>
        ) : null}
        {onOpenAiChat ? (
          <ControlHintTooltip title={t('ai.chat.title')} side="bottom">
            <Button
              variant="ghost"
              size="icon-md"
              aria-label={t('ai.chat.title')}
              onClick={onOpenAiChat}
            >
              <Sparkles className="size-4" />
            </Button>
          </ControlHintTooltip>
        ) : null}
        <WorkspaceHelpMenuButton
          onShowAbout={showAbout}
          onOpenManual={onOpenManual ?? (() => {})}
        />
      </div>
    </header>
  );
}
