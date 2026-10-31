import { useCallback, useEffect, useRef } from 'react';
import * as monaco from 'monaco-editor';
import { initMonacoTheme, VELOTERM_MONACO_THEME } from '@/editor/monacoTheme';
import { setupMonacoEnvironment } from '@/editor/monacoEnvironment';
import { ensureExtendedLanguages } from '@/editor/languages';
import { ensureBashCompletion, registerBashHostModel } from '@/editor/bashLanguage';
import { attachHtmlAutoInsert } from '@/editor/htmlAutoInsert';
import { attachFindReplaceCommands, handleEditorKeyDownCapture } from '@/editor/findReplace';
import type { EditorFileDescriptor, MonacoDocumentCardProps } from '@/editor/types';
import { Spinner } from '@/components/ui/spinner';

/**
 * Monaco 文档卡片（ZCode 文档范式：外层 bg-background p-2，内层 rounded-xl 边框卡片，编辑器融入卡片）。
 *
 * 关键结构决策：
 * - 单实例：一个 monaco.editor.create 挂容器 ref；模型模块级缓存（fileId → ITextModel），
 *   切文件 setModel + saveViewState/restoreViewState（viewState 亦模块级缓存）；
 *   组件卸载 dispose 编辑器但保留模型缓存（应用生命周期存活）
 * - Diff 模式：diffRemoteContent 非 null 时在同一容器换装 createDiffEditor（懒创建复用），
 *   original = 远端内容临时 model、modified = 当前文件 model，退出 diff 重建普通编辑器
 * - worker：MonacoEnvironment.getWorker 走 new Worker(new URL(...), { type: 'module' })，
 *   Rsbuild 将其识别为独立 worker chunk
 */

/* —— 模块级模型 / viewState 缓存（应用生命周期存活） —— */

const modelCache = new Map<number, monaco.editor.ITextModel>();
const viewStateCache = new Map<number, monaco.editor.ICodeEditorViewState>();

function getFileModel(file: EditorFileDescriptor, content: string): monaco.editor.ITextModel {
  let model = modelCache.get(file.id);
  if (!model) {
    const uri = monaco.Uri.parse(
      `veloterm://editor/${file.id}/${encodeURIComponent(file.fileName)}`,
    );
    model = monaco.editor.createModel(content, file.languageId, uri);
    modelCache.set(file.id, model);
    // 关联远端主机：bash 补全的远端命令全集按 host 取（幂等）
    registerBashHostModel(model, file.hostId);
  } else if (model.getLanguageId() !== file.languageId) {
    monaco.editor.setModelLanguage(model, file.languageId);
  }
  return model;
}

const MONACO_EDITOR_OPTIONS = {
  theme: VELOTERM_MONACO_THEME,
  automaticLayout: true,
  fontFamily: 'var(--font-mono)',
  fontLigatures: true,
  fontSize: 13,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  renderWhitespace: 'selection',
  smoothScrolling: true,
  fixedOverflowWidgets: true,
  padding: { top: 8, bottom: 8 },
  // ZCode 风格细胶囊滚动条：8px 槽位、无阴影无箭头、悬停时出现
  scrollbar: {
    vertical: 'auto',
    horizontal: 'auto',
    verticalScrollbarSize: 8,
    horizontalScrollbarSize: 8,
    useShadows: false,
    verticalHasArrows: false,
    horizontalHasArrows: false,
  },
} as const;

