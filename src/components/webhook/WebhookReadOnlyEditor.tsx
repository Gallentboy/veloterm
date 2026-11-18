/**
 * WebHook 只读 Monaco 代码查看器：
 * - 专门用于模拟请求体 (JSON Body)、执行结果 stdout/stderr 等展示场景
 * - 严格只读（readOnly + domReadOnly），禁止用户误触输入
 * - 高度按内容自适应（基于 editor.getContentHeight 动态自适应，不留大片空白，也不突兀撑死）
 * - 内置一键复制与语言标识
 */

import * as monaco from 'monaco-editor';
import { Copy, Check, FileJson, Terminal, AlertCircle } from 'lucide-react';
import { memo, useEffect, useRef, useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { setupMonacoEnvironment } from '@/editor/monacoEnvironment';
import { initMonacoTheme, VELOTERM_MONACO_THEME } from '@/editor/monacoTheme';
import { ensureExtendedLanguages } from '@/editor/languages';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

export interface WebhookReadOnlyEditorProps {
  value: string;
  language?: 'json' | 'shell' | 'batch' | 'plaintext';
  className?: string;
  minHeight?: number;
  maxHeight?: number;
  isError?: boolean;
  headerTitle?: string;
}

export const WebhookReadOnlyEditor = memo(function WebhookReadOnlyEditor({
  value,
  language = 'json',
  className,
  minHeight = 64,
  maxHeight = 360,
  isError = false,
  headerTitle,
}: WebhookReadOnlyEditorProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const modelRef = useRef<monaco.editor.ITextModel | null>(null);
  const [copied, setCopied] = useState(false);

  // 初始基于文本行数预估高度，避免首帧高度为 0 导致跳动（每行约 19px + 上下 padding 16px）
  const initialEstimatedHeight = Math.max(
    minHeight,
    Math.min(maxHeight, (value.split('\n').length || 1) * 19 + 16),
  );
  const [editorHeight, setEditorHeight] = useState<number>(initialEstimatedHeight);

  // 动态更新高度以按内容自适应
  const updateContentHeight = useCallback(() => {
    if (!editorRef.current) return;
    const contentHeight = editorRef.current.getContentHeight();
    const clampedHeight = Math.max(minHeight, Math.min(maxHeight, contentHeight));
    setEditorHeight(clampedHeight);
    editorRef.current.layout();
  }, [minHeight, maxHeight]);

  // 1. 初始化 Monaco 只读编辑器
  useEffect(() => {
    if (!containerRef.current) return;
    setupMonacoEnvironment();
    initMonacoTheme();
    ensureExtendedLanguages(monaco);

    const uri = monaco.Uri.parse(`inmemory://webhook-readonly-${crypto.randomUUID()}.${language === 'json' ? 'json' : 'txt'}`);
    const model = monaco.editor.createModel(value, language, uri);
    modelRef.current = model;

    const editor = monaco.editor.create(containerRef.current, {
      model,
      theme: VELOTERM_MONACO_THEME,
      automaticLayout: true,
      fontFamily: 'var(--font-mono)',
      fontLigatures: true,
      fontSize: 12.5,
      lineHeight: 19,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      wordWrap: 'on',
      glyphMargin: false,
      folding: false,
      lineNumbers: 'off',
      readOnly: true,
      domReadOnly: true,
      quickSuggestions: false,
      contextmenu: false,
      renderLineHighlight: 'none',
      overviewRulerLanes: 0,
      hideCursorInOverviewRuler: true,
      overviewRulerBorder: false,
      scrollbar: {
        vertical: 'auto',
        horizontal: 'hidden',
        verticalScrollbarSize: 6,
        useShadows: false,
      },
      padding: { top: 8, bottom: 8 },
    });
    editorRef.current = editor;

    // 内容尺寸变化时自适应高度
    const sizeListener = editor.onDidContentSizeChange((e) => {
      if (e.contentHeightChanged) {
        updateContentHeight();
      }
    });

    // 初始布局校正
    requestAnimationFrame(() => {
      updateContentHeight();
    });

    return () => {
      sizeListener.dispose();
      editor.dispose();
      model.dispose();
      editorRef.current = null;
      modelRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [language]);

  // 当外部 value 改变时同步到 model
  useEffect(() => {
    if (modelRef.current && modelRef.current.getValue() !== value) {
      modelRef.current.setValue(value);
      requestAnimationFrame(() => {
        updateContentHeight();
      });
    }
  }, [value, updateContentHeight]);

  const handleCopy = () => {
    void navigator.clipboard.writeText(value);
    setCopied(true);
    setTimeout(() => setCopied(false), 1800);
  };

  const getLanguageLabel = () => {
    if (headerTitle) return headerTitle;
    if (language === 'json') return 'JSON';
    if (language === 'shell') return 'Shell Output';
    if (language === 'batch') return 'Batch Output';
    return 'Plain Text';
  };

  const renderIcon = () => {
    if (isError) return <AlertCircle className="size-3 text-destructive" />;
    if (language === 'json') return <FileJson className="size-3 text-terminal-yellow" />;
    return <Terminal className="size-3 text-terminal-green" />;
  };

  return (
    <div
      className={cn(
        'group flex flex-col rounded-lg border overflow-hidden shadow-xs transition-colors',
        isError ? 'border-destructive/40 bg-input' : 'border-border bg-input',
        className,
      )}
    >
      {/* 顶部微型状态栏（语言说明 + 快捷复制） */}
      <div className="flex h-6.5 shrink-0 items-center justify-between border-b border-border/50 bg-surface/50 px-2.5 text-[11px] text-foreground-subtle select-none">
        <div className="flex items-center gap-1.5">
          {renderIcon()}
          <span className="font-mono font-medium text-foreground text-[10px]">
            {getLanguageLabel()}
          </span>
          <span className="text-[10px] text-foreground-subtlest">
            ({t('common.readOnly', { defaultValue: '只读' })})
          </span>
        </div>

        <div className="flex items-center gap-1.5">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={handleCopy}
            className="h-5 px-1.5 text-[10px] text-foreground-subtle hover:text-foreground"
          >
            {copied ? <Check className="size-2.5 text-terminal-green mr-1" /> : <Copy className="size-2.5 mr-1" />}
            <span>{copied ? t('common.copied', { defaultValue: '已复制' }) : t('common.copy', { defaultValue: '复制' })}</span>
          </Button>
        </div>
      </div>

      {/* Monaco 只读容器：高度按内容严格动态自适应 */}
      <div
        style={{ height: `${editorHeight}px` }}
        className="relative w-full transition-[height] duration-150 ease-out"
      >
        <div ref={containerRef} className="absolute inset-0 h-full w-full" />
      </div>
    </div>
  );
});
