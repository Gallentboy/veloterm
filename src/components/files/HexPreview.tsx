import { Check, Copy, DownloadCloud } from 'lucide-react';
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { cn } from '@/lib/utils';
import { Spinner } from '@/components/ui/spinner';

export interface HexPreviewProps {
  /** 如果已经有现成内容（例如从文本预览切到 Hex），直接转字节 */
  initialContent?: string;
  hostId?: number;
  remotePath?: string;
  className?: string;
}

interface SftpFileHexContent {
  totalSize: number;
  offset: number;
  bytesBase64: string;
}

const BYTES_PER_ROW = 16;
const ROW_HEIGHT = 24;
const OVERSCAN = 10;
const CHUNK_SIZE = 64 * 1024; // 64 KB

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

function base64ToUint8Array(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

export const HexPreview = memo(function HexPreview({
  initialContent,
  hostId,
  remotePath,
  className,
}: HexPreviewProps) {
  const { t } = useTranslation();

  // 1. 累积的原始字节数据
  const [data, setData] = useState<Uint8Array>(() => {
    if (initialContent !== undefined) {
      return new TextEncoder().encode(initialContent);
    }
    return new Uint8Array(0);
  });
  const [totalSize, setTotalSize] = useState<number>(() => {
    if (initialContent !== undefined) {
      return new TextEncoder().encode(initialContent).length;
    }
    return 0;
  });
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  // 2. 从远端分块读取
  const loadChunk = useCallback(
    async (offset: number, isAppend: boolean) => {
      if (hostId === undefined || !remotePath) return;
      setLoading(true);
      setLoadError(null);
      try {
        const res = await invoke<SftpFileHexContent>('sftp_read_file_hex', {
          hostId,
          remotePath,
          offset,
          length: CHUNK_SIZE,
        });

        const newBytes = base64ToUint8Array(res.bytesBase64);
        setTotalSize(res.totalSize);
        if (isAppend) {
          setData((prev) => {
            const combined = new Uint8Array(prev.length + newBytes.length);
            combined.set(prev, 0);
            combined.set(newBytes, prev.length);
            return combined;
          });
        } else {
          setData(newBytes);
        }
      } catch (err) {
        setLoadError(err instanceof Error ? err.message : String(err));
      } finally {
        setLoading(false);
      }
    },
    [hostId, remotePath],
  );

  // 首次挂载且无初始内容时读取首个 64KB 切片
  useEffect(() => {
    if (initialContent === undefined && hostId !== undefined && remotePath) {
      void loadChunk(0, false);
    }
  }, [initialContent, hostId, remotePath, loadChunk]);

  // 3. 悬浮光标索引（实现 Hex 字节与 ASCII 字符的双向高亮联动）
  const [hoveredByteIndex, setHoveredByteIndex] = useState<number | null>(null);

  // 4. 虚拟滚动计算
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [containerHeight, setContainerHeight] = useState(500);

  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    setScrollTop(e.currentTarget.scrollTop);
  }, []);

  const totalRows = Math.ceil(data.length / BYTES_PER_ROW);
  const totalHeight = totalRows * ROW_HEIGHT;
  const visibleCount = Math.ceil(containerHeight / ROW_HEIGHT);
  const startIndex = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const endIndex = Math.min(totalRows, startIndex + visibleCount + 2 * OVERSCAN);
  const offsetY = startIndex * ROW_HEIGHT;

  const rows = useMemo(() => {
    const list: { rowIndex: number; offset: number; bytes: number[] }[] = [];
    for (let r = startIndex; r < endIndex; r++) {
      const rowOffset = r * BYTES_PER_ROW;
      const rowBytes: number[] = [];
      for (let c = 0; c < BYTES_PER_ROW; c++) {
        const byteIndex = rowOffset + c;
        if (byteIndex < data.length) {
          rowBytes.push(data[byteIndex]);
        }
      }
      list.push({ rowIndex: r, offset: rowOffset, bytes: rowBytes });
    }
    return list;
  }, [data, startIndex, endIndex]);

  // 5. 复制标准 Hex Dump
  const [copied, setCopied] = useState(false);
  const handleCopyDump = useCallback(async () => {
    const lines: string[] = [];
    const max = Math.min(data.length, 16 * 1024); // 限制复制最多 16KB
    for (let i = 0; i < max; i += BYTES_PER_ROW) {
      const offsetHex = i.toString(16).padStart(8, '0');
      const hexParts: string[] = [];
      let ascii = '';
      for (let j = 0; j < BYTES_PER_ROW; j++) {
        if (i + j < max) {
          const b = data[i + j];
          hexParts.push(b.toString(16).padStart(2, '0'));
          ascii += b >= 32 && b <= 126 ? String.fromCharCode(b) : '.';
        } else {
          hexParts.push('  ');
        }
        if (j === 7) hexParts.push('');
      }
      lines.push(`${offsetHex}  ${hexParts.join(' ')}  |${ascii}|`);
    }
    try {
      await navigator.clipboard.writeText(lines.join('\n'));
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.warn('Copy dump failed', err);
    }
  }, [data]);

  // 6. 悬停字节详情计算
  const inspector = useMemo(() => {
    if (hoveredByteIndex === null || hoveredByteIndex >= data.length) return null;
    const b = data[hoveredByteIndex];
    return {
      offset: `0x${hoveredByteIndex.toString(16).toUpperCase().padStart(8, '0')} (${hoveredByteIndex})`,
      hex: `0x${b.toString(16).toUpperCase().padStart(2, '0')}`,
      dec: String(b),
      bin: b.toString(2).padStart(8, '0'),
      char: b >= 32 && b <= 126 ? `'${String.fromCharCode(b)}'` : 'non-printable',
    };
  }, [hoveredByteIndex, data]);

  const hasMore = data.length < totalSize;

  return (
    <div className={cn('flex h-full w-full flex-col bg-popover select-text font-mono text-ui-xs', className)}>
      {/* 顶部控制栏 */}
      <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border/70 bg-surface/50 px-4 py-2 text-ui-xs">
        <div className="flex items-center gap-2 select-none">
          <span className="text-foreground font-semibold">HEX DUMP</span>
          <span className="text-foreground-subtle">
            {t('files.loadedBytes', {
              loaded: formatSize(data.length),
              total: formatSize(totalSize || data.length),
            })}
          </span>
          {hasMore ? (
            <button
              type="button"
              disabled={loading}
              onClick={() => void loadChunk(data.length, true)}
              className="flex items-center gap-1 rounded bg-surface px-2 py-0.5 text-foreground hover:bg-hover disabled:opacity-40"
            >
              {loading ? (
                <Spinner className="size-3 text-foreground-subtle" />
              ) : (
                <DownloadCloud className="size-3" />
              )}
              <span>{t('files.loadMoreBytes', { size: '64 KB' })}</span>
            </button>
          ) : null}
        </div>

        <button
          type="button"
          onClick={handleCopyDump}
          className="flex items-center gap-1.5 rounded-md border border-border/70 bg-background px-2.5 py-1 text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground select-none"
          title={t('files.copyHexDump')}
        >
          {copied ? (
            <>
              <Check className="size-3 text-terminal-green" />
              <span className="text-terminal-green">{t('files.copied')}</span>
            </>
          ) : (
            <>
              <Copy className="size-3" />
              <span>{t('files.copyHexDump')}</span>
            </>
          )}
        </button>
      </div>

      {loadError ? (
        <div className="p-3 text-ui-xs text-warning">
          {loadError}
        </div>
      ) : null}

      {/* Hex 查看器主区域（虚拟滚动） */}
      <div
        ref={(el) => {
          containerRef.current = el;
          if (el && el.clientHeight !== containerHeight) {
            setContainerHeight(el.clientHeight);
          }
        }}
        onScroll={handleScroll}
        className="relative min-h-0 flex-1 overflow-auto p-3"
      >
        <div style={{ height: `${totalHeight}px` }} className="relative w-full">
          <div
            style={{ transform: `translateY(${offsetY}px)` }}
            className="absolute top-0 left-0 w-full"
          >
            {rows.map(({ offset, bytes }) => (
              <div
                key={offset}
                className="flex items-center gap-4 h-6 px-1 transition-colors hover:bg-surface/60"
              >
                {/* 1. 偏移量列 */}
                <div className="w-20 shrink-0 text-foreground-subtlest font-medium select-none">
                  {offset.toString(16).padStart(8, '0').toUpperCase()}
                </div>

                {/* 2. 十六进制字节列（每 8 字节一组，中间有间隙） */}
                <div className="flex shrink-0 items-center gap-1.5">
                  <div className="flex items-center gap-1">
                    {bytes.slice(0, 8).map((b, cIdx) => {
                      const byteIdx = offset + cIdx;
                      const isHovered = hoveredByteIndex === byteIdx;
                      const hexStr = b.toString(16).padStart(2, '0').toUpperCase();
                      const isZero = b === 0;
                      const isPrintable = b >= 32 && b <= 126;
                      const isControl = b < 32 && !isZero;

                      return (
                        <span
                          key={cIdx}
                          onMouseEnter={() => setHoveredByteIndex(byteIdx)}
                          onMouseLeave={() => setHoveredByteIndex(null)}
                          className={cn(
                            'inline-block w-5 text-center transition-colors cursor-pointer rounded-xs',
                            isHovered
                              ? 'bg-selected text-foreground font-bold'
                              : isZero
                              ? 'text-foreground-subtlest/60'
                              : isControl
                              ? 'text-terminal-magenta font-semibold'
                              : isPrintable
                              ? 'text-foreground'
                              : 'text-terminal-cyan',
                          )}
                        >
                          {hexStr}
                        </span>
                      );
                    })}
                  </div>
                  <span className="w-1" />
                  <div className="flex items-center gap-1">
                    {bytes.slice(8, 16).map((b, cIdx) => {
                      const byteIdx = offset + 8 + cIdx;
                      const isHovered = hoveredByteIndex === byteIdx;
                      const hexStr = b.toString(16).padStart(2, '0').toUpperCase();
                      const isZero = b === 0;
                      const isPrintable = b >= 32 && b <= 126;
                      const isControl = b < 32 && !isZero;

                      return (
                        <span
                          key={cIdx}
                          onMouseEnter={() => setHoveredByteIndex(byteIdx)}
                          onMouseLeave={() => setHoveredByteIndex(null)}
                          className={cn(
                            'inline-block w-5 text-center transition-colors cursor-pointer rounded-xs',
                            isHovered
                              ? 'bg-selected text-foreground font-bold'
                              : isZero
                              ? 'text-foreground-subtlest/60'
                              : isControl
                              ? 'text-terminal-magenta font-semibold'
                              : isPrintable
                              ? 'text-foreground'
                              : 'text-terminal-cyan',
                          )}
                        >
                          {hexStr}
                        </span>
                      );
                    })}
                  </div>
                </div>

                {/* 3. 分隔符 */}
                <div className="w-px h-3.5 bg-border/60 shrink-0 select-none" />

                {/* 4. ASCII 字符展示列 */}
                <div className="flex items-center gap-0.5 tracking-widest">
                  {bytes.map((b, cIdx) => {
                    const byteIdx = offset + cIdx;
                    const isHovered = hoveredByteIndex === byteIdx;
                    const char = b >= 32 && b <= 126 ? String.fromCharCode(b) : '·';
                    return (
                      <span
                        key={cIdx}
                        onMouseEnter={() => setHoveredByteIndex(byteIdx)}
                        onMouseLeave={() => setHoveredByteIndex(null)}
                        className={cn(
                          'inline-block w-3 text-center transition-colors cursor-pointer rounded-xs',
                          isHovered
                            ? 'bg-selected text-foreground font-bold'
                            : b >= 32 && b <= 126
                            ? 'text-foreground'
                            : 'text-foreground-subtlest/60',
                        )}
                      >
                        {char}
                      </span>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* 底部字节探测器状态栏（鼠标悬浮字节时实时解码） */}
      <div className="flex h-6 shrink-0 items-center justify-between border-t border-border/60 bg-surface/80 px-4 text-[11px] text-foreground-subtle select-none">
        {inspector ? (
          <div className="flex items-center gap-3">
            <span>
              Offset: <span className="text-foreground">{inspector.offset}</span>
            </span>
            <span>
              Hex: <span className="text-terminal-cyan font-bold">{inspector.hex}</span>
            </span>
            <span>
              Dec: <span className="text-terminal-yellow font-bold">{inspector.dec}</span>
            </span>
            <span>
              Bin: <span className="text-foreground font-mono">{inspector.bin}</span>
            </span>
            <span>
              Char: <span className="text-terminal-green font-bold">{inspector.char}</span>
            </span>
          </div>
        ) : (
          <span className="text-foreground-subtlest">
            移动光标至字节查看详细十六进制/ASCII/十进制信息
          </span>
        )}
      </div>
    </div>
  );
});
