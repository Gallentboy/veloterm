import {
  AlertCircle,
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  Filter,
  Loader2,
  Minimize2,
  Maximize2,
  RotateCcw,
} from 'lucide-react';
import { memo, useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { cn } from '@/lib/utils';

export interface JsonPreviewProps {
  content: string;
  fileName?: string;
  className?: string;
}

type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

/** 递归解析普通 JSON 或 JSON Lines */
function parseJsonInput(raw: string, isJsonl: boolean): { data: JsonValue; isLines: boolean; error: string | null } {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { data: null, isLines: false, error: null };
  }

  // 1. 如果明确是 jsonl，或者多行且每行以 { 或 [ 开头
  if (isJsonl) {
    try {
      const lines = trimmed
        .split(/\r?\n/)
        .map((l) => l.trim())
        .filter(Boolean);
      const items = lines.map((line) => JSON.parse(line) as JsonValue);
      return { data: items, isLines: true, error: null };
    } catch {
      // jsonl 解析失败则尝试作为常规 json 解析
    }
  }

  // 2. 常规 JSON 解析
  try {
    const data = JSON.parse(trimmed) as JsonValue;
    return { data, isLines: false, error: null };
  } catch (err) {
    // 3. 兜底检测是否为未经声明后缀的 jsonl
    const lines = trimmed
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter(Boolean);
    if (lines.length > 1) {
      try {
        const items = lines.map((line) => JSON.parse(line) as JsonValue);
        return { data: items, isLines: true, error: null };
      } catch {
        // ignore
      }
    }
    return {
      data: null,
      isLines: false,
      error: err instanceof Error ? err.message : String(err),
    };
  }
}

/** 单个 JSON 节点的树状展示 */
const JsonNode = memo(function JsonNode({
  name,
  value,
  path,
  depth,
  defaultExpanded,
  expandAllSignal,
  collapseAllSignal,
}: {
  name?: string | number;
  value: JsonValue;
  path: string;
  depth: number;
  defaultExpanded: boolean;
  expandAllSignal: number;
  collapseAllSignal: number;
}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState<boolean>(defaultExpanded);
  const [copiedType, setCopiedType] = useState<'path' | 'val' | null>(null);

  // 响应全局展开/折叠信号
  useEffect(() => {
    if (expandAllSignal > 0) setExpanded(true);
  }, [expandAllSignal]);

  useEffect(() => {
    if (collapseAllSignal > 0) setExpanded(false);
  }, [collapseAllSignal]);

  const isObject = value !== null && typeof value === 'object' && !Array.isArray(value);
  const isArray = Array.isArray(value);
  const isContainer = isObject || isArray;

  const copyText = useCallback(async (text: string, type: 'path' | 'val') => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedType(type);
      setTimeout(() => setCopiedType(null), 1500);
    } catch (err) {
      console.warn('Copy failed', err);
    }
  }, []);

  // 渲染基础标量值
  const renderPrimitive = (val: JsonValue) => {
    if (val === null) {
      return <span className="text-foreground-subtlest italic">null</span>;
    }
    if (typeof val === 'boolean') {
      return <span className="font-semibold text-terminal-magenta">{val ? 'true' : 'false'}</span>;
    }
    if (typeof val === 'number') {
      return <span className="text-terminal-yellow">{val}</span>;
    }
    if (typeof val === 'string') {
      return <span className="text-terminal-green break-all">"{val}"</span>;
    }
    return null;
  };

  const currentPath = name !== undefined ? (typeof name === 'number' ? `${path}[${name}]` : `${path}.${name}`) : path;

  return (
    <div className="font-mono text-ui-xs leading-5">
      <div className="group flex items-center gap-1 rounded py-0.5 pr-2 transition-colors hover:bg-hover/50">
        {isContainer ? (
          <button
            type="button"
            onClick={() => setExpanded(!expanded)}
            className="flex size-4 shrink-0 items-center justify-center rounded text-foreground-subtle hover:text-foreground"
          >
            {expanded ? (
              <ChevronDown className="size-3" />
            ) : (
              <ChevronRight className="size-3" />
            )}
          </button>
        ) : (
          <span className="size-4 shrink-0" />
        )}

        {name !== undefined ? (
          <span className="shrink-0 text-terminal-cyan select-none">
            {typeof name === 'number' ? name : `"${name}"`}:
          </span>
        ) : null}

        {isContainer ? (
          <div
            onClick={() => setExpanded(!expanded)}
            className="flex cursor-pointer items-center gap-1 text-foreground-subtle select-none"
          >
            {isArray ? (
              <>
                <span className="text-foreground font-semibold">[</span>
                {!expanded ? (
                  <span className="rounded bg-surface px-1 text-[0.8em] text-foreground-subtle">
                    {(value as JsonValue[]).length} items
                  </span>
                ) : null}
                {!expanded ? <span className="text-foreground font-semibold">]</span> : null}
              </>
            ) : (
              <>
                <span className="text-foreground font-semibold">{'{'}</span>
                {!expanded ? (
                  <span className="rounded bg-surface px-1 text-[0.8em] text-foreground-subtle">
                    {Object.keys(value as object).length} keys
                  </span>
                ) : null}
                {!expanded ? <span className="text-foreground font-semibold">{'}'}</span> : null}
              </>
            )}
          </div>
        ) : (
          renderPrimitive(value)
        )}

        {/* 悬停辅助快捷动作 */}
        <div className="ml-auto hidden items-center gap-1.5 opacity-0 transition-opacity group-hover:flex group-hover:opacity-100 select-none">
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              void copyText(currentPath || '.', 'path');
            }}
            className="flex items-center gap-0.5 rounded px-1 text-[0.8em] text-foreground-subtle hover:bg-surface hover:text-foreground"
            title={copiedType === 'path' ? t('files.copied') : t('files.copyPath')}
          >
            {copiedType === 'path' ? (
              <Check className="size-2.5 text-terminal-green" />
            ) : (
              <span className="text-terminal-blue">path</span>
            )}
          </button>
          {!isContainer ? (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void copyText(String(value), 'val');
              }}
              className="flex items-center gap-0.5 rounded px-1 text-[0.8em] text-foreground-subtle hover:bg-surface hover:text-foreground"
              title={copiedType === 'val' ? t('files.copied') : t('files.copyValue')}
            >
              {copiedType === 'val' ? (
                <Check className="size-2.5 text-terminal-green" />
              ) : (
                <Copy className="size-2.5" />
              )}
            </button>
          ) : null}
        </div>
      </div>

      {/* 展开子树 */}
      {isContainer && expanded ? (
        <div className="ml-2.5 border-l border-border/50 pl-2">
          {isArray
            ? (value as JsonValue[]).map((child, idx) => (
                <JsonNode
                  key={idx}
                  name={idx}
                  value={child}
                  path={currentPath}
                  depth={depth + 1}
                  defaultExpanded={depth < 1}
                  expandAllSignal={expandAllSignal}
                  collapseAllSignal={collapseAllSignal}
                />
              ))
            : Object.entries(value as { [k: string]: JsonValue }).map(([k, v]) => (
                <JsonNode
                  key={k}
                  name={k}
                  value={v}
                  path={currentPath}
                  depth={depth + 1}
                  defaultExpanded={depth < 1}
                  expandAllSignal={expandAllSignal}
                  collapseAllSignal={collapseAllSignal}
                />
              ))}
          <div className="text-foreground font-semibold select-none">
            {isArray ? ']' : '}'}
          </div>
        </div>
      ) : null}
    </div>
  );
});

