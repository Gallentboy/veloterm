/**
 * .cast 录制的导出编排：调后端 `export_text_file`（rfd 保存对话框 + 落盘）。
 *
 * 两种产物：
 * - `.cast`：serializeCast 的 JSON Lines 原文（可上传 asciinema.org / 官方 CLI 播放）；
 * - `.log`：剥除 ANSI 转义序列的纯文本（拼接全部 "o" 事件，\r\n 归一为 \n）。
 *
 * 建议文件名 `veloterm-cast-{label}-{YYYYMMDD-HHmm}.{ext}`，label 来自录制标题
 * （`user@host` 或 local），仅保留文件名安全字符。返回是否真实落盘
 * （用户取消对话框返回 false，不视为错误）。
 */

import { invoke } from '@tauri-apps/api/core';

import { serializeCast, type CastRecording } from './castRecorder';

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
