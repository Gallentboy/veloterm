import { ChevronRight, SquareTerminal } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { PRIMARY_MODIFIER_LABEL } from '@/lib/keyboard';
import { cn } from '@/lib/utils';

/** 佛祖保佑字符画（程序员的经典护身符，形态对齐社区流传原版；
 * 转义校准经 node 打印逐行核对） */
const BUDDHA_LINES = [
  '                   _ooOoo_',
  '                  o8888888o',
  '                  88" . "88',
  '                  (| -_- |)',
  '                  O\\  =  /O',
  "               ____/`---'\\____",
  "             .'  \\\\|     |//  `.",
  '            /  \\\\|||  :  |||//  \\',
  '           /  _||||| -:- |||||-  \\',
  '           |   | \\\\\\  -  /// |   |',
  '           | \\_|  \'\'\\---/\'\'  |   |',
  '           \\  .-\\__  `-`  ___/-. /',
  '         ___`. .\'  /--.--\\  `. . __',
  '      ."" \'<  `.___\\_<|>_/___.\'  >\'"".',
  '     | | :  `- \\`.;\\ _ /`;.`/ - ` : | |',
  '     \\  \\ `-.   \\_ __\\ /__ _/   .-` /  /',
  '======`-.____`-.___\\_____/___.-`____.-\'======',
  '                   `=---=\'',
];
/** 眼睛所在行（88" . "88）与眨眼形态 */
const BUDDHA_EYES_LINE = 2;
const BUDDHA_BLINK_LINE = '                  88" - "88';

function ShortcutHint({
  keys,
  label,
  onClick,
}: {
  /** 快捷键（可空：无键位的入口右侧给右箭头引导） */
  keys?: string[];
  label: string;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={!onClick}
      className="flex w-full items-center justify-between gap-3 whitespace-nowrap rounded-md px-3 py-2.5 text-left text-ui-base text-foreground-subtle transition-colors hover:bg-surface disabled:cursor-default disabled:opacity-60"
    >
      <span>{label}</span>
      <span className="flex shrink-0 items-center gap-1">
        {keys ? (
          keys.map((key) => (
            <kbd
              key={key}
              className="rounded border border-border bg-surface px-1.5 py-0.5 font-mono text-ui-sm text-foreground"
            >
              {key}
            </kbd>
          ))
        ) : (
          <ChevronRight className="size-3.5 text-foreground-subtlest" />
        )}
      </span>
    </button>
  );
}

/** 主页欢迎内容（空状态样式参考 ZCode ChatEmptyState / WelcomeScreen）。 */
export function WelcomeScreen({
  onNewTerminal,
  onOpenSettings,
  onNavigateHosts,
  onOpenCommandPalette,
  onOpenManual,
}: {
  onNewTerminal?: () => void;
  onOpenSettings?: () => void;
  /** 去主机页连接服务器（含 SSH 多跳/中继） */
  onNavigateHosts?: () => void;
  onOpenCommandPalette?: () => void;
  /** 打开使用手册（F1 同路径） */
  onOpenManual?: () => void;
}) {
  const { t } = useTranslation();

  // 佛祖眨眼：约 4s 一次，每次 160ms（眼睛行的两个 . 短暂变 -）
  const [blink, setBlink] = useState(false);
  useEffect(() => {
    let closeTimer = 0;
    const id = window.setInterval(() => {
      setBlink(true);
      closeTimer = window.setTimeout(() => setBlink(false), 160);
    }, 4200);
    return () => {
      window.clearInterval(id);
      window.clearTimeout(closeTimer);
    };
  }, []);

  return (
    <div className="flex h-full flex-col items-center justify-center gap-7 overflow-y-auto px-8 py-10 scrollbar-slim select-none">
      <div className="flex flex-col items-center gap-2 text-center">
        <div className="mb-1 flex size-16 items-center justify-center rounded-2xl border border-border bg-surface">
          <SquareTerminal className="size-8 text-foreground" strokeWidth={1.5} />
        </div>
        <h1 className="text-ui-xl font-semibold tracking-tight text-foreground">VeloTerm</h1>
        <p className="text-ui-base text-foreground-subtle">{t('welcome.subtitle')}</p>
      </div>

      <div className={cn('w-full max-w-md overflow-hidden rounded-lg border border-border')}>
        <div className="border-b border-border/50 bg-surface px-3.5 py-2 text-ui-sm text-foreground-subtlest">
          {t('welcome.getStarted')}
        </div>
        <div className="flex flex-col p-1">
          <ShortcutHint
            keys={[PRIMARY_MODIFIER_LABEL, 'N']}
            label={t('welcome.newTerminal')}
            onClick={onNewTerminal}
          />
          <ShortcutHint
            label={t('welcome.connectHosts')}
            onClick={onNavigateHosts}
          />
          <ShortcutHint
            keys={[PRIMARY_MODIFIER_LABEL, 'K']}
            label={t('welcome.commandPalette')}
            onClick={onOpenCommandPalette}
          />
          <ShortcutHint keys={['F1']} label={t('welcome.manual')} onClick={onOpenManual} />
          <ShortcutHint
            keys={[PRIMARY_MODIFIER_LABEL, ',']}
            label={t('welcome.openSettings')}
            onClick={onOpenSettings}
          />
        </div>
      </div>

      {/* 佛祖保佑 · 永无 BUG：逐行呼吸浮动（stagger 相位差形成波纹）+ 眨眼，
          动效类与 prefers-reduced-motion 降级见 styles.css */}
      <div className="flex w-full flex-col items-center gap-3" aria-hidden="true">
        <div className="flex flex-col font-mono text-[10px] leading-[1.35] text-foreground-subtle sm:text-[11px]">
          {BUDDHA_LINES.map((line, i) => (
            <span
              key={i}
              className="welcome-ascii-line whitespace-pre"
              style={{ animationDelay: `${i * 110}ms` }}
            >
              {i === BUDDHA_EYES_LINE && blink ? BUDDHA_BLINK_LINE : line}
            </span>
          ))}
        </div>
        <p className="text-ui-xs tracking-[0.35em] text-foreground-subtlest">
          {t('welcome.buddha')}
        </p>
      </div>
    </div>
  );
}
