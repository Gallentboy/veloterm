import { ArrowDown, ArrowUp, CaseSensitive, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { SearchAddon } from '@xterm/addon-search';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

interface TerminalSearchBarProps {
  searchAddon: SearchAddon;
  visible: boolean;
  onClose: () => void;
}

/**
 * 终端搜索栏（自绘，对齐 ZCode / 浏览器 Ctrl+F 交互）：
 * - 输入即搜（incremental 扩展选区）；Enter/Shift+Enter 与 ↑↓ 按钮在匹配项间跳转；
 * - Aa 切换大小写敏感并重搜；Esc / X 关闭并清除全部高亮；
 * - 匹配计数经 `onDidChangeResults` 订阅展示「当前/总数」（超阈值时 addon 上报 -1，显示省略）；
 * - 样式走工程色板（bg-menu / border-border），浅深主题自动适配；
 *   匹配高亮沿用 addon 默认黄底/红描边，与浏览器行为一致。
 */
export function TerminalSearchBar({ searchAddon, visible, onClose }: TerminalSearchBarProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [resultIndex, setResultIndex] = useState(-1);
  const [resultTotal, setResultTotal] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const caseSensitiveRef = useRef(caseSensitive);
  caseSensitiveRef.current = caseSensitive;

  // 显示时聚焦并全选上次输入，隐藏时清除高亮
  useEffect(() => {
    if (visible) {
      inputRef.current?.focus();
      inputRef.current?.select();
    } else {
      searchAddon.clearDecorations();
    }
  }, [visible, searchAddon]);

  // 匹配计数订阅（组件常驻挂载，仅 visible 时展示）
  useEffect(() => {
    const disposable = searchAddon.onDidChangeResults((event) => {
      setResultIndex(event.resultIndex);
      setResultTotal(event.resultCount);
    });
    return () => disposable.dispose();
  }, [searchAddon]);

  if (!visible) return null;

  const searchOptions = () => ({ caseSensitive: caseSensitiveRef.current });

  const runSearch = (nextQuery: string, opts?: { incremental?: boolean; reverse?: boolean }) => {
    if (!nextQuery) {
      searchAddon.clearDecorations();
      return;
    }
    if (opts?.reverse) {
      searchAddon.findPrevious(nextQuery, searchOptions());
    } else {
      searchAddon.findNext(nextQuery, {
        ...searchOptions(),
        incremental: opts?.incremental ?? false,
      });
    }
  };

  const close = () => {
    searchAddon.clearDecorations();
    setQuery('');
    onClose();
  };

  const toggleCase = () => {
    const next = !caseSensitive;
    setCaseSensitive(next);
    if (query) {
      searchAddon.findNext(query, { caseSensitive: next });
    }
  };

  // 超过装饰器上限时 addon 上报 resultIndex = -1，此时展示总数省略当前序号
  const resultLabel =
    query && resultTotal > 0
      ? resultIndex >= 0
        ? `${resultIndex + 1}/${resultTotal}`
        : `${resultTotal}+`
      : '';

  return (
    <div
      data-terminal-search-bar
      className={cn(
        'absolute top-2 right-3 z-40 flex h-8 items-center gap-1 rounded-lg border border-border bg-menu pl-2 pr-1 shadow-md',
        !visible && 'hidden',
      )}
    >
      <input
        ref={inputRef}
        value={query}
        onChange={(event) => {
          setQuery(event.target.value);
          runSearch(event.target.value, { incremental: true });
        }}
        onKeyDown={(event) => {
          event.stopPropagation();
          if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
            event.preventDefault();
            runSearch(query, { reverse: event.shiftKey });
          } else if (event.key === 'Escape') {
            event.preventDefault();
            close();
          }
        }}
        onPointerDown={(event) => event.stopPropagation()}
        onDoubleClick={(event) => event.stopPropagation()}
        placeholder={t('terminal.search.placeholder')}
        aria-label={t('terminal.search.placeholder')}
        className="h-6.5 w-44 rounded border border-border bg-input px-1.5 text-ui-sm text-foreground outline-none placeholder:text-foreground-subtlest select-text focus:border-border-hover"
      />
      <span className="w-12 shrink-0 text-center font-mono text-ui-sm text-foreground-subtle select-none">
        {resultLabel}
      </span>

      <ControlHintTooltip title={t('terminal.search.caseSensitive')} side="bottom">
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          onClick={toggleCase}
          aria-pressed={caseSensitive}
          className={cn(
            'size-6 rounded text-foreground-subtle hover:text-foreground hover:bg-hover',
            caseSensitive && 'bg-selected text-foreground',
          )}
        >
          <CaseSensitive className="size-3.5" />
        </Button>
      </ControlHintTooltip>
      <ControlHintTooltip title={t('terminal.search.previous')} side="bottom">
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          onClick={() => runSearch(query, { reverse: true })}
          disabled={!query}
          aria-label={t('terminal.search.previous')}
          className="size-6 rounded text-foreground-subtle hover:text-foreground hover:bg-hover"
        >
          <ArrowUp className="size-3.5" />
        </Button>
      </ControlHintTooltip>
      <ControlHintTooltip title={t('terminal.search.next')} side="bottom">
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          onClick={() => runSearch(query)}
          disabled={!query}
          aria-label={t('terminal.search.next')}
          className="size-6 rounded text-foreground-subtle hover:text-foreground hover:bg-hover"
        >
          <ArrowDown className="size-3.5" />
        </Button>
      </ControlHintTooltip>
      <ControlHintTooltip title={t('common.close')} side="bottom">
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          onClick={close}
          aria-label={t('common.close')}
          className="size-6 rounded text-foreground-subtle hover:text-foreground hover:bg-hover"
        >
          <X className="size-3.5" />
        </Button>
      </ControlHintTooltip>
    </div>
  );
}
