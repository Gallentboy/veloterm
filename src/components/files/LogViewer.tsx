import {
  ArrowDown,
  Check,
  Copy,
  History,
  Radio,
  Search,
  Trash2,
  X,
} from 'lucide-react';
import React, { memo, useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { cn } from '@/lib/utils';
import { Spinner } from '@/components/ui/spinner';

export interface LogViewerProps {
  initialContent?: string;
  hostId?: number;
  remotePath?: string;
  className?: string;
}

interface SftpFileTailContent {
  totalSize: number;
  startOffset: number;
  endOffset: number;
  content: string;
}

type LogLevel = 'all' | 'error' | 'warn' | 'info';

function detectLineLevel(line: string): 'error' | 'warn' | 'info' | 'debug' | 'normal' {
  if (/\b(ERROR|FATAL|CRITICAL|SEVERE|EXCEPTION)\b/i.test(line)) return 'error';
  if (/\b(WARN|WARNING)\b/i.test(line)) return 'warn';
  if (/\b(INFO|NOTICE)\b/i.test(line)) return 'info';
  if (/\b(DEBUG|TRACE)\b/i.test(line)) return 'debug';
  return 'normal';
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

/** 智能高亮行内文本（关键词与日志级别） */
function HighlightedLine({
  text,
  search,
  level,
}: {
  text: string;
  search: string;
  level: 'error' | 'warn' | 'info' | 'debug' | 'normal';
}) {
  const parts = useMemo(() => {
    if (!search) return [text];
    const regex = new RegExp(`(${search.replace(/[-/\\^$*+?.()|[\]{}]/g, '\\$&')})`, 'gi');
    return text.split(regex);
  }, [text, search]);

  const levelClass =
    level === 'error'
      ? 'text-terminal-red font-semibold'
      : level === 'warn'
      ? 'text-terminal-yellow font-medium'
      : level === 'info'
      ? 'text-terminal-blue'
      : level === 'debug'
      ? 'text-foreground-subtle'
      : 'text-foreground';

  return (
    <span className={levelClass}>
      {parts.map((part, idx) =>
        search && part.toLowerCase() === search.toLowerCase() ? (
          <mark
            key={idx}
            className="rounded-xs bg-terminal-yellow/30 px-0.5 text-foreground font-bold"
          >
            {part}
          </mark>
        ) : (
          <React.Fragment key={idx}>{part}</React.Fragment>
        ),
      )}
    </span>
  );
}

export const LogViewer = memo(function LogViewer({
  initialContent,
  hostId,
  remotePath,
  className,
}: LogViewerProps) {
  const { t } = useTranslation();

  // 1. 日志行缓冲与文件偏移状态
  const [lines, setLines] = useState<string[]>(() => {
    if (initialContent !== undefined) {
      return initialContent.split(/\r?\n/);
    }
    return [];
  });
  const [totalSize, setTotalSize] = useState<number>(0);
  const [startOffset, setStartOffset] = useState<number>(0);
  const [endOffset, setEndOffset] = useState<number>(0);
  const [loading, setLoading] = useState(false);
  const [loadingEarlier, setLoadingEarlier] = useState(false);

  // 2. 实时跟随 (Tail -f) 与自动滚底
  const [follow, setFollow] = useState(true);
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const [userScrolledUp, setUserScrolledUp] = useState(false);

  // 3. 过滤器状态
  const [levelFilter, setLevelFilter] = useState<LogLevel>('all');
  const [searchInput, setSearchInput] = useState('');
  const deferredSearch = useDeferredValue(searchInput.trim().toLowerCase());

  // 4. 初次加载：拉取末尾 256KB
  useEffect(() => {
    if (initialContent !== undefined || hostId === undefined || !remotePath) return;
    let cancelled = false;
    setLoading(true);
    (async () => {
      try {
        const res = await invoke<SftpFileTailContent>('sftp_read_file_tail', {
          hostId,
          remotePath,
          maxBytes: 256 * 1024,
        });
        if (!cancelled) {
          setTotalSize(res.totalSize);
          setStartOffset(res.startOffset);
          setEndOffset(res.endOffset);
          const split = res.content.split(/\r?\n/);
          setLines(split);
        }
      } catch (err) {
        console.warn('Tail read failed', err);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [initialContent, hostId, remotePath]);

  // 5. 加载更早的历史日志（向前读取 256KB）
  const handleLoadEarlier = useCallback(async () => {
    if (loadingEarlier || startOffset <= 0 || hostId === undefined || !remotePath) return;
    setLoadingEarlier(true);
    const readLen = 256 * 1024;
    const fetchOffset = startOffset > readLen ? startOffset - readLen : 0;
    const fetchLen = (startOffset - fetchOffset) as number;

    const container = scrollContainerRef.current;
    const prevScrollHeight = container?.scrollHeight || 0;
    const prevScrollTop = container?.scrollTop || 0;

    try {
      const res = await invoke<SftpFileTailContent>('sftp_read_file_range', {
        hostId,
        remotePath,
        offset: fetchOffset,
        length: fetchLen,
      });

      setStartOffset(res.startOffset);
      const earlierLines = res.content.split(/\r?\n/);
      // 如果不是从 0 开始，丢弃切开的第一行半截日志
      if (fetchOffset > 0 && earlierLines.length > 1) {
        earlierLines.shift();
      }

      setLines((prev) => [...earlierLines, ...prev]);

      // 保持视觉滚动位置不动（新行在顶部插入后平移 scrollTop）
      requestAnimationFrame(() => {
        if (container) {
          const delta = container.scrollHeight - prevScrollHeight;
          container.scrollTop = prevScrollTop + delta;
        }
      });
    } catch (err) {
      console.warn('Load earlier failed', err);
    } finally {
      setLoadingEarlier(false);
    }
  }, [loadingEarlier, startOffset, hostId, remotePath]);

  // 6. Tail -f 实时追随轮询（每 1.5 秒检查一次新字节）
  useEffect(() => {
    if (!follow || hostId === undefined || !remotePath || endOffset === 0) return;
    let timer: NodeJS.Timeout | null = null;
    let cancelled = false;

    const pollNext = async () => {
      try {
        const res = await invoke<SftpFileTailContent>('sftp_read_file_range', {
          hostId,
          remotePath,
          offset: endOffset,
          length: 128 * 1024,
        });

        if (!cancelled && res.content.length > 0) {
          setTotalSize(res.totalSize);
          setEndOffset(res.endOffset);
          const newLines = res.content.split(/\r?\n/);
          setLines((prev) => {
            // 合并最后一行和新输入的第一行
            if (prev.length === 0) return newLines;
            const last = prev[prev.length - 1];
            const next = [...prev.slice(0, -1), last + newLines[0], ...newLines.slice(1)];
            return next;
          });

          // 如果没有主动向上滚，自动平滑滚到底部
          if (!userScrolledUp) {
            requestAnimationFrame(() => {
              const el = scrollContainerRef.current;
              if (el) el.scrollTop = el.scrollHeight;
            });
          }
        }
      } catch (err) {
        console.warn('Tail polling failed', err);
      } finally {
        if (!cancelled && follow) {
          timer = setTimeout(pollNext, 1500);
        }
      }
    };

    timer = setTimeout(pollNext, 1500);

    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [follow, hostId, remotePath, endOffset, userScrolledUp]);

  // 初次加载完成自动滚底
  const initialScrolledRef = useRef(false);
  useEffect(() => {
    if (lines.length > 0 && !initialScrolledRef.current) {
      initialScrolledRef.current = true;
      requestAnimationFrame(() => {
        const el = scrollContainerRef.current;
        if (el) el.scrollTop = el.scrollHeight;
      });
    }
  }, [lines.length]);

  // 监听用户滚动：距离底部超过 80px 标记为向上查阅，暂停自动触底
  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const distanceToBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    setUserScrolledUp(distanceToBottom > 80);
  }, []);

  const scrollToBottom = useCallback(() => {
    const el = scrollContainerRef.current;
    if (el) {
      el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
      setUserScrolledUp(false);
    }
  }, []);

  // 7. 计算过滤后的行数据
  const displayItems = useMemo(() => {
    const result: { index: number; line: string; level: 'error' | 'warn' | 'info' | 'debug' | 'normal' }[] = [];
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      const level = detectLineLevel(line);

      // 级别过滤
      if (levelFilter === 'error' && level !== 'error') continue;
      if (levelFilter === 'warn' && level !== 'error' && level !== 'warn') continue;
      if (levelFilter === 'info' && level === 'debug') continue;

      // 关键词过滤
      if (deferredSearch && !line.toLowerCase().includes(deferredSearch)) {
        continue;
      }

      result.push({ index: i + 1, line, level });
    }
    return result;
  }, [lines, levelFilter, deferredSearch]);

  // 8. 复制当前可见日志
  const [copied, setCopied] = useState(false);
  const handleCopy = useCallback(async () => {
    const text = displayItems.map((item) => item.line).join('\n');
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.warn('Copy log failed', err);
    }
  }, [displayItems]);

  const handleClear = useCallback(() => {
    setLines([]);
  }, []);

  return (
    <div className={cn('flex h-full w-full flex-col bg-popover select-text font-mono text-ui-xs', className)}>
      {/* 顶部控制栏 */}
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-2.5 border-b border-border/70 bg-surface/50 px-4 py-2 select-none">
        {/* 左侧：搜索 + 级别过滤 */}
        <div className="flex items-center gap-2">
          {/* 搜索框 */}
          <div className="relative flex items-center">
            <Search className="pointer-events-none absolute left-2 size-3 text-foreground-subtle" />
            <input
              type="text"
              value={searchInput}
              onChange={(e) => setSearchInput(e.target.value)}
              placeholder={t('files.searchLogs')}
              className="h-7 w-36 rounded-md border border-border/70 bg-background pr-6 pl-7 text-ui-xs text-foreground outline-none transition-all focus:w-52 focus:border-border"
            />
            {searchInput ? (
              <button
                type="button"
                onClick={() => setSearchInput('')}
                className="absolute right-1.5 p-0.5 text-foreground-subtle hover:text-foreground"
              >
                <X className="size-3" />
              </button>
            ) : null}
          </div>

          {/* 级别过滤器 Segmented Buttons */}
          <div className="flex items-center rounded-lg border border-border/70 bg-background p-0.5 text-[11px]">
            {(['all', 'error', 'warn', 'info'] as LogLevel[]).map((lvl) => {
              const active = levelFilter === lvl;
              const label =
                lvl === 'all'
                  ? t('files.allLevels')
                  : lvl === 'error'
                  ? t('files.errorOnly')
                  : lvl === 'warn'
                  ? t('files.warnOnly')
                  : t('files.infoOnly');
              return (
                <button
                  key={lvl}
                  type="button"
                  onClick={() => setLevelFilter(lvl)}
                  className={cn(
                    'rounded-md px-2 py-0.5 font-medium transition-colors',
                    active
                      ? lvl === 'error'
                        ? 'bg-terminal-red/15 text-terminal-red font-bold'
                        : lvl === 'warn'
                        ? 'bg-terminal-yellow/15 text-terminal-yellow font-bold'
                        : 'bg-selected text-foreground font-bold shadow-xs'
                      : 'text-foreground-subtle hover:text-foreground',
                  )}
                >
                  {label}
                </button>
              );
            })}
          </div>
        </div>

        {/* 右侧：状态统计 + Follow 模式开关 + 动作 */}
        <div className="flex items-center gap-2 text-ui-xs">
          {/* 大小与行数统计 */}
          <span className="text-foreground-subtlest">
            {displayItems.length} 行 · 已载入 {formatSize(endOffset - startOffset)} / 总 {formatSize(totalSize || endOffset)}
          </span>

          {/* 实时追随开关 (Tail -f) */}
          <button
            type="button"
            onClick={() => setFollow(!follow)}
            className={cn(
              'flex items-center gap-1.5 rounded-md border px-2.5 py-1 transition-colors',
              follow
                ? 'border-terminal-green/50 bg-terminal-green/10 text-terminal-green font-medium'
                : 'border-border/70 bg-background text-foreground-subtle hover:text-foreground',
            )}
            title={follow ? t('files.followMode') + ' (ON)' : t('files.followMode') + ' (OFF)'}
          >
            <Radio className={cn('size-3', follow ? 'animate-pulse text-terminal-green' : '')} />
            <span>{t('files.followMode')}</span>
          </button>

          {/* 清屏 */}
          <button
            type="button"
            onClick={handleClear}
            className="rounded p-1 text-foreground-subtle hover:bg-hover hover:text-foreground"
            title={t('files.clearScreen')}
          >
            <Trash2 className="size-3.5" />
          </button>

          {/* 复制 */}
          <button
            type="button"
            onClick={handleCopy}
            className="flex items-center gap-1 rounded border border-border/70 bg-background px-2 py-1 text-foreground-subtle hover:bg-hover hover:text-foreground"
            title={copied ? t('files.copied') : t('files.copyContent')}
          >
            {copied ? (
              <>
                <Check className="size-3 text-terminal-green" />
                <span className="text-terminal-green">{t('files.copied')}</span>
              </>
            ) : (
              <>
                <Copy className="size-3" />
                <span>{t('files.copyContent')}</span>
              </>
            )}
          </button>
        </div>
      </div>

      {/* 日志内容滚动主容器 */}
      <div
        ref={scrollContainerRef}
        onScroll={handleScroll}
        className="relative min-h-0 flex-1 overflow-auto p-3 text-ui-xs leading-relaxed"
      >
        {/* 加载更早历史按钮 */}
        {startOffset > 0 ? (
          <div className="flex justify-center pb-2">
            <button
              type="button"
              disabled={loadingEarlier}
              onClick={() => void handleLoadEarlier()}
              className="flex items-center gap-1.5 rounded-full border border-border/70 bg-surface/70 px-3 py-1 text-[11px] text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground disabled:opacity-40"
            >
              {loadingEarlier ? (
                <Spinner className="size-3 text-foreground-subtle" />
              ) : (
                <History className="size-3" />
              )}
              <span>{t('files.loadEarlier')}</span>
            </button>
          </div>
        ) : lines.length > 0 ? (
          <div className="flex justify-center pb-2 text-[10px] text-foreground-subtlest select-none">
            ── {t('files.reachedTop')} ──
          </div>
        ) : null}

        {/* 首次加载状态 */}
        {loading && lines.length === 0 ? (
          <div className="flex h-32 items-center justify-center gap-2 text-foreground-subtle">
            <Spinner className="size-4" />
            <span>Tail 读取中...</span>
          </div>
        ) : null}

        {/* 日志行列表 */}
        <div className="space-y-0.5">
          {displayItems.map(({ index, line, level }) => (
            <div
              key={index}
              className="flex items-start gap-2.5 rounded-xs px-1 hover:bg-surface/50 font-mono"
            >
              {/* 序号列 */}
              <span className="w-10 shrink-0 text-right text-foreground-subtlest select-none">
                {index}
              </span>
              {/* 日志正文 */}
              <div className="min-w-0 flex-1 break-all whitespace-pre-wrap">
                <HighlightedLine text={line} search={deferredSearch} level={level} />
              </div>
            </div>
          ))}
        </div>

        {/* 到底部浮标：当开启 Follow 且用户向上翻阅时提示 */}
        {userScrolledUp ? (
          <button
            type="button"
            onClick={scrollToBottom}
            className="sticky bottom-3 left-1/2 -translate-x-1/2 flex items-center gap-1.5 rounded-full border border-border/80 bg-popover/90 px-3 py-1 shadow-lg backdrop-blur-xs text-[11px] text-foreground transition-all hover:bg-hover"
          >
            <ArrowDown className="size-3 text-terminal-green" />
            <span>滚动到最新日志</span>
          </button>
        ) : null}
      </div>
    </div>
  );
});
