import { useCallback, useEffect, useState } from 'react';

/** 窗口宽度低于该值时自动收起侧边栏（宽度阈值方案，参考 ZCode shell 的自动收起策略） */
const AUTO_COLLAPSE_WINDOW_WIDTH_PX = 680;
const RESIZE_DEBOUNCE_MS = 300;

export function useAppPanels() {
  const [isSidebarVisible, setIsSidebarVisible] = useState(true);

  const toggleSidebar = useCallback(() => {
    setIsSidebarVisible((visible) => !visible);
  }, []);

  // 窄窗口自动收起侧边栏；只收不展，避免覆盖用户手动展开的意愿。
  useEffect(() => {
    let timer: number | undefined;
    const onResize = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        if (window.innerWidth < AUTO_COLLAPSE_WINDOW_WIDTH_PX) {
          setIsSidebarVisible(false);
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
