import * as monaco from 'monaco-editor';

/**
 * Monaco 主题：从 styles.css 的 CSS 令牌（--color-background / --color-foreground /
 * --color-terminal-* / --color-surface / --color-border ...）动态生成 'veloterm' 主题，
 * 使编辑器背景与卡片/终端融为一体；深浅主题切换由 MutationObserver 观察 html class 自动跟随。
 *
 * 转换手法与 src/terminal/terminalTheme.ts 一致：令牌可能含 var()/color-mix()/现代 rgb 语法，
 * 先用隐藏元素让浏览器解析、再经 canvas 规范化。注意 Monaco 的两套取色格式：
 * - themeData.colors 走 Color.fromHex：只认 `#RRGGBB` / `#RRGGBBAA`
 * - themeData.rules.foreground 走 `'#' + color`：必须是不带井号的 hex
 */

export const VELOTERM_MONACO_THEME = 'veloterm';

/* —— 以下 normalizeCssColor 与 terminalTheme.ts 的私有实现同源（复制保持模块独立） —— */

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

/* —— Monaco 专用格式转换 —— */

const RGBA_RE =
  /^rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)(?:[,\s/]+([\d.]+%?))?\s*\)$/;

function toHex2(value: number): string {
  return Math.max(0, Math.min(255, Math.round(value))).toString(16).padStart(2, '0');
}

/** 规范化后的 CSS 颜色 → `#rrggbb(aa)`（themeData.colors 要求的格式；fallback 本身须已是该格式） */
function toMonacoColor(normalized: string, fallback: string): string {
  const match = RGBA_RE.exec(normalized);
  if (match) {
    const [, r, g, b, a] = match;
    let alpha = 1;
    if (a !== undefined) {
      alpha = a.endsWith('%') ? parseFloat(a) / 100 : parseFloat(a);
      if (!Number.isFinite(alpha)) alpha = 1;
    }
    return `#${toHex2(+r)}${toHex2(+g)}${toHex2(+b)}${alpha >= 1 ? '' : toHex2(alpha * 255)}`;
  }
  if (/^#[0-9a-f]{6}([0-9a-f]{2})?$/i.test(normalized)) {
    return normalized.toLowerCase();
  }
  return fallback;
}

/** 在已规范化的 `#rrggbb(aa)` 上叠加透明度（diff 底色等衍生色用） */
function withAlpha(hexColor: string, alpha: number): string {
  const base = /^#[0-9a-f]{6}/i.exec(hexColor)?.[0];
  return base ? `${base}${toHex2(alpha * 255)}` : hexColor;
}

