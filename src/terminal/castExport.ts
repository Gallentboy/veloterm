/**
 * .cast 录制的导出编排：调后端 `export_text_file`（rfd 保存对话框 + 落盘）。
 *
 * 三种产物：
 * - `.cast`：serializeCast 的 JSON Lines 原文（可上传 asciinema.org / 官方 CLI 播放）；
 * - `.log`：剥除 ANSI 转义序列的纯文本（拼接全部 "o" 事件，\r\n 归一为 \n）；
 * - `.gif`：后端 `export_recording_gif` 用 vendor agg 渲染（主题色与字体链随命令
 *   传入，与终端/回放器观感同源），进度经 onProgress 回传给调用方的
 *   全屏 busy 遮罩；
 * - `.mp4`：后端 `export_recording_mp4` 两阶段链——同一 agg 管线先渲染
 *   临时 GIF，再经 libgif2mp4（静态链入）转码为 MP4，临时件用完即删；
 *   进度回调带阶段（gif/mp4），遮罩按阶段切换文案。
 *
 * 建议文件名 `veloterm-cast-{label}-{YYYYMMDD-HHmm}.{ext}`，label 来自录制标题
 * （`user@host` 或 local），仅保留文件名安全字符。返回是否真实落盘
 * （用户取消对话框返回 false，不视为错误）。
 */

import { Channel, invoke } from '@tauri-apps/api/core';

import { serializeCast, type CastRecording } from './castRecorder';
import { mergeTerminalTheme } from './terminalTheme';

/** OSC（含 ST 终止形，终止符可选）优先匹配，再 CSI，最后单字符转义 */
const ANSI_ESCAPE_RE =
  /\x1b(?:\][^\x07\x1b]*(?:\x07|\x1b\\)?|\[[0-?]*[ -/]*[@-~]|[@-Z\\-_])/g;

/** 剥除 ANSI 转义的纯文本（保留可打印内容与换行） */
export function castToPlainText(rec: CastRecording): string {
  let raw = '';
  for (const event of rec.events) {
    if (event[1] === 'o') raw += event[2];
  }
  return raw.replace(ANSI_ESCAPE_RE, '').replace(/\r\n?/g, '\n');
}

function sanitizeLabel(title: string): string {
  const label = title.replace(/[^a-zA-Z0-9._-]+/g, '-').replace(/^-+|-+$/g, '');
  return label || 'session';
}

function suggestedName(rec: CastRecording, ext: string): string {
  const d = new Date(rec.header.timestamp * 1000);
  const pad = (n: number) => String(n).padStart(2, '0');
  const stamp = `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}`;
  return `veloterm-cast-${sanitizeLabel(rec.header.title)}-${stamp}.${ext}`;
}

async function exportTextFile(
  rec: CastRecording,
  ext: string,
  filterLabel: string,
  content: string,
): Promise<boolean> {
  try {
    const saved = await invoke<string | null>('export_text_file', {
      suggestedName: suggestedName(rec, ext),
      ext,
      filterLabel,
      content,
    });
    return saved != null;
  } catch (error) {
    console.warn('[castExport] export failed:', error);
    return false;
  }
}

/** 导出 .cast（JSON Lines 原文） */
export function exportCastFile(rec: CastRecording): Promise<boolean> {
  return exportTextFile(rec, 'cast', 'Asciinema Cast', serializeCast(rec));
}

/** 导出 .log（剥 ANSI 纯文本） */
export function exportLogFile(rec: CastRecording): Promise<boolean> {
  return exportTextFile(rec, 'log', 'Log', castToPlainText(rec));
}

/** CSS 颜色（#hex / rgb() / rgba()，逗号或空格分隔均可）→ 'rrggbb'；解析失败返回 null */
function cssColorToHex(css: string | undefined): string | null {
  if (!css) return null;
  const s = css.trim();
  const hex = s.match(/^#([0-9a-f]{6})$/i) ?? s.match(/^#([0-9a-f]{3})$/i);
  if (hex) {
    const digits = hex[1].length === 3 ? hex[1].split('').map((c) => c + c).join('') : hex[1];
    return digits.toLowerCase();
  }
  const rgb = s.match(/^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/i);
  if (rgb) {
    const to2 = (v: string) => Math.max(0, Math.min(255, Math.round(Number(v)))).toString(16).padStart(2, '0');
    return (to2(rgb[1]) + to2(rgb[2]) + to2(rgb[3]));
  }
  return null;
}

/** agg 主题串：bg,fg,c0..c15 共 18 个 rrggbb（agg 的 Theme::Custom 解析格式）。
 * 取当前终端主题（跟随 dark/light 与主机配色覆盖），与终端/回放器同源；
 * 单项解析失败时用终端主题的缺省黑色系兜底，保证恒为合法 18 项。 */
export function buildGifThemeString(): string {
  const theme = mergeTerminalTheme();
  const order = [
    theme.background, theme.foreground,
    theme.black, theme.red, theme.green, theme.yellow,
    theme.blue, theme.magenta, theme.cyan, theme.white,
    theme.brightBlack, theme.brightRed, theme.brightGreen, theme.brightYellow,
    theme.brightBlue, theme.brightMagenta, theme.brightCyan, theme.brightWhite,
  ] as const;
  return order.map((c) => cssColorToHex(c) ?? '000000').join(',');
}

/** 导出 GIF（后端 agg 渲染，秒级耗时）。fontFamily 为终端字体回退链（含 CJK
 * 系统字体，fontdb 在系统字体里按序匹配，缺失项自动跳过）；onProgress 收
 * 0-100 整数百分比（选完保存路径即 0%，整数变化才上报，经 Tauri Channel
 * 逐调用回传），调用方接全屏 busy 遮罩展示进度 */
export function exportGifFile(
  rec: CastRecording,
  fontFamily: string,
  onProgress?: (percent: number) => void,
): Promise<boolean> {
  const channel = new Channel<{ percent: number }>();
  channel.onmessage = (msg) => onProgress?.(msg.percent);
  return invoke<string | null>('export_recording_gif', {
    suggestedName: suggestedName(rec, 'gif'),
    castContent: serializeCast(rec),
    theme: buildGifThemeString(),
    fontFamily,
    onProgress: channel,
  })
    .then((saved) => saved != null)
    .catch((error) => {
      console.warn('[castExport] GIF export failed:', error);
      return false;
    });
}

/** MP4 导出的两阶段进度：phase=gif 为渲染临时 GIF（0-100），phase=mp4 为
 * 转码（percent 可为 -1 = 总帧数未知的不确定态） */
export type Mp4ExportPhase = 'gif' | 'mp4';

/** 导出 MP4（后端两阶段链：agg 渲染临时 GIF → libgif2mp4 转码 → 删临时件）。
 * 参数与导出 GIF 同源（主题/字体链）；onProgress 带阶段与百分比，调用方
 * 遮罩按阶段切换步骤文案 */
export function exportMp4File(
  rec: CastRecording,
  fontFamily: string,
  onProgress?: (phase: Mp4ExportPhase, percent: number) => void,
): Promise<boolean> {
  const channel = new Channel<{ phase: Mp4ExportPhase; percent: number }>();
  channel.onmessage = (msg) => onProgress?.(msg.phase, msg.percent);
  return invoke<string | null>('export_recording_mp4', {
    suggestedName: suggestedName(rec, 'mp4'),
    castContent: serializeCast(rec),
    theme: buildGifThemeString(),
    fontFamily,
    onProgress: channel,
  })
    .then((saved) => saved != null)
    .catch((error) => {
      console.warn('[castExport] MP4 export failed:', error);
      return false;
    });
}
