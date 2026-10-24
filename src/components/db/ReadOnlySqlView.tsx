/**
 * 只读 SQL 代码查看器（对象详情的 DDL / 定义渲染用）。
 *
 * 独立实例形态：每个视图一个 editor + model（挂载建、卸载 dispose），
 * 直接在自身容器上创建（不经游离 div，初始化测量有效）。曾试验共享
 * 单实例 + setModel 切换（省内存），但 DOM 搬移 + ResizeObserver +
 * StrictMode 双执行的时序组合两轮未修平（DDL 白屏），回退到本简单
 * 形态——keep-alive 切页不卸载故各视图常驻，实例数由对象页上限
 * （OBJECT_PAGE_LIMIT）兜底。
 *
 * model 不指定 Uri（Monaco 自动唯一）：固定 Uri 在多会话同时打开同一
 * 对象时会撞「model already exists」直接崩树；只读高亮走语言参数，
 * 不依赖 Uri 索引。
 */

import * as monaco from 'monaco-editor';
import { useCallback, useEffect, useImperativeHandle, useRef } from 'react';

import { setupMonacoEnvironment } from '@/editor/monacoEnvironment';
import { initMonacoTheme, VELOTERM_MONACO_THEME } from '@/editor/monacoTheme';
import { ensureExtendedLanguages } from '@/editor/languages';
import { formatSqlText } from '@/components/db/sqlFormat';

let languagesRegistered = false;

/** ReadOnlySqlView 对外暴露的命令句柄（工具栏「格式化」按钮等调用） */
export interface ReadOnlySqlViewHandle {
  /** 格式化当前内容（只读展示层变换：model.setValue 程序化写入，
   * 用户仍不可手改；刷新/重开会还原服务端原文） */
  format: () => Promise<void>;
}

interface ReadOnlySqlViewProps {
  content: string;
  className?: string;
  /** 格式化方言（引擎名，双路择优见 sqlFormat.ts；不传回落默认 plsql */
  formatEngine?: string | null;
  /** 命令句柄（React 19 ref 即 prop） */
  ref?: React.Ref<ReadOnlySqlViewHandle>;
}

export function ReadOnlySqlView({ content, className, formatEngine = null, ref }: ReadOnlySqlViewProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const modelRef = useRef<monaco.editor.ITextModel | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);

  // 实例生命周期：挂载时在自身容器上创建（editor + model 各一），卸载释放
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    // 与 SqlEditor 同序：先 worker 环境，再主题/扩展语言（幂等）
    setupMonacoEnvironment();
    if (!languagesRegistered) {
      initMonacoTheme();
      ensureExtendedLanguages(monaco);
      languagesRegistered = true;
    }
    const model = monaco.editor.createModel(content, 'sql');
    const editor = monaco.editor.create(container, {
      model,
      theme: VELOTERM_MONACO_THEME,
      automaticLayout: true,
      fontFamily: 'var(--font-mono)',
      fontSize: 13,
      lineHeight: 20,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      // 对齐旧 <pre> 的 whitespace-pre-wrap：长 DDL 行折行不横向滚动
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
      readOnly: true,
      // 只读场景禁掉一切输入态交互：无补全、无右键、选中文本仍可复制
      quickSuggestions: false,
      contextmenu: false,
      // Dom 字符计数在超大 DDL 上有开销；静态内容没有增量编辑，关掉
      domReadOnly: true,
    });
    modelRef.current = model;
    editorRef.current = editor;
    return () => {
      editor.dispose();
      model.dispose();
      modelRef.current = null;
      editorRef.current = null;
    };
    // 仅挂载时建：content 初值捕获于创建，后续变化走 setValue 不重建
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 内容更新（对象页刷新/重复打开）：setValue 重 token 化，无历史残留
  useEffect(() => {
    const model = modelRef.current;
    if (model && !model.isDisposed() && model.getValue() !== content) {
      model.setValue(content);
    }
  }, [content]);

  // 格式化（展示层变换）：model.setValue 是程序化写入，不受编辑器
  // readOnly/domReadOnly 门控——用户依然不能手改，仅按钮可触发
  const formatContent = useCallback(async () => {
    const model = modelRef.current;
    if (!model || model.isDisposed()) return;
    const source = model.getValue();
    if (!source.trim()) return;
    const formatted = await formatSqlText(source, formatEngineRef.current ?? undefined);
    if (formatted !== source && !model.isDisposed()) {
      model.setValue(formatted);
    }
  }, []);
  const formatRef = useRef(formatContent);
  formatRef.current = formatContent;
  const formatEngineRef = useRef(formatEngine);
  formatEngineRef.current = formatEngine;
  useImperativeHandle(ref, () => ({ format: () => formatRef.current() }), []);

  return <div ref={containerRef} className={className} />;
}
