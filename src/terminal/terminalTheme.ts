import type { ITheme } from '@xterm/xterm';

function readTerminalColor(style: CSSStyleDeclaration, name: string, fallback: string): string {
  return style.getPropertyValue(name).trim() || fallback;
}

/**
 * xterm 的 `css.toColor`（尤其 WebGL addon）不认 `color-mix()`、`var()`、现代 `rgb(r g b / a)` 语法。
 * 这里先用隐藏元素让浏览器解析掉 `var()/color-mix()`，再通过 canvas 将结果规范化为
 * xterm 能解析的 `rgba(r, g, b, a)` 字符串（移植自 ZCode terminalTheme.ts）。
 */
const colorResolverEl: HTMLSpanElement | null =
  typeof document === 'undefined' ? null : document.createElement('span');

const colorNormalizeCtx: CanvasRenderingContext2D | null = (() => {
  if (typeof document === 'undefined') return null;
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 1;
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  if (ctx) ctx.globalCompositeOperation = 'copy';
  return ctx;
})();

function normalizeCssColor(raw: string, fallback: string): string {
  if (!raw || !colorResolverEl || !colorNormalizeCtx || !document.body) {
    return raw || fallback;
  }
  try {
    document.body.appendChild(colorResolverEl);
    colorResolverEl.style.color = '';
    colorResolverEl.style.color = raw;
    const resolved = getComputedStyle(colorResolverEl).color;
    colorNormalizeCtx.fillStyle = '#000';
    colorNormalizeCtx.fillStyle = resolved;
    colorNormalizeCtx.fillRect(0, 0, 1, 1);
    const [r = 0, g = 0, b = 0, a = 255] = colorNormalizeCtx.getImageData(0, 0, 1, 1).data;
    return `rgba(${r}, ${g}, ${b}, ${+(a / 255).toFixed(3)})`;
  } catch {
    return fallback;
  } finally {
    colorResolverEl.remove();
  }
}

const TERMINAL_THEME_TOKENS = {
  background: ['--color-terminal-bg', '#161616'],
  foreground: ['--color-terminal-fg', '#bdbdbd'],
  cursor: ['--color-terminal-cursor', '#f8f8f8'],
  cursorAccent: ['--color-terminal-cursor-accent', '#161616'],
  selectionBackground: ['--color-terminal-selection', 'rgba(64, 153, 255, 0.28)'],
  selectionInactiveBackground: ['--color-terminal-selection-inactive', 'rgba(255, 255, 255, 0.1)'],
  /* xterm 6 自绘滚动条：默认是 foreground 40-50% 透明度的大宽条，改为细胶囊主题色 */
  scrollbarSliderBackground: ['--color-terminal-scrollbar', 'rgba(255, 255, 255, 0.16)'],
  scrollbarSliderHoverBackground: ['--color-terminal-scrollbar-hover', 'rgba(255, 255, 255, 0.26)'],
  scrollbarSliderActiveBackground: ['--color-terminal-scrollbar-active', 'rgba(255, 255, 255, 0.34)'],
  black: ['--color-terminal-black', '#363636'],
  red: ['--color-terminal-red', '#ff5c5c'],
  green: ['--color-terminal-green', '#46bf72'],
  yellow: ['--color-terminal-yellow', '#ff8a30'],
  blue: ['--color-terminal-blue', '#4099ff'],
  magenta: ['--color-terminal-magenta', '#7b5ce5'],
  cyan: ['--color-terminal-cyan', '#42c8c8'],
  white: ['--color-terminal-white', '#adadad'],
  brightBlack: ['--color-terminal-bright-black', '#747474'],
  brightRed: ['--color-terminal-bright-red', '#ff9999'],
  brightGreen: ['--color-terminal-bright-green', '#87d9a4'],
  brightYellow: ['--color-terminal-bright-yellow', '#ffb26b'],
  brightBlue: ['--color-terminal-bright-blue', '#80beff'],
  brightMagenta: ['--color-terminal-bright-magenta', '#a888f2'],
  brightCyan: ['--color-terminal-bright-cyan', '#8ee5e5'],
  brightWhite: ['--color-terminal-bright-white', '#f8f8f8'],
} satisfies Partial<Record<keyof ITheme, readonly [string, string]>>;

/** 从 CSS 变量读取 terminal 主题色，跟随 dark/light 切换 */
export function getTerminalTheme(): ITheme {
  const style = getComputedStyle(document.documentElement);
  return Object.fromEntries(
    Object.entries(TERMINAL_THEME_TOKENS).map(([key, [tokenName, fallback]]) => [
      key,
      normalizeCssColor(readTerminalColor(style, tokenName, fallback), fallback),
    ]),
  ) as ITheme;
}

export function restrictInheritedTerminalTheme(
  profileTheme: ITheme | undefined,
): ITheme | undefined {
  if (!profileTheme) {
    return undefined;
  }

  const inheritedTheme = { ...profileTheme };
  delete inheritedTheme.background;
  delete inheritedTheme.foreground;
  delete inheritedTheme.cursor;
  delete inheritedTheme.cursorAccent;

  return Object.keys(inheritedTheme).length > 0 ? inheritedTheme : undefined;
}

export function mergeTerminalTheme(profileTheme?: ITheme): ITheme {
  const appTheme = getTerminalTheme();
  const inheritedTheme = restrictInheritedTerminalTheme(profileTheme);
  return inheritedTheme ? { ...appTheme, ...inheritedTheme } : appTheme;
}
