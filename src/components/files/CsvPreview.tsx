import { ArrowDown, ArrowUp, ArrowUpDown, Copy, Check, Search, X } from 'lucide-react';
import React, { memo, useCallback, useDeferredValue, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { CellTooltipPortal, useCellTooltip } from '@/components/files/CellTooltip';
import { cn } from '@/lib/utils';

export interface CsvPreviewProps {
  content: string;
  fileName?: string;
  className?: string;
}

/** RFC 4180 合规的高性能分隔符文本解析（支持双引号转义、跨行字段） */
export function parseDelimitedText(
  text: string,
  explicitDelimiter?: string,
): { headers: string[]; rows: string[][]; delimiter: string } {
  let delimiter = explicitDelimiter;
  if (!delimiter) {
    const sample = text.slice(0, 4096);
    const firstLine = sample.split(/\r?\n/)[0] || '';
    const commas = (firstLine.match(/,/g) || []).length;
    const tabs = (firstLine.match(/\t/g) || []).length;
    const semicolons = (firstLine.match(/;/g) || []).length;
    if (tabs > commas && tabs > semicolons) delimiter = '\t';
    else if (semicolons > commas) delimiter = ';';
    else delimiter = ',';
  }

  const allRows: string[][] = [];
  let currentRow: string[] = [];
  let field = '';
  let inQuotes = false;
  let i = 0;
  const len = text.length;

  while (i < len) {
    const c = text[i];
    if (inQuotes) {
      if (c === '"') {
        if (i + 1 < len && text[i + 1] === '"') {
          field += '"';
          i += 2;
        } else {
          inQuotes = false;
          i++;
        }
      } else {
        field += c;
        i++;
      }
    } else {
      if (c === '"') {
        inQuotes = true;
        i++;
      } else if (c === delimiter) {
        currentRow.push(field);
        field = '';
        i++;
      } else if (c === '\r') {
        if (i + 1 < len && text[i + 1] === '\n') {
          i += 2;
        } else {
          i++;
        }
        currentRow.push(field);
        field = '';
        if (currentRow.some((f) => f.trim().length > 0)) {
          allRows.push(currentRow);
        }
        currentRow = [];
      } else if (c === '\n') {
        i++;
        currentRow.push(field);
        field = '';
        if (currentRow.some((f) => f.trim().length > 0)) {
          allRows.push(currentRow);
        }
        currentRow = [];
      } else {
        field += c;
        i++;
      }
    }
  }

  if (field.length > 0 || currentRow.length > 0) {
    currentRow.push(field);
    if (currentRow.some((f) => f.trim().length > 0)) {
      allRows.push(currentRow);
    }
  }

  if (allRows.length === 0) {
    return { headers: [], rows: [], delimiter };
  }

  const maxCols = Math.max(...allRows.map((r) => r.length));
  // 规范化每行长度一致，防止表格锯齿
  const normalizedRows = allRows.map((row) => {
    if (row.length < maxCols) {
      return [...row, ...new Array(maxCols - row.length).fill('')];
    }
    return row;
  });

  const headers = normalizedRows[0].map((h, idx) => h.trim() || `Col ${idx + 1}`);
  const rows = normalizedRows.slice(1);
  return { headers, rows, delimiter };
}

const ROW_HEIGHT = 32;
const OVERSCAN = 8;

export const CsvPreview = memo(function CsvPreview({
  content,
  fileName,
  className,
}: CsvPreviewProps) {
  const { t } = useTranslation();

  // 1. 解析表格数据
  const { headers, rows } = useMemo(() => {
    const isTsv = fileName ? /\.tsv$/i.test(fileName) : false;
    return parseDelimitedText(content, isTsv ? '\t' : undefined);
  }, [content, fileName]);

  // 2. 排序态
  const [sortCol, setSortCol] = useState<number | null>(null);
  const [sortOrder, setSortOrder] = useState<'asc' | 'desc' | null>(null);

  const toggleSort = useCallback((colIndex: number) => {
    setSortCol((prevCol) => {
      if (prevCol !== colIndex) {
        setSortOrder('asc');
        return colIndex;
      }
      setSortOrder((prevOrder) => {
        if (prevOrder === 'asc') return 'desc';
        if (prevOrder === 'desc') {
          return null;
        }
        return 'asc';
      });
      return colIndex;
    });
  }, []);

  // 3. 过滤/搜索态
  const [filterInput, setFilterInput] = useState('');
  const deferredFilter = useDeferredValue(filterInput.trim().toLowerCase());

  // 4. 计算经过排序和过滤的数据
  const processedRows = useMemo(() => {
    let result = rows;
    if (deferredFilter) {
      result = result.filter((row) =>
        row.some((cell) => cell.toLowerCase().includes(deferredFilter)),
      );
    }

    if (sortCol !== null && sortOrder !== null) {
      const col = sortCol;
      const orderMul = sortOrder === 'asc' ? 1 : -1;
      result = [...result].sort((a, b) => {
        const valA = a[col] ?? '';
        const valB = b[col] ?? '';
        const numA = Number(valA);
        const numB = Number(valB);
        if (!Number.isNaN(numA) && !Number.isNaN(numB) && valA !== '' && valB !== '') {
          return (numA - numB) * orderMul;
        }
        return valA.localeCompare(valB, undefined, { numeric: true }) * orderMul;
      });
    }

    return result;
  }, [rows, deferredFilter, sortCol, sortOrder]);

  // 5. 虚拟滚动计算
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [containerHeight, setContainerHeight] = useState(500);

  // 自定义单元格 Tooltip（Portal 到 document.body，非原生，支持视口自适应与复制）
  const {
    activeAnchor,
    handleCellMouseEnter,
    handleCellMouseLeave,
    handleContainerScroll,
  } = useCellTooltip();

  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    setScrollTop(e.currentTarget.scrollTop);
    handleContainerScroll();
  }, [handleContainerScroll]);

  const totalRows = processedRows.length;
  const totalHeight = totalRows * ROW_HEIGHT;
  const visibleCount = Math.ceil(containerHeight / ROW_HEIGHT);
  const startIndex = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const endIndex = Math.min(totalRows, startIndex + visibleCount + 2 * OVERSCAN);
  const visibleRows = processedRows.slice(startIndex, endIndex);
  const offsetY = startIndex * ROW_HEIGHT;

  // 复制当前过滤后结果为 TSV
  const [copied, setCopied] = useState(false);
  const handleCopyTable = useCallback(async () => {
    const text = [
      headers.join('\t'),
      ...processedRows.map((r) => r.join('\t')),
    ].join('\n');
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.warn('Failed to copy table', err);
    }
  }, [headers, processedRows]);

  return (
    <div className={cn('flex h-full w-full flex-col bg-popover select-text', className)}>
      {/* 顶部操作与检索条 */}
      <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border/70 bg-surface/50 px-4 py-2 text-ui-xs">
        <div className="flex items-center gap-2">
          <div className="relative flex items-center">
            <Search className="pointer-events-none absolute left-2 size-3.5 text-foreground-subtle" />
            <input
              type="text"
              value={filterInput}
              onChange={(e) => setFilterInput(e.target.value)}
              placeholder={t('files.searchTable', '搜索表格内容...')}
              className="h-7 w-44 rounded-md border border-border/70 bg-background pr-6 pl-7 text-ui-xs text-foreground outline-none transition-all focus:w-60 focus:border-border"
            />
            {filterInput ? (
              <button
                type="button"
                onClick={() => setFilterInput('')}
                className="absolute right-1.5 p-0.5 text-foreground-subtle hover:text-foreground"
              >
                <X className="size-3" />
              </button>
            ) : null}
          </div>
          <span className="text-foreground-subtle select-none">
            {deferredFilter ? (
              <>
                {t('files.matchedRows', '匹配 {{count}} / {{total}} 行', {
                  count: processedRows.length,
                  total: rows.length,
                })}
              </>
            ) : (
              <>
                {t('files.tableStats', '{{rows}} 行 · {{cols}} 列', {
                  rows: rows.length,
                  cols: headers.length,
                })}
              </>
            )}
          </span>
        </div>

        <button
          type="button"
          onClick={handleCopyTable}
          className="flex items-center gap-1.5 rounded-md border border-border/70 bg-background px-2.5 py-1 text-ui-xs text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
          title={copied ? t('files.copied', '已复制') : t('files.copyTable', '复制表格数据')}
        >
          {copied ? (
            <>
              <Check className="size-3 text-terminal-green" />
              <span className="text-terminal-green">{t('files.copied', '已复制')}</span>
            </>
          ) : (
            <>
              <Copy className="size-3" />
              <span>{t('files.copyTable', '复制数据')}</span>
            </>
          )}
        </button>
      </div>

      {/* 表格容器 */}
      <div
        ref={(el) => {
          containerRef.current = el;
          if (el && el.clientHeight !== containerHeight) {
            setContainerHeight(el.clientHeight);
          }
        }}
        onScroll={handleScroll}
        className="relative min-h-0 flex-1 overflow-auto"
      >
        <table className="w-full border-collapse font-mono text-ui-xs">
          {/* 固定表头 */}
          <thead className="sticky top-0 z-20 bg-surface/95 backdrop-blur-xs shadow-xs">
            <tr>
              <th className="w-12 border-b border-r border-border/60 bg-surface px-2 py-2 text-center font-normal text-foreground-subtle select-none">
                #
              </th>
              {headers.map((header, colIdx) => {
                const isSorted = sortCol === colIdx && sortOrder !== null;
                return (
                  <th
                    key={colIdx}
                    onClick={() => toggleSort(colIdx)}
                    onMouseEnter={(e) => handleCellMouseEnter(e, header)}
                    onMouseLeave={handleCellMouseLeave}
                    className="cursor-pointer border-b border-r border-border/60 px-3 py-2 text-left font-semibold text-foreground transition-colors hover:bg-hover select-none last:border-r-0"
                  >
                    <div className="flex items-center justify-between gap-1.5">
                      <span className="truncate">{header}</span>
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

          {/* 虚拟滚动内容体 */}
          <tbody>
            {/* 顶部撑高垫片 */}
            {offsetY > 0 ? (
              <tr style={{ height: `${offsetY}px` }}>
                <td colSpan={headers.length + 1} />
              </tr>
            ) : null}

            {visibleRows.map((row, rowIdx) => {
              const actualRowIndex = startIndex + rowIdx + 1;
              return (
                <tr
                  key={startIndex + rowIdx}
                  className="h-8 border-b border-border/30 transition-colors hover:bg-hover/60"
                >
                  <td className="border-r border-border/40 bg-surface/30 px-2 text-center text-foreground-subtlest select-none">
                    {actualRowIndex}
                  </td>
                  {row.map((cell, colIdx) => (
                    <td
                      key={colIdx}
                      onMouseEnter={(e) => handleCellMouseEnter(e, cell)}
                      onMouseLeave={handleCellMouseLeave}
                      className="max-w-xs truncate border-r border-border/30 px-3 text-foreground last:border-r-0"
                    >
                      {cell}
                    </td>
                  ))}
                </tr>
              );
            })}

            {/* 底部撑高垫片 */}
            {totalHeight - (offsetY + visibleRows.length * ROW_HEIGHT) > 0 ? (
              <tr
                style={{
                  height: `${Math.max(
                    0,
                    totalHeight - (offsetY + visibleRows.length * ROW_HEIGHT),
                  )}px`,
                }}
              >
                <td colSpan={headers.length + 1} />
              </tr>
            ) : null}
          </tbody>
        </table>

        {/* 无数据空态 */}
        {totalRows === 0 ? (
          <div className="flex h-32 items-center justify-center text-ui-sm text-foreground-subtle">
            {deferredFilter
              ? t('files.noMatchingRows', '未找到匹配的行')
              : t('files.emptyTable', '表格内容为空')}
          </div>
        ) : null}
      </div>

      {/* 共享非原生单元格浮动 Tooltip */}
      <CellTooltipPortal anchor={activeAnchor} />
    </div>
  );
});
