import {
  AlertCircle,
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  Check,
  Copy,
  Database,
  Loader2,
  Play,
  Search,
  Table2,
  X,
} from 'lucide-react';
import React, { memo, useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { CellTooltipPortal, useCellTooltip } from '@/components/files/CellTooltip';
import { cn } from '@/lib/utils';

export interface SqliteTableSummary {
  name: string;
  rowCount: number;
}

export interface SqliteMetadata {
  tables: SqliteTableSummary[];
  defaultTable: string;
}

export interface SqliteQueryResult {
  columns: string[];
  rows: string[][];
  totalRows: number;
  query: string;
}

export interface SqlitePreviewProps {
  filePath: string;
  className?: string;
}

const ROW_HEIGHT = 32;
const OVERSCAN = 8;

export const SqlitePreview = memo(function SqlitePreview({
  filePath,
  className,
}: SqlitePreviewProps) {
  const { t } = useTranslation();

  // 1. 数据库元数据与表清单
  const [metaLoading, setMetaLoading] = useState(true);
  const [metaError, setMetaError] = useState<string | null>(null);
  const [metadata, setMetadata] = useState<SqliteMetadata | null>(null);
  const [activeTable, setActiveTable] = useState<string>('');

  // 2. SQL 查询输入与执行结果
  const [sqlInput, setSqlInput] = useState('');
  const [queryLoading, setQueryLoading] = useState(false);
  const [queryError, setQueryError] = useState<string | null>(null);
  const [queryResult, setQueryResult] = useState<SqliteQueryResult | null>(null);

  // 3. 排序态
  const [sortCol, setSortCol] = useState<number | null>(null);
  const [sortOrder, setSortOrder] = useState<'asc' | 'desc' | null>(null);

  // 4. 全局搜索过滤
  const [filterInput, setFilterInput] = useState('');
  const deferredFilter = useDeferredValue(filterInput.trim().toLowerCase());

  // 5. 虚拟滚动容器态
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [containerHeight, setContainerHeight] = useState(500);

  const {
    activeAnchor,
    handleCellMouseEnter,
    handleCellMouseLeave,
    handleContainerScroll,
  } = useCellTooltip();

  // 执行指定 SQL 查询
  const runQuery = useCallback(
    async (sql: string) => {
      const trimmed = sql.trim();
      if (!trimmed) return;
      setQueryLoading(true);
      setQueryError(null);
      setSortCol(null);
      setSortOrder(null);
      try {
        const res = await invoke<SqliteQueryResult>('query_sqlite_data', {
          filePath,
          query: trimmed,
        });
        setQueryResult(res);
      } catch (err) {
        setQueryError(err instanceof Error ? err.message : String(err));
      } finally {
        setQueryLoading(false);
      }
    },
    [filePath],
  );

  // 加载数据库表元数据
  useEffect(() => {
    setMetaLoading(true);
    setMetaError(null);
    invoke<SqliteMetadata>('read_sqlite_metadata', { filePath })
      .then((meta) => {
        setMetadata(meta);
        if (meta.defaultTable) {
          setActiveTable(meta.defaultTable);
          const initialSql = `SELECT * FROM "${meta.defaultTable.replace(/"/g, '""')}" LIMIT 2000;`;
          setSqlInput(initialSql);
          void runQuery(initialSql);
        }
      })
      .catch((err) => {
        setMetaError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        setMetaLoading(false);
      });
  }, [filePath, runQuery]);

  // 切换选中数据表时自动刷新对应查询
  const handleSelectTable = useCallback(
    (tableName: string) => {
      setActiveTable(tableName);
      const sql = `SELECT * FROM "${tableName.replace(/"/g, '""')}" LIMIT 2000;`;
      setSqlInput(sql);
      void runQuery(sql);
    },
    [runQuery],
  );

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

  const rawRows = queryResult?.rows || [];
  const columns = queryResult?.columns || [];

  // 计算排序与过滤
  const processedRows = useMemo(() => {
    let result = rawRows;
    if (deferredFilter) {
      result = result.filter((row) =>
        row.some((cell) => cell.toLowerCase().includes(deferredFilter)),
      );
    }

    if (sortCol !== null && sortOrder !== null) {
      const col = sortCol;
      const orderMul = sortOrder === 'asc' ? 1 : -1;
      result = [...result].sort((a, b) => {
        const valA = (a[col] ?? '').trim();
        const valB = (b[col] ?? '').trim();
        const numA = Number(valA);
        const numB = Number(valB);
        if (!Number.isNaN(numA) && !Number.isNaN(numB) && valA !== '' && valB !== '') {
          return (numA - numB) * orderMul;
        }
        return valA.localeCompare(valB, undefined, { numeric: true }) * orderMul;
      });
    }

    return result;
  }, [rawRows, deferredFilter, sortCol, sortOrder]);

  const handleScroll = useCallback(
    (e: React.UIEvent<HTMLDivElement>) => {
      setScrollTop(e.currentTarget.scrollTop);
      handleContainerScroll();
    },
    [handleContainerScroll],
  );

  const totalRows = processedRows.length;
  const totalHeight = totalRows * ROW_HEIGHT;
  const visibleCount = Math.ceil(containerHeight / ROW_HEIGHT);
  const startIndex = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const endIndex = Math.min(totalRows, startIndex + visibleCount + 2 * OVERSCAN);
  const visibleRows = processedRows.slice(startIndex, endIndex);
  const offsetY = startIndex * ROW_HEIGHT;

  // 复制当前结果为 TSV
  const [copied, setCopied] = useState(false);
  const handleCopy = useCallback(async () => {
    if (!queryResult) return;
    const lines = [columns.join('\t'), ...processedRows.map((r) => r.join('\t'))];
    try {
      await navigator.clipboard.writeText(lines.join('\n'));
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.warn('Copy failed', err);
    }
  }, [columns, processedRows, queryResult]);

  if (metaLoading) {
    return (
      <div className={cn('flex h-full w-full items-center justify-center bg-popover text-ui-sm text-foreground-subtle', className)}>
        <Loader2 className="size-5 animate-spin mr-2" />
        <span>正在读取 SQLite 数据库结构...</span>
      </div>
    );
  }

  if (metaError || !metadata) {
    return (
      <div className={cn('flex h-full w-full flex-col items-center justify-center gap-2.5 bg-popover p-6 text-center', className)}>
        <AlertCircle className="size-6 text-destructive" />
        <span className="text-ui-sm font-semibold text-foreground">无法打开 SQLite 数据库</span>
        <p className="max-w-md font-mono text-ui-xs text-foreground-subtle break-all">
          {metaError}
        </p>
      </div>
    );
  }

  return (
    <div className={cn('flex h-full w-full flex-col bg-popover select-text font-mono text-ui-xs', className)}>
      {/* 顶部 SQL 输入与全局检索条 */}
      <div className="flex shrink-0 flex-col gap-2 border-b border-border/70 bg-surface/50 p-3 select-none">
        <div className="flex items-center gap-2">
          {/* SQL 只读查询输入框 */}
          <div className="relative flex flex-1 items-center">
            <span className="pointer-events-none absolute left-2.5 font-bold text-[11px] text-terminal-blue">
              SQL
            </span>
            <input
              type="text"
              value={sqlInput}
              onChange={(e) => setSqlInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void runQuery(sqlInput);
              }}
              placeholder="输入 SELECT 只读查询语句..."
              className="h-7 w-full rounded-md border border-border/70 bg-background pr-16 pl-12 text-ui-xs text-foreground outline-none transition-colors focus:border-border font-mono"
            />
            <button
              type="button"
              disabled={queryLoading || !sqlInput.trim()}
              onClick={() => void runQuery(sqlInput)}
              className="absolute right-1 flex items-center gap-1 rounded bg-surface px-2 py-0.5 text-ui-xs font-sans font-medium text-foreground hover:bg-hover disabled:opacity-40"
            >
              {queryLoading ? (
                <Loader2 className="size-3 animate-spin text-foreground-subtle" />
              ) : (
                <Play className="size-3 text-terminal-green fill-terminal-green" />
              )}
              <span>{t('files.runSql')}</span>
            </button>
          </div>

          {/* 搜索表格内容 */}
          <div className="relative flex items-center">
            <Search className="pointer-events-none absolute left-2 size-3 text-foreground-subtle" />
            <input
              type="text"
              value={filterInput}
              onChange={(e) => setFilterInput(e.target.value)}
              placeholder={t('files.searchTable')}
              className="h-7 w-36 rounded-md border border-border/70 bg-background pr-6 pl-7 text-ui-xs text-foreground outline-none transition-all focus:w-48 focus:border-border"
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

          {/* 复制数据按钮 */}
          <button
            type="button"
            onClick={handleCopy}
            className="flex items-center gap-1.5 rounded-md border border-border/70 bg-background px-2.5 py-1 text-ui-xs text-foreground-subtle hover:bg-hover hover:text-foreground"
            title={t('files.copyTable')}
          >
            {copied ? (
              <>
                <Check className="size-3 text-terminal-green" />
                <span className="text-terminal-green">{t('files.copied')}</span>
              </>
            ) : (
              <>
                <Copy className="size-3" />
                <span>{t('files.copyTable')}</span>
              </>
            )}
          </button>
        </div>

        {/* 查询报错反馈 */}
        {queryError ? (
          <div className="flex items-center gap-1.5 text-ui-xs text-destructive">
            <AlertCircle className="size-3.5 shrink-0" />
            <span className="break-all">{queryError}</span>
          </div>
        ) : null}
      </div>

      {/* 中部主内容：表格视图 */}
      <div
        ref={(el) => {
          containerRef.current = el;
          if (el && el.clientHeight !== containerHeight) {
            setContainerHeight(el.clientHeight);
          }
        }}
        onScroll={handleScroll}
        className="relative min-h-0 flex-1 overflow-auto scrollbar-slim"
      >
        <table className="w-full border-collapse font-mono text-ui-xs">
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
            {offsetY > 0 ? (
              <tr style={{ height: `${offsetY}px` }}>
                <td colSpan={columns.length + 1} />
              </tr>
            ) : null}

            {visibleRows.map((row, rIdx) => {
              const actualRowIndex = startIndex + rIdx + 1;
              return (
                <tr key={startIndex + rIdx} className="h-8 border-b border-border/30 hover:bg-hover/60">
                  <td className="border-r border-border/40 bg-surface/30 px-2 text-center text-foreground-subtlest select-none">
                    {actualRowIndex}
                  </td>
                  {row.map((cell, cIdx) => (
                    <td
                      key={cIdx}
                      onMouseEnter={(e) => handleCellMouseEnter(e, cell)}
                      onMouseLeave={handleCellMouseLeave}
                      className={cn(
                        'max-w-xs truncate border-r border-border/30 px-3 text-foreground last:border-r-0',
                        cell === 'NULL' ? 'text-foreground-subtlest italic' : '',
                      )}
                    >
                      {cell}
                    </td>
                  ))}
                </tr>
              );
            })}

            {totalHeight - (offsetY + visibleRows.length * ROW_HEIGHT) > 0 ? (
              <tr
                style={{
                  height: `${Math.max(0, totalHeight - (offsetY + visibleRows.length * ROW_HEIGHT))}px`,
                }}
              >
                <td colSpan={columns.length + 1} />
              </tr>
            ) : null}
          </tbody>
        </table>

        {totalRows === 0 && !queryLoading ? (
          <div className="flex h-32 items-center justify-center text-ui-sm text-foreground-subtle font-sans">
            {deferredFilter ? t('files.noMatchingRows') : '无查询结果'}
          </div>
        ) : null}
      </div>

      {/* 底部表清单 Tabs 栏与状态统计 */}
      <div className="flex shrink-0 items-center justify-between border-t border-border/70 bg-surface/60 px-3 py-1.5 text-ui-xs">
        {/* 数据表选择横向滑动栏：细胶囊滚动条（scrollbar-hair 3px、hover 4px），防止压边切半 */}
        <div className="scrollbar-hair flex items-center gap-1 overflow-x-auto min-w-0 pr-2">
          <div className="flex shrink-0 items-center gap-1 text-foreground-subtle mr-1 select-none font-sans font-medium">
            <Database className="size-3.5 text-terminal-cyan" />
            <span>{t('files.sqliteTables')}:</span>
          </div>
          {metadata.tables.map((tbl) => {
            const isActive = tbl.name === activeTable;
            return (
              <button
                key={tbl.name}
                type="button"
                onClick={() => handleSelectTable(tbl.name)}
                className={cn(
                  'flex shrink-0 items-center gap-1.5 rounded-md px-2.5 py-1 text-ui-xs transition-colors border select-none',
                  isActive
                    ? 'bg-selected font-semibold text-foreground border-border shadow-xs'
                    : 'text-foreground-subtle border-transparent hover:bg-hover hover:text-foreground',
                )}
              >
                <Table2 className="size-3 text-foreground-subtle" />
                <span className="max-w-[140px] truncate">{tbl.name}</span>
                <span className="rounded bg-surface px-1 text-[10px] text-foreground-subtler">
                  {tbl.rowCount}
                </span>
              </button>
            );
          })}
        </div>

        {/* 状态统计 */}
        <div className="shrink-0 flex items-center gap-2 pl-3 border-l border-border/50 text-foreground-subtle font-mono text-ui-xs select-none">
          {deferredFilter ? (
            <span>
              {t('files.matchedRows', { count: processedRows.length, total: rawRows.length })}
            </span>
          ) : (
            <span>
              {rawRows.length} 行 · {columns.length} 列
            </span>
          )}
        </div>
      </div>

      {/* 单元格截断 Tooltip */}
      <CellTooltipPortal anchor={activeAnchor} />
    </div>
  );
});
