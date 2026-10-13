import { SquareTerminal } from 'lucide-react';
import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useAppVersion } from '@/hooks/useAppVersion';

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
 * 关于弹窗（移植自 ZCode aboutWindow：256×340 自绘卡片）。
 * 名称+版本同一行、运行时版本三行（tabular-nums 对齐）、构建时间与 Apple Silicon 说明、
 * 圆角胶囊确定按钮；OK/Escape/Enter 关闭。跟随应用主题（浅色 #f4f4f5 / 深色 #171717）。
 */
export function AboutDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t } = useTranslation();
  const { version, tauriVersion, webkitVersion, reactVersion, isAppleSilicon } = useAppVersion();
  const okButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    okButtonRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' || event.key === 'Enter') {
        onClose();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [open, onClose]);

  if (!open) {
    return null;
  }

  const runtimeRows = [
    { label: 'Tauri', value: tauriVersion || '—' },
    { label: 'WebKit', value: webkitVersion },
    { label: 'React', value: reactVersion },
  ];

  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/25"
      role="dialog"
      aria-modal="true"
      aria-label={t('help.about')}
    >
      <section className="flex h-[340px] w-64 flex-col rounded-lg bg-[#f4f4f5] p-[22px] pb-3.5 pl-[15px] text-[#1d1d1f] shadow-2xl ring-1 ring-black/5 select-none dark:bg-[#171717] dark:text-[#e8e8e8]">
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
          </div>
        </div>

        <button
          ref={okButtonRef}
          type="button"
          onClick={onClose}
          className="mt-auto h-9 w-full rounded-full bg-[#0a0a0a] text-[13px] font-medium text-[#fafafa] outline-none active:opacity-80 dark:bg-[#fafafa] dark:text-[#0a0a0a]"
        >
          {t('about.ok')}
        </button>
      </section>
    </div>
  );
}
