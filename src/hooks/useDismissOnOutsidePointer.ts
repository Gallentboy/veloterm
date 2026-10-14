import { useEffect, useRef, type RefObject } from 'react';

/**
 * 右键菜单「点击外部关闭」通用逻辑（点击外部 / Escape）。
 *
 * 为什么不用朴素的 `window.addEventListener('pointerdown', close)`（冒泡阶段）：
 *
 * 1. **冒泡到 window 的链路很脆弱**：React 17+ 把事件委托挂在 `#root` 上，子树里任何
 *    `e.stopPropagation()`（例如 Tab 上的关闭按钮、重命名输入框）都会让事件止步于 `#root`，
 *    永远到不了 `window`，表现为「点了没反应，要再点一次才关」。
 * 2. **`data-tauri-drag-region` 区域不派发 DOM 事件**：标题栏 / 侧边栏空白处的点击会被
 *    窗口拖拽逻辑吃掉，冒泡阶段的监听同样收不到。
 * 3. **右键的顺序问题**：右键会先派发 `pointerdown`（button=2）再派发 `contextmenu`，
 *    冒泡阶段是「先关、后又被 contextmenu 重开」，视觉上等于原地没动。
 *
 * 因此改为在 **`document` 的捕获阶段** 监听：它先于 React 在 `#root` 上的委托监听、
 * 也先于任何子树的 `stopPropagation` 执行，保证「第一次点击就一定关掉」。
 * 命中菜单自身（`menuRef` 子树）则跳过，避免点菜单项时把自己关掉。
 *
 * @param isOpen   菜单是否打开（关闭时自动卸载监听）
 * @param menuRef  菜单根节点 ref，用于排除菜单内部的点击
 * @param onDismiss 触发关闭的回调
 */
export function useDismissOnOutsidePointer(
  isOpen: boolean,
  menuRef: RefObject<HTMLElement | null>,
  onDismiss: () => void,
): void {
  // 用 ref 持有最新回调，避免因回调每次渲染变化而反复重挂监听
  const dismissRef = useRef(onDismiss);
  dismissRef.current = onDismiss;

  useEffect(() => {
    if (!isOpen) {
      return;
    }

    const isInsideMenu = (target: EventTarget | null): boolean =>
      target instanceof Node && Boolean(menuRef.current?.contains(target));

    const onPointerDown = (event: PointerEvent) => {
      if (isInsideMenu(event.target)) {
        return;
      }
      dismissRef.current();
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        dismissRef.current();
      }
    };

    document.addEventListener('pointerdown', onPointerDown, true);
    document.addEventListener('keydown', onKeyDown, true);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown, true);
      document.removeEventListener('keydown', onKeyDown, true);
    };
  }, [isOpen, menuRef]);
}
