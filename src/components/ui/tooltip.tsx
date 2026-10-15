import { useState, type ReactNode } from 'react';
import { cn } from '@/lib/utils';

export interface TooltipProps {
  content: ReactNode;
  children: ReactNode;
  side?: 'top' | 'bottom' | 'left' | 'right';
  align?: 'center' | 'start' | 'end';
  className?: string;
  delay?: number;
}

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

  let posClass = '';
  if (side === 'bottom') {
    posClass =
      align === 'end'
        ? 'top-full right-0 mt-1.5'
        : align === 'start'
          ? 'top-full left-0 mt-1.5'
          : 'top-full left-1/2 -translate-x-1/2 mt-1.5';
  } else if (side === 'top') {
    posClass =
      align === 'end'
        ? 'bottom-full right-0 mb-1.5'
        : align === 'start'
          ? 'bottom-full left-0 mb-1.5'
          : 'bottom-full left-1/2 -translate-x-1/2 mb-1.5';
  } else if (side === 'left') {
    posClass = 'right-full top-1/2 -translate-y-1/2 mr-1.5';
  } else {
    posClass = 'left-full top-1/2 -translate-y-1/2 ml-1.5';
  }

  return (
    <div
      className="relative inline-flex items-center"
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocus={show}
      onBlur={hide}
    >
      {children}
      {visible && content && (
        <div
          role="tooltip"
          className={cn(
            'pointer-events-none absolute z-[60] whitespace-nowrap rounded-md px-2 py-1 text-ui-xs font-normal text-foreground bg-popover/95 border border-border/80 shadow-md backdrop-blur-xs transition-opacity duration-150 animate-in fade-in select-none',
            posClass,
            className,
          )}
        >
          {content}
        </div>
      )}
    </div>
  );
}
