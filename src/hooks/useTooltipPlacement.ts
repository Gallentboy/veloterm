import {
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
} from 'react';

export type TooltipSide = 'top' | 'bottom' | 'left' | 'right';
export type TooltipAlign = 'center' | 'start' | 'end';

/** 与视口边缘的安全间距（对齐 useAnchoredPanel 的取值习惯） */
const VIEWPORT_GAP_PX = 8;
/** 提示与锚点的间距 */
const ANCHOR_GAP_PX = 6;

const OPPOSITE_SIDE: Record<TooltipSide, TooltipSide> = {
  top: 'bottom',
  bottom: 'top',
  left: 'right',
  right: 'left',
};

export interface TooltipPlacement<T extends HTMLElement> {
  anchorRef: RefObject<T | null>;
  tooltipRef: RefObject<T | null>;
  /** fixed 视口坐标；首帧（测量前）为 null，配合 visibility:hidden 使用 */
  style: CSSProperties | null;
}

/**
 * tooltip 的「portal + fixed 定位 + 空间不足自动翻转 + 视口夹取」定位逻辑，
 * 与 useAnchoredPanel（下拉/菜单）同一套弹层约定在 tooltip 上的对应物：
 *
 * - 为什么必须 portal：触发器常位于 `overflow` 裁剪容器内（tab 栏的
 *   `overflow-x-auto overflow-y-hidden`、底部广播栏的父容器），内联绝对定位的
 *   提示会被裁掉一角甚至整层不可见；挂到 document.body + fixed 坐标彻底摆脱
 *   祖先裁剪。
 * - 方向自适应：`side` 是「期望侧」，下方/上方空间放不下时自动翻到对侧；
 *   两侧都放不下则退回期望侧做视口夹取（极端小窗口仍完整可见）。
 * - 位置在提示挂载后的 layout effect 里按锚点 rect + 提示实际尺寸一次算出，
 *   首帧以 visibility:hidden 挂载避免 (0,0) 闪烁，setState 发生在 paint 前。
 */
export function useTooltipPlacement<T extends HTMLElement>(
  visible: boolean,
  side: TooltipSide,
  align: TooltipAlign = 'center',
): TooltipPlacement<T> {
  const anchorRef = useRef<T>(null);
  const tooltipRef = useRef<T>(null);
  const [style, setStyle] = useState<CSSProperties | null>(null);

  useLayoutEffect(() => {
    if (!visible) {
      setStyle(null);
      return;
    }
    const anchor = anchorRef.current?.getBoundingClientRect();
    const tip = tooltipRef.current;
    if (!anchor || !tip) return;

    const tipWidth = tip.offsetWidth;
    const tipHeight = tip.offsetHeight;
    const maxTop = window.innerHeight - VIEWPORT_GAP_PX - tipHeight;
    const maxLeft = window.innerWidth - VIEWPORT_GAP_PX - tipWidth;

    const place = (s: TooltipSide): { top: number; left: number } => {
      // 主轴（弹出方向）坐标
      const main =
        s === 'bottom'
          ? anchor.bottom + ANCHOR_GAP_PX
          : s === 'top'
            ? anchor.top - ANCHOR_GAP_PX - tipHeight
            : s === 'right'
              ? anchor.right + ANCHOR_GAP_PX
              : anchor.left - ANCHOR_GAP_PX - tipWidth;
      // 交叉轴坐标（水平方向或垂直方向按 align 取 start/end/center）
      const crossCenter =
        s === 'top' || s === 'bottom'
          ? anchor.left + anchor.width / 2 - tipWidth / 2
          : anchor.top + anchor.height / 2 - tipHeight / 2;
      const crossStart = s === 'top' || s === 'bottom' ? anchor.left : anchor.top;
      const crossEnd =
        s === 'top' || s === 'bottom' ? anchor.right - tipWidth : anchor.bottom - tipHeight;
      const cross = align === 'start' ? crossStart : align === 'end' ? crossEnd : crossCenter;
      return s === 'top' || s === 'bottom' ? { top: main, left: cross } : { top: cross, left: main };
    };

    const fits = (p: { top: number; left: number }) =>
      p.top >= VIEWPORT_GAP_PX &&
      p.top <= maxTop &&
      p.left >= VIEWPORT_GAP_PX &&
      p.left <= maxLeft;

    const preferred = place(side);
    const flipped = place(OPPOSITE_SIDE[side]);
    const picked = fits(preferred) ? preferred : fits(flipped) ? flipped : preferred;

    setStyle({
      position: 'fixed',
      top: Math.max(VIEWPORT_GAP_PX, Math.min(picked.top, maxTop)),
      left: Math.max(VIEWPORT_GAP_PX, Math.min(picked.left, maxLeft)),
    });
  }, [visible, side, align]);

  return { anchorRef, tooltipRef, style };
}
