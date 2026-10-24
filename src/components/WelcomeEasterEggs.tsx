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
    // 左右渐隐遮罩：进出场的车体在容器边缘融化而非硬切边
    <div className="welcome-train-mask w-full max-w-2xl overflow-hidden">
      <div className="welcome-train-track flex flex-col font-mono text-[9px] leading-[1.3] whitespace-pre text-foreground-subtle sm:text-[10px]">
        {TRAIN_LINES.map((line, i) => (
          <span key={i}>{line}</span>
        ))}
      </div>
    </div>
  );
}

// ── 4. Matrix 代码雨（透明画布直叠页面背景：无面板、无边框、无底色） ─────

function MatrixRain() {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = ref.current;
    const ctx = canvas?.getContext('2d');
    if (!canvas || !ctx) return;
    const W = (canvas.width = 520);
    const H = (canvas.height = 190);
    const fontSize = 13;
    const cols = Math.floor(W / fontSize);
    const rows = Math.floor(H / fontSize);
    const chars = '01<>{}/$#;=+*'.split('');
    const randChar = () => chars[Math.floor(Math.random() * chars.length)] ?? '0';
    // 透明底做不了覆盖淡出，改为逐格管理生命期：每个格子记住字符与透明度，
    // 头格亮、身后格子原地渐隐（字符不换——真雨的拖尾是被记住的字符）
    type Cell = { ch: string; alpha: number; head: boolean };
    const grid: (Cell | null)[][] = Array.from({ length: cols }, () =>
      Array<Cell | null>(rows).fill(null),
    );
    const heads = Array.from({ length: cols }, () => Math.floor(Math.random() * -40));
    const speeds = Array.from({ length: cols }, () => 0.3 + Math.random() * 0.7);
    const tick = () => {
      for (const col of grid) {
        for (let r = 0; r < rows; r += 1) {
          const cell = col[r];
          if (!cell) continue;
          cell.alpha -= 0.013;
          if (cell.alpha <= 0.02) col[r] = null;
        }
      }
      heads.forEach((h, c) => {
        if (Math.random() > (speeds[c] ?? 0.5)) return; // 本帧该列不推进（速度差异）
        const nh = h + 1;
        if (nh >= 0 && nh < rows) {
          const prev = grid[c]?.[h];
          if (prev) prev.head = false; // 头走过后降级为普通拖尾格
          grid[c]![nh] = { ch: randChar(), alpha: 0.95, head: true };
        }
        heads[c] = nh;
        if (nh > rows + 10 && Math.random() > 0.9) {
          heads[c] = Math.floor(Math.random() * -30); // 出屏后随机延迟再落
        }
      });
    };
    const draw = () => {
      ctx.clearRect(0, 0, W, H);
      ctx.font = `${fontSize}px ui-monospace, monospace`;
      ctx.textBaseline = 'top';
      for (let c = 0; c < cols; c += 1) {
        for (let r = 0; r < rows; r += 1) {
          const cell = grid[c]?.[r];
          if (!cell) continue;
          ctx.fillStyle = cell.head
            ? 'rgba(170, 255, 205, 0.95)'
            : `rgba(70, 191, 114, ${Math.max(cell.alpha, 0).toFixed(3)})`;
          ctx.fillText(cell.ch, c * fontSize, r * fontSize);
        }
      }
    };
    let raf = 0;
    let last = 0;
    const loop = (ts: number) => {
      if (ts - last > 45) {
        last = ts;
        tick();
        draw();
      }
      raf = requestAnimationFrame(loop);
    };
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
      // 降级：随机铺一屏静帧拖尾，不动画
      tick();
      tick();
      tick();
      draw();
    } else {
      raf = requestAnimationFrame(loop);
    }
    return () => cancelAnimationFrame(raf);
  }, []);
  // 透明画布直出：无底色、无边框、无圆角——雨直接落在页面背景上
  return <canvas ref={ref} className="max-w-full" aria-hidden="true" />;
}

// ── 5. VeloTerm 打字机（程序内建 5 行像素字体拼横幅，逐列打印） ────────

/** 仅收录所需字母的 5×5 像素字体（# 为墨点）：拼 "VELOTERM" 横幅 */
const BANNER_FONT: Record<string, string[]> = {
  V: ['#   #', '#   #', '#   #', ' # # ', '  #  '],
  E: ['#####', '#    ', '#### ', '#    ', '#####'],
  L: ['#    ', '#    ', '#    ', '#    ', '#####'],
  O: [' ### ', '#   #', '#   #', '#   #', ' ### '],
  T: ['#####', '  #  ', '  #  ', '  #  ', '  #  '],
  R: ['#### ', '#   #', '#### ', '#  # ', '#   #'],
  M: ['#   #', '## ##', '# # #', '#   #', '#   #'],
};
const BANNER_TEXT = 'VELOTERM';
const BANNER_GAP = 2;

/** 拼横幅：每字母 5 列 + 间隔列，逐行拼接 */
function buildBanner(): string[] {
  const rows = ['','','','',''];
  for (const ch of BANNER_TEXT) {
    const glyph = BANNER_FONT[ch] ?? ['','','','',''];
    for (let r = 0; r < 5; r += 1) {
      rows[r] = (rows[r] ?? '') + (glyph[r] ?? '') + ' '.repeat(BANNER_GAP);
    }
  }
  return rows;
}

function TypewriterBanner() {
  const banner = useMemo(buildBanner, []);
  const width = banner[0]?.length ?? 0;
  const [cols, setCols] = useState(0);
  const [done, setDone] = useState(false);
  const [cursorOn, setCursorOn] = useState(true);

  useEffect(() => {
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
      setCols(width);
      setDone(true);
      return;
    }
    // 逐列打印（~45ms/列，像点阵打印机扫描）；写完 8s 后重打
    let replay = 0;
    const id = window.setInterval(() => {
      setCols((c) => {
        if (c >= width) {
          window.clearInterval(id);
          setDone(true);
          replay = window.setTimeout(() => {
            setDone(false);
            setCols(0);
          }, 8000);
          return c;
        }
        return c + 1;
      });
    }, 45);
    const blink = window.setInterval(() => setCursorOn((v) => !v), 530);
    return () => {
      window.clearInterval(id);
      window.clearInterval(blink);
      window.clearTimeout(replay);
    };
  }, [width]);

  const printRow = Math.min(2, 4); // 打印头光标块所在的行（中间行）
  return (
    <div className="flex flex-col font-mono text-[10px] leading-[1.4] text-terminal-green sm:text-[11px]">
      {banner.map((line, i) => (
        <span key={i} className="whitespace-pre">
          {line.slice(0, cols)}
          {!done && i === printRow && cols < width ? (
            <span className="text-terminal-green">█</span>
          ) : null}
        </span>
      ))}
      {/* 写完：终端提示符 + 闪烁光标 */}
      {done ? (
        <span className="mt-1 whitespace-pre text-foreground-subtle">
          {'~$ '}
          <span className={cursorOn ? 'opacity-100' : 'opacity-0'}>█</span>
        </span>
      ) : null}
    </div>
  );
}

// ── 轮播容器 ───────────────────────────────────────────────────────────────

const SETS = ['buddha', 'cow', 'train', 'matrix', 'banner'] as const;
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
    banner: t('welcome.bannerCaption'),
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
          {current === 'banner' ? <TypewriterBanner /> : null}
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
