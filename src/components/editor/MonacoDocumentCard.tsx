import { Loader2 } from 'lucide-react';
import { useCallback, useEffect, useRef } from 'react';
import * as monaco from 'monaco-editor';
import { initMonacoTheme, VELOTERM_MONACO_THEME } from '@/editor/monacoTheme';
import type { EditorFileDescriptor, MonacoDocumentCardProps } from '@/editor/types';

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

/* —— worker 环境（模块级一次） —— */

/**
 * worker 走 Rspack 原生的 `new Worker(new URL(...), import.meta.url)` 模式编译为独立 chunk。
 * 注意两点：
 * - `new URL(...)` 必须内联在 `new Worker(...)` 里（外提成辅助函数会退化为按 asset 原样拷贝，
 *   产物里的相对 import 在运行期 404）
 * - 子路径用包 exports 映射（`./*.js` → `./esm/vs/*.js`）展开后的形态：
 *   带 `esm/vs/` 前缀的子路径会被二次映射而无法解析（Rspack 遵循 exports 映射）
 */
let monacoEnvironmentReady = false;

function setupMonacoEnvironment(): void {
  if (monacoEnvironmentReady) return;
  monacoEnvironmentReady = true;
  self.MonacoEnvironment = {
    getWorker(_workerId, label) {
      switch (label) {
        case 'json':
          return new Worker(
            new URL('monaco-editor/language/json/json.worker.js', import.meta.url),
            { type: 'module' },
          );
        case 'typescript':
        case 'javascript':
          return new Worker(
            new URL('monaco-editor/language/typescript/ts.worker.js', import.meta.url),
            { type: 'module' },
          );
        default:
          return new Worker(
            new URL('monaco-editor/editor/editor.worker.js', import.meta.url),
            { type: 'module' },
          );
      }
    },
  };
}

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
  } else if (model.getLanguageId() !== file.languageId) {
    monaco.editor.setModelLanguage(model, file.languageId);
  }
  return model;
}

const MONACO_EDITOR_OPTIONS = {
  theme: VELOTERM_MONACO_THEME,
  automaticLayout: true,
  fontFamily: 'var(--font-mono)',
  fontSize: 13,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  renderWhitespace: 'selection',
  smoothScrolling: true,
  fixedOverflowWidgets: true,
  padding: { top: 8, bottom: 8 },
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
    latestPropsRef.current.onViewStateChange(state);
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

  /* —— 编辑器生命周期（普通 / diff 换装；容器与 mode 变化时重建） —— */

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    initMonacoTheme();
    setupMonacoEnvironment();

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
          latestPropsRef.current.onViewStateChange(prevState);
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
    <div className="h-full w-full overflow-hidden bg-background p-2">
      <div className="relative h-full min-h-full overflow-hidden rounded-xl border border-border">
        <div ref={containerRef} className="h-full w-full" />
        {content === null && (
          <div className="absolute inset-0 flex items-center justify-center bg-background">
            <Loader2 className="animate-spin text-foreground-subtlest" size={20} />
          </div>
        )}
      </div>
    </div>
  );
}
