import { Check, Copy } from 'lucide-react';
import React, { memo, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { cn } from '@/lib/utils';

export interface CellTooltipAnchor {
  text: string;
  target: HTMLElement;
}

interface CellTooltipPortalProps {
  anchor: CellTooltipAnchor | null;
  className?: string;
}

/** 与视口边缘的安全间距 */
const VIEWPORT_GAP_PX = 8;
const ANCHOR_GAP_PX = 6;

/**
 * 高精度检测文本是否真正被截断（显示不全）：
 * 1. 优先通过 scrollWidth > clientWidth 判定；
 * 2. 针对 HTML <table> 单元格（td 的 table-cell 特性常使 scrollWidth 等于 clientWidth），
 *    使用 Range 高精度测量文本实际排版宽度，只要实际文本宽度超过容器可视宽度即判定为截断。
 */
export function checkIsTruncated(el: HTMLElement): boolean {
  // 1. 常规溢出检测
  if (el.scrollWidth > el.clientWidth) {
    return true;
  }

  // 2. Range 测量法（精准到 0.1 像素，消除 table-cell 内部尺寸测不准的问题）
  try {
    const range = document.createRange();
    range.selectNodeContents(el);
    const textWidth = range.getBoundingClientRect().width;
    // 获取元素内部的有效内容宽度（扣减左右 padding）
    const style = window.getComputedStyle(el);
    const paddingLeft = parseFloat(style.paddingLeft) || 0;
    const paddingRight = parseFloat(style.paddingRight) || 0;
    const contentWidth = el.clientWidth - paddingLeft - paddingRight;

    if (textWidth > contentWidth + 1) {
      return true;
    }
  } catch {
    // 忽略测量异常，回退到普通 scrollWidth 比较
  }

  return false;
}

export const CellTooltipPortal = memo(function CellTooltipPortal({
  anchor,
  className,
}: CellTooltipPortalProps) {
  const { t } = useTranslation();
  const tooltipRef = useRef<HTMLDivElement | null>(null);
  const [copied, setCopied] = useState(false);
  const [style, setStyle] = useState<React.CSSProperties | null>(null);

  const handleCopy = useCallback(async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!anchor?.text) return;
    try {
      await navigator.clipboard.writeText(anchor.text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (err) {
      console.warn('Copy cell text failed', err);
    }
  }, [anchor?.text]);

  // 使用 layout effect 计算精确定位（支持空间不足自动翻转与视口安全夹取）
  useLayoutEffect(() => {
    if (!anchor || !anchor.text.trim()) {
      setStyle(null);
      setCopied(false);
      return;
    }

    const tip = tooltipRef.current;
    if (!tip) return;

    const anchorRect = anchor.target.getBoundingClientRect();
    const tipWidth = tip.offsetWidth || 260;
    const tipHeight = tip.offsetHeight || 60;

    const maxTop = window.innerHeight - VIEWPORT_GAP_PX - tipHeight;
    const maxLeft = window.innerWidth - VIEWPORT_GAP_PX - tipWidth;

    // 水平方向：居中对齐锚点并夹取在屏幕内
    let left = anchorRect.left + anchorRect.width / 2 - tipWidth / 2;
    left = Math.max(VIEWPORT_GAP_PX, Math.min(left, maxLeft));

    // 垂直方向：默认下方，下方放不下自动翻转到上方
    const spaceBelow = window.innerHeight - anchorRect.bottom;
    const spaceAbove = anchorRect.top;

    let top: number;
    if (spaceBelow >= tipHeight + ANCHOR_GAP_PX + VIEWPORT_GAP_PX || spaceBelow >= spaceAbove) {
      top = anchorRect.bottom + ANCHOR_GAP_PX;
    } else {
      top = anchorRect.top - tipHeight - ANCHOR_GAP_PX;
    }
    top = Math.max(VIEWPORT_GAP_PX, Math.min(top, maxTop));

    setStyle({
      position: 'fixed',
      top: `${top}px`,
      left: `${left}px`,
      visibility: 'visible',
    });
  }, [anchor]);

  if (!anchor || !anchor.text.trim()) return null;

  return createPortal(
    <div
      ref={tooltipRef}
      role="tooltip"
      style={style ?? { position: 'fixed', visibility: 'hidden' }}
      className={cn(
        'pointer-events-auto z-[90] flex max-h-56 max-w-md min-w-[120px] flex-col rounded-xl border border-border/90 bg-popover/95 p-2 shadow-2xl backdrop-blur-md animate-in fade-in select-text font-mono text-ui-xs',
        className,
      )}
    >
      <div className="flex items-center justify-between border-b border-border/50 pb-1 mb-1 text-[11px] text-foreground-subtle select-none">
        <span className="font-sans font-medium">完整内容</span>
        <button
          type="button"
          onClick={handleCopy}
          className="flex items-center gap-1 rounded px-1.5 py-0.5 text-foreground-subtle hover:bg-surface hover:text-foreground transition-colors"
          title={copied ? t('files.copied', '已复制') : t('files.copyCode', '复制')}
        >
          {copied ? (
            <>
              <Check className="size-3 text-terminal-green" />
              <span className="text-terminal-green text-[10px]">{t('files.copied', '已复制')}</span>
            </>
          ) : (
            <>
              <Copy className="size-3" />
              <span className="text-[10px]">{t('files.copyCode', '复制')}</span>
            </>
          )}
        </button>
      </div>
      <div className="overflow-y-auto max-h-40 break-all whitespace-pre-wrap leading-relaxed text-foreground select-text pr-1">
        {anchor.text}
      </div>
    </div>,
    document.body,
  );
});

/** 单元格 Tooltip 的管理 Hook：严格且精准地「仅在显示不全/截断时展示」 */
export function useCellTooltip() {
  const [activeAnchor, setActiveAnchor] = useState<CellTooltipAnchor | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearTimer = useCallback(() => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const handleCellMouseEnter = useCallback(
    (e: React.MouseEvent<HTMLElement>, text: string) => {
      clearTimer();
      const target = e.currentTarget;
      if (!text || !text.trim()) {
        setActiveAnchor(null);
        return;
      }

      // 精确判定：只有真正显示不全（被截断）或含有换行时才触发展示！
      const isTruncated = checkIsTruncated(target) || text.includes('\n');
      if (!isTruncated) {
        setActiveAnchor(null);
        return;
      }

      // 100ms 短防抖，鼠标稍作停顿即平滑浮出
      timerRef.current = setTimeout(() => {
        setActiveAnchor({ text, target });
      }, 100);
    },
    [clearTimer],
  );

  const handleCellMouseLeave = useCallback(() => {
    clearTimer();
    setActiveAnchor(null);
  }, [clearTimer]);

  const handleContainerScroll = useCallback(() => {
    clearTimer();
    setActiveAnchor(null);
  }, [clearTimer]);

  useEffect(() => {
    return () => clearTimer();
  }, [clearTimer]);

  return {
    activeAnchor,
    handleCellMouseEnter,
    handleCellMouseLeave,
    handleContainerScroll,
  };
}
