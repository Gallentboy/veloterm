/**
 * Asciinema v2 (.cast) 终端会话录制器（纯逻辑，零 DOM 依赖）。
 *
 * 格式：JSON Lines——首行 header（version/width/height/timestamp/title/env），
 * 其后每行 `[time_offset, event_type, data]`。只录两种事件：
 * - `"o"`：终端输出流（必须是**高亮注入前**的原始解码文本，见 TerminalSession
 *   挂点注释——highlightOutput 会向文本注入 ANSI 颜色码，录进去会污染回放）；
 * - `"r"`：视口尺寸变化（`"120x34"`），回放端按序 term.resize 保证 reflow 正确。
 * 不录 `"i"`（输入流）：终端不回显密码，只录输出即天然不含敏感键入。
 *
 * 时间轴用 performance.now()（单调钟）取相对秒；导出时事件按到达序自然有序。
 * 空闲不做录制端压缩（保真优先），跳空闲由回放端 CastPlayer 的 skipIdle 承担。
 *
 * 实例经模块级 Map 按 sessionId 存活：组件重挂载（切页 / StrictMode 双挂载 /
 * registry 复用快路径）不中断录制；会话真正销毁（entry.dispose）时调
 * dropCastSession 清理。上次成品（stop 的快照）保留到下次开始录制，期间
 * 右键菜单可再次「回放上次录制」。
 */

export interface CastHeader {
  version: 2;
  width: number;
  height: number;
  /** Unix 秒（录制开始时刻） */
  timestamp: number;
  title: string;
  env: { TERM: string };
}

export type CastEventType = 'o' | 'r';

/** 三元组事件：[相对秒, 类型, 数据] */
export type CastEvent = [number, CastEventType, string];

export interface CastRecording {
  header: CastHeader;
  events: CastEvent[];
}

export class CastRecorder {
  private header: CastHeader | null = null;
  private events: CastEvent[] = [];
  private base = 0;
  private lastSize = '';
  private running = false;

  get isRecording(): boolean {
    return this.running;
  }

  start(cols: number, rows: number, title: string): void {
    this.events = [];
    this.header = {
      version: 2,
      width: cols,
      height: rows,
      timestamp: Math.floor(Date.now() / 1000),
      title,
      env: { TERM: 'xterm-256color' },
    };
    this.base = performance.now();
    this.lastSize = `${cols}x${rows}`;
    this.running = true;
  }

  /** 录制已进行的毫秒数（指示器计时用；未开始恒 0） */
  elapsedMs(): number {
    return this.running ? performance.now() - this.base : 0;
  }

  private nowSec(): number {
    return (performance.now() - this.base) / 1000;
  }

  feedOutput(text: string): void {
    if (!this.running || !text) return;
    this.events.push([this.nowSec(), 'o', text]);
  }

  /** 尺寸与上次相同则丢弃（fit 抖动不产生事件） */
  feedResize(cols: number, rows: number): void {
    if (!this.running) return;
    const size = `${cols}x${rows}`;
    if (size === this.lastSize) return;
    this.lastSize = size;
    this.events.push([this.nowSec(), 'r', size]);
  }

  /** 冻结并返回快照；未在录制返回 null（幂等：重复 stop 返回 null） */
  stop(): CastRecording | null {
    if (!this.running || !this.header) return null;
    this.running = false;
    return { header: this.header, events: this.events };
  }
}

/** 序列化为 .cast 文本（首行 header + 每事件一行，JSON.stringify 处理控制字符转义） */
export function serializeCast(rec: CastRecording): string {
  const lines: string[] = [JSON.stringify(rec.header)];
  for (const event of rec.events) {
    lines.push(JSON.stringify(event));
  }
  return `${lines.join('\n')}\n`;
}

// ─── 模块级会话注册表 ───

const activeRecorders = new Map<number, CastRecorder>();
const lastRecordings = new Map<number, CastRecording>();

export function getActiveRecorder(sessionId: number): CastRecorder | null {
  return activeRecorders.get(sessionId) ?? null;
}

/** 开始录制（已在录制则 no-op 返回 false） */
export function startCastRecording(
  sessionId: number,
  cols: number,
  rows: number,
  title: string,
): boolean {
  const existing = activeRecorders.get(sessionId);
  if (existing?.isRecording) return false;
  const recorder = existing ?? new CastRecorder();
  recorder.start(cols, rows, title);
  activeRecorders.set(sessionId, recorder);
  // 新录制开始，旧成品失效
  lastRecordings.delete(sessionId);
  return true;
}

/** 停止并留存成品；未在录制返回 null */
export function stopCastRecording(sessionId: number): CastRecording | null {
  const recorder = activeRecorders.get(sessionId);
  if (!recorder) return null;
  const rec = recorder.stop();
  if (rec) lastRecordings.set(sessionId, rec);
  return rec;
}

export function getLastRecording(sessionId: number): CastRecording | null {
  return lastRecordings.get(sessionId) ?? null;
}

/** 会话销毁（关 Tab）时清理录制器与成品 */
export function dropCastSession(sessionId: number): void {
  activeRecorders.delete(sessionId);
  lastRecordings.delete(sessionId);
}
