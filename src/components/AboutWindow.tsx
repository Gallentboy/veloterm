import { SquareTerminal } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { emit } from '@tauri-apps/api/event';
import { useAppVersion } from '@/hooks/useAppVersion';
import { cn } from '@/lib/utils';

/** 出场动画时长（与 styles.css 的 .about-card-out 保持一致，关窗延时对齐） */
const ABOUT_EXIT_ANIMATION_MS = 150;

/** 构建时间展示本地时区紧凑格式（与 ZCode formatAboutBuildTime 一致） */
function formatBuildTime(iso: string): string {
  const parsed = new Date(iso);
  if (Number.isNaN(parsed.getTime())) {
    return iso;
  }
  const pad = (value: number) => `${value}`.padStart(2, '0');
  return `${parsed.getFullYear()}-${pad(parsed.getMonth() + 1)}-${pad(parsed.getDate())} ${pad(parsed.getHours())}:${pad(parsed.getMinutes())}`;
}

/**
 * 「关于」独立窗口内容（对齐 ZCode aboutWindow.ts 的自绘卡片）。
 *
 * 窗口本体是无边框透明子窗口（Rust 侧 about.rs 创建，架构同 ZCode 的
 * Electron frameless+transparent），本组件只负责卡片：
 * - 根面板 256×340 铺满窗口，CSS 圆角 12px（ZCode 的面板圆角语言，
 *   desktopWindowChrome「内层 12px 面板」），透明窗口负责露出四角；
 * - 布局/配色逐项对应 ZCode 的 .about-card（padding 22/15/14、内容宽 222、
 *   tabular-nums 版本行、胶囊确定键），主题跟随 html.dark；
 * - 整卡可拖（data-tauri-drag-region，同 ZCode -webkit-app-region: drag），
 *   OK/Escape/Enter 关闭窗口。
 */
export function AboutWindow() {
  const { t } = useTranslation();
  const { version, tauriVersion, webkitVersion, reactVersion, isAppleSilicon } = useAppVersion();
  const okButtonRef = useRef<HTMLButtonElement>(null);
  const [isExiting, setIsExiting] = useState(false);

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

  const runtimeRows = [
    { label: 'Tauri', value: tauriVersion || '—' },
    { label: 'WebKit', value: webkitVersion },
    { label: 'React', value: reactVersion },
  ];

  return (
    // 透明出血容器：窗口比卡片大 22px×2，给卡片投影留绘制空间（窗口内不会被裁），
    // 整个出血区也允许拖动窗口
    <div
      data-tauri-drag-region
      className="flex h-full w-full items-center justify-center p-[22px]"
    >
      <div
        data-tauri-drag-region
        className={cn(
          'flex h-[340px] w-64 flex-col overflow-hidden rounded-[12px] bg-[#f4f4f5] p-[22px] pb-3.5 pl-[15px] text-[#1d1d1f] select-none shadow-[0_18px_38px_-10px_rgb(0_0_0/0.38),0_6px_16px_-6px_rgb(0_0_0/0.22)] dark:bg-[#171717] dark:text-[#e8e8e8] dark:shadow-[0_18px_38px_-10px_rgb(0_0_0/0.55),0_6px_16px_-6px_rgb(0_0_0/0.4)]',
          isExiting ? 'about-card-out' : 'about-card-in',
        )}
        role="dialog"
        aria-modal="true"
        aria-label={t('help.about')}
      >
      <div className="mx-auto flex w-full max-w-[222px] min-h-0 flex-1 flex-col">
        <div
          aria-hidden="true"
          className="flex size-[52px] items-center justify-center rounded-xl border border-white/10 bg-gradient-to-b from-black to-[#151718] text-white shadow-[0_10px_13px_-3px_rgb(0_0_0/0.2),0_4px_5px_-3px_rgb(0_0_0/0.2)]"
        >
          <SquareTerminal className="size-8" strokeWidth={1.5} />
        </div>

        <h1 className="mt-4 text-[13.5px] leading-[1.18] font-bold tracking-normal">
          {t('app.name')} {t('about.version')} {version}
        </h1>

        <dl className="mt-[13px] flex flex-col gap-1.5 border-y border-black/[0.07] py-2.5 text-xs leading-[1.25] tabular-nums dark:border-white/10">
          {runtimeRows.map((row) => (
            <div key={row.label} className="grid grid-cols-[auto_1fr] items-baseline gap-2.5">
              <dt className="text-[#8a8a8e] dark:text-[#9a9a9e]">{row.label}</dt>
              <dd className="m-0 text-right">{row.value}</dd>
            </div>
          ))}
        </dl>

        <div className="mt-[13px] flex flex-col gap-2.5 text-[13px] leading-[1.2] tabular-nums text-[#303033] dark:text-[#e2e2e2]">
          <div>
            <span className="text-[#8a8a8e] dark:text-[#9a9a9e]">{t('about.buildTime')}</span>{' '}
            {formatBuildTime(__BUILD_TIME__)}
          </div>
          {isAppleSilicon ? <div>{t('about.appleSilicon')}</div> : null}
          {/* HarmonyOS Sans 字体授权要求：须在软件内显著标注所用字体 */}
          <div>{t('about.fontNotice')}</div>
        </div>
      </div>

      <button
        ref={okButtonRef}
        type="button"
        onClick={closeWithAnimation}
        className="mt-auto h-9 w-full rounded-full bg-[#0a0a0a] text-[13px] font-medium text-[#fafafa] outline-none active:opacity-80 dark:bg-[#fafafa] dark:text-[#0a0a0a]"
      >
        {t('about.ok')}
      </button>
      </div>
    </div>
  );
}
