/**
 * WebHook JS / TS 脚本 Monaco 代码编辑器：
 * - 采用与「JS 脚本 (JsScriptScreen)」完全同构的 Monaco Editor 引擎与样式主题
 * - 注入全局 `webhook` 对象的 TypeScript 类型声明（`webhook.req`, `webhook.prev`, `webhook.steps`）
 * - 注入标准沙箱环境 d.ts（console, fetch, fs, crypto, sys）
 * - 支持查找替换、自动换行、代码折叠与快捷格式化
 */

import * as monaco from 'monaco-editor';
import { Copy, Check, Code2, Sparkles } from 'lucide-react';
import { memo, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { setupMonacoEnvironment } from '@/editor/monacoEnvironment';
import { initMonacoTheme, VELOTERM_MONACO_THEME } from '@/editor/monacoTheme';
import { attachFindReplaceCommands, handleEditorKeyDownCapture } from '@/editor/findReplace';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

export interface WebhookScriptEditorProps {
  value: string;
  onChange: (val: string) => void;
  className?: string;
  height?: number | string;
  readOnly?: boolean;
}

const WEBHOOK_EXTRA_DTS = `
declare interface WebhookRequest<TBody = any> {
  readonly method: 'GET' | 'POST' | 'PUT' | 'DELETE' | 'PATCH' | string;
  readonly path: string;
  readonly clientIp: string;
  readonly pathParams: Readonly<Record<string, string>>;
  readonly queryParams: Readonly<Record<string, string>>;
  readonly headers: Readonly<Record<string, string>>;
  readonly bodyJson: Readonly<TBody> | null;
  readonly rawBody: string;
}

declare interface WebhookPrevOutput<TData = any> {
  readonly stdout: string;
  readonly stderr: string;
  readonly exitCode?: number | null;
  readonly success: boolean;
  readonly data?: Readonly<TData> | null;
  readonly output: string;
}

declare interface WebhookStepHistoryItem<TData = any> {
  readonly stepIndex: number;
  readonly stepId: string;
  readonly stepName: string;
  readonly actionType: 'local_command' | 'ssh_command' | 'js_script';
  readonly success: boolean;
  readonly exitCode?: number | null;
  readonly stdout: string;
  readonly stderr: string;
  readonly data?: Readonly<TData> | null;
  readonly durationMs: number;
}

declare interface WebhookGlobalContext<TBody = any, TData = any> {
  /** 触发本次 WebHook 的原始 HTTP 请求上下文 */
  readonly req: Readonly<WebhookRequest<TBody>>;
  /** 链式流水线中紧邻的前一个步骤执行输出（首个步骤执行时为 null） */
  readonly prev: Readonly<WebhookPrevOutput<TData> | null>;
  /** 链式流水线中先前已完成的所有步骤执行历史与返回数据 */
  readonly steps: ReadonlyArray<WebhookStepHistoryItem<TData>>;
}

/**
 * WebHook 全局上下文对象
 * 访问请求信息：webhook.req.bodyJson, webhook.req.queryParams, webhook.req.headers
 * 访问上一步输出：webhook.prev?.data, webhook.prev?.stdout, webhook.prev?.success
 * 访问历史步骤：webhook.steps[0].data
 */
declare const webhook: WebhookGlobalContext;

declare namespace console {
  function log(...args: any[]): void;
  function info(...args: any[]): void;
  function warn(...args: any[]): void;
  function error(...args: any[]): void;
}
`;

export const WebhookScriptEditor = memo(function WebhookScriptEditor({
  value,
  onChange,
  className,
  height = 240,
  readOnly = false,
}: WebhookScriptEditorProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const modelRef = useRef<monaco.editor.ITextModel | null>(null);
  const [copied, setCopied] = useState(false);

  const isTs =
    value.includes('interface ') ||
    value.includes('type ') ||
    value.includes(': string') ||
    value.includes(': number') ||
    value.includes(': boolean');
  const language = isTs ? 'typescript' : 'javascript';

  // 1. 注入 WebHook 上下文 d.ts 声明
  useEffect(() => {
    const ts = monaco.typescript;
    const jsPrev = ts.javascriptDefaults.getCompilerOptions();
    const tsPrev = ts.typescriptDefaults.getCompilerOptions();
    const tuned = {
      target: ts.ScriptTarget.ESNext,
      lib: ['esnext'],
      allowNonTsExtensions: true,
    };

    const d1 = ts.javascriptDefaults.addExtraLib(WEBHOOK_EXTRA_DTS, 'veloterm-webhook.d.ts');
    const d2 = ts.typescriptDefaults.addExtraLib(WEBHOOK_EXTRA_DTS, 'veloterm-webhook.d.ts');
    ts.javascriptDefaults.setCompilerOptions({ ...jsPrev, ...tuned });
    ts.typescriptDefaults.setCompilerOptions({ ...tsPrev, ...tuned });

    return () => {
      d1.dispose();
      d2.dispose();
      ts.javascriptDefaults.setCompilerOptions(jsPrev);
      ts.typescriptDefaults.setCompilerOptions(tsPrev);
    };
  }, []);

  // 2. 初始化 Monaco 编辑器
  useEffect(() => {
    if (!containerRef.current) return;
    setupMonacoEnvironment();
    initMonacoTheme();

    const uri = monaco.Uri.parse(`inmemory://webhook-editor-${crypto.randomUUID()}.${isTs ? 'ts' : 'js'}`);
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
      suggestSelection: 'first',
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

  // 当外部 value 改变且与当前 model 不一致时同步（如切换步骤）
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

  const handleInsertTemplate = (snippet: string) => {
    if (!editorRef.current) return;
    const editor = editorRef.current;
    const selection = editor.getSelection();
    if (!selection) return;
    editor.executeEdits('insert-snippet', [
      {
        range: selection,
        text: snippet,
        forceMoveMarkers: true,
      },
    ]);
    editor.focus();
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
          <Code2 className="size-3 text-terminal-yellow" />
          <span className="font-mono font-medium text-foreground">
            {language === 'typescript' ? 'TypeScript' : 'JavaScript'} (V8)
          </span>
          <span className="text-border">|</span>
          {/* 快捷插值提示按钮 */}
          <button
            type="button"
            onClick={() => handleInsertTemplate('const { method, path, bodyJson } = webhook.req;\n')}
            className="flex items-center gap-1 text-[10px] text-foreground-subtlest hover:text-foreground transition-colors"
            title={t('webhook.insertReqSnippet', { defaultValue: '插入请求解构' })}
          >
            <Sparkles className="size-2.5 text-terminal-blue" />
            <span>+ webhook.req</span>
          </button>
          <button
            type="button"
            onClick={() => handleInsertTemplate('const prevData = webhook.prev?.data;\n')}
            className="flex items-center gap-1 text-[10px] text-foreground-subtlest hover:text-foreground transition-colors"
            title={t('webhook.insertPrevSnippet', { defaultValue: '插入前一步数据' })}
          >
            <Sparkles className="size-2.5 text-terminal-green" />
            <span>+ webhook.prev</span>
          </button>
          <button
            type="button"
            onClick={() => handleInsertTemplate('return { success: true, timestamp: Date.now() };\n')}
            className="flex items-center gap-1 text-[10px] text-foreground-subtlest hover:text-foreground transition-colors"
            title={t('webhook.insertReturnSnippet', { defaultValue: '返回供后续步骤消费的对象' })}
          >
            <Sparkles className="size-2.5 text-terminal-yellow" />
            <span>+ return data</span>
          </button>
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
