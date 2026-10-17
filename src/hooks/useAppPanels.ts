import { useCallback, useEffect, useRef, useState } from 'react';

/** 窗口宽度低于该值时自动收起侧边栏（宽度阈值方案，参考 ZCode shell 的自动收起策略） */
const AUTO_COLLAPSE_WINDOW_WIDTH_PX = 680;
/**
 * 自动展开阈值（含回滞带）：比收起阈值高 24px，避免在 680 附近拖动窗口时
 * 侧边栏反复开合抖动。680–704 之间保持现状（粘滞）
 */
const AUTO_EXPAND_WINDOW_WIDTH_PX = 704;

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

  // 窄窗口自动收起 / 放大自动展开：**越过阈值立即响应，不做防抖**——连续拖动
  // 窗口时防抖计时器被逐帧重置，只有停下 300ms 才动作，体感迟钝突兀；改为
  // 每帧只做一次廉价的阈值比较 + 回滞带防抖动。事件回调经 ref 读最新可见态
  // （effect 只挂一次，闭包不能捕获旧状态）
  useEffect(() => {
    const onResize = () => {
      const width = window.innerWidth;
      if (width < AUTO_COLLAPSE_WINDOW_WIDTH_PX) {
        if (visibleRef.current) {
          autoCollapsedRef.current = true;
          setIsSidebarVisible(false);
        }
      } else if (width >= AUTO_EXPAND_WINDOW_WIDTH_PX) {
        if (autoCollapsedRef.current && !visibleRef.current) {
          autoCollapsedRef.current = false;
          setIsSidebarVisible(true);
        }
      }
    };
    onResize();
    window.addEventListener('resize', onResize);
    return () => {
      window.removeEventListener('resize', onResize);
    };
  }, []);

  return { isSidebarVisible, toggleSidebar } as const;
}
