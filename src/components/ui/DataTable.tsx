/**
 * 公用虚拟滚动数据表格（@tanstack/react-table + @tanstack/react-virtual）：
 * 固定表头、三态列排序（数值/文本自适应 sortingFn）、实时过滤（globalFilter）、
 * TSV 复制、行号列与单元格截断 Tooltip。从 SqlitePreview/CsvPreview/ExcelPreview
 * 三处同构实现抽出（数据库工作台首先消费；旧三处的替换随后续批次回归验证）。
 *
 * 分工：react-table（headless）负责排序与过滤管线——sortingFn 保数值/文本
 * 自适应比较，globalFilterFn 是全行任意列的子串匹配；react-virtual 负责滚动
 * 窗口（useSyncExternalStore 订阅）。渲染保持真实 <table> 语义——上下各一条
 * 高度撑开的占位 <tr> 代替绝对定位（绝对定位与 table 自动列宽布局相斥，
 * TanStack 官方 table 示例为此要求固定像素列宽，本表格列宽自适应不适用）。
 * 行高固定 32px（h-8 由样式钉死），estimateSize 恒准、无测量层开销。
 */

import { ArrowDown, ArrowUp, ArrowUpDown, Check, Copy } from 'lucide-react';
import {
  memo,
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';
import {
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
  type Row,
  type SortingState,
} from '@tanstack/react-table';
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

/** 数值/文本自适应比较（asc 语义；desc 由 react-table 内部取反）：两值都
 * 能 parse 成数字则数值比较，否则 localeCompare（numeric: true 让 "2" < "10"） */
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
  const [sorting, setSorting] = useState<SortingState>([]);
  const [filterInput, setFilterInput] = useState('');
  const deferredFilter = useDeferredValue(filterInput.trim().toLowerCase());
  const [copied, setCopied] = useState(false);

  const containerRef = useRef<HTMLDivElement | null>(null);
  // 鼠标悬停的数据单元格文本 span（Cmd/Ctrl+A 全选的作用目标），null = 不在单元格上
  const hoveredCellRef = useRef<HTMLElement | null>(null);

  const { activeAnchor, handleCellMouseEnter, handleCellMouseLeave, handleContainerScroll } =
    useCellTooltip();

  // 列定义：纯字符串矩阵无 schema，按列序号动态生成（id = 下标字符串）
  const tableColumns = useMemo(
    () =>
      columns.map((_, idx) => ({
        id: String(idx),
        accessorFn: (row: string[]) => row[idx] ?? '',
        // 自定义 sortingFn：跨类型混排（数字串与文本同列）的稳定比较
        sortingFn: (a: Row<string[]>, b: Row<string[]>) =>
          compareCells(a.original[idx] ?? '', b.original[idx] ?? ''),
        // 三态循环首击 asc（默认对字符串列首击 asc、数值列 desc，行为统一）
        sortDescFirst: false,
      })),
    [columns],
  );

  const table = useReactTable({
    data: rows,
    columns: tableColumns,
    state: { sorting, globalFilter: deferredFilter },
    onSortingChange: setSorting,
    // 全行任意列子串匹配（大小写不敏感）：deferredFilter 已 lowercase
    globalFilterFn: (row: Row<string[]>, _columnId: string, value: string) =>
      row.original.some((cell) => cell.toLowerCase().includes(value as string)),
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  // 排序 + 过滤后的最终行序（虚拟化的数据源；空过滤串时 globalFilter 短路全过）
  const processedRows = table.getRowModel().rows.map((r) => r.original);

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
    setSorting([]);
    if (containerRef.current) {
      containerRef.current.scrollTop = 0;
    }
    // DOM scrollTop 置零会触发 virtualizer 的滚动订阅自行重算窗口
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [columns]);

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
              {table.getHeaderGroups()[0]?.headers.map((header) => {
                // 三态：getIsSorted() = false | 'asc' | 'desc'（三态循环由
                // toggleSorting 默认的 enableSortingRemoval 驱动）
                const sorted = header.column.getIsSorted();
                const idx = Number(header.column.id);
                return (
                  <th
                    key={header.column.id}
                    onClick={header.column.getToggleSortingHandler()}
                    onMouseEnter={(e) => handleCellMouseEnter(e, columns[idx] ?? '')}
                    onMouseLeave={handleCellMouseLeave}
                    className="cursor-pointer border-b border-r border-border/60 px-3 py-2 text-left font-semibold text-foreground hover:bg-hover select-none last:border-r-0"
                  >
                    <div className="flex items-center justify-between gap-1.5">
                      <span className="truncate">{columns[idx]}</span>
                      <span className="shrink-0 text-foreground-subtle">
                        {sorted ? (
                          sorted === 'asc' ? (
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
