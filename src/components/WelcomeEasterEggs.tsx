/**
 * 主页彩蛋轮播（俏皮四件套，30s 自动换台 + 点击下一套 + 圆点跳选）：
 *
 * 1. **佛祖保佑 · 永无 BUG**——程序员的经典护身符 ASCII（社区流传原版），
 *    逐行呼吸浮动（stagger 相位差形成波纹）+ 眼睛行定时眨眼；
 * 2. **cowsay 奶牛**——终端文化鼻祖级彩蛋：经典奶牛 + 随机程序员语录
 *    （每次轮到它换一条），眨眼 + 上台「哞~」一声；
 * 3. **sl 小火车**——把 ls 敲成 sl 的传说级惩罚，真·跑马灯：全编组从右
 *    向左横穿循环（translateX 动画）；
 * 4. **Matrix 代码雨**——canvas 手写（透明底、绿色字符雨、12fps 终端
 *    质感），激活时才挂载，rAF 随卸载即停。
 *
 * 所有集共用：淡入过渡、prefers-reduced-motion 降级（佛祖/奶牛静止、
 * 小火车停靠、代码雨画一帧静帧）。文案走 i18n 双语。
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { cn } from '@/lib/utils';

/** 换台间隔（跑马灯节奏） */
const ROTATE_MS = 30_000;

// ── 1. 佛祖（形态经 node 打印逐行核对） ─────────────────────────────────────

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
const BUDDHA_EYES_LINE = 2;
const BUDDHA_BLINK_LINE = '                  88" - "88';

/** 通用眨眼 hook：约 everyMs 闭眼 closeMs 毫秒 */
function useBlink(everyMs: number, closeMs: number) {
  const [blink, setBlink] = useState(false);
  useEffect(() => {
    let closeTimer = 0;
    const id = window.setInterval(() => {
      setBlink(true);
      closeTimer = window.setTimeout(() => setBlink(false), closeMs);
    }, everyMs);
    return () => {
      window.clearInterval(id);
      window.clearTimeout(closeTimer);
    };
  }, [everyMs, closeMs]);
  return blink;
}

function BuddhaArt() {
  const blink = useBlink(4200, 160);
  return (
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
  );
}

// ── 2. cowsay 奶牛 ─────────────────────────────────────────────────────────

const COW_LINES = [
  '        \\   ^__^',
  '         \\  (oo)\\_______',
  '            (__)\\       )\\/\\',
  '                ||----w |',
  '                ||     ||',
];
const COW_BLINK_LINES = [
  '        \\   ^__^',
  '         \\  (..)\\_______',
  '            (__)\\       )\\/\\',
  '                ||----w |',
  '                ||     ||',
];

/** cowsay 对话框：多行时首行 / 尾行 \ 边框（单行即 / \），宽度取最长行 */
function speechBubble(text: string): string[] {
  const wrapped: string[] = [];
  for (const seg of text.split('\n')) {
    for (let i = 0; i < seg.length; i += 34) {
      wrapped.push(seg.slice(i, i + 34));
    }
    if (seg.length === 0) wrapped.push('');
  }
  const w = Math.max(...wrapped.map((l) => l.length), 4);
  const border = (ch: string) => ' ' + ch.repeat(w + 2);
  const body = wrapped.map((l, i) => {
    const lt = i === 0 ? '/' : '|';
    const rt = i === wrapped.length - 1 ? '\\' : '|';
    return ` ${lt} ${l.padEnd(w)} ${rt}`;
  });
  return [border('_'), ...body, border('-')];
}

function CowArt() {
  const { t } = useTranslation();
  // 每次轮到奶牛上台换一条语录（挂载即随机）
  const quotes = useMemo(() => t('welcome.cowQuotes').split('\n').filter(Boolean), [t]);
  const quote = useMemo(
    () => quotes[Math.floor(Math.random() * quotes.length)] ?? '',
    [quotes],
  );
  const blink = useBlink(5200, 150);
  const [moo, setMoo] = useState(true);
  useEffect(() => {
    const id = window.setTimeout(() => setMoo(false), 900);
    return () => window.clearTimeout(id);
  }, []);
  const art = blink ? COW_BLINK_LINES : COW_LINES;
  return (
    <div className="flex flex-col items-center font-mono text-[10px] leading-[1.35] text-foreground-subtle sm:text-[11px]">
      {speechBubble(quote).map((line, i) => (
        <span key={`b${i}`} className="whitespace-pre">
          {line}
        </span>
      ))}
      {art.map((line, i) => (
        <span key={`c${i}`} className="whitespace-pre">
          {line}
        </span>
      ))}
      <span
        className={cn(
          'mt-1 text-terminal-green transition-opacity duration-500',
          moo ? 'opacity-100' : 'opacity-0',
        )}
      >
        {t('welcome.cowMoo')}
      </span>
    </div>
  );
}

// ── 3. sl 小火车（真·跑马灯：全编组右进左出循环横穿） ────────────────────────

