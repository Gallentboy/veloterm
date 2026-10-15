import { useLayoutEffect, useRef, useState, type RefObject } from 'react';

/** 弹层位置（fixed 视口坐标） */
export interface AnchoredPanelPosition {
  top: number;
  left: number;
  /** 锚点宽度：下拉弹层与触发器等宽时作为 minWidth 使用；w-max 弹层忽略 */
  width: number;
}

/** 弹层与视口边缘的安全间距 */
const VIEWPORT_GAP_PX = 8;
/** 弹层与锚点的垂直间距 */
const ANCHOR_GAP_PX = 4;

/**
 * 锚定弹层的「portal + fixed 定位 + 视口夹取」通用逻辑（新建会话菜单 / 下拉选择共用）。
 *
 * 为什么必须 portal：触发按钮常位于 `overflow` 滚动/裁剪容器内（如 tab 栏的
 * `overflow-x-auto overflow-y-hidden`），内联绝对定位的弹层会被裁剪成「点了没反应」。
 * 挂到 `document.body` + fixed 坐标则彻底摆脱祖先裁剪。
 *
 * 位置在展开时按锚点 rect 计算（调用 `measure()`），弹层挂载后按实际尺寸做一次
 * 视口夹取：右侧越界左移、下方空间不足上翻到锚点上方；值不变则不 setState，
 * effect 自然收敛，不会来回抖动。
 *
 * 用法：
 * ```
 * const { anchorRef, panelRef, position, measure } = useAnchoredPanel(open);
 * // 触发：measure(); setOpen(true);
 * // 渲染：<div ref={anchorRef}>触发按钮</div> + createPortal(<div ref={panelRef}
 * //   style={{ position: 'fixed', top: position.top, left: position.left }} />, document.body)
 * ```
 */
export function useAnchoredPanel(open: boolean): {
  anchorRef: RefObject<HTMLDivElement | null>;
  panelRef: RefObject<HTMLDivElement | null>;
  position: AnchoredPanelPosition | null;
  measure: () => void;
} {
  const anchorRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<AnchoredPanelPosition | null>(null);

  const measure = () => {
    const rect = anchorRef.current?.getBoundingClientRect();
    if (!rect) return;
    setPosition({ top: rect.bottom + ANCHOR_GAP_PX, left: rect.left, width: rect.width });
  };

  useLayoutEffect(() => {
    if (!open || !position) return;
    const panel = panelRef.current;
    const anchor = anchorRef.current?.getBoundingClientRect();
    if (!panel || !anchor) return;

    const maxLeft = window.innerWidth - VIEWPORT_GAP_PX - panel.offsetWidth;
    const nextLeft = Math.max(VIEWPORT_GAP_PX, Math.min(position.left, maxLeft));
    const belowBottom = anchor.bottom + ANCHOR_GAP_PX + panel.offsetHeight;
    const nextTop =
      belowBottom > window.innerHeight - VIEWPORT_GAP_PX
        ? Math.max(VIEWPORT_GAP_PX, anchor.top - ANCHOR_GAP_PX - panel.offsetHeight)
        : anchor.bottom + ANCHOR_GAP_PX;

    if (nextLeft !== position.left || nextTop !== position.top) {
      setPosition({ ...position, top: nextTop, left: nextLeft });
    }
  }, [open, position]);

  return { anchorRef, panelRef, position, measure };
}
