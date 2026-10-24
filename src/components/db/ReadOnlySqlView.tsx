/**
 * 只读 SQL 代码查看器（对象详情的 DDL / 定义渲染用）。
 *
 * **共享实例 + setModel 切换模式**（Monaco 推荐形态）：模块级单例 editor，
 * 多个视图（keep-alive 的对象页 × DDL/定义子视图）共用一个实例——激活者
 * 把 editor 的 DOM 节点搬进自己容器并 setModel；切页/切子视图只换轻量
 * model（内容 + 行缓冲），实例不增、库与 worker 本就全局一份。此前每页
 * 一个 editor 实例（DOM 视图 + ViewModel 各一套），页数多时线性吃内存。
 *
 * editor 惰性到首次附着时在真实容器上创建（不挂游离 div 初始化——游离
 * 容器无盒模型，初始化测量 0×0，搬进文档后 automaticLayout 的
 * ResizeObserver 对「游离→挂载」转换不可靠，会停在 0×0 白屏）。
 *
 * 模型生命周期：挂载 createModel（不指定 Uri——Monaco 自动生成唯一 Uri，
 * 固定 Uri 在多会话同开同一对象/重开时序下会撞「model already exists」
 * 直接崩树；只读高亮走语言参数不依赖 Uri）、content 变化 setValue（只读
 * 无 undo 顾虑）、卸载 dispose——keep-alive 切页不卸载故模型保留（切回
 * 零重取零 token 化），子视图切换/页关闭会卸载即释放。
 */

import * as monaco from 'monaco-editor';
import { useEffect, useRef } from 'react';

import { setupMonacoEnvironment } from '@/editor/monacoEnvironment';
import { initMonacoTheme, VELOTERM_MONACO_THEME } from '@/editor/monacoTheme';
import { ensureExtendedLanguages } from '@/editor/languages';

let languagesRegistered = false;

let sharedEditor: monaco.editor.IStandaloneCodeEditor | null = null;
/** editor 无主时的空模型（dispose 仍被 editor 引用的 model 会抛异常） */
let emptyModel: monaco.editor.ITextModel | null = null;

function getEmptyModel(): monaco.editor.ITextModel {
  if (!emptyModel || emptyModel.isDisposed()) {
    emptyModel = monaco.editor.createModel('', 'sql');
  }
  return emptyModel;
}

function getSharedEditor(host: HTMLElement): monaco.editor.IStandaloneCodeEditor {
  if (!sharedEditor) {
    // 与 SqlEditor 同序：先 worker 环境，再主题/扩展语言（幂等）
    setupMonacoEnvironment();
    if (!languagesRegistered) {
      initMonacoTheme();
      ensureExtendedLanguages(monaco);
      languagesRegistered = true;
    }
    // 直接在真实容器上创建（host 在文档流内，初始化测量有效）；后续切页
    // 只搬 DOM 节点 + setModel，不重建
    sharedEditor = monaco.editor.create(host, {
      model: getEmptyModel(),
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
  }
  return sharedEditor;
}

interface ReadOnlySqlViewProps {
  content: string;
  className?: string;
  /** 是否激活（激活者独占共享 editor：搬 DOM + setModel）。keep-alive 的
   * 非激活视图持有 model 但不附着。默认 true（单视图场景） */
  active?: boolean;
}

export function ReadOnlySqlView({ content, className, active = true }: ReadOnlySqlViewProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const modelRef = useRef<monaco.editor.ITextModel | null>(null);

  // 模型生命周期：挂载建一次（无 Uri，Monaco 自动唯一——固定 Uri 多会话
  // 同开同一对象会撞「model already exists」崩树）、卸载释放
  useEffect(() => {
    const model = monaco.editor.createModel(content, 'sql');
    modelRef.current = model;
    return () => {
      // editor 还挂着这个 model 时先摘下（dispose 被引用的 model 会抛）；
      // editor 尚未创建（从未激活过）则直接释放
      if (sharedEditor && !model.isDisposed() && sharedEditor.getModel() === model) {
        sharedEditor.setModel(getEmptyModel());
      }
      model.dispose();
      modelRef.current = null;
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

  // editor 附着：active 者独占——DOM 搬进自己容器 + setModel + layout。
  // 非 active 不抢（上一个激活者可能在 hidden 容器里仍持有 DOM，无碍，
  // 下一个激活者会搬走）
  useEffect(() => {
    if (!active) return;
    const container = containerRef.current;
    const model = modelRef.current;
    if (!container || !model || model.isDisposed()) return;
    // 首次调用时 editor 直接在本容器上创建（消除游离初始化的 0×0 测量）
    const editor = getSharedEditor(container);
    if (editor.getModel() !== model) {
      editor.setModel(model);
    }
    // getDomNode 类型含 null（实际已创建必有），收窄后使用
    const domNode = editor.getDomNode();
    if (domNode && domNode.parentElement !== container) {
      container.appendChild(domNode);
    }
    // 同步 layout + rAF 二次 layout：确保「hidden→可见」「容器首挂」两种
    // 时序下尺寸测量都生效（ResizeObserver 对游离→挂载转换不可靠）
    editor.layout();
    const raf = requestAnimationFrame(() => editor.layout());
    return () => cancelAnimationFrame(raf);
  }, [active]);

  return <div ref={containerRef} className={className} />;
}
