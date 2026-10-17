import { useCallback, useEffect, useRef, useState } from 'react';

/** 窗口宽度低于该值时自动收起侧边栏（宽度阈值方案，参考 ZCode shell 的自动收起策略） */
const AUTO_COLLAPSE_WINDOW_WIDTH_PX = 680;
const RESIZE_DEBOUNCE_MS = 300;

export function useAppPanels() {
  const [isSidebarVisible, setIsSidebarVisible] = useState(true);
  // 自动收起标记：true = 当前的收起是「窄窗口自动收起」所致，窗口放大回
  // 阈值以上时应自动恢复展开（收起前的展开意图被记住）。任何手动切换都会
  // 清掉标记——用户手动收起的不自动恢复，手动展开的无需恢复
  const autoCollapsedRef = useRef(false);
  const visibleRef = useRef(isSidebarVisible);
  visibleRef.current = isSidebarVisible;

  const toggleSidebar = useCallback(() => {
    autoCollapsedRef.current = false;
    setIsSidebarVisible((visible) => !visible);
  }, []);

  // 窄窗口自动收起侧边栏；窗口放大且收起系自动收起所致时自动展开。
  // 事件回调经 ref 读最新可见态（effect 只挂一次，闭包不能捕获旧状态）
  useEffect(() => {
    let timer: number | undefined;
    const onResize = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        if (window.innerWidth < AUTO_COLLAPSE_WINDOW_WIDTH_PX) {
          if (visibleRef.current) {
            autoCollapsedRef.current = true;
            setIsSidebarVisible(false);
          }
        } else if (autoCollapsedRef.current && !visibleRef.current) {
          autoCollapsedRef.current = false;
          setIsSidebarVisible(true);
        }
      }, RESIZE_DEBOUNCE_MS);
    };
    onResize();
    window.addEventListener('resize', onResize);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener('resize', onResize);
    };
  }, []);

  return { isSidebarVisible, toggleSidebar } as const;
}