export const JsonPreview = memo(function JsonPreview({
  content,
  fileName,
  className,
}: JsonPreviewProps) {
  const { t } = useTranslation();
  const isJsonl = fileName ? /\.jsonl$/i.test(fileName) : false;

  // 1. 解析原始 JSON 数据
  const parsed = useMemo(() => parseJsonInput(content, isJsonl), [content, isJsonl]);

  // 2. 当前活跃数据（如果经过 jq 过滤，展示过滤后结果；否则为原数据）
  const [activeData, setActiveData] = useState<JsonValue>(parsed.data);
  const [jqError, setJqError] = useState<string | null>(null);
  const [jqLoading, setJqLoading] = useState(false);
  const [jqFilter, setJqFilter] = useState('');
  const [isFiltered, setIsFiltered] = useState(false);

  // 当原文件内容改变时重置
  useEffect(() => {
    setActiveData(parsed.data);
    setJqError(null);
    setIsFiltered(false);
  }, [parsed.data]);

  // 3. 执行 jq 过滤
  const handleRunJq = useCallback(async () => {
    const expr = jqFilter.trim();
    if (!expr) {
      setActiveData(parsed.data);
      setIsFiltered(false);
      setJqError(null);
      return;
    }

    setJqLoading(true);
    setJqError(null);
    try {
      // 传递格式化好的标准 JSON 传给后端内置 jq
      const input = parsed.isLines && Array.isArray(parsed.data)
        ? parsed.data.map((item) => JSON.stringify(item)).join('\n')
        : typeof parsed.data === 'string'
        ? parsed.data
        : JSON.stringify(parsed.data, null, 2);

      const res = await invoke<string>('execute_jq_filter', {
        jsonInput: input,
        filter: expr,
      });

      const next = parseJsonInput(res, false);
      if (next.error) {
        // 如果输出的是非标准 json（例如 jq 输出标量文本），保留原始字符串
        setActiveData(res.trim());
      } else {
        setActiveData(next.data);
      }
      setIsFiltered(true);
    } catch (err) {
      setJqError(err instanceof Error ? err.message : String(err));
    } finally {
      setJqLoading(false);
    }
  }, [jqFilter, parsed]);

  const handleResetJq = useCallback(() => {
    setJqFilter('');
    setActiveData(parsed.data);
    setIsFiltered(false);
    setJqError(null);
  }, [parsed.data]);

  // 4. 全局展开/折叠信号
  const [expandSignal, setExpandSignal] = useState(0);
  const [collapseSignal, setCollapseSignal] = useState(0);

  // 5. 复制当前格式化 JSON
  const [copiedAll, setCopiedAll] = useState(false);
  const handleCopyAll = useCallback(async () => {
    try {
      const text = typeof activeData === 'string'
        ? activeData
        : JSON.stringify(activeData, null, 2);
      await navigator.clipboard.writeText(text);
      setCopiedAll(true);
      setTimeout(() => setCopiedAll(false), 2000);
    } catch (err) {
      console.warn('Copy all failed', err);
    }
  }, [activeData]);

  if (parsed.error && !activeData) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center">
        <AlertCircle className="size-6 text-warning" />
        <span className="text-ui-sm font-semibold text-foreground">
          {t('files.jsonInvalid')}
        </span>
        <p className="max-w-md font-mono text-ui-xs text-foreground-subtle break-all">
          {parsed.error}
        </p>
      </div>
    );
  }

  return (
    <div className={cn('flex h-full w-full flex-col bg-popover select-text', className)}>
      {/* 顶部 jq 过滤器与快捷控制条 */}
      <div className="flex shrink-0 flex-col gap-1.5 border-b border-border/70 bg-surface/50 px-4 py-2">
        <div className="flex items-center justify-between gap-3">
          {/* jq 表达式输入框 */}
          <div className="relative flex flex-1 items-center">
            <span className="pointer-events-none absolute left-2.5 flex items-center gap-1 font-mono text-[11px] font-bold text-terminal-magenta">
              <Filter className="size-3" />
              jq
            </span>
            <input
              type="text"
              value={jqFilter}
              onChange={(e) => setJqFilter(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void handleRunJq();
              }}
              placeholder={t('files.jqFilterPlaceholder')}
              className="h-7 w-full rounded-md border border-border/70 bg-background pr-16 pl-12 font-mono text-ui-xs text-foreground outline-none transition-colors focus:border-border"
            />
            <div className="absolute right-1 flex items-center gap-1">
              {isFiltered ? (
                <button
                  type="button"
                  onClick={handleResetJq}
                  className="rounded p-1 text-foreground-subtle hover:bg-hover hover:text-foreground"
                  title={t('files.resetJq')}
                >
                  <RotateCcw className="size-3" />
                </button>
              ) : null}
              <button
                type="button"
                onClick={() => void handleRunJq()}
                disabled={jqLoading || !jqFilter.trim()}
                className="flex items-center gap-1 rounded bg-surface px-2 py-0.5 text-ui-xs font-medium text-foreground transition-colors hover:bg-hover disabled:opacity-40"
              >
                {jqLoading ? (
                  <Loader2 className="size-3 animate-spin text-foreground-subtle" />
                ) : (
                  <span>{t('files.runJq')}</span>
                )}
              </button>
            </div>
          </div>

          {/* 辅助按钮组 */}
          <div className="flex shrink-0 items-center gap-1 text-ui-xs select-none">
            <button
              type="button"
              onClick={() => setExpandSignal((s) => s + 1)}
              className="flex items-center gap-1 rounded border border-border/60 bg-background px-2 py-1 text-foreground-subtle hover:bg-hover hover:text-foreground"
              title={t('files.expandAll')}
            >
              <Maximize2 className="size-3" />
              <span>{t('files.expandAll')}</span>
            </button>
            <button
              type="button"
              onClick={() => setCollapseSignal((s) => s + 1)}
              className="flex items-center gap-1 rounded border border-border/60 bg-background px-2 py-1 text-foreground-subtle hover:bg-hover hover:text-foreground"
              title={t('files.collapseAll')}
            >
              <Minimize2 className="size-3" />
              <span>{t('files.collapseAll')}</span>
            </button>
            <button
              type="button"
              onClick={handleCopyAll}
              className="flex items-center gap-1 rounded border border-border/60 bg-background px-2 py-1 text-foreground-subtle hover:bg-hover hover:text-foreground"
              title={copiedAll ? t('files.copied') : t('files.copyJson')}
            >
              {copiedAll ? (
                <>
                  <Check className="size-3 text-terminal-green" />
                  <span className="text-terminal-green">{t('files.copied')}</span>
                </>
              ) : (
                <>
                  <Copy className="size-3" />
                  <span>{t('files.copyJson')}</span>
                </>
              )}
            </button>
          </div>
        </div>

        {/* jq 错误警示 */}
        {jqError ? (
          <div className="flex items-center gap-1.5 text-ui-xs text-warning">
            <AlertCircle className="size-3.5 shrink-0" />
            <span className="font-mono break-all">{jqError}</span>
          </div>
        ) : null}

        {/* JSONL 识别提示 */}
        {parsed.isLines ? (
          <div className="text-[11px] text-foreground-subtle select-none">
            💡 {t('files.jsonlHint')}
          </div>
        ) : null}
      </div>

      {/* JSON 树内容区 */}
      <div className="min-h-0 flex-1 overflow-auto p-4">
        <JsonNode
          value={activeData}
          path=""
          depth={0}
          defaultExpanded={true}
          expandAllSignal={expandSignal}
          collapseAllSignal={collapseSignal}
        />
      </div>
    </div>
  );
});
