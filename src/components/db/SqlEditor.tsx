/**
 * 轻量 Monaco SQL 编辑器包装：数据库工作台的查询输入框专用。
 * 不用 MonacoDocumentCard（与文件仓/草稿生命周期强耦合）；这里只做一个
 * 自持 model 的只读-可编辑小编辑器，languageId=sql（静态补全由
 * ensureSqlLanguage 在宿主侧注册；表/列/别名的 schema 感知补全由
 * schemaCompleter prop 注入数据源，与静态关键字补全共存）。
 * ⌘/Ctrl+Enter 执行全部；
 * ⌘/Ctrl+Shift+Enter 执行选中（选区经 onSelectionChange 上报，
 * 空选中上报空串，调用方据此禁用入口）。
 * 右键菜单为项目自绘弹层（portal + fixed，与对象树右键菜单同型）：
 * 复制/粘贴/执行选中/全部执行/查看执行计划（redisMode 隐藏执行计划）。
 * redisMode：模型语言换 plaintext（Redis 是逐行命令不是 SQL），并挂静态
 * 命令词表补全（redisCommands.ts，数据离线无网络依赖）。
 */

import * as monaco from 'monaco-editor';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { createPortal } from 'react-dom';

import { setupMonacoEnvironment } from '@/editor/monacoEnvironment';
import { initMonacoTheme, VELOTERM_MONACO_THEME } from '@/editor/monacoTheme';
import { ensureExtendedLanguages } from '@/editor/languages';
import { REDIS_COMMAND_SUGGESTIONS } from '@/components/db/redisCommands';
import { dbSqlCompletions } from '@/services/dbConnectionsService';
import { readClipboardText, writeClipboardText } from '@/lib/clipboard';
import { useDismissOnOutsidePointer } from '@/hooks/useDismissOnOutsidePointer';
import { cn } from '@/lib/utils';

// 模块级幂等注册（伞入口内部各 ensure 亦幂等）
let languagesRegistered = false;

export interface SqlEditorProps {
  value: string;
  onChange: (value: string) => void;
  onRun: () => void;
  /** 执行选中文本（⌘/Ctrl+Shift+Enter）；空选中时调用方自行保证不触发 */
  onRunSelection?: () => void;
  /** 选中文本变化（空串 = 无有效选中），驱动调用方「执行选中」按钮态 */
  onSelectionChange?: (selected: string) => void;
  /** 查看执行计划（入参为选中文本；引擎方言由调用方拼，本组件不感知） */
  onExplain?: (selected: string) => void;
  className?: string;
  /** Redis 会话：plaintext 模型 + 静态命令词表补全，右键菜单隐藏执行计划 */
  redisMode?: boolean;
  /** schema 感知补全（表/列/别名）的会话 id；后端 db_sql_completions
   * 解析 cursor 上下文与目录（带缓存），null/不传 = 不启用（Redis 不传） */
  completionsSessionId?: string | null;
}

