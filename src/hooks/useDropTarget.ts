import { useEffect, useRef, type RefObject } from 'react';
import {
  registerDropTarget,
  type AcceptedDragKind,
  type DragPosition,
  type DropEvent,
} from '@/services/dragDropService';

export interface UseDropTargetOptions<T extends HTMLElement> {
  /** 目标元素 ref（挂载前后都可能为 null，惰性取用） */
  ref: RefObject<T | null>;
  accepts: AcceptedDragKind[];
  onDrop: (event: DropEvent) => void;
  /** 命中/离开回调（供高亮）；position 为最后一次 Over 的 CSS 坐标 */
  onHoverChange?: (hovered: boolean, position?: DragPosition) => void;
}

/**
 * 组件级拖放目标注册：回调经 ref 取最新闭包，accepts 变化时重注册。
 * 原生事件流与命中测试见 `services/dragDropService`。
 */
export function useDropTarget<T extends HTMLElement>(
  options: UseDropTargetOptions<T>,
): void {
  // 回调走 ref：避免每次渲染重订阅原生事件回调链
  const optionsRef = useRef(options);
  optionsRef.current = options;
  const acceptsKey = options.accepts.join(',');

  useEffect(() => {
    return registerDropTarget({
      accepts: acceptsKey.split(',') as AcceptedDragKind[],
      getElement: () => optionsRef.current.ref.current,
      onDrop: (event) => optionsRef.current.onDrop(event),
      onHoverChange: (hovered, position) =>
        optionsRef.current.onHoverChange?.(hovered, position),
    });
  }, [acceptsKey]);
}
