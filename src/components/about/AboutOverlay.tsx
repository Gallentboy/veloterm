import { useEffect, useRef, useState } from 'react';
import { AboutCard } from '@/components/about/AboutCard';

/** 退场动画时长（与 styles.css 的 .about-card-out 一致） */
const ABOUT_EXIT_ANIMATION_MS = 220;

interface AboutOverlayProps {
  open: boolean;
  onClose: () => void;
}

/**
 * 「关于」纯前端弹层（Windows 路径）。
 *
 * 为什么不用独立窗口：Windows 的 WebView2 在 `transparent(true)` 分层窗口下
 * 对 `on_page_load` 回调与 `skip_taskbar` 的处理不稳定——实测窗口建出来
 * 但永不显示（表现为「点关于无反应」）。ZCode 在 Electron 上用
 * `ready-to-show` + `data:` 内联 HTML 规避，Tauri 没有等价机制，
 * 因此 Windows 直接在主窗口内渲染卡片（macOS 仍用独立窗口 + 菜单栏）。
 *
 * 视觉与 `AboutWindow` 完全一致（共用 `AboutCard`）：遮罩淡化下层 +
 * 卡片入场/退场动画；OK / Esc / Enter 关闭；点击遮罩不关闭（与窗口版
 * 语义一致——对话框只能由确定收尾）。
 */
export function AboutOverlay({ open, onClose }: AboutOverlayProps) {
  const okButtonRef = useRef<HTMLButtonElement>(null);
  const [isMounted, setIsMounted] = useState(false);
  const [isExiting, setIsExiting] = useState(false);

  // 打开时挂载并播入场；关闭时先播退场再卸载
  useEffect(() => {
    if (open) {
      setIsMounted(true);
      setIsExiting(false);
      return;
    }
    if (!isMounted) return;
    setIsExiting(true);
    const timer = window.setTimeout(() => {
      setIsMounted(false);
      setIsExiting(false);
    }, ABOUT_EXIT_ANIMATION_MS);
    return () => window.clearTimeout(timer);
  }, [open, isMounted]);

  // 键盘关闭：Esc / Enter（与窗口版一致）
  useEffect(() => {
    if (!isMounted || isExiting) return;
    okButtonRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' || event.key === 'Enter') {
        onClose();
        return;
      }
      // 模态焦点陷阱：Tab 只在弹层内循环（卡片内可聚焦元素：X 按钮、确定按钮），
      // 阻止焦点跑到背景主窗上——模态语义要求背景不可交互
      if (event.key === 'Tab') {
        const card = okButtonRef.current?.closest('[role="dialog"]');
        if (!card) return;
        const focusables = Array.from(
          card.querySelectorAll<HTMLElement>('button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'),
        ).filter((el) => !el.hasAttribute('disabled'));
        if (focusables.length === 0) return;
        const first = focusables[0]!;
        const last = focusables[focusables.length - 1]!;
        const active = document.activeElement;
        if (event.shiftKey && (active === first || !card.contains(active))) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && (active === last || !card.contains(active))) {
          event.preventDefault();
          first.focus();
        }
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [isMounted, isExiting, onClose]);

  if (!isMounted) return null;

  return (
    <>
      {/* 遮罩：淡化下层 + 阻断交互（与窗口版 Rust 侧广播驱动的遮罩同款）。
          点击不关闭——对话框只能由 OK 按钮（或 Esc/Enter）收尾 */}
      <div
        aria-hidden="true"
        className={`fixed inset-0 z-[95] cursor-default bg-black/45 transition-opacity duration-[320ms] ${
          isExiting ? 'opacity-0' : 'opacity-100'
        }`}
      />
      <div className="pointer-events-none fixed inset-0 z-[96] flex items-center justify-center">
        <div className="pointer-events-auto">
          <AboutCard
            isExiting={isExiting}
            onClose={onClose}
            okButtonRef={okButtonRef}
            showCloseButton
          />
        </div>
      </div>
    </>
  );
}
