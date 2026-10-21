/**
 * Asciinema v2 回放引擎（时间轴 / RAF 帧调度 / 变速 / seek / 跳空闲，纯逻辑）。
 *
 * 帧循环：requestAnimationFrame 推进 currentTime（dt × speed），游标左侧的
 * 事件逐个应用（`"o"` → term.write、`"r"` → term.resize）。跳空闲（skipIdle）：
 * 距下一事件超过阈值的整段静止直接跳到下一事件时刻（静止段无任何输出
 * 可渲染）——时间轴与进度条仍按真实录制时长映射。
 *
 * seek(t)：term.reset() 后从 0 快速重建——区间内 `"o"` 按序拼接一次 write
 * （xterm 内部写队列自行分帧），`"r"` 依次 resize。O(n) 单次，几 MB 量级瞬时。
 *
 * 未知事件类型（`"i"` / `"m"` / 未来扩展）解析时即过滤，回放端只见 o/r。
 */

import type { Terminal } from '@xterm/xterm';

import type { CastEvent, CastEventType } from './castRecorder';

/** 跳空闲阈值：单段静止最多回放的秒数 */
export const CAST_IDLE_THRESHOLD_SEC = 2;

export interface ParsedCastEvent {
  time: number;
  type: CastEventType;
  data: string;
}

/** 事件过滤（容错：形态非法 / 未知类型 / 负时间直接丢弃，时间升序稳定） */
export function parseCastEvents(events: readonly CastEvent[]): ParsedCastEvent[] {
  const out: ParsedCastEvent[] = [];
  for (const event of events) {
    if (!Array.isArray(event) || event.length < 3) continue;
    const [time, type, data] = event;
    if (typeof time !== 'number' || !Number.isFinite(time) || time < 0) continue;
    if ((type !== 'o' && type !== 'r') || typeof data !== 'string') continue;
    out.push({ time, type, data });
  }
  out.sort((a, b) => a.time - b.time);
  return out;
}

export interface CastPlayerState {
  playing: boolean;
  /** 当前播放头（录制时间轴秒） */
  currentTime: number;
  duration: number;
  speed: number;
  skipIdle: boolean;
  ended: boolean;
}

export interface CastPlayerOptions {
  speed?: number;
  skipIdle?: boolean;
  idleThresholdSec?: number;
}

export class CastPlayer {
  private readonly term: Terminal;
  private readonly events: readonly ParsedCastEvent[];
  private readonly idleThreshold: number;
  private cursor = 0;
  private raf = 0;
  private lastTick = 0;
  private state: CastPlayerState;

  /** 每帧状态回调（进度条/时间显示订阅；构造后赋值） */
  onChange: ((state: CastPlayerState) => void) | null = null;

  constructor(term: Terminal, events: readonly ParsedCastEvent[], opts: CastPlayerOptions = {}) {
    this.term = term;
    this.events = events;
    this.idleThreshold = opts.idleThresholdSec ?? CAST_IDLE_THRESHOLD_SEC;
    this.state = {
      playing: false,
      currentTime: 0,
      duration: events.length > 0 ? events[events.length - 1]!.time : 0,
      speed: opts.speed ?? 1,
      skipIdle: opts.skipIdle ?? true,
      ended: false,
    };
  }

  getState(): Readonly<CastPlayerState> {
    return this.state;
  }

  play(): void {
    if (this.state.playing) return;
    if (this.state.ended) {
      this.seek(0);
    }
    this.state = { ...this.state, playing: true };
    this.lastTick = performance.now();
    this.raf = requestAnimationFrame(this.tick);
    this.emit();
  }

  pause(): void {
    if (!this.state.playing) return;
    this.state = { ...this.state, playing: false };
    this.cancelRaf();
    this.emit();
  }

  toggle(): void {
    if (this.state.playing) {
      this.pause();
    } else {
      this.play();
    }
  }

  setSpeed(speed: number): void {
    if (speed <= 0 || speed === this.state.speed) return;
    this.state = { ...this.state, speed };
    this.emit();
  }

  setSkipIdle(enabled: boolean): void {
    if (enabled === this.state.skipIdle) return;
    this.state = { ...this.state, skipIdle: enabled };
    this.emit();
  }

  /** 跳到任意时间点（保持播放态：暂停 → 重建 → 续播） */
  seek(t: number): void {
    const target = Math.max(0, Math.min(t, this.state.duration));
    const wasPlaying = this.state.playing;
    this.pause();
    this.rebuildTo(target);
    this.state = { ...this.state, currentTime: target, ended: target >= this.state.duration };
    if (wasPlaying) {
      this.play();
    } else {
      this.emit();
    }
  }

  dispose(): void {
    this.cancelRaf();
    this.onChange = null;
  }

  private cancelRaf(): void {
    if (this.raf) {
      cancelAnimationFrame(this.raf);
      this.raf = 0;
    }
  }

  private emit(): void {
    this.onChange?.(this.state);
  }

  private applyEvent(event: ParsedCastEvent): void {
    if (event.type === 'o') {
      this.term.write(event.data);
    } else {
      const match = event.data.match(/^(\d+)x(\d+)$/);
      if (match) {
        this.term.resize(Number(match[1]), Number(match[2]));
      }
    }
  }

  /** reset 后从零重建到 t（含 t 时刻事件），游标停在第一个未应用事件上 */
  private rebuildTo(t: number): void {
    this.term.reset();
    let idx = 0;
    while (idx < this.events.length && this.events[idx]!.time <= t) {
      idx += 1;
    }
    // "o" 数据按 "r" 边界分段拼接，一次 write 让 xterm 写队列自行消化
    let buffer = '';
    for (let i = 0; i < idx; i += 1) {
      const event = this.events[i]!;
      if (event.type === 'o') {
        buffer += event.data;
      } else {
        if (buffer) {
          this.term.write(buffer);
          buffer = '';
        }
        this.applyEvent(event);
      }
    }
    if (buffer) {
      this.term.write(buffer);
    }
    this.cursor = idx;
  }

  private tick = (now: number): void => {
    if (!this.state.playing) return;
    const dt = (now - this.lastTick) / 1000;
    this.lastTick = now;
    let next = this.currentTime + dt * this.state.speed;
    // 跳空闲：距下一事件超过阈值（整段静止）时直接跳到下一事件时刻——
    // 静止段内无任何输出可渲染，保留阈值秒的停顿没有视觉意义
    const upcoming = this.events[this.cursor];
    if (this.state.skipIdle && upcoming && upcoming.time - this.currentTime > this.idleThreshold) {
      next = upcoming.time;
    }
    this.state = { ...this.state, currentTime: next };
    while (this.cursor < this.events.length && this.events[this.cursor]!.time <= next) {
      this.applyEvent(this.events[this.cursor]!);
      this.cursor += 1;
    }
    if (this.cursor >= this.events.length && next >= this.state.duration) {
      this.state = { ...this.state, currentTime: this.state.duration, playing: false, ended: true };
      this.cancelRaf();
      this.emit();
      return;
    }
    this.raf = requestAnimationFrame(this.tick);
    this.emit();
  };

  private get currentTime(): number {
    return this.state.currentTime;
  }
}