const TRAIN_LINES = [
  '      ====        ________                ___________',
  '  _D_|  |_______/        \\__I_I_____===__|_________|',
  '   |(_)---  |   H\\________/ |   |        =|___ ___|',
  '   /     |  |   H  |  |     |   |         ||_| |_|||',
  '  |      |  |   H  |__--------------------| [___] |',
  '  | ________|___H__/__|_____/[][]~\\_______|       |',
  '  |/ |   |-----------I_____I [][] []  D   |=======|__',
  '__/ =| o |=-~~\\  /~~\\  /~~\\  /~~\\ ____Y___________|__',
  '|/-=|___|=    O=====O=====O=====O|_____/~\\___/',
  ' \\_/      \\__/  \\__/  \\__/  \\__/      \\_/ \\_/',
];

function TrainArt() {
  return (
    <div className="w-full max-w-2xl overflow-hidden">
      <div className="welcome-train-track flex flex-col font-mono text-[9px] leading-[1.3] whitespace-pre text-foreground-subtle sm:text-[10px]">
        {TRAIN_LINES.map((line, i) => (
          <span key={i}>{line}</span>
        ))}
      </div>
    </div>
  );
}

// ── 4. Matrix 代码雨（canvas，透明底；激活时挂载、卸载即停） ─────────────────

function MatrixRain() {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = ref.current;
    const ctx = canvas?.getContext('2d');
    if (!canvas || !ctx) return;
    const W = (canvas.width = 520);
    const H = (canvas.height = 170);
    const fontSize = 12;
    const cols = Math.floor(W / fontSize);
    const chars = '01<>{}/$#;=+*'.split('');
    const heads = Array.from({ length: cols }, () => Math.random() * (H / fontSize));
    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    let raf = 0;
    let last = 0;
    const draw = (ts: number) => {
      if (ts - last > 80) {
        // ~12fps：终端质感的断续感
        last = ts;
        ctx.clearRect(0, 0, W, H);
        ctx.font = `${fontSize}px ui-monospace, monospace`;
        heads.forEach((y, c) => {
          for (let tail = 0; tail < 6; tail += 1) {
            const ty = y - tail;
            if (ty < 0) continue;
            const alpha = tail === 0 ? 0.95 : 0.55 - tail * 0.08;
            ctx.fillStyle = `rgba(70, 191, 114, ${alpha})`;
            ctx.fillText(
              chars[Math.floor(Math.random() * chars.length)] ?? '0',
              c * fontSize,
              ty * fontSize,
            );
          }
          heads[c] = (heads[c] ?? 0) + 1;
          if ((heads[c] ?? 0) * fontSize > H + 6 && Math.random() > 0.975) {
            heads[c] = 0;
          }
        });
      }
      raf = requestAnimationFrame(draw);
    };
    if (reduced) {
      // 降级：画一帧静帧（无 rAF 循环）
      last = -1000;
      draw(0);
      cancelAnimationFrame(raf);
    } else {
      raf = requestAnimationFrame(draw);
    }
    return () => cancelAnimationFrame(raf);
  }, []);
  return <canvas ref={ref} className="max-w-full" aria-hidden="true" />;
}

// ── 轮播容器 ───────────────────────────────────────────────────────────────

const SETS = ['buddha', 'cow', 'train', 'matrix'] as const;
type EggSet = (typeof SETS)[number];

/** 主页彩蛋轮播：30s 自动换台、点击整个区域下一套、圆点跳选 */
export function WelcomeEasterEggs() {
  const { t } = useTranslation();
  const [idx, setIdx] = useState(0);
  useEffect(() => {
    const id = window.setInterval(() => {
      setIdx((i) => (i + 1) % SETS.length);
    }, ROTATE_MS);
    return () => window.clearInterval(id);
  }, []);

  const captions: Record<EggSet, string> = {
    buddha: t('welcome.buddha'),
    cow: t('welcome.cowCaption'),
    train: t('welcome.trainCaption'),
    matrix: t('welcome.matrixCaption'),
  };
  const current = SETS[idx] ?? 'buddha';

  return (
    // 点击换台（整个彩蛋区都是热区）；圆点各自 stopPropagation 跳选
    <div
      className="flex w-full flex-col items-center gap-3"
      onClick={() => setIdx((i) => (i + 1) % SETS.length)}
      role="button"
      tabIndex={0}
      aria-label={t('welcome.eggSwitch')}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') setIdx((i) => (i + 1) % SETS.length);
      }}
    >
      {/* 固定高舞台：四套切换不跳布局 */}
      <div className="welcome-egg-stage flex h-[290px] w-full max-w-2xl cursor-pointer items-center justify-center overflow-hidden">
        <div key={current} className="welcome-egg-enter flex flex-col items-center gap-3">
          {current === 'buddha' ? <BuddhaArt /> : null}
          {current === 'cow' ? <CowArt /> : null}
          {current === 'train' ? <TrainArt /> : null}
          {current === 'matrix' ? <MatrixRain /> : null}
          <p className="text-ui-xs tracking-[0.3em] text-foreground-subtlest">
            {captions[current]}
          </p>
        </div>
      </div>
      <div className="flex items-center gap-1.5">
        {SETS.map((s, i) => (
          <button
            key={s}
            type="button"
            aria-label={`${t('welcome.eggSwitch')} ${i + 1}`}
            onClick={(e) => {
              e.stopPropagation();
              setIdx(i);
            }}
            className={cn(
              'size-1.5 rounded-full transition-colors',
              i === idx ? 'bg-foreground-subtle' : 'bg-foreground-subtlest/50 hover:bg-foreground-subtlest',
            )}
          />
        ))}
      </div>
    </div>
  );
}
