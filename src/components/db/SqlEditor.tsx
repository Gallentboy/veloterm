/**
 * 轻量 Monaco SQL 编辑器包装：数据库工作台的查询输入框专用。
 * 不用 MonacoDocumentCard（与文件仓/草稿生命周期强耦合）；这里只做一个
 * 自持 model 的只读-可编辑小编辑器，languageId=sql（静态补全由
 * ensureSqlLanguage 在宿主侧注册）。⌘/Ctrl+Enter 执行。
 * redisMode：模型语言换 plaintext（Redis 是逐行命令不是 SQL），并挂静态
 * 命令词表补全（redisCommands.ts，数据离线无网络依赖）。
 */

import * as monaco from 'monaco-editor';
import { useEffect, useRef } from 'react';

import { setupMonacoEnvironment } from '@/editor/monacoEnvironment';
import { initMonacoTheme, VELOTERM_MONACO_THEME } from '@/editor/monacoTheme';
import { ensureExtendedLanguages } from '@/editor/languages';
import { REDIS_COMMAND_SUGGESTIONS } from '@/components/db/redisCommands';
import { cn } from '@/lib/utils';

// 模块级幂等注册（伞入口内部各 ensure 亦幂等）
let languagesRegistered = false;

export interface SqlEditorProps {
  value: string;
  onChange: (value: string) => void;
  onRun: () => void;
  className?: string;
  /** Redis 会话：plaintext 模型 + 静态命令词表补全 */
  redisMode?: boolean;
}

export function SqlEditor({ value, onChange, onRun, className, redisMode = false }: SqlEditorProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const modelRef = useRef<monaco.editor.ITextModel | null>(null);
  const runRef = useRef(onRun);
  runRef.current = onRun;

  useEffect(() => {
    if (!containerRef.current) return;
    // 必须先于任何 worker 触点（词补全/link 计算都会拉 editor worker）：
    // 缺了 MonacoEnvironment.getWorker 时 monaco 走 esmModuleLocationBundler
    // 兜底裸 URL，dev 下被 SPA fallback 回 index.html → MIME 报错
    setupMonacoEnvironment();
    if (!languagesRegistered) {
      initMonacoTheme();
      ensureExtendedLanguages(monaco);
      languagesRegistered = true;
    }

    const model = monaco.editor.createModel(
      value,
      redisMode ? 'plaintext' : 'sql',
      monaco.Uri.parse(`inmemory://db-query-${crypto.randomUUID()}`),
    );
    modelRef.current = model;
    model.onDidChangeContent(() => {
      onChange(model.getValue());
    });

    // Redis 命令词表补全：注册在 plaintext 语言上，按 model 归属过滤到本编辑器
    //（plaintext 是 Monaco 内置语言，其它 plaintext 模型不受影响）；dispose 随组件卸载
    let redisProvider: monaco.IDisposable | null = null;
    if (redisMode) {
      redisProvider = monaco.languages.registerCompletionItemProvider('plaintext', {
        triggerCharacters: [],
        provideCompletionItems: (doc, position) => {
          if (doc !== model) return { suggestions: [] };
          const word = doc.getWordUntilPosition(position);
          const range = {
            startLineNumber: position.lineNumber,
            endLineNumber: position.lineNumber,
            startColumn: word.startColumn,
            endColumn: word.endColumn,
          };
          return {
            suggestions: REDIS_COMMAND_SUGGESTIONS.map((cmd, i) => ({
              label: cmd.label,
              kind: monaco.languages.CompletionItemKind.Function,
              detail: cmd.detail,
              insertText: cmd.insertText,
              range,
              // sortText：命令名前缀排在参数骨架之前，稳定顺序
              sortText: String(i).padStart(4, '0'),
            })),
          };
        },
      });
    }

    const editor = monaco.editor.create(containerRef.current, {
      model,
      theme: VELOTERM_MONACO_THEME,
      automaticLayout: true,
      fontSize: 13,
      lineHeight: 20,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      wordWrap: 'on',
      lineNumbers: 'off',
      glyphMargin: false,
      folding: false,
      renderLineHighlight: 'none',
      overviewRulerLanes: 0,
      hideCursorInOverviewRuler: true,
      overviewRulerBorder: false,
      scrollbar: { verticalScrollbarSize: 8, horizontalScrollbarSize: 8, useShadows: false },
      padding: { top: 8, bottom: 8 },
      quickSuggestions: { other: true, comments: false, strings: false },
      suggestSelection: 'first',
      tabSize: 2,
    });
    editorRef.current = editor;

    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => {
      runRef.current();
    });

    return () => {
      redisProvider?.dispose();
      editor.dispose();
      model.dispose();
      editorRef.current = null;
      modelRef.current = null;
    };
    // 编辑器实例只建一次；value 变更经下方 effect 同步（redisMode 亦不热切：
    // 引擎由会话决定，会话生命周期内不变，切会话 = 换组件实例）
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 外部 value 变化（如树右键插入语句）同步进 model；光标 editStack 保留 undo 链
  useEffect(() => {
    const model = modelRef.current;
    if (model && value !== model.getValue()) {
      model.pushEditOperations([], [{ range: model.getFullModelRange(), text: value }], () => null);
    }
  }, [value]);

  return <div ref={containerRef} className={cn('h-full w-full overflow-hidden', className)} />;
}
