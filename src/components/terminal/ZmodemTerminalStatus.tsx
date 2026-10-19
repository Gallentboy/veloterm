import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { terminalSessionRegistry } from '@/terminal/terminalSessionRegistry';

/** 后端 terminal/zmodem/transfer.rs 的 ZmodemProgress 事件载荷（camelCase） */
interface ZmodemProgress {
  sessionId: string;
  direction: 'download' | 'upload';
  fileName: string;
  transferred: number;
  total: number;
  speedBps: number;
  /** 传输耗时（毫秒；后端引擎实测，isFinished 时有值） */
  elapsedMs: number;
  isFinished: boolean;
  error: string | null;
}

/** 事件名与后端 ZMODEM_EVENT 常量一致 */
const ZMODEM_EVENT = 'zmodem-progress';

/** ANSI：回到行首 + 擦除整行（动态刷新状态行用） */
const CR_ERASE = '\r\x1b[2K';

/**
 * 文件传输状态**就地写入终端缓冲**（不叠 DOM 覆盖层——项目约定：
 * 终端内提示直接写 xterm）。用户视角只是「传文件」，不出现协议名。
 *
 * 三态（紧跟 rz/sz 输出之后，单行动态刷新）：
 * ```text
 * rz waiting to receive.
 * ↑ 开始上传  report.log
 * ↑ 上传中…  report.log  42%  1.2 MB/s
 * ↑ 上传完成  report.log  3.4 MB  用时 2.8s
 * ```
 */
export function ZmodemTerminalStatus() {
  const startedAtRef = useRef<Map<number, number>>(new Map());

  useEffect(() => {
    let disposed = false;
    const unlistenPromise = listen<ZmodemProgress>(ZMODEM_EVENT, (event) => {
      if (disposed) return;
      const payload = event.payload;
      // 后端事件带的是 terminalId（TerminalManager 的计数），不是前端
      // sessionId（panel state 的自增序号）——两者是独立编号，必须反查
      const terminalId = Number(payload.sessionId.replace('terminal-', ''));
      if (!Number.isFinite(terminalId)) return;
      const numericId = terminalId;
      const term = terminalSessionRegistry.findByTerminalId(terminalId)?.term;
      if (!term) return;

      const isDownload = payload.direction === 'download';
      const arrow = isDownload ? '↓' : '↑';
      const verb = isDownload ? '下载' : '上传';
      const dim = '\x1b[90m';
      const reset = '\x1b[0m';

      if (!payload.isFinished) {
        // 首帧（带文件名的才算真正开始——对话框阶段的事件无文件名，跳过显示）
        if (!startedAtRef.current.has(numericId)) {
          if (!payload.fileName && payload.transferred === 0) return;
          startedAtRef.current.set(numericId, Date.now());
          const name = payload.fileName ? `  ${payload.fileName}` : '';
          term.write(`${CR_ERASE}${dim}${arrow} 开始${verb}${name}${reset}`);
          return;
        }
        const parts = [`${arrow} ${verb}中…`];
        if (payload.fileName) parts.push(payload.fileName);
        // 进度：已传/总量 + 百分比（总量未知时只显示已传）
        if (payload.total > 0) {
          const pct = Math.min(100, Math.round((payload.transferred / payload.total) * 100));
          parts.push(`${formatBytes(payload.transferred)}/${formatBytes(payload.total)}`);
          parts.push(`${pct}%`);
        } else if (payload.transferred > 0) {
          parts.push(formatBytes(payload.transferred));
        }
        if (payload.speedBps > 0) parts.push(`${formatBytes(payload.speedBps)}/s`);
        term.write(`${CR_ERASE}${dim}${parts.join('  ')}${reset}`);
        return;
      }

      // 结束：三态收尾。无论首帧是否显示过，完成行必须输出——这是用户
      // 唯一的结果反馈（实测：首帧被门限跳过后完成行也不显示，传输结果
      // 全部丢失）
      const startedAt = startedAtRef.current.get(numericId);
      const localElapsed = startedAt ? Date.now() - startedAt : 0;
      const elapsedMs = payload.elapsedMs > 0 ? payload.elapsedMs : localElapsed;
      startedAtRef.current.delete(numericId);
      const ok = !payload.error;
      const cancelled = payload.error === 'cancelled';
      const head = ok ? `${arrow} ${verb}完成` : `${arrow} ${verb}${cancelled ? '已取消' : '失败'}`;
      const parts: string[] = [head];
      // 完成行展示真实文件名（后端多文件时给出「首个 等 N 个文件」）
      const shownName = payload.fileName?.trim();
      if (shownName) parts.push(shownName);
      if (ok) {
        // 整体大小 + 平均速度 + 用时（下载/上传格式一致）
        const totalSize = payload.total > 0 ? payload.total : payload.transferred;
        if (totalSize > 0) parts.push(formatBytes(totalSize));
        if (payload.speedBps > 0) parts.push(`${formatBytes(payload.speedBps)}/s`);
        // 用时：无条件显示（快传也显示，<1s 用两位小数）
        if (elapsedMs > 0) {
          parts.push(`用时 ${elapsedMs >= 1000 ? (elapsedMs / 1000).toFixed(1) : (elapsedMs / 1000).toFixed(2)}s`);
        }
      }
      const color = ok ? '\x1b[32m' : '\x1b[33m';
      term.write(`${CR_ERASE}${color}${parts.join('  ')}${reset}\r\n`);
    });

    return () => {
      disposed = true;
      // 兜底：组件卸载时仍在传输（或事件丢失）的会话，恢复光标，
      // 避免光标永久隐藏
      startedAtRef.current.clear();
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  return null;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
