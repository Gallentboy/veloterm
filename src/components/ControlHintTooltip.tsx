import { useState, type ReactNode } from 'react';
import { isMacOS } from '@/lib/platform';
import { cn } from '@/lib/utils';

interface ControlHintTooltipProps {
  title: string;
  shortcut?: string;
  /** 提示出现在触发器的哪一侧；ZCode 顶栏按钮用 bottom。 */
  side?: 'top' | 'bottom';
  /** 水平对齐：center 居中（默认）；end 与触发器右对齐（靠右边缘的按钮用，避免被容器裁剪）。 */
  align?: 'center' | 'end';
  className?: string;
  children: ReactNode;
}

/**
 * 控件提示（CSS 版移植自 ZCode ControlHintTooltip 无描述场景）：
 * 标题 + 快捷键徽标横排、bg-tooltip 圆角弹层、macOS 快捷键用 SF 字体。
 * ZCode Tooltip delayDuration=0（悬停立即显示），这里去掉延迟保持一致。
 */
export function ControlHintTooltip({
  title,
  shortcut,
  side = 'top',
  align = 'center',
  className,
  children,
}: ControlHintTooltipProps) {
  const shortcutFontClassName = isMacOS ? 'tracking-normal' : 'font-mono';
  const shortcutFontStyle = isMacOS
    ? {
        fontFamily:
          '-apple-system, BlinkMacSystemFont, "SF Pro Text", "SF Pro Display", "Helvetica Neue", sans-serif',
      }
    : undefined;

  const [dismissedByInteraction, setDismissedByInteraction] = useState(false);

  return (
    <span
      className="group/tip relative inline-flex shrink-0"
      // 点击/键盘触发后立即隐藏 tooltip，指针离开后重新武装（与 Radix 行为一致）
      onPointerDownCapture={() => setDismissedByInteraction(true)}
      onKeyDownCapture={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          setDismissedByInteraction(true);
        }
      }}
      onPointerLeave={() => setDismissedByInteraction(false)}
      onBlurCapture={() => setDismissedByInteraction(false)}
    >
      {children}
      <span
        role="tooltip"
        className={cn(
          'pointer-events-none absolute z-50 inline-flex w-fit items-center gap-2 rounded-lg border border-border bg-tooltip px-2.5 py-1 text-left whitespace-nowrap opacity-0 transition-opacity duration-100',
          align === 'end' ? 'right-0' : 'left-1/2 -translate-x-1/2',
          !dismissedByInteraction && 'group-hover/tip:opacity-100',
          side === 'bottom' ? 'top-full mt-0.5' : 'bottom-full mb-0.5',
          shortcut && 'pr-1',
          className,
        )}
      >
        <span className="text-ui-sm font-medium text-tooltip-foreground">{title}</span>
        {shortcut ? (
          <kbd
            data-slot="kbd"
            style={shortcutFontStyle}
            className={cn(
              'inline-flex h-4 shrink-0 items-center rounded-md bg-tooltip-tag px-1.5 text-ui-xs font-medium text-tooltip-tag-foreground',
              shortcutFontClassName,
            )}
          >
            {shortcut}
          </kbd>
        ) : null}
      </span>
    </span>
  );
}
