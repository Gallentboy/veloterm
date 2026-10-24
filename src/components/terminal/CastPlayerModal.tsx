/**
 * Asciinema v2 终端录制回放弹窗。
 *
 * 复用 xterm.js 建只读实例（尺寸取录制 header 的 width/height，`"r"` 事件
 * 由 CastPlayer 按序 term.resize），打开即自动播放。控制栏：播放/暂停（空格
 * 切换，焦点在按钮/输入框上时交给原生行为）、进度条拖动 seek（reset 后从零
 * 快速重建）、倍速（0.5/1/2/4×）、跳过空闲开关（默认开：单段静止最多回放
 * 2 秒）、复制屏幕（选区优先，无选区取整个缓冲）、导出 .cast / 纯文本 .log /
 * GIF（后端 agg 渲染，秒级耗时：按钮转 spinner 禁用至完成）。
 *
 * 渲染器用默认 DOM（弹窗内一次性回放无需 WebGL 上下文开销）；主题走
 * mergeTerminalTheme 与主终端一致。
 */

import { Copy, FileDown, FileText, Film, Pause, Play, RotateCcw, Video } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { useTranslation } from 'react-i18next';

import { Modal } from '@/components/ui/modal';
import { Spinner } from '@/components/ui/spinner';
import { writeClipboardText } from '@/lib/clipboard';
import { cn } from '@/lib/utils';
import { exportCastFile, exportGifFile, exportLogFile, exportMp4File } from '@/terminal/castExport';
import { CastPlayer, parseCastEvents, type CastPlayerState } from '@/terminal/castPlayer';
import type { CastRecording } from '@/terminal/castRecorder';
import { mergeTerminalTheme } from '@/terminal/terminalTheme';
import { formatRecordElapsed } from '@/components/terminal/RecordingPill';

// 与 TerminalSession 的 DEFAULT_TERMINAL_FONT_FAMILY 同款（那边是模块私有常量，
// 回放观感需与主终端一致，此处原样复制）
const PLAYER_FONT_FAMILY =
  'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", "Microsoft YaHei UI", "Microsoft YaHei", "PingFang SC", "Noto Sans CJK SC", monospace';

const PLAYER_SPEEDS = [0.5, 1, 2, 4];
/** 回放缓冲行数（忠实保留滚动历史，同主终端档位量级） */
const PLAYER_SCROLLBACK_LINES = 5000;

interface CastPlayerModalProps {
  recording: CastRecording;
  onClose: () => void;
}

