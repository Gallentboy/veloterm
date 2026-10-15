import { PanelLeftClose, PanelLeftOpen } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { shortcutLabel } from '@/lib/keyboard';
import { isMacOS, MACOS_TRAFFIC_LIGHT_LEFT_PADDING_PX } from '@/lib/platform';

/** ZCode shortcutCommands: toggleSidebar 默认绑定主修饰键(⌘/Ctrl)+B */
export const TOGGLE_SIDEBAR_SHORTCUT = shortcutLabel('B');

interface DesktopTopOverlayProps {
  isSidebarVisible: boolean;
  onToggleSidebar: () => void;
}

/**
 * 左上角工具浮层（移植自 ZCode DesktopTopOverlay）。
 * 悬在侧边栏顶部；按钮垂直中心 = 28px，与红绿灯（tao inset y:30 → 中心 28px）
 * 及右侧主面板头部中心线（4px 顶部间隙 + 48px 头部 → 中心 28px）三线对齐。
 * 侧边栏收起后开关仍悬在内容区上方，可达。
 */
export function DesktopTopOverlay({ isSidebarVisible, onToggleSidebar }: DesktopTopOverlayProps) {
  const { t } = useTranslation();
  const ToggleIcon = isSidebarVisible ? PanelLeftClose : PanelLeftOpen;

  return (
    <div
      style={isSidebarVisible ? { width: 'var(--sidebar-panel-width)' } : undefined}
      className="pointer-events-none absolute left-0 top-0 z-20 flex h-14 w-fit"
    >
      <div
        style={
          isMacOS
            ? { paddingLeft: `${MACOS_TRAFFIC_LIGHT_LEFT_PADDING_PX}px` }
            : { paddingLeft: '8px' }
        }
        className="flex h-14 items-center"
      >
        <ControlHintTooltip
          title={t('sidebar.toggle')}
          shortcut={TOGGLE_SIDEBAR_SHORTCUT}
          side="bottom"
        >
          <Button
            variant="ghost"
            size="icon-md"
            aria-label={t('sidebar.toggle')}
            onClick={onToggleSidebar}
            className="pointer-events-auto shrink-0"
          >
            <ToggleIcon />
          </Button>
        </ControlHintTooltip>
      </div>
    </div>
  );
}
