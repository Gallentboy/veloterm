import { useEffect, useRef, useState } from 'react';
import { getCurrentWindow, Window } from '@tauri-apps/api/window';
import { LogicalSize, PhysicalPosition } from '@tauri-apps/api/dpi';
import { emit } from '@tauri-apps/api/event';
import { AboutCard } from '@/components/about/AboutCard';

/** 出场动画时长（与 styles.css 的 .about-card-out 保持一致，关窗延时对齐） */
const ABOUT_EXIT_ANIMATION_MS = 220;

/** 卡片四周的透明出血边距（与容器 p-[22px] 一致）：窗口 = 卡片 + 2×出血 */
const ABOUT_BLEED_PX = 22;

/**
 * 「关于」独立窗口内容（macOS 路径；对齐 ZCode aboutWindow.ts 的自绘卡片）。
 *
 * 窗口本体是无边框透明子窗口（Rust 侧 about.rs 创建，架构同 ZCode 的
 * Electron frameless+transparent），本组件只负责窗口生命周期与动画编排；
 * 卡片视觉在 `AboutCard`（与 Windows 的纯前端弹层共用同一份实现）。
 *
 * - 根面板铺满窗口，透明窗口负责露出卡片圆角四角；
 * - 整卡可拖（data-tauri-drag-region，同 ZCode -webkit-app-region: drag），
 *   OK/Escape/Enter 关闭窗口。
 *
 * Windows 不走本组件——见 `AboutOverlay`（纯前端弹层，绕开 WebView2
 * 透明分层窗口的合成问题）。
 */
export function AboutWindow() {
  const okButtonRef = useRef<HTMLButtonElement>(null);
  const cardRef = useRef<HTMLDivElement>(null);
  const [isExiting, setIsExiting] = useState(false);

  // 窗口尺寸随卡片内容自适应（卡片高度是内容驱动的）：mount 后实测卡片
  // 尺寸，窗口 = 卡片 + 2×出血；字体就绪后再校一遍（webfont 换行高度会变）。
  // 改完尺寸重新居中到主窗（与 Rust 建窗同一算式）——窗口居中锚点变了。
  // Rust 建窗的 300×384 只是初始值，webview 透明区多余部分不可见、无闪烁
  useEffect(() => {
    const card = cardRef.current;
    const win = getCurrentWindow();
    if (!card) return;
    let cancelled = false;
    const fit = async () => {
      const rect = card.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0 || cancelled) return;
      const w = Math.ceil(rect.width) + ABOUT_BLEED_PX * 2;
      const h = Math.ceil(rect.height) + ABOUT_BLEED_PX * 2;
      await win.setSize(new LogicalSize(w, h));
      const main = await Window.getByLabel('main');
      if (!main || cancelled) return;
      const factor = await win.scaleFactor();
      const pos = await main.outerPosition();
      const size = await main.outerSize();
      const x = pos.x + (size.width - w * factor) / 2;
      const y = pos.y + (size.height - h * factor) / 2;
      await win.setPosition(new PhysicalPosition(Math.round(x), Math.round(y)));
    };
    void fit();
    document.fonts?.ready.then(() => {
      void fit();
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // 关闭编排（对齐 ZCode Dialog 的同步退场）：卡片滑退与主窗遮罩淡出**同帧**
  // 开始（先广播 about-visibility=false 再播退场动画，动画结束后才真正关窗）。
  // 关闭路径仅两条：OK 按钮 / Esc·Enter 键盘——主窗遮罩点击不关闭（点击
  // 别处无效，对话框只能由确定收尾）；窗口销毁时 Rust 侧还有一道
  // Destroyed→广播 false 的兜底（幂等）
  const closeWithAnimation = () => {
    if (isExiting) return;
    setIsExiting(true);
    void emit('about-visibility', false);
    window.setTimeout(() => {
      void getCurrentWindow().close();
    }, ABOUT_EXIT_ANIMATION_MS);
  };

  // 入场同步：卡片滑入动效与主窗遮罩渐暗同帧开始（mount 时广播 true，
  // 替代 Rust 侧建窗即广播的粗粒度时机）
  useEffect(() => {
    void emit('about-visibility', true);
  }, []);

  useEffect(() => {
    okButtonRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' || event.key === 'Enter') {
        closeWithAnimation();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- closeWithAnimation 依赖 isExiting，重挂监听无必要
  }, [isExiting]);

  return (
    // 透明出血容器：窗口比卡片大 22px×2，给卡片投影留绘制空间（窗口内不会被裁），
    // 整个出血区也允许拖动窗口；尺寸由上方 fit 逻辑随卡片内容自适应
    <div
      data-tauri-drag-region
      className="flex h-full w-full items-center justify-center p-[22px]"
    >
      <div data-tauri-drag-region ref={cardRef}>
        <AboutCard
          isExiting={isExiting}
          onClose={closeWithAnimation}
          okButtonRef={okButtonRef}
        />
      </div>
    </div>
  );
}