export function MonacoDocumentCard(props: MonacoDocumentCardProps) {
  const { file, content, diffRemoteContent, readOnly, viewState } = props;
  const isDiff = diffRemoteContent !== null;

  const containerRef = useRef<HTMLDivElement | null>(null);
  const codeEditorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const diffEditorRef = useRef<monaco.editor.IStandaloneDiffEditor | null>(null);
  const diffOriginalModelRef = useRef<monaco.editor.ITextModel | null>(null);

  const currentFileIdRef = useRef<number | null>(null);
  /** 最近一次与父层同步过的内容（fileId + content），用于区分「父层重置内容」与「本地编辑领先于父层」 */
  const lastSyncedContentRef = useRef<{ fileId: number; content: string } | null>(null);
  /** 外部 setValue 的内容（该次 model 变更不回报 onContentChange） */
  const externalValueRef = useRef<string | null>(null);

  /** props 回调经 ref 取最新引用，避免编辑器生命周期内闭包过期 / 每次渲染重建监听 */
  const latestPropsRef = useRef(props);
  useEffect(() => {
    latestPropsRef.current = props;
  });

  /** 保存并上报当前文件的 viewState（失焦 / 切换 / 卸载时调用） */
  const flushViewState = useCallback(() => {
    const editor = codeEditorRef.current;
    const fileId = currentFileIdRef.current;
    if (!editor || fileId == null || !editor.getModel()) return;
    const state = editor.saveViewState();
    if (!state) return;
    viewStateCache.set(fileId, state);
    latestPropsRef.current.onViewStateChange(fileId, state);
  }, []);

  /** 父层下发的内容与 model 不一致且确非本地编辑回报时，重置 model 内容（加载完成/保存回执/回滚） */
  const syncModelContent = useCallback(
    (model: monaco.editor.ITextModel, targetFile: EditorFileDescriptor, targetContent: string) => {
      if (model.getValue() !== targetContent) {
        const last = lastSyncedContentRef.current;
        if (!last || last.fileId !== targetFile.id || last.content !== targetContent) {
          externalValueRef.current = targetContent;
          model.setValue(targetContent);
        }
      }
      lastSyncedContentRef.current = { fileId: targetFile.id, content: targetContent };
    },
    [],
  );

  /** 格式化当前文档（SQL/JSON 增强兜底 + Monaco 内置 DocumentFormattingEditProvider） */
  const formatDocument = useCallback(async () => {
    const editor = codeEditorRef.current;
    if (!editor || readOnly || isDiff) return;
    const model = editor.getModel();
    if (!model || model.isDisposed()) return;
    const original = model.getValue();
    const lang = model.getLanguageId();

    // 1. 对于 SQL 文件，优先使用强大的 formatSqlText
    if (lang === 'sql' || file.fileName.toLowerCase().endsWith('.sql')) {
      try {
        const { formatSqlText } = await import('@/components/db/sqlFormat');
        const formatted = await formatSqlText(original);
        if (formatted && formatted !== original && !model.isDisposed()) {
          editor.executeEdits('format-sql', [
            { range: model.getFullModelRange(), text: formatted, forceMoveMarkers: false },
          ]);
          latestPropsRef.current.onContentChange(formatted);
        }
        return;
      } catch (e) {
        console.warn('SQL format failed, fallback to Monaco action', e);
      }
    }

    // 2. 对于 JSON 文件，若 Monaco 未能格式化或语法健全，尝试 JSON 格式化兜底
    if (lang === 'json' || file.fileName.toLowerCase().endsWith('.json')) {
      try {
        const parsed = JSON.parse(original);
        const formatted = JSON.stringify(parsed, null, 2);
        if (formatted && formatted !== original && !model.isDisposed()) {
          editor.executeEdits('format-json', [
            { range: model.getFullModelRange(), text: formatted, forceMoveMarkers: false },
          ]);
          latestPropsRef.current.onContentChange(formatted);
          return;
        }
      } catch {
        // 语法错误跳过兜底，由 Monaco action 尝试
      }
    }

    // 3. 通用语言：触发 Monaco 自带的 formatDocument Action
    const action = editor.getAction('editor.action.formatDocument');
    if (action && action.isSupported()) {
      await action.run();
      const updated = model.getValue();
      if (updated !== original) {
        latestPropsRef.current.onContentChange(updated);
      }
    }
  }, [file.fileName, isDiff, readOnly]);

  useEffect(() => {
    if (latestPropsRef.current.onRegisterFormat) {
      latestPropsRef.current.onRegisterFormat(formatDocument);
    }
    return () => {
      latestPropsRef.current.onRegisterFormat?.(null);
    };
  }, [formatDocument]);

  /* —— 编辑器生命周期（普通 / diff 换装；容器与 mode 变化时重建） —— */

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    initMonacoTheme();
    setupMonacoEnvironment();
    // 自建语言（nginx / dockerfile / compose）幂等注册；
    // 数据懒加载，见 src/editor/languages.ts
    ensureExtendedLanguages(monaco);
    // bash 补全（静态词表 + Rust tree-sitter 符号 + 远端命令）幂等注册，
    // 见 bashLanguage.ts
    ensureBashCompletion(monaco);

    if (isDiff) {
      const diffEditor = monaco.editor.createDiffEditor(container, {
        ...MONACO_EDITOR_OPTIONS,
        renderSideBySide: true,
        readOnly: true,
      });
      diffEditorRef.current = diffEditor;
      return () => {
        diffEditorRef.current = null;
        diffOriginalModelRef.current?.dispose();
        diffOriginalModelRef.current = null;
        diffEditor.dispose();
      };
    }

    const editor = monaco.editor.create(container, {
      ...MONACO_EDITOR_OPTIONS,
      model: null,
    });

    // addCommand 绑定在编辑器实例上，setModel 切换后依然有效
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
      latestPropsRef.current.onSaveRequest();
    });
    // Alt+Shift+F 格式化文档（VS Code 习惯）
    editor.addCommand(monaco.KeyMod.Alt | monaco.KeyMod.Shift | monaco.KeyCode.KeyF, () => {
      void formatDocument();
    });
    // 跨平台查找 (Ctrl+F / ⌘F) 与替换 (Ctrl+R / ⌘R / Ctrl+H)
    attachFindReplaceCommands(editor);
    // HTML 键入辅助（`>` 闭合标签 / `</` 补名 / `=` 补引号），仅 html 模型生效
    attachHtmlAutoInsert(editor);
    editor.onDidChangeModelContent(() => {
      const model = editor.getModel();
      if (!model) return;
      const value = model.getValue();
      if (externalValueRef.current === value) {
        externalValueRef.current = null;
        return;
      }
      latestPropsRef.current.onContentChange(value);
    });
    editor.onDidChangeCursorPosition((e) => {
      latestPropsRef.current.onCursorChange(e.position.lineNumber, e.position.column);
    });
    editor.onDidBlurEditorText(() => {
      flushViewState();
    });

    codeEditorRef.current = editor;
    return () => {
      flushViewState();
      codeEditorRef.current = null;
      editor.dispose();
    };
  }, [isDiff, flushViewState]);

  /* —— 普通模式：文件切换 / 内容同步 / viewState 恢复 —— */

  useEffect(() => {
    if (isDiff) return;
    const editor = codeEditorRef.current;
    if (!editor) return;

    const prevFileId = currentFileIdRef.current;
    if (prevFileId != null && prevFileId !== file.id) {
      // 切出前保存并上报旧文件的 viewState
      if (editor.getModel()) {
        const prevState = editor.saveViewState();
        if (prevState) {
          viewStateCache.set(prevFileId, prevState);
          latestPropsRef.current.onViewStateChange(prevFileId, prevState);
        }
      }
    }
    currentFileIdRef.current = file.id;

    if (content === null) {
      // 内容未加载：不初始化模型（loading 遮罩由渲染层展示）
      editor.setModel(null);
      lastSyncedContentRef.current = null;
      return;
    }

    const model = getFileModel(file, content);
    syncModelContent(model, file, content);

    if (editor.getModel() !== model) {
      editor.setModel(model);
      const cachedState = viewStateCache.get(file.id);
      const state =
        cachedState ?? (viewState as monaco.editor.ICodeEditorViewState | null) ?? null;
      if (state) {
        editor.restoreViewState(state);
      }
    }
  }, [isDiff, file, content, viewState, syncModelContent]);

  /* —— Diff 模式：original = 远端临时 model，modified = 当前文件 model —— */

  useEffect(() => {
    if (!isDiff) return;
    const diffEditor = diffEditorRef.current;
    if (!diffEditor) return;

    if (content === null) {
      diffEditor.setModel(null);
      return;
    }

    const modified = getFileModel(file, content);
    syncModelContent(modified, file, content);

    const original = monaco.editor.createModel(diffRemoteContent, file.languageId);
    registerBashHostModel(original, file.hostId);
    diffOriginalModelRef.current?.dispose();
    diffOriginalModelRef.current = original;
    diffEditor.setModel({ original, modified });

    return () => {
      diffOriginalModelRef.current?.dispose();
      diffOriginalModelRef.current = null;
    };
  }, [isDiff, diffRemoteContent, file, content, syncModelContent]);

  /* —— 只读状态（dirty 等状态不在此推导，仅服从 props；diff 期间恒只读） —— */

  useEffect(() => {
    codeEditorRef.current?.updateOptions({ readOnly });
  }, [readOnly, isDiff]);

  return (
    <div
      className="h-full w-full overflow-hidden bg-background p-2"
      onKeyDownCapture={(e) => handleEditorKeyDownCapture(e, codeEditorRef.current)}
    >
      <div className="relative h-full min-h-full overflow-hidden rounded-xl border border-border">
        <div ref={containerRef} className="h-full w-full" />
        {content === null && (
          <div className="absolute inset-0 flex items-center justify-center bg-background">
            <Spinner className="size-5 text-foreground-subtlest" />
          </div>
        )}
      </div>
    </div>
  );
}