/** `#rrggbb(aa)` → 不带井号的 hex（themeData.rules.foreground 要求的格式） */
function toRuleForeground(hexColor: string): string {
  return hexColor.replace(/^#/, '');
}

function readToken(style: CSSStyleDeclaration, token: string, fallback: string): string {
  return toMonacoColor(normalizeCssColor(style.getPropertyValue(token).trim() || fallback, fallback), fallback);
}

/** 当前深浅模式下从令牌构建 veloterm 主题数据 */
function buildVeloTermThemeData(): monaco.editor.IStandaloneThemeData {
  const dark = document.documentElement.classList.contains('dark');
  const style = getComputedStyle(document.documentElement);
  const token = (name: string, fallback: string) => readToken(style, name, fallback);

  const background = token('--color-background', dark ? '#161616' : '#f8f8f8');
  const foreground = token('--color-foreground', dark ? '#bdbdbd' : '#262626');
  const border = token('--color-border', dark ? '#ffffff1a' : '#0d0d0d1a');
  const surface = token('--color-surface', dark ? '#ffffff0d' : '#0d0d0d08');
  const selection = token('--color-terminal-selection', '#4099ff47');
  const popover = token('--color-popover', dark ? '#2b2b2b' : '#ffffff');
  const green = token('--color-terminal-green', dark ? '#46bf72' : '#1e8a3e');
  const red = token('--color-terminal-red', dark ? '#ff5c5c' : '#e03131');
  const blue = token('--color-terminal-blue', dark ? '#4099ff' : '#0b7fff');
  const yellow = token('--color-terminal-yellow', dark ? '#ff8a30' : '#e07b00');
  const cyan = token('--color-terminal-cyan', dark ? '#42c8c8' : '#0aa7a7');
  const subtleGray = token('--color-terminal-bright-black', dark ? '#747474' : '#888888');

  return {
    base: dark ? 'vs-dark' : 'vs',
    inherit: true,
    rules: [
      { token: '', foreground: toRuleForeground(foreground) },
      { token: 'comment', foreground: toRuleForeground(subtleGray), fontStyle: 'italic' },
      { token: 'string', foreground: toRuleForeground(green) },
      { token: 'number', foreground: toRuleForeground(yellow) },
      { token: 'keyword', foreground: toRuleForeground(blue) },
      { token: 'type', foreground: toRuleForeground(cyan) },
      { token: 'type.identifier', foreground: toRuleForeground(cyan) },
      { token: 'identifier', foreground: toRuleForeground(foreground) },
      { token: 'function', foreground: toRuleForeground(yellow) },
      { token: 'delimiter', foreground: toRuleForeground(foreground) },
    ],
    colors: {
      'editor.background': background,
      'editor.foreground': foreground,
      'editorGutter.background': background,
      'editorLineNumber.foreground': subtleGray,
      'editorLineNumber.activeForeground': foreground,
      'editorCursor.foreground': token('--color-terminal-cursor', dark ? '#f8f8f8' : '#0d0d0d'),
      'editor.selectionBackground': selection,
      // 滚动条滑块：与 xterm/全局滚动条同一套令牌（细胶囊、随主题深浅）
      /* 查找替换（⌘F）高亮：当前命中用选区蓝，其余命中低透明度 */
      'editor.findMatchBackground': selection,
      'editor.findMatchHighlightBackground': token('--color-terminal-selection-inactive', dark ? '#ffffff1a' : '#0d0d0d1a'),
      'editor.findRangeHighlightBackground': token('--color-terminal-selection-inactive', dark ? '#ffffff1a' : '#0d0d0d1a'),
      'editorWidget.border': border,
      'scrollbarSlider.background': token('--color-terminal-scrollbar', dark ? '#ffffff29' : '#0d0d0d2e'),
      'scrollbarSlider.hoverBackground': token('--color-terminal-scrollbar-hover', dark ? '#ffffff42' : '#0d0d0d47'),
      'scrollbarSlider.activeBackground': token('--color-terminal-scrollbar-active', dark ? '#ffffff57' : '#0d0d0d5c'),
      'editor.lineHighlightBackground': surface,
      'editorIndentGuide.background1': border,
      'editorIndentGuide.activeBackground1': token('--color-border-hover', dark ? '#ffffff26' : '#0d0d0d26'),
      'editorBracketMatch.background': selection,
      'editorBracketMatch.border': border,
      'editorWidget.background': popover,
      'editorHoverWidget.background': popover,
      'editorOverviewRuler.border': background,
      /* Diff 模式（冲突对比）配色：终端 16 色的绿/红叠加低透明度 */
      'diffEditor.border': border,
      'diffEditor.insertedTextBackground': withAlpha(green, 0.14),
      'diffEditor.removedTextBackground': withAlpha(red, 0.14),
      'diffEditor.insertedLineBackground': withAlpha(green, 0.1),
      'diffEditor.removedLineBackground': withAlpha(red, 0.1),
      'diffEditorGutter.insertedLineBackground': withAlpha(green, 0.1),
      'diffEditorGutter.removedLineBackground': withAlpha(red, 0.1),
    },
  };
}

/** 注册（或以最新令牌重定义）'veloterm' 主题，返回主题名 */
export function registerVeloTermTheme(): string {
  monaco.editor.defineTheme(VELOTERM_MONACO_THEME, buildVeloTermThemeData());
  return VELOTERM_MONACO_THEME;
}

/** 按当前深浅模式重新取令牌 defineTheme + setTheme（主题切换时调用） */
export function applyMonacoTheme(): void {
  registerVeloTermTheme();
  monaco.editor.setTheme(VELOTERM_MONACO_THEME);
}

/* —— 深浅主题自动跟随：观察 html 的 class 变化（模块级单例） —— */

let themeObserver: MutationObserver | null = null;
let themeSwitchScheduled = false;

function scheduleThemeSwitch(): void {
  if (themeSwitchScheduled) return;
  themeSwitchScheduled = true;
  const apply = () => {
    themeSwitchScheduled = false;
    applyMonacoTheme();
  };
  // class 变化与令牌重算之间留一帧，确保 getComputedStyle 读到新令牌
  if (typeof requestAnimationFrame === 'function') {
    requestAnimationFrame(() => requestAnimationFrame(apply));
  } else {
    setTimeout(apply, 32);
  }
}

/** 初始化主题自动跟随（幂等；registerVeloTermTheme 首次调用时自动完成） */
export function ensureMonacoThemeAutoSwitch(): void {
  if (themeObserver || typeof document === 'undefined' || typeof MutationObserver === 'undefined') {
    return;
  }
  themeObserver = new MutationObserver((mutations) => {
    if (mutations.some((m) => m.type === 'attributes' && m.attributeName === 'class')) {
      scheduleThemeSwitch();
    }
  });
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['class'],
  });
}

/** 供编辑器宿主初始化时调用：注册主题 + 启动自动跟随 + 立即应用 */
export function initMonacoTheme(): string {
  const name = registerVeloTermTheme();
  ensureMonacoThemeAutoSwitch();
  monaco.editor.setTheme(name);
  return name;
}
