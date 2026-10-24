/**
 * 公用虚拟滚动数据表格（@tanstack/react-virtual 驱动）：固定表头、三态列
 * 排序（数值/文本自适应）、实时过滤、TSV 复制、行号列与单元格截断 Tooltip。
 * 从 SqlitePreview/CsvPreview/ExcelPreview 三处同构实现抽出（数据库工作台
 * 首先消费；旧三处的替换随后续批次回归验证）。
 *
 * 虚拟化形态：useVirtualizer 提供滚动订阅（useSyncExternalStore）与窗口
 * 切片，渲染保持真实 <table> 语义——上下各一条高度撑开的占位 <tr> 代替
 * 绝对定位（绝对定位与 table 自动列宽布局相斥，TanStack 官方 table 示例
 * 为此要求固定像素列宽，本表格列宽自适应不适用）。行高固定 32px
 * （h-8 由样式钉死），estimateSize 恒准、无测量层开销。
 */

import { ArrowDown, ArrowUp, ArrowUpDown, Check, Copy } from 'lucide-react';
import {
  memo,
  useCallback,
  useDeferredValue,
  useEffect,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';
import { useVirtualizer } from '@tanstack/react-virtual';

import {
  CellTooltipPortal,
  useCellTooltip,
} from '@/components/files/CellTooltip';
import { cn } from '@/lib/utils';

const ROW_HEIGHT = 32;
const OVERSCAN = 8;

export interface DataTableProps {
  columns: string[];
  /** 字符串化后的行数据（与预览矩阵的展示契约一致：NULL/BLOB 占位文本） */
  rows: string[][];
  className?: string;
  /** 是否展示工具栏（过滤框 + 复制按钮）；默认展示 */
  showToolbar?: boolean;
  /** 空态文案 */
  emptyLabel?: string;
  /** 底部状态条自定义内容（行数统计等）；不传显示默认统计 */
  statusFooter?: React.ReactNode;
}

function compareCells(a: string, b: string): number {
  const valA = (a ?? '').trim();
  const valB = (b ?? '').trim();
  const numA = Number(valA);
  const numB = Number(valB);
  if (!Number.isNaN(numA) && !Number.isNaN(numB) && valA !== '' && valB !== '') {
    return numA - numB;
  }
  return valA.localeCompare(valB, undefined, { numeric: true });
}

export const DataTable = memo(function DataTable({
  columns,
  rows,
  className,
  showToolbar = true,
  emptyLabel,
  statusFooter,
}: DataTableProps) {
  const { t } = useTranslation();
  const [sortCol, setSortCol] = useState<number | null>(null);
  const [sortOrder, setSortOrder] = useState<'asc' | 'desc' | null>(null);
  const [filterInput, setFilterInput] = useState('');
  const deferredFilter = useDeferredValue(filterInput.trim().toLowerCase());
  const [copied, setCopied] = useState(false);

  const containerRef = useRef<HTMLDivElement | null>(null);
  // 鼠标悬停的数据单元格文本 span（Cmd/Ctrl+A 全选的作用目标），null = 不在单元格上
  const hoveredCellRef = useRef<HTMLElement | null>(null);

  const { activeAnchor, handleCellMouseEnter, handleCellMouseLeave, handleContainerScroll } =
    useCellTooltip();

  const processedRows = (() => {
    let result = rows;
    if (deferredFilter) {
      result = result.filter((row) => row.some((cell) => cell.toLowerCase().includes(deferredFilter)));
    }
    if (sortCol != null && sortOrder != null && sortCol < columns.length) {
      const col = sortCol;
      const orderMul = sortOrder === 'asc' ? 1 : -1;
      result = [...result].sort((a, b) => compareCells(a[col] ?? '', b[col] ?? '') * orderMul);
    }
    return result;
  })();

  const virtualizer = useVirtualizer({
    count: processedRows.length,
    getScrollElement: () => containerRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: OVERSCAN,
  });
  const virtualRows = virtualizer.getVirtualItems();
  // 上下占位高度：保持滚动条全长正确（table 语义下的绝对定位替代方案）
  const padTop = virtualRows.length > 0 ? virtualRows[0]!.start : 0;
  const padBottom =
    virtualRows.length > 0
      ? Math.max(0, virtualizer.getTotalSize() - virtualRows[virtualRows.length - 1]!.end)
      : 0;

  // 数据集变化（新查询）时复位排序与滚动位置
  useEffect(() => {
    setSortCol(null);
    setSortOrder(null);
    if (containerRef.current) {
      containerRef.current.scrollTop = 0;
    }
    // scrollToOffset(0) 不必显式调用：DOM scrollTop 置零会触发 virtualizer 的
    // 滚动订阅自行重算窗口
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [columns]);

  const toggleSort = useCallback((colIndex: number) => {
    setSortCol((prevCol) => {
      if (prevCol !== colIndex) {
        setSortOrder('asc');
        return colIndex;
      }
      setSortOrder((prevOrder) => {
        if (prevOrder === 'asc') return 'desc';
        if (prevOrder === 'desc') return null;
        return 'asc';
      });
      return colIndex;
    });
  }, []);

  const handleScroll = useCallback(
    (_e: React.UIEvent<HTMLDivElement>) => {
      // 滚动窗口切片由 virtualizer 订阅自行处理，这里只驱动单元格 Tooltip 收起
      handleContainerScroll();
    },
    [handleContainerScroll],
  );

  // Cmd/Ctrl+A 限定为「当前单元格内容全选」：鼠标悬停在任意数据单元格上即生效，拦截默认的
  // 整页全选，改选该单元格 span 内容（DOM 顺序是先 enter 新格后 leave 旧格，leave 只在仍是
  // 自己时才清空）。悬停空白/空单元格、工具栏输入框等其余场景走默认行为。
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.code !== 'KeyA' || !(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
      const container = containerRef.current;
      const span = hoveredCellRef.current;
      if (!container || !span || !container.contains(span)) return;
      if (!span.textContent) return;
      e.preventDefault();
      const sel = window.getSelection();
      if (!sel) return;
      const range = document.createRange();
      range.selectNodeContents(span);
      sel.removeAllRanges();
      sel.addRange(range);
    };
    document.addEventListener('keydown', onKeyDown, true);
    return () => document.removeEventListener('keydown', onKeyDown, true);
  }, []);

  const handleCopy = useCallback(async () => {
    const lines = [columns.join('\t'), ...processedRows.map((r) => r.join('\t'))];
    try {
      await navigator.clipboard.writeText(lines.join('\n'));
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.warn('Copy failed', err);
    }
  }, [columns, processedRows]);

  return (
    <div className={cn('flex h-full min-h-0 w-full flex-col bg-popover font-mono text-ui-xs', className)}>
      {showToolbar ? (
        <div className="flex shrink-0 items-center justify-end gap-2 border-b border-border/60 bg-surface/40 px-2 py-1 select-none">
          <div className="relative flex items-center">
            <input
              type="text"
              value={filterInput}
              onChange={(e) => setFilterInput(e.target.value)}
              placeholder={t('dataTable.filter', '过滤...')}
              className="h-6 w-32 rounded-md border border-border/60 bg-background px-2 text-ui-xs text-foreground outline-none transition-all focus:w-44 focus:border-border"
            />
          </div>
          <button
            type="button"
            onClick={handleCopy}
            className="flex items-center gap-1 rounded-md border border-border/60 bg-background px-2 py-0.5 text-ui-xs text-foreground-subtle hover:bg-hover hover:text-foreground"
            title={t('files.copyTable', '复制表格')}
          >
            {copied ? (
              <>
                <Check className="size-3 text-terminal-green" />
                <span className="text-terminal-green">{t('files.copied', '已复制')}</span>
              </>
            ) : (
              <>
                <Copy className="size-3" />
                <span>{t('files.copyTable', '复制表格')}</span>
              </>
            )}
          </button>
        </div>
      ) : null}

      <div
        ref={containerRef}
        onScroll={handleScroll}
        className="relative min-h-0 flex-1 overflow-auto scrollbar-slim"
      >
        <table className="w-full border-collapse">
          <thead className="sticky top-0 z-20 bg-surface/95 backdrop-blur-xs shadow-xs">
            <tr>
              <th className="w-12 border-b border-r border-border/60 bg-surface px-2 py-2 text-center font-normal text-foreground-subtle select-none">
                #
              </th>
              {columns.map((col, idx) => {
                const isSorted = sortCol === idx && sortOrder !== null;
                return (
                  <th
                    key={idx}
                    onClick={() => toggleSort(idx)}
                    onMouseEnter={(e) => handleCellMouseEnter(e, col)}
                    onMouseLeave={handleCellMouseLeave}
                    className="cursor-pointer border-b border-r border-border/60 px-3 py-2 text-left font-semibold text-foreground hover:bg-hover select-none last:border-r-0"
                  >
                    <div className="flex items-center justify-between gap-1.5">
                      <span className="truncate">{col}</span>
                      <span className="shrink-0 text-foreground-subtle">
                        {isSorted ? (
                          sortOrder === 'asc' ? (
                            <ArrowUp className="size-3 text-terminal-blue" />
                          ) : (
                            <ArrowDown className="size-3 text-terminal-blue" />
                          )
                        ) : (
                          <ArrowUpDown className="size-3 opacity-30 hover:opacity-100" />
                        )}
                      </span>
                    </div>
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {padTop > 0 ? (
              <tr style={{ height: `${padTop}px` }}>
                <td colSpan={columns.length + 1} />
              </tr>
            ) : null}
            {virtualRows.map((vr) => {
              const row = processedRows[vr.index] ?? [];
              return (
                <tr key={vr.key} className="h-8 border-b border-border/30 hover:bg-hover/60">
                  <td className="border-r border-border/40 bg-surface/30 px-2 text-center align-middle leading-none text-foreground-subtlest select-none">
                    {vr.index + 1}
                  </td>
                  {row.map((cell, cIdx) => (
                    <td
                      key={cIdx}
                      onMouseEnter={(e) => {
                        handleCellMouseEnter(e, cell);
                        hoveredCellRef.current =
                          e.currentTarget.querySelector<HTMLElement>('[data-cell-text]');
                      }}
                      onMouseLeave={(e) => {
                        handleCellMouseLeave();
                        const span = e.currentTarget.querySelector<HTMLElement>('[data-cell-text]');
                        if (hoveredCellRef.current === span) hoveredCellRef.current = null;
                      }}
                      className="border-r border-border/30 px-3 align-middle leading-none text-foreground select-none last:border-r-0"
                    >
                      {/* 单元格空白区不可起选（td select-none），仅内容可选：inline-block 收缩到内容宽 +
                          span select-text 重新开启选中，高亮只盖文本本身 */}
                      <span
                        data-cell-text
                        className={cn(
                          'inline-block max-w-xs truncate select-text',
                          cell === 'NULL' ? 'text-foreground-subtlest italic' : '',
                        )}
                      >
                        {cell}
                      </span>
                    </td>
                  ))}
                </tr>
              );
            })}
            {padBottom > 0 ? (
              <tr style={{ height: `${padBottom}px` }}>
                <td colSpan={columns.length + 1} />
              </tr>
            ) : null}
          </tbody>
        </table>

        {processedRows.length === 0 ? (
          <div className="flex h-24 items-center justify-center font-sans text-ui-sm text-foreground-subtle select-none">
            {deferredFilter
              ? (emptyLabel ?? t('files.noMatchingRows', '无匹配行'))
              : (emptyLabel ?? t('dataTable.empty', '无数据'))}
          </div>
        ) : null}
      </div>

      <div className="flex shrink-0 items-center justify-between border-t border-border/60 bg-surface/40 px-3 py-1 text-foreground-subtle select-none">
        {statusFooter ?? (
          <span>
            {deferredFilter
              ? t('files.matchedRows', { count: processedRows.length, total: rows.length })
              : `${rows.length} 行 · ${columns.length} 列`}
          </span>
        )}
      </div>

      <CellTooltipPortal anchor={activeAnchor} />
    </div>
  );
});