export function CastPlayerModal({ recording, onClose }: CastPlayerModalProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<XTerm | null>(null);
  const playerRef = useRef<CastPlayer | null>(null);
  const [state, setState] = useState<CastPlayerState>(() => ({
    playing: false,
    currentTime: 0,
    duration: 0,
    speed: 1,
    skipIdle: true,
    ended: false,
  }));
  // GIF 导出秒级耗时：按钮转 spinner 禁用至完成（成功/取消/失败均复位），
  // percent 为 agg 逐帧回传的整数百分比（保存对话框阶段为 null）
  const [gifBusy, setGifBusy] = useState(false);
  const [gifPercent, setGifPercent] = useState<number | null>(null);
  const [mp4Busy, setMp4Busy] = useState(false);
  const [mp4Phase, setMp4Phase] = useState<'gif' | 'mp4'>('gif');
  const [mp4Percent, setMp4Percent] = useState<number | null>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const events = parseCastEvents(recording.events);
    const term = new XTerm({
      cols: recording.header.width,
      rows: recording.header.height,
      // 与主终端同参（TerminalSession 默认 13；后端不回填 profile 字段，
      // 主终端实际生效的即是这套默认值）——现感需与真终端一致
      fontSize: 13,
      fontFamily: PLAYER_FONT_FAMILY,
      cursorBlink: false,
      scrollback: PLAYER_SCROLLBACK_LINES,
      theme: mergeTerminalTheme(),
    });
    term.open(container);
    termRef.current = term;
    const player = new CastPlayer(term, events, { skipIdle: true });
    player.onChange = (next) => setState({ ...next });
    playerRef.current = player;
    setState({ ...player.getState() });
    player.play();
    return () => {
      player.dispose();
      playerRef.current = null;
      termRef.current = null;
      try {
        term.dispose();
      } catch (error) {
        console.warn('[CastPlayerModal] term.dispose failed:', error);
      }
    };
  }, [recording]);

  // 空格切换播放/暂停：焦点在按钮/输入框上时交给原生行为（避免双重触发）
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.code !== 'Space' || event.repeat) return;
      const target = event.target;
      if (
        target instanceof HTMLInputElement ||
        target instanceof HTMLButtonElement ||
        target instanceof HTMLSelectElement
      ) {
        return;
      }
      event.preventDefault();
      playerRef.current?.toggle();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  const copyScreen = () => {
    const term = termRef.current;
    if (!term) return;
    let text = term.hasSelection() ? term.getSelection() : '';
    if (!text) {
      const buffer = term.buffer.active;
      const lines: string[] = [];
      for (let i = 0; i < buffer.length; i += 1) {
        const line = buffer.getLine(i);
        if (line) lines.push(line.translateToString(true));
      }
      text = lines.join('\n');
    }
    void writeClipboardText(text).catch((err) =>
      console.warn('[CastPlayerModal] copy screen failed:', err),
    );
  };

  return (
    <Modal
      title={t('terminal.recording.player.title')}
      onClose={onClose}
      className="w-[780px]"
    >
      <div className="relative flex min-h-0 flex-col gap-3">
        {/* GIF/MP4 导出 busy 遮罩（与终端会话全屏遮罩同款）：转圈 + 步骤
            文案 + 百分比；GIF 单步、MP4 双阶段按 phase 切换 */}
        {gifBusy || mp4Busy ? (
          <div className="absolute inset-0 z-40 flex items-center justify-center bg-background/70 backdrop-blur-[2px]">
            <div className="flex flex-col items-center gap-3">
              <Spinner className="size-8 text-primary" strokeWidth={2} />
              <span className="font-mono text-ui-sm text-foreground select-none">
                {mp4Busy
                  ? mp4Phase === 'mp4'
                    ? mp4Percent != null
                      ? `${t('terminal.recording.exportingMp4Video')} ${mp4Percent}%`
                      : t('terminal.recording.exportingMp4Video')
                    : mp4Percent != null
                      ? `${t('terminal.recording.exportingMp4Gif')} ${mp4Percent}%`
                      : t('terminal.recording.exportingMp4Gif')
                  : gifPercent != null
                    ? `${t('terminal.recording.exportingGif')} ${gifPercent}%`
                    : t('terminal.recording.exportingGif')}
              </span>
            </div>
          </div>
        ) : null}
        {/* terminal-xterm-shell 与主终端同款：背景由主题背景填满（不能另加
            bg-black/p-2——主题背景非黑时会露出黑边）、滚动条走同一套
            xterm-scrollable-element 细胶囊样式；overflow-hidden 交给
            xterm 自管，容器不二次滚动 */}
        <div
          ref={containerRef}
          className="terminal-xterm-shell max-h-[55vh] min-h-0 overflow-hidden rounded-md border border-border"
        />
        <div className="flex flex-wrap items-center gap-2 text-ui-sm text-foreground-subtle">
          <button
            type="button"
            aria-label={state.ended ? t('terminal.recording.player.replay') : state.playing ? t('terminal.recording.player.pause') : t('terminal.recording.player.play')}
            onClick={() => playerRef.current?.toggle()}
            className="inline-flex size-8 items-center justify-center rounded-lg text-foreground transition-colors hover:bg-hover"
          >
            {state.ended ? (
              <RotateCcw className="size-4" />
            ) : state.playing ? (
              <Pause className="size-4" />
            ) : (
              <Play className="size-4" />
            )}
          </button>
          <span className="font-mono text-ui-xs tabular-nums">
            {formatRecordElapsed(state.currentTime * 1000)} / {formatRecordElapsed(state.duration * 1000)}
          </span>
          <input
            type="range"
            min={0}
            max={Math.max(state.duration, 0.1)}
            step={0.05}
            value={Math.min(state.currentTime, state.duration)}
            onChange={(e) => playerRef.current?.seek(Number(e.target.value))}
            className="h-1.5 min-w-32 flex-1 accent-foreground"
            aria-label={t('terminal.recording.player.progress')}
          />
          <div className="flex items-center rounded-lg border border-border">
            {PLAYER_SPEEDS.map((speed) => (
              <button
                key={speed}
                type="button"
                onClick={() => playerRef.current?.setSpeed(speed)}
                className={cn(
                  'px-2 py-1 font-mono text-ui-xs transition-colors first:rounded-l-lg last:rounded-r-lg hover:bg-hover',
                  state.speed === speed
                    ? 'bg-selected text-foreground'
                    : 'text-foreground-subtle',
                )}
              >
                {speed}×
              </button>
            ))}
          </div>
          <button
            type="button"
            onClick={() => playerRef.current?.setSkipIdle(!state.skipIdle)}
            className={cn(
              'rounded-lg border px-2 py-1 text-ui-xs transition-colors hover:bg-hover',
              state.skipIdle
                ? 'border-primary/60 bg-selected text-foreground'
                : 'border-border text-foreground-subtle',
            )}
          >
            {t('terminal.recording.player.skipIdle')}
          </button>
          <button
            type="button"
            onClick={copyScreen}
            className="inline-flex items-center gap-1.5 rounded-lg border border-border px-2 py-1 text-ui-xs text-foreground-subtle transition-colors hover:bg-hover"
          >
            <Copy className="size-3.5" />
            {t('terminal.recording.player.copyScreen')}
          </button>
          <div className="ml-auto flex items-center gap-2">
            <button
              type="button"
              onClick={() => void exportCastFile(recording)}
              className="inline-flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-ui-xs text-foreground transition-colors hover:bg-hover"
            >
              <FileDown className="size-3.5" />
              {t('terminal.recording.player.exportCast')}
            </button>
            <button
              type="button"
              onClick={() => void exportLogFile(recording)}
              className="inline-flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-ui-xs text-foreground transition-colors hover:bg-hover"
            >
              <FileText className="size-3.5" />
              {t('terminal.recording.player.exportLog')}
            </button>
            <button
              type="button"
              disabled={gifBusy}
              onClick={() => {
                setGifBusy(true);
                setGifPercent(null);
                exportGifFile(recording, PLAYER_FONT_FAMILY, (p) => setGifPercent(p)).finally(() => {
                  setGifBusy(false);
                  setGifPercent(null);
                });
              }}
              className="inline-flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-ui-xs text-foreground transition-colors hover:bg-hover disabled:opacity-50"
            >
              {gifBusy ? <Spinner className="size-3.5" /> : <Film className="size-3.5" />}
              {t('terminal.recording.player.exportGif')}
            </button>
            <button
              type="button"
              disabled={mp4Busy}
              onClick={() => {
                setMp4Busy(true);
                setMp4Phase('gif');
                setMp4Percent(null);
                exportMp4File(recording, PLAYER_FONT_FAMILY, (phase, percent) => {
                  setMp4Phase(phase);
                  setMp4Percent(percent < 0 ? null : percent);
                }).finally(() => {
                  setMp4Busy(false);
                  setMp4Percent(null);
                });
              }}
              className="inline-flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-ui-xs text-foreground transition-colors hover:bg-hover disabled:opacity-50"
            >
              {mp4Busy ? <Spinner className="size-3.5" /> : <Video className="size-3.5" />}
              {t('terminal.recording.player.exportMp4')}
            </button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
