/**
 * WebHook Shell / Batch 命令 Monaco 代码编辑器：
 * - 支持 Shell (Bash/Zsh/sh) 与 Windows Batch (cmd/bat) 语法高亮
 * - 智能自动适配：Windows 本地命令默认 Batch，非 Windows 或 SSH 远端默认 Shell
 * - 支持光标处精准插入插值变量（${prev.stdout}, ${prev.data.xxx}, ${path.xxx}, ${body.xxx}）
 * - 支持查找替换、自动换行、快捷复制代码
 */

import * as monaco from 'monaco-editor';
import { Copy, Check, Terminal, Sparkles, ArrowLeftRight } from 'lucide-react';
import { memo, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { setupMonacoEnvironment } from '@/editor/monacoEnvironment';
import { initMonacoTheme, VELOTERM_MONACO_THEME } from '@/editor/monacoTheme';
import { ensureExtendedLanguages } from '@/editor/languages';
import { attachFindReplaceCommands, handleEditorKeyDownCapture } from '@/editor/findReplace';
import { Button } from '@/components/ui/button';
import { platformKind } from '@/lib/platform';
import { cn } from '@/lib/utils';

export interface WebhookCommandEditorProps {
  value: string;
  onChange: (val: string) => void;
  actionType: 'local_command' | 'ssh_command';
  className?: string;
  height?: number | string;
  isMultiStep?: boolean;
  stepIndex?: number;
  readOnly?: boolean;
}

export const WebhookCommandEditor = memo(function WebhookCommandEditor({
  value,
  onChange,
  actionType,
  className,
  height = 180,
  isMultiStep = false,
  stepIndex = 0,
  readOnly = false,
}: WebhookCommandEditorProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const modelRef = useRef<monaco.editor.ITextModel | null>(null);
  const [copied, setCopied] = useState(false);

  // 默认语言检测：如果是 Windows 本地命令，默认为 Batch；否则为 Bash / Shell
  const defaultLanguage =
    actionType === 'local_command' && platformKind === 'windows' ? 'batch' : 'shell';

  const [language, setLanguage] = useState<'shell' | 'batch'>(defaultLanguage);

  // 1. 初始化 Monaco 编辑器
  useEffect(() => {
    if (!containerRef.current) return;
    setupMonacoEnvironment();
    initMonacoTheme();
    ensureExtendedLanguages(monaco);

    const ext = language === 'batch' ? 'bat' : 'sh';
    const uri = monaco.Uri.parse(`inmemory://webhook-cmd-editor-${crypto.randomUUID()}.${ext}`);
    const model = monaco.editor.createModel(value, language, uri);
    model.onDidChangeContent(() => {
      onChange(model.getValue());
    });
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
      folding: true,
      readOnly,
      overviewRulerLanes: 0,
      hideCursorInOverviewRuler: true,
      overviewRulerBorder: false,
      scrollbar: {
        vertical: 'auto',
        horizontal: 'auto',
        verticalScrollbarSize: 7,
        horizontalScrollbarSize: 7,
        useShadows: false,
      },
      padding: { top: 8, bottom: 8 },
      tabSize: 2,
    });
    editorRef.current = editor;

    attachFindReplaceCommands(editor);

    return () => {
      editor.dispose();
      model.dispose();
      editorRef.current = null;
      modelRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 当语言模式改变时动态切换 Monaco 语言
  useEffect(() => {
    if (modelRef.current) {
      monaco.editor.setModelLanguage(modelRef.current, language);
    }
  }, [language]);

  // 当外部 value 改变且与当前 model 不一致时同步（如切换流水线步骤）
  useEffect(() => {
    if (modelRef.current && modelRef.current.getValue() !== value) {
      modelRef.current.setValue(value);
    }
  }, [value]);

  const handleCopy = () => {
    if (!editorRef.current) return;
    void navigator.clipboard.writeText(editorRef.current.getValue());
    setCopied(true);
    setTimeout(() => setCopied(false), 1800);
  };

  const handleInsertVariable = (snippet: string) => {
    if (!editorRef.current) return;
    const editor = editorRef.current;
    const selection = editor.getSelection();
    if (!selection) return;
    editor.executeEdits('insert-variable', [
      {
        range: selection,
        text: snippet,
        forceMoveMarkers: true,
      },
    ]);
    editor.focus();
  };

  const toggleLanguage = () => {
    setLanguage((prev) => (prev === 'shell' ? 'batch' : 'shell'));
  };

  return (
    <div
      className={cn(
        'group flex flex-col rounded-lg border border-border bg-surface/80 overflow-hidden shadow-xs transition-colors focus-within:border-border-hover',
        className,
      )}
    >
      {/* 编辑器顶栏辅助条 */}
      <div className="flex h-7 shrink-0 items-center justify-between border-b border-border/50 bg-surface/50 px-2.5 text-[11px] text-foreground-subtle select-none">
        <div className="flex items-center gap-2">
          <Terminal className="size-3 text-terminal-green" />
          <span className="font-mono font-medium text-foreground">
            {language === 'batch' ? 'Batch (CMD)' : 'Bash (Shell)'}
          </span>
          <button
            type="button"
            onClick={toggleLanguage}
            className="flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] text-foreground-subtlest hover:bg-hover hover:text-foreground transition-colors"
            title={t('webhook.switchLanguage', { defaultValue: '切换语法高亮 (Bash / Batch)' })}
          >
            <ArrowLeftRight className="size-2.5" />
            <span>{language === 'batch' ? t('webhook.switchToBash', { defaultValue: '切至 Bash' }) : t('webhook.switchToBat', { defaultValue: '切至 Bat' })}</span>
          </button>

          <span className="text-border">|</span>

          {/* 快捷变量插值药丸 */}
          <div className="flex items-center gap-1.5 text-[10px]">
            {isMultiStep && stepIndex > 0 && (
              <>
                <button
                  type="button"
                  onClick={() => handleInsertVariable('${prev.stdout}')}
                  className="flex items-center gap-0.5 rounded bg-surface px-1.5 py-0.5 font-mono text-terminal-green border border-border/50 hover:border-border hover:bg-hover transition-colors"
                  title={t('webhook.insertPrevStdout', { defaultValue: '插入前一步标准输出' })}
                >
                  <Sparkles className="size-2 text-terminal-green" />
                  <span>${'{prev.stdout}'}</span>
                </button>
                <button
                  type="button"
                  onClick={() => handleInsertVariable('${prev.data.tag}')}
                  className="flex items-center gap-0.5 rounded bg-surface px-1.5 py-0.5 font-mono text-terminal-yellow border border-border/50 hover:border-border hover:bg-hover transition-colors"
                  title={t('webhook.insertPrevData', { defaultValue: '插入前一步结构化数据字段' })}
                >
                  <Sparkles className="size-2 text-terminal-yellow" />
                  <span>${'{prev.data.tag}'}</span>
                </button>
              </>
            )}
            <button
              type="button"
              onClick={() => handleInsertVariable('${path.service}')}
              className="flex items-center gap-0.5 rounded bg-surface px-1.5 py-0.5 font-mono text-terminal-blue border border-border/50 hover:border-border hover:bg-hover transition-colors"
              title={t('webhook.insertPathParam', { defaultValue: '插入路径动态参数' })}
            >
              <span>${'{path.service}'}</span>
            </button>
            <button
              type="button"
              onClick={() => handleInsertVariable('${body.ref}')}
              className="flex items-center gap-0.5 rounded bg-surface px-1.5 py-0.5 font-mono text-purple-400 border border-border/50 hover:border-border hover:bg-hover transition-colors"
              title={t('webhook.insertBodyParam', { defaultValue: '插入请求体 JSON 字段' })}
            >
              <span>${'{body.ref}'}</span>
            </button>
          </div>
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

      {/* Monaco 容器 */}
      <div
        style={{ height: typeof height === 'number' ? `${height}px` : height }}
        onKeyDownCapture={(e) => handleEditorKeyDownCapture(e, editorRef.current)}
        className="relative w-full"
      >
        <div ref={containerRef} className="absolute inset-0 h-full w-full" />
      </div>
    </div>
  );
});
