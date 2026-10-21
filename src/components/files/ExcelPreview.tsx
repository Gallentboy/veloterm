import { ArrowDown, ArrowUp, ArrowUpDown, Check, Copy, FileSpreadsheet, Loader2, Search, X, AlertCircle } from 'lucide-react';
import React, { memo, useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { CellTooltipPortal, useCellTooltip } from '@/components/files/CellTooltip';
import { cn } from '@/lib/utils';

export interface ExcelWorkbookInfo {
  sheetNames: string[];
  defaultSheet: string;
}

export interface ExcelSheetData {
  sheetName: string;
  totalRows: number;
  totalCols: number;
  headers: string[];
  rows: string[][];
}

export interface ExcelPreviewProps {
  filePath: string;
  className?: string;
}

const ROW_HEIGHT = 32;
const OVERSCAN = 8;

export const ExcelPreview = memo(function ExcelPreview({
  filePath,
  className,
}: ExcelPreviewProps) {
  const { t } = useTranslation();

  // 1. 工作簿元数据状态
  const [workbookLoading, setWorkbookLoading] = useState(true);
  const [workbookError, setWorkbookError] = useState<string | null>(null);
  const [workbookInfo, setWorkbookInfo] = useState<ExcelWorkbookInfo | null>(null);

  // 2. 当前选中工作表状态
  const [activeSheet, setActiveSheet] = useState<string>('');
  const activeSheetRef = useRef<string>('');
  activeSheetRef.current = activeSheet;

  // 3. 当前工作表数据与加载状态
  const [sheetLoading, setSheetLoading] = useState(false);
  const [sheetError, setSheetError] = useState<string | null>(null);
  const [sheetData, setSheetData] = useState<ExcelSheetData | null>(null);

  // 工作表缓存，避免重复切换时重复读取
  const sheetCacheRef = useRef<Map<string, ExcelSheetData>>(new Map());

  // 4. 排序态
  const [sortCol, setSortCol] = useState<number | null>(null);
  const [sortOrder, setSortOrder] = useState<'asc' | 'desc' | null>(null);

  // 5. 过滤/搜索态
  const [filterInput, setFilterInput] = useState('');
  const deferredFilter = useDeferredValue(filterInput.trim().toLowerCase());

  // 6. 虚拟滚动容器态
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

  // 加载工作簿目录
  const loadWorkbook = useCallback(async () => {
    setWorkbookLoading(true);
    setWorkbookError(null);
    setSheetError(null);
    sheetCacheRef.current.clear();
    setSheetData(null);

    try {
      const info = await invoke<ExcelWorkbookInfo>('read_excel_workbook', { filePath });
      setWorkbookInfo(info);
      const initialSheet = info.defaultSheet || info.sheetNames[0] || '';
      setActiveSheet(initialSheet);
    } catch (err) {
      setWorkbookError(err instanceof Error ? err.message : String(err));
    } finally {
      setWorkbookLoading(false);
    }
  }, [filePath]);

  useEffect(() => {
    void loadWorkbook();
  }, [loadWorkbook]);

  // 加载单表数据
  const loadSheetData = useCallback(
    async (sheetName: string, force = false) => {
      if (!sheetName) return;

      if (!force) {
        const cached = sheetCacheRef.current.get(sheetName);
        if (cached) {
          setSheetData(cached);
          setSheetLoading(false);
          setSheetError(null);
          return;
        }
      }

      setSheetLoading(true);
      setSheetError(null);

      try {
        const data = await invoke<ExcelSheetData>('read_excel_sheet_data', {
          filePath,
          sheetName,
        });
        if (activeSheetRef.current !== sheetName) return;
        sheetCacheRef.current.set(sheetName, data);
        setSheetData(data);
      } catch (err) {
        if (activeSheetRef.current !== sheetName) return;
        setSheetError(err instanceof Error ? err.message : String(err));
      } finally {
        if (activeSheetRef.current === sheetName) {
          setSheetLoading(false);
        }
      }
    },
    [filePath],
  );

  useEffect(() => {
    if (activeSheet) {
      setSortCol(null);
      setSortOrder(null);
      setFilterInput('');
      setScrollTop(0);
      if (containerRef.current) {
        containerRef.current.scrollTop = 0;
      }
      void loadSheetData(activeSheet);
    }
  }, [activeSheet, loadSheetData]);

  // 表头与行数据标准化
  const displayHeaders = useMemo(() => {
    if (!sheetData) return [];
    if (sheetData.headers && sheetData.headers.length > 0) {
      return sheetData.headers.map((h, idx) => (h && h.trim()) || `Col ${idx + 1}`);
    }
    const maxCols = Math.max(
      sheetData.totalCols || 0,
      ...(sheetData.rows || []).map((r) => r.length),
    );
    return Array.from({ length: maxCols }, (_, idx) => `Col ${idx + 1}`);
  }, [sheetData]);

  const normalizedRows = useMemo(() => {
    if (!sheetData || !sheetData.rows) return [];
    const colCount = displayHeaders.length;
    return sheetData.rows.map((row) => {
      if (row.length < colCount) {
        return [...row, ...new Array(colCount - row.length).fill('')];
      }
      return row;
    });
  }, [sheetData, displayHeaders.length]);

  // 排序切换 (asc -> desc -> reset)
  const toggleSort = useCallback((colIndex: number) => {
    setSortCol((prevCol) => {
      if (prevCol !== colIndex) {
        setSortOrder('asc');
        return colIndex;
      }
      if (sortOrder === 'asc') {
        setSortOrder('desc');
        return colIndex;
      }
      if (sortOrder === 'desc') {
        setSortOrder(null);
        return null;
      }
      setSortOrder('asc');
      return colIndex;
    });
  }, [sortOrder]);

  // 计算过滤与排序后的行列表
  const processedRows = useMemo(() => {
    let result = normalizedRows;

    if (deferredFilter) {
      result = result.filter((row) =>
        row.some((cell) => (cell ?? '').toLowerCase().includes(deferredFilter)),
      );
    }

    if (sortCol !== null && sortOrder !== null) {
      const col = sortCol;
      const orderMul = sortOrder === 'asc' ? 1 : -1;
      result = [...result].sort((a, b) => {
        const valA = (a[col] ?? '').trim();
        const valB = (b[col] ?? '').trim();
        if (valA === '' && valB === '') return 0;
        if (valA === '') return 1;
        if (valB === '') return -1;
        const numA = Number(valA);
        const numB = Number(valB);
        if (!Number.isNaN(numA) && !Number.isNaN(numB)) {
          return (numA - numB) * orderMul;
        }
        return valA.localeCompare(valB, undefined, { numeric: true }) * orderMul;
      });
    }

    return result;
  }, [normalizedRows, deferredFilter, sortCol, sortOrder]);

  // 监听容器视口高度
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        setContainerHeight(entry.contentRect.height);
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    setScrollTop(e.currentTarget.scrollTop);
    handleContainerScroll();
  }, [handleContainerScroll]);

  // 虚拟滚动计算
  const totalRows = processedRows.length;
  const totalHeight = totalRows * ROW_HEIGHT;
  const visibleCount = Math.ceil(containerHeight / ROW_HEIGHT);
  const startIndex = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const endIndex = Math.min(totalRows, startIndex + visibleCount + 2 * OVERSCAN);
  const visibleRows = processedRows.slice(startIndex, endIndex);
  const offsetY = startIndex * ROW_HEIGHT;

  // 复制当前表格数据为 TSV 文本
  const [copied, setCopied] = useState(false);
  const handleCopyTable = useCallback(async () => {
    if (!sheetData) return;
    const lines: string[] = [];
    if (displayHeaders.length > 0) {
      lines.push(displayHeaders.join('\t'));
    }
    for (const row of processedRows) {
      lines.push(row.join('\t'));
    }
    const text = lines.join('\n');
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.warn('Failed to copy sheet data', err);
    }
  }, [displayHeaders, processedRows, sheetData]);

    if (workbookLoading) {
    return (
      <div className={cn('flex h-full w-full flex-col items-center justify-center gap-3 bg-popover', className)}>
        <Loader2 className="size-5 animate-spin text-foreground-subtle" />
        <span className="text-ui-sm text-foreground-subtle">
          {t('files.loadingWorkbook', '正在解析工作簿...')}
        </span>
      </div>
    );
  }

  if (workbookError) {
    return (
      <div className={cn('flex h-full w-full flex-col items-center justify-center gap-2 bg-popover px-4', className)}>
        <AlertCircle className="size-6 text-destructive" />
        <span className="max-w-md text-center text-ui-sm text-destructive">{workbookError}</span>
        <button
          type="button"
          onClick={() => void loadWorkbook()}
          className="mt-2 rounded-md border border-border px-3 py-1 text-ui-xs text-foreground-subtle hover:border-border-hover hover:text-foreground"
        >
          {t('pdf.retry', '重试')}
        </button>
      </div>
    );
  }

  if (!workbookInfo || workbookInfo.sheetNames.length === 0) {
    return (
      <div className={cn('flex h-full w-full flex-col items-center justify-center gap-2 bg-popover', className)}>
        <FileSpreadsheet className="size-8 text-foreground-subtle opacity-50" />
        <span className="text-ui-sm text-foreground-subtle">
          {t('files.emptyWorkbook', '工作簿中未包含任何工作表')}
        </span>
      </div>
    );
  }

  return (
    <div className={cn('flex h-full w-full flex-col bg-popover select-text', className)}>
      {/* 顶部操作与搜索条 */}
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
          {deferredFilter ? (
            <span className="text-foreground-subtle select-none">
              {t('files.matchedRows', '匹配 {{count}} / {{total}} 行', {
                count: processedRows.length,
                total: normalizedRows.length,
              })}
            </span>
          ) : null}
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

      {/* 中部表格内容区 */}
      <div
        ref={containerRef}
        onScroll={handleScroll}
        className="relative min-h-0 flex-1 overflow-auto"
      >
        {sheetLoading ? (
          <div className="flex h-full min-h-[200px] w-full flex-col items-center justify-center gap-2">
            <Loader2 className="size-5 animate-spin text-foreground-subtle" />
            <span className="text-ui-xs text-foreground-subtle">
              {t('excel.loadingSheet', '正在读取工作表数据...')}
            </span>
          </div>
        ) : sheetError ? (
          <div className="flex h-full min-h-[200px] w-full flex-col items-center justify-center gap-2 px-4">
            <span className="max-w-md text-center text-ui-xs text-destructive">{sheetError}</span>
            <button
              type="button"
              onClick={() => {
                if (activeSheet) void loadSheetData(activeSheet, true);
              }}
              className="rounded-md border border-border px-2.5 py-1 text-ui-xs text-foreground-subtle hover:border-border-hover hover:text-foreground"
            >
              {t('pdf.retry', '重试')}
            </button>
          </div>
        ) : (
          <>
            <table className="w-full border-collapse font-mono text-ui-xs">
              {/* 固定表头 */}
              <thead className="sticky top-0 z-20 bg-surface/95 backdrop-blur-xs shadow-xs">
                <tr>
                  <th className="w-12 border-b border-r border-border/60 bg-surface px-2 py-2 text-center font-normal text-foreground-subtle select-none">
                    #
                  </th>
                  {displayHeaders.map((header, colIdx) => {
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
                    <td colSpan={displayHeaders.length + 1} />
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
                    <td colSpan={displayHeaders.length + 1} />
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
          </>
        )}
      </div>

      {/* 底部工作表 Tab 栏与状态栏 */}
      <div className="flex shrink-0 items-center justify-between border-t border-border/70 bg-surface/60 px-3 py-1.5 text-ui-xs">
        {/* 工作表切换标签栏（水平滚动） */}
        <div className="flex items-center gap-1 overflow-x-auto min-w-0 pr-2">
          <div className="flex shrink-0 items-center gap-1 text-foreground-subtle mr-1 select-none">
            <FileSpreadsheet className="size-3.5" />
          </div>
          {workbookInfo.sheetNames.map((name) => {
            const isActive = name === activeSheet;
            return (
              <button
                key={name}
                type="button"
                onClick={() => setActiveSheet(name)}
                className={cn(
                  'flex shrink-0 items-center gap-1 rounded-md px-2.5 py-1 text-ui-xs transition-colors border select-none',
                  isActive
                    ? 'bg-selected font-medium text-foreground border-border shadow-xs'
                    : 'text-foreground-subtle border-transparent hover:bg-hover hover:text-foreground',
                )}
                title={name}
              >
                <span className="max-w-[160px] truncate">{name}</span>
              </button>
            );
          })}
        </div>

        {/* 状态栏：X 行 · Y 列 · 工作表: SheetName */}
        <div className="shrink-0 flex items-center gap-2 pl-3 border-l border-border/50 text-foreground-subtle font-mono text-ui-xs select-none">
          {deferredFilter ? (
            <span>
              {t('files.statusBarFiltered', '{{matched}} / {{total}} 行 · {{cols}} 列 · 工作表: {{sheet}}', {
                matched: processedRows.length,
                total: normalizedRows.length,
                cols: displayHeaders.length,
                sheet: activeSheet || '-',
              })}
            </span>
          ) : (
            <span>
              {t('files.statusBar', '{{rows}} 行 · {{cols}} 列 · 工作表: {{sheet}}', {
                rows: normalizedRows.length,
                cols: displayHeaders.length,
                sheet: activeSheet || '-',
              })}
            </span>
          )}
        </div>
      </div>

      {/* 共享非原生单元格浮动 Tooltip */}
      <CellTooltipPortal anchor={activeAnchor} />
    </div>
  );
});
