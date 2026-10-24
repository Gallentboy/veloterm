import { SquareTerminal, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useAppVersion } from '@/hooks/useAppVersion';
import { cn } from '@/lib/utils';

/** 构建时间展示本地时区紧凑格式（与 ZCode formatAboutBuildTime 一致） */
export function formatBuildTime(iso: string): string {
  const parsed = new Date(iso);
  if (Number.isNaN(parsed.getTime())) {
    return iso;
  }
  const pad = (value: number) => `${value}`.padStart(2, '0');
  return `${parsed.getFullYear()}-${pad(parsed.getMonth() + 1)}-${pad(parsed.getDate())} ${pad(parsed.getHours())}:${pad(parsed.getMinutes())}`;
}

interface AboutCardProps {
  /** 退场动画中（卡片滑出） */
  isExiting?: boolean;
  /** 点确定 / Esc / Enter 时调用 */
  onClose: () => void;
  okButtonRef?: React.RefObject<HTMLButtonElement | null>;
  /**
   * 显示右上角关闭按钮（X）。Windows 的纯前端弹层需要（窗口版有关闭路径
   * 之外的确定键即可，但模态弹层习惯上要给 X）；macOS 独立窗口不显示。
   */
  showCloseButton?: boolean;
}

/**
 * 「关于」卡片本体（纯视觉 + 数据，不含任何窗口 API）。
 *
 * 两种宿主复用：
 * - macOS：`AboutWindow` 渲染在独立无边框透明子窗口里（Rust 侧 about.rs 建窗）；
 * - Windows：`AboutOverlay` 直接渲染在主窗口内的遮罩层上（纯前端弹层，
 *   绕开 WebView2 透明分层窗口的合成问题）。
 *
 * 布局/配色逐项对应 ZCode 的 .about-card（padding 22/15/14、内容宽 222、
 * tabular-nums 版本行、胶囊确定键），主题跟随 html.dark。
 */
export function AboutCard({ isExiting = false, onClose, okButtonRef, showCloseButton = false }: AboutCardProps) {
  const { t } = useTranslation();
  const { version, tauriVersion, webkitVersion, reactVersion, isAppleSilicon } = useAppVersion();

  const runtimeRows = [
    { label: 'Tauri', value: tauriVersion || '—' },
    { label: 'WebKit', value: webkitVersion },
    { label: 'React', value: reactVersion },
  ];

  return (
    <div
      className={cn(
        'relative flex h-auto w-64 flex-col overflow-hidden rounded-[12px] bg-[#f4f4f5] p-[22px] pb-3.5 pl-[15px] text-[#1d1d1f] select-none shadow-[0_18px_38px_-10px_rgb(0_0_0/0.38),0_6px_16px_-6px_rgb(0_0_0/0.22)] dark:bg-[#171717] dark:text-[#e8e8e8] dark:shadow-[0_18px_38px_-10px_rgb(0_0_0/0.55),0_6px_16px_-6px_rgb(0_0_0/0.4)]',
        isExiting ? 'about-card-out' : 'about-card-in',
      )}
      role="dialog"
      aria-modal="true"
      aria-label={t('help.about')}
    >
      {showCloseButton ? (
        <button
          type="button"
          onClick={onClose}
          aria-label={t('about.close')}
          className="absolute top-2 right-2 flex size-6 items-center justify-center rounded-md text-[#8a8a8e] transition-colors hover:bg-black/[0.06] hover:text-[#1d1d1f] dark:text-[#9a9a9e] dark:hover:bg-white/10 dark:hover:text-[#e8e8e8]"
        >
          <X className="size-3.5" strokeWidth={2} />
        </button>
      ) : null}
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
        onClick={onClose}
        className="mt-5 h-9 w-full shrink-0 rounded-full bg-[#0a0a0a] text-[13px] font-medium text-[#fafafa] outline-none active:opacity-80 dark:bg-[#fafafa] dark:text-[#0a0a0a]"
      >
        {t('about.ok')}
      </button>
    </div>
  );
}
