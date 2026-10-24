import { useEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import {
  TOOLTIP_HIDDEN_STYLE,
  useTooltipPlacement,
  type TooltipAlign,
  type TooltipSide,
} from '@/hooks/useTooltipPlacement';
import { isMacOS } from '@/lib/platform';
import { cn } from '@/lib/utils';

interface ControlHintTooltipProps {
  title: string;
  shortcut?: string;
  /** 期望出现侧；空间不足自动翻到对侧，两侧都放不下时视口夹取（见 useTooltipPlacement） */
  side?: TooltipSide;
  /** 水平对齐：center 居中（默认）；end 与触发器右对齐（靠右边缘的按钮用）。 */
  align?: TooltipAlign;
  /** 块级包装（树行/列表行等块级触发器）：wrapper span 从 inline-flex 改
   * block w-full，不破坏父层块级流；默认 inline-flex 适配按钮/图标 */
  block?: boolean;
  /** 悬停延迟（ms，默认 0 立即显示）：树/列表等密集行场景传延迟，
   * 避免指针划过时一路弹 */
  delayMs?: number;
  className?: string;
  children: ReactNode;
}

/**
 * 控件提示（CSS 版移植自 ZCode ControlHintTooltip 无描述场景）：
 * 标题 + 快捷键徽标横排、bg-tooltip 圆角弹层、macOS 快捷键用 SF 字体。
 * ZCode Tooltip delayDuration=0（悬停立即显示），这里去掉延迟保持一致。
 *
 * 定位走 useTooltipPlacement：portal 到 document.body + fixed 坐标 + 空间不足
 * 自动翻转——触发器常在 overflow 裁剪容器（tab 栏/底部工具栏）内，旧的
 * 内联 absolute 版会被祖先裁掉一角，方向也固定死了。
 */
export function ControlHintTooltip({
  title,
  shortcut,
  side = 'top',
  align = 'center',
  block = false,
  delayMs = 0,
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

  const [hovering, setHovering] = useState(false);
  // 点击/键盘触发后立即隐藏 tooltip，指针离开后重新武装（与 Radix 行为一致）
  const [dismissedByInteraction, setDismissedByInteraction] = useState(false);
  // 延迟显示的定时器（delayMs>0 时）：离开/按下时取消，未到点不弹
  const enterTimerRef = useRef<number | undefined>(undefined);
  const armOpen = (open: boolean) => {
    window.clearTimeout(enterTimerRef.current);
    if (open && delayMs > 0) {
      enterTimerRef.current = window.setTimeout(() => setHovering(true), delayMs);
    } else {
      setHovering(open);
    }
  };
  useEffect(() => () => window.clearTimeout(enterTimerRef.current), []);
  const open = hovering && !dismissedByInteraction;

  const { anchorRef, tooltipRef, style } = useTooltipPlacement<HTMLSpanElement>(
    open,
    side,
    align,
  );

  return (
    <span
      ref={anchorRef}
      className={block ? 'block w-full' : 'inline-flex shrink-0'}
      onPointerEnter={() => armOpen(true)}
      onPointerLeave={() => {
        armOpen(false);
        setDismissedByInteraction(false);
      }}
      onFocus={() => armOpen(true)}
      onBlur={() => {
        armOpen(false);
        setDismissedByInteraction(false);
      }}
      onPointerDownCapture={() => {
        armOpen(false);
        setDismissedByInteraction(true);
      }}
      onKeyDownCapture={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          setDismissedByInteraction(true);
        }
      }}
    >
      {children}
      {open &&
        createPortal(
          <span
            ref={tooltipRef}
            role="tooltip"
            // 首帧测量前隐藏（fixed + hidden：测量与最终渲染同一布局上下文，见 TOOLTIP_HIDDEN_STYLE）
            style={style ?? TOOLTIP_HIDDEN_STYLE}
            className={cn(
              'pointer-events-none z-[80] inline-flex w-fit items-center gap-2 rounded-lg border border-border bg-tooltip px-2.5 py-1 text-left whitespace-nowrap animate-in fade-in duration-100',
              shortcut && 'pr-1',
              className,
            )}
          >
            <span className="text-ui-sm font-normal text-tooltip-foreground">{title}</span>
            {shortcut ? (
              <kbd
                data-slot="kbd"
                style={shortcutFontStyle}
                className={cn(
                  'inline-flex h-4 shrink-0 items-center rounded-md bg-tooltip-tag px-1.5 text-ui-xs font-normal text-tooltip-tag-foreground',
                  shortcutFontClassName,
                )}
              >
                {shortcut}
              </kbd>
            ) : null}
          </span>,
          document.body,
        )}
    </span>
  );
}
