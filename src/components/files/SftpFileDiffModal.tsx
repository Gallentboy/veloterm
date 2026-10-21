import {
  AlertCircle,
  ArrowLeftRight,
  Columns2,
  FileDiff,
  Loader2,
  Rows2,
  X,
} from 'lucide-react';
import { memo, useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as monaco from 'monaco-editor';
import { Button } from '@/components/ui/button';
import { loadRemoteContent } from '@/editor/contentLoader';
import { resolveLanguageIdWithContent } from '@/editor/languageMap';
import { setupMonacoEnvironment } from '@/editor/monacoEnvironment';
import { initMonacoTheme } from '@/editor/monacoTheme';
import { ensureNginxLanguage } from '@/editor/nginxLanguage';
import { ensureBashCompletion, registerBashHostModel } from '@/editor/bashLanguage';

export interface SftpDiffFileTarget {
  name: string;
  path: string;
}

export interface SftpFileDiffModalProps {
  hostId: number;
  original: SftpDiffFileTarget;
  modified: SftpDiffFileTarget;
  onClose: () => void;
}

const DIFF_EDITOR_OPTIONS: monaco.editor.IDiffEditorConstructionOptions = {
  readOnly: true,
  renderSideBySide: true,
  automaticLayout: true,
  fontFamily: 'var(--font-mono)',
  fontSize: 13,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  wordWrap: 'off',
  renderWhitespace: 'selection',
  smoothScrolling: true,
  originalEditable: false,
  scrollbar: {
    vertical: 'auto',
    horizontal: 'auto',
    verticalScrollbarSize: 8,
    horizontalScrollbarSize: 8,
    useShadows: false,
  },
};

export const SftpFileDiffModal = memo(function SftpFileDiffModal({
  hostId,
  original: initialOriginal,
  modified: initialModified,
  onClose,
}: SftpFileDiffModalProps) {
  const { t } = useTranslation();

  // 1. 左右文件对象（支持一键交换）
  const [origTarget, setOrigTarget] = useState<SftpDiffFileTarget>(initialOriginal);
  const [modTarget, setModTarget] = useState<SftpDiffFileTarget>(initialModified);

  // 2. 双栏并排 / 单栏内联展示切换
  const [sideBySide, setSideBySide] = useState(true);

  // 3. 内容加载状态
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [origContent, setOrigContent] = useState<string | null>(null);
  const [modContent, setModContent] = useState<string | null>(null);

  // 4. 差异变更统计
  const [diffStats, setDiffStats] = useState<{ added: number; removed: number; identical: boolean } | null>(null);

  const containerRef = useRef<HTMLDivElement | null>(null);
  const diffEditorRef = useRef<monaco.editor.IStandaloneDiffEditor | null>(null);
  const modelsRef = useRef<{
    original: monaco.editor.ITextModel;
    modified: monaco.editor.ITextModel;
  } | null>(null);

  // 加载两侧文件内容
  const fetchContents = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const [resOrig, resMod] = await Promise.all([
        loadRemoteContent(hostId, origTarget.path),
        loadRemoteContent(hostId, modTarget.path),
      ]);
      if (!resOrig) throw new Error(`${origTarget.name}: ${t('files.previewUnavailable')}`);
      if (!resMod) throw new Error(`${modTarget.name}: ${t('files.previewUnavailable')}`);
      setOrigContent(resOrig.content);
      setModContent(resMod.content);
    } catch (err) {
      setLoadError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [hostId, origTarget, modTarget, t]);

  useEffect(() => {
    void fetchContents();
  }, [fetchContents]);

  // 交换左右两栏文件
  const handleSwap = useCallback(() => {
    const prevOrig = origTarget;
    const prevMod = modTarget;
    setOrigTarget(prevMod);
    setModTarget(prevOrig);
    setOrigContent(modContent);
    setModContent(origContent);
  }, [origTarget, modTarget, origContent, modContent]);

  // 实例化 Monaco Diff Editor
  useEffect(() => {
    if (origContent === null || modContent === null) return;
    const container = containerRef.current;
    if (!container) return;

    initMonacoTheme();
    setupMonacoEnvironment();
    ensureNginxLanguage(monaco);
    ensureBashCompletion(monaco);

    const diffEditor = monaco.editor.createDiffEditor(container, {
      ...DIFF_EDITOR_OPTIONS,
      renderSideBySide: sideBySide,
    });
    diffEditorRef.current = diffEditor;

    // 内容嗅探：conf 系文件实为 nginx 时切 nginx（高亮/补全/hover）
    const originalLang = resolveLanguageIdWithContent(origTarget.name, origContent);
    const modifiedLang = resolveLanguageIdWithContent(modTarget.name, modContent);

    const originalModel = monaco.editor.createModel(origContent, originalLang);
    const modifiedModel = monaco.editor.createModel(modContent, modifiedLang);
    registerBashHostModel(originalModel, hostId);
    registerBashHostModel(modifiedModel, hostId);
    modelsRef.current = { original: originalModel, modified: modifiedModel };

    diffEditor.setModel({
      original: originalModel,
      modified: modifiedModel,
    });

    // 监听差异计算完成事件，统计增删行数
    const diffListener = diffEditor.onDidUpdateDiff(() => {
      const changes = diffEditor.getLineChanges();
      if (!changes || changes.length === 0) {
        setDiffStats({ added: 0, removed: 0, identical: origContent === modContent });
        return;
      }

      let added = 0;
      let removed = 0;
      for (const change of changes) {
        if (change.modifiedEndLineNumber >= change.modifiedStartLineNumber) {
          added += change.modifiedEndLineNumber - change.modifiedStartLineNumber + 1;
        }
        if (change.originalEndLineNumber >= change.originalStartLineNumber) {
          removed += change.originalEndLineNumber - change.originalStartLineNumber + 1;
        }
      }
      setDiffStats({ added, removed, identical: false });
    });

    return () => {
      diffListener.dispose();
      diffEditorRef.current = null;
      diffEditor.dispose();
      originalModel.dispose();
      modifiedModel.dispose();
      modelsRef.current = null;
    };
  }, [origContent, modContent, origTarget.name, modTarget.name, sideBySide]);

  // 切换双栏/单栏内联
  const toggleSideBySide = useCallback(() => {
    setSideBySide((prev) => {
      const next = !prev;
      diffEditorRef.current?.updateOptions({ renderSideBySide: next });
      return next;
    });
  }, []);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t('files.compare')}
      onClick={onClose}
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/25 p-6 animate-in fade-in duration-150"
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="flex h-[min(85vh,800px)] w-[min(92vw,1200px)] max-w-full flex-col overflow-hidden rounded-2xl border border-border/80 bg-popover shadow-2xl animate-in zoom-in-95 duration-150"
      >
        {/* 顶部标题栏与对比工具栏 */}
        <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border/60 px-4 py-2.5">
          {/* 左侧：文件对比标签 */}
          <div className="flex min-w-0 flex-1 items-center gap-2">
            <FileDiff className="size-4 shrink-0 text-terminal-blue" />
            {/* Original 文件 */}
            <div className="flex min-w-0 items-center gap-1">
              <span className="shrink-0 rounded bg-terminal-red/10 px-1.5 py-0.5 text-[10px] font-semibold text-terminal-red">
                原版
              </span>
              <span className="truncate font-mono text-ui-xs text-foreground" title={origTarget.path}>
                {origTarget.name}
              </span>
            </div>

            {/* 交换左右按钮 */}
            <button
              type="button"
              onClick={handleSwap}
              className="flex shrink-0 items-center justify-center rounded p-1 text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
              title={t('files.swapSides')}
            >
              <ArrowLeftRight className="size-3.5" />
            </button>

            {/* Modified 文件 */}
            <div className="flex min-w-0 items-center gap-1">
              <span className="shrink-0 rounded bg-terminal-green/10 px-1.5 py-0.5 text-[10px] font-semibold text-terminal-green">
                对比
              </span>
              <span className="truncate font-mono text-ui-xs text-foreground" title={modTarget.path}>
                {modTarget.name}
              </span>
            </div>

            {/* 差异统计徽章 */}
            {diffStats ? (
              diffStats.identical ? (
                <span className="shrink-0 rounded-full bg-surface px-2 py-0.5 text-[11px] text-foreground-subtle select-none">
                  {t('files.diffNoChanges')}
                </span>
              ) : (
                <div className="flex shrink-0 items-center gap-1.5 text-ui-xs font-mono select-none">
                  <span className="text-terminal-green font-semibold">+{diffStats.added}</span>
                  <span className="text-terminal-red font-semibold">-{diffStats.removed}</span>
                </div>
              )
            ) : null}
          </div>

          {/* 右侧：视图切换与关闭 */}
          <div className="flex shrink-0 items-center gap-2">
            {/* 视图切换按钮 */}
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={toggleSideBySide}
              className="h-7 gap-1.5 rounded-md px-2.5 text-ui-xs font-medium text-foreground-subtle hover:text-foreground"
              title={sideBySide ? t('files.diffInline') : t('files.diffSideBySide')}
            >
              {sideBySide ? <Rows2 className="size-3.5" /> : <Columns2 className="size-3.5" />}
              <span>{sideBySide ? t('files.diffInline') : t('files.diffSideBySide')}</span>
            </Button>

            <button
              type="button"
              onClick={onClose}
              aria-label={t('common.cancel')}
              className="shrink-0 rounded-md p-1 text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
            >
              <X className="size-4" />
            </button>
          </div>
        </div>

        {/* Diff 视图容器 */}
        <div className="relative min-h-0 flex-1">
          <div ref={containerRef} className="h-full w-full" />

          {loading ? (
            <div className="absolute inset-0 flex items-center justify-center bg-popover/80 backdrop-blur-xs">
              <div className="flex items-center gap-2 text-ui-sm text-foreground-subtle">
                <Loader2 className="size-4 animate-spin" />
                <span>加载文件对比数据中...</span>
              </div>
            </div>
          ) : null}

          {loadError ? (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-2.5 bg-popover p-6 text-center">
              <AlertCircle className="size-6 text-warning" />
              <p className="max-w-md text-ui-sm text-foreground-subtle break-all">{loadError}</p>
              <Button type="button" size="sm" variant="outline" onClick={() => void fetchContents()}>
                重试加载
              </Button>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
});
