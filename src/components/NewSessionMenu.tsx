import { Laptop, Plus, Server, type LucideIcon } from 'lucide-react';
import { useState } from 'react';
import { createPortal } from 'react-dom';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { useAnchoredPanel } from '@/hooks/useAnchoredPanel';
import { useDismissOnOutsidePointer } from '@/hooks/useDismissOnOutsidePointer';

/** 菜单里的主机条目（HostNode 的结构子集，避免绑定主机模块类型） */
export interface NewSessionMenuHost {
  id: number;
  name: string;
  username: string;
  host: string;
}

export interface NewSessionMenuProps {
  /** “+” 按钮 tooltip 与 aria-label 文案 */
  title: string;
  /** 主机分组小节标题（如「远程主机」）；缺省时不渲染小节 */
  hostSectionLabel?: string;
  /** 本地终端项文案；与 onNewLocal 成对出现 */
  localLabel?: string;
  onNewLocal?: () => void;
  /** 无任何主机可选的退化动作：点击 “+” 直接执行，不展开单项目菜单 */
  onDirectAction?: () => void;
  hosts: NewSessionMenuHost[];
  onSelectHost: (hostId: number) => void;
}

/** 菜单行：统一 16px 图标槽位 + 等宽左对齐文本（本地终端与各主机的文字左缘严格对齐） */
function MenuRow({
  icon: Icon,
  label,
  onSelect,
}: {
  icon: LucideIcon;
  label: string;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onSelect}
      className="flex w-full cursor-default items-center gap-2 rounded-md px-2 py-1.5 text-left text-ui-sm text-foreground transition-colors hover:bg-menu-hover focus-visible:bg-menu-hover focus-visible:outline-none"
    >
      <span className="flex size-4 shrink-0 items-center justify-center">
        <Icon className="size-3.5 text-foreground-subtle" />
      </span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
    </button>
  );
}

/**
 * 新建会话下拉菜单（终端页 / 文件页共用）。
 *
 * 自绘弹层替代原先盖在 “+” 按钮上的透明原生 `<select>`：原生下拉的弹层由系统绘制，
 * macOS 下是圆角系统菜单、Windows 下是直角系统列表框，两端观感割裂，深色主题下
 * 也无法跟随工程配色。这里统一走工程菜单 token（bg-menu / menu-hover / border /
 * shadow，均随亮暗模式切换），并保证所有菜单项左对齐。
 *
 * 弹层经 `createPortal` 挂到 `document.body`（定位/夹取逻辑见 `useAnchoredPanel`）：
 * 触发按钮位于 tab 栏的 `overflow-x-auto overflow-y-hidden` 滚动容器内，内联
 * 绝对定位的弹层会被裁剪成「点了没反应」；portal + fixed 定位彻底摆脱祖先裁剪。
 */
export function NewSessionMenu({
  title,
  hostSectionLabel,
  localLabel,
  onNewLocal,
  onDirectAction,
  hosts,
  onSelectHost,
}: NewSessionMenuProps) {
  const [open, setOpen] = useState(false);
  // anchorRef 必须挂在根 div 上：hook 的 measure() 读它取锚点 rect，不挂就是
  // 点了没反应（position 恒为 null，弹层渲染条件永远不成立）
  const { anchorRef, panelRef, position, measure } = useAnchoredPanel(open);
  // 关闭判定的排除区是两段不同子树的 DOM：触发按钮（anchorRef）+ portal 弹层（panelRef）
  useDismissOnOutsidePointer(open, [anchorRef, panelRef], () => setOpen(false));

  const hasLocal = Boolean(localLabel && onNewLocal);
  const selectableCount = (hasLocal ? 1 : 0) + hosts.length;

  const handleTrigger = () => {
    // 无主机（或仅一个选项）时退化为直接执行，保持「点 + 即开终端」的旧手感
    if (selectableCount <= 1 && onDirectAction) {
      onDirectAction();
      return;
    }
    if (open) {
      setOpen(false);
      return;
    }
    measure();
    setOpen(true);
  };

  const closeAndRun = (action: () => void) => {
    setOpen(false);
    action();
  };

  return (
    <div ref={anchorRef} className="relative inline-flex shrink-0 items-center">
      <ControlHintTooltip title={title} side="bottom">
        <Button
          type="button"
          size="icon-xs"
          variant="ghost"
          className="h-7 w-7 rounded-lg text-foreground-subtle hover:text-foreground hover:bg-hover active:bg-hover/80"
          onClick={handleTrigger}
          aria-label={title}
          aria-haspopup="menu"
          aria-expanded={open}
        >
          <Plus className="size-3.5" />
        </Button>
      </ControlHintTooltip>

      {open && position
        ? createPortal(
            <div
              ref={panelRef}
              role="menu"
              aria-label={title}
              style={{ position: 'fixed', top: position.top, left: position.left }}
              className="scrollbar-slim z-[60] max-h-80 w-max min-w-48 max-w-80 overflow-y-auto rounded-lg border border-border bg-menu p-1 shadow-lg"
            >
              {hasLocal ? (
                <MenuRow
                  icon={Laptop}
                  label={localLabel!}
                  onSelect={() => closeAndRun(onNewLocal!)}
                />
              ) : null}
              {hasLocal && hosts.length > 0 ? (
                <div className="mx-1 my-1 h-px bg-border/60" />
              ) : null}
              {hosts.length > 0 && hostSectionLabel ? (
                <div className="px-2 pt-0.5 pb-1 text-ui-xs text-foreground-subtlest select-none">
                  {hostSectionLabel}
                </div>
              ) : null}
              {hosts.map((host) => (
                <MenuRow
                  key={host.id}
                  icon={Server}
                  label={`${host.name} (${host.username}@${host.host})`}
                  onSelect={() => closeAndRun(() => onSelectHost(host.id))}
                />
              ))}
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}
