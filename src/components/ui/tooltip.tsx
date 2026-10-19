import { useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useTooltipPlacement, TOOLTIP_HIDDEN_STYLE, type TooltipAlign, type TooltipSide } from '@/hooks/useTooltipPlacement';
import { cn } from '@/lib/utils';

export type { TooltipSide, TooltipAlign };

export interface TooltipProps {
  content: ReactNode;
  children: ReactNode;
  /** 期望出现侧；空间不足自动翻到对侧，两侧都放不下时视口夹取（见 useTooltipPlacement） */
  side?: TooltipSide;
  align?: TooltipAlign;
  className?: string;
  delay?: number;
}

/**
 * 通用 tooltip：portal 到 document.body + fixed 定位 + 空间不足自动翻转
 * （useTooltipPlacement），摆脱祖先 overflow 裁剪与固定方向弹出被视口挡住的问题。
 */
export function Tooltip({
  content,
  children,
  side = 'top',
  align = 'center',
  className,
  delay = 200,
}: TooltipProps) {
  const [visible, setVisible] = useState(false);
  const [timer, setTimer] = useState<ReturnType<typeof setTimeout> | null>(null);

  const show = () => {
    const t = setTimeout(() => {
      setVisible(true);
    }, delay);
    setTimer(t);
  };

  const hide = () => {
    if (timer) {
      clearTimeout(timer);
      setTimer(null);
    }
    setVisible(false);
  };

  const { anchorRef, tooltipRef, style } = useTooltipPlacement<HTMLDivElement>(
    visible,
    side,
    align,
  );

  return (
    <div
      ref={anchorRef}
      className="inline-flex items-center"
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocus={show}
      onBlur={hide}
    >
      {children}
      {visible &&
        content &&
        createPortal(
          <div
            ref={tooltipRef}
            role="tooltip"
            // 首帧测量前隐藏（fixed + hidden：测量与最终渲染同一布局上下文，见 TOOLTIP_HIDDEN_STYLE）
            style={style ?? TOOLTIP_HIDDEN_STYLE}
            className={cn(
              'pointer-events-none z-[80] w-fit max-w-[calc(100vw-16px)] whitespace-nowrap rounded-md px-2 py-1 text-ui-xs font-normal text-foreground bg-popover/95 border border-border/80 shadow-md backdrop-blur-xs transition-opacity duration-150 animate-in fade-in select-none',
              className,
            )}
          >
            {content}
          </div>,
          document.body,
        )}
    </div>
  );
}