export function SqlEditor({
  value,
  onChange,
  onRun,
  onRunSelection,
  onSelectionChange,
  onExplain,
  className,
  redisMode = false,
  completionsSessionId = null,
}: SqlEditorProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const modelRef = useRef<monaco.editor.ITextModel | null>(null);
  const runRef = useRef(onRun);
  runRef.current = onRun;
  const runSelRef = useRef(onRunSelection);
  runSelRef.current = onRunSelection;
  const selChangeRef = useRef(onSelectionChange);
  selChangeRef.current = onSelectionChange;
  const explainRef = useRef(onExplain);
  explainRef.current = onExplain;
  const completionsSidRef = useRef(completionsSessionId);
  completionsSidRef.current = completionsSessionId;

  // 右键菜单（portal 到 body + fixed 定位，项目弹层规范；与对象树右键菜单
  // 同型样式）。selected 为开菜单瞬间的选区快照
  const [menu, setMenu] = useState<{ x: number; y: number; selected: string } | null>(
    null,
  );
  const menuRef = useRef<HTMLDivElement | null>(null);
  useDismissOnOutsidePointer(menu !== null, menuRef, () => setMenu(null));

  const openContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    const sel = editorRef.current?.getSelection();
    const model = modelRef.current;
    const selected = sel && !sel.isEmpty() && model ? model.getValueInRange(sel) : '';
    setMenu({ x: e.clientX, y: e.clientY, selected });
  };

  const pasteFromClipboard = () => {
    const editor = editorRef.current;
    const model = modelRef.current;
    if (!editor || !model) return;
    void readClipboardText()
      .then((text) => {
        // 覆盖当前选区（零宽选区即光标处插入）；executeEdits 走 undo 栈
        const sel = editor.getSelection();
        editor.executeEdits('ctx-paste', [
          { range: sel ?? model.getFullModelRange(), text, forceMoveMarkers: false },
        ]);
        editor.focus();
      })
      .catch(() => undefined);
  };

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

    // schema 感知补全（表/列/别名）：cursor 上下文解析与目录解析全在
    // Rust 后端（db_sql_completions，会话级目录缓存），这里只做薄映射。
    // 注册在 sql 语言上按 model 归属过滤（同 Redis 词表补全模式；与静态
    // 关键字补全共存，Monaco 自动合并多 provider 建议）；`.` 触发限定符
    // 二段补全。不需要防抖：Monaco 建议框自带触发节流，后端缓存命中后
    // 每次只是一趟轻量 IPC
    let schemaProvider: monaco.IDisposable | null = null;
    if (!redisMode) {
      const draftKind = {
        column: monaco.languages.CompletionItemKind.Field,
        table: monaco.languages.CompletionItemKind.Class,
        schema: monaco.languages.CompletionItemKind.Module,
      } as const;
      schemaProvider = monaco.languages.registerCompletionItemProvider('sql', {
        triggerCharacters: ['.'],
        provideCompletionItems: async (doc, position) => {
          const sid = completionsSidRef.current;
          if (!sid || doc !== model) return { suggestions: [] };
          const offset = model.getOffsetAt(position);
          const result = await dbSqlCompletions(sid, model.getValue(), offset).catch(
            () => null,
          );
          if (!result || result.suggestions.length === 0) return { suggestions: [] };
          // 替换范围：后端给的 wordStart（UTF-16 偏移）到光标
          const startPos = model.getPositionAt(result.wordStart);
          const range = {
            startLineNumber: startPos.lineNumber,
            startColumn: startPos.column,
            endLineNumber: position.lineNumber,
            endColumn: position.column,
          };
          return {
            suggestions: result.suggestions.map((d) => ({
              label: d.label,
              kind: draftKind[d.kind],
              detail: d.detail,
              insertText: d.insertText,
              range,
              sortText: d.sort,
              // schema 建议预选：建议列表初始高亮项直接落在本组
              //（sortText 已压过静态关键字，preselect 再兑一层——多 provider
              // 合并时 Monaco 对 preselect 项优先作为初始选中）
              preselect: true,
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
      // 关掉 Monaco 自带右键菜单，换成项目自绘弹层（Go to Definition 等
      // 对工作台无意义，只要复制/粘贴/执行/执行计划）
      contextmenu: false,
    });
    editorRef.current = editor;

    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => {
      runRef.current();
    });
    editor.addCommand(
      monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.Enter,
      () => {
        runSelRef.current?.();
      },
    );

    // 选区跟踪：空选区/纯空白上报空串（调用方据此禁用「执行选中」）
    editor.onDidChangeCursorSelection(() => {
      const sel = editor.getSelection();
      if (!sel) return;
      selChangeRef.current?.(sel.isEmpty() ? '' : model.getValueInRange(sel));
    });

    return () => {
      schemaProvider?.dispose();
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

  return (
    <div
      ref={containerRef}
      onContextMenu={openContextMenu}
      className={cn('h-full w-full overflow-hidden', className)}
    >
      {menu
        ? createPortal(
            <div
              ref={menuRef}
              role="menu"
              style={{ position: 'fixed', top: menu.y, left: menu.x }}
              className="z-[120] overflow-hidden rounded-lg border border-border bg-menu p-1 shadow-lg select-none"
            >
              {(() => {
                const hasSel = menu.selected.trim().length > 0;
                const items: {
                  key: string;
                  label: string;
                  disabled?: boolean;
                  action: () => void;
                }[] = [
                  {
                    key: 'copy',
                    label: t('db.ctxCopy'),
                    disabled: !hasSel,
                    action: () => {
                      void writeClipboardText(menu.selected).catch(() => undefined);
                    },
                  },
                  {
                    key: 'paste',
                    label: t('db.ctxPaste'),
                    action: pasteFromClipboard,
                  },
                  {
                    key: 'runSel',
                    label: t('db.runSelected'),
                    disabled: !hasSel,
                    action: () => runSelRef.current?.(),
                  },
                  {
                    key: 'runAll',
                    label: t('db.runAll'),
                    disabled: !value.trim(),
                    action: () => runRef.current(),
                  },
                ];
                if (!redisMode) {
                  items.push({
                    key: 'explain',
                    label: t('db.ctxExplain'),
                    disabled: !hasSel,
                    action: () => explainRef.current?.(menu.selected),
                  });
                }
                return items.map((item) => (
                  <button
                    key={item.key}
                    type="button"
                    role="menuitem"
                    disabled={item.disabled}
                    onClick={() => {
                      setMenu(null);
                      item.action();
                    }}
                    className="flex w-full items-center whitespace-nowrap rounded-md px-2.5 py-1.5 text-left text-ui-sm text-foreground transition-colors hover:bg-menu-hover disabled:pointer-events-none disabled:opacity-40"
                  >
                    {item.label}
                  </button>
                ));
              })()}
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}
