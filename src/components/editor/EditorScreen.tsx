import { AlertTriangle, FileCode, RotateCw } from 'lucide-react';
import { lazy, memo, Suspense, useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { EditorTabBar } from '@/components/editor/EditorTabBar';
import { Button } from '@/components/ui/button';
import { useEditorStore } from '@/editor/EditorProvider';
import type { EditorFileDescriptor } from '@/editor/types';

const MemoizedMonacoDocumentCard = memo(
  lazy(() =>
    import('@/components/editor/MonacoDocumentCard').then((m) => ({
      default: m.MonacoDocumentCard,
    })),
  ),
);

/** 状态栏右侧状态文本（dirty 前加 ●） */
function EditorStatusText({ status }: { status: EditorFileDescriptor['status'] }) {
  const { t } = useTranslation();
  if (status === 'dirty') {
    return (
      <span className="inline-flex items-center gap-1">
        <span className="text-warning">●</span>
        <span>{t('editor.status.dirty')}</span>
      </span>
    );
  }
  return <span>{t(`editor.status.${status}`)}</span>;
}

/** 冲突动作按钮（用本地覆盖远端 / 取远端丢弃本地） */
function EditorConflictActions({ fileId }: { fileId: number }) {
  const { t } = useTranslation();
  const { resolveConflict } = useEditorStore();
  return (
    <span className="inline-flex items-center gap-1">
      <Button
        type="button"
        variant="outline"
        size="xs"
        onClick={() => void resolveConflict(fileId, 'overwrite')}
      >
        {t('editor.conflict.overwrite')}
      </Button>
      <Button
        type="button"
        variant="outline"
        size="xs"
        onClick={() => void resolveConflict(fileId, 'take-remote')}
      >
        {t('editor.conflict.takeRemote')}
      </Button>
    </span>
  );
}

/**
 * 远程文件编辑页：
 * - 空状态：FileCode 图标 + 引导文案（前往文件页打开文件）；
 * - 有文件：顶部 EditorTabBar + 中部 Monaco 文档卡片 + 底部 h-6 状态栏；
 * - 状态栏左=remotePath · languageId 大写，右=Ln/Col · 状态文本（dirty 前加 ●）；
 * - conflict 时状态栏提供「覆盖远端 / 取远端」动作；Cmd/Ctrl+S 由 Monaco 卡片内部回调 onSaveRequest。
 */
export function EditorScreen() {
  const { t } = useTranslation();
  const {
    files,
    activeFile,
    activeFileId,
    contents,
    viewStates,
    conflictRemotes,
    saveErrors,
    loadErrors,
    closeFile,
    activateFile,
    saveFile,
    reloadFile,
    handleContentChange,
    handleViewStateChange,
  } = useEditorStore();

  const [cursor, setCursor] = useState({ line: 1, column: 1 });
  const formatRef = useRef<(() => Promise<void>) | null>(null);
  const [isFormatting, setIsFormatting] = useState(false);

  const handleFormat = useCallback(async () => {
    if (!formatRef.current || isFormatting) return;
    setIsFormatting(true);
    try {
      await formatRef.current();
    } catch (err) {
      console.warn('Format failed', err);
    } finally {
      setIsFormatting(false);
    }
  }, [isFormatting]);

  const handleRegisterFormat = useCallback((fn: (() => Promise<void>) | null) => {
    formatRef.current = fn;
  }, []);

  // 切换文件时重置光标显示（真实位置由 Monaco 恢复 viewState 后回报）
  useEffect(() => {
    setCursor({ line: 1, column: 1 });
  }, [activeFileId]);

  const handleContentChangeForActive = useCallback(
    (content: string) => {
      if (activeFileId !== null) {
        handleContentChange(activeFileId, content);
      }
    },
    [activeFileId, handleContentChange],
  );

  const handleSaveRequestForActive = useCallback(() => {
    if (activeFileId !== null) {
      void saveFile(activeFileId);
    }
  }, [activeFileId, saveFile]);

  // 契约 v2：viewState 上报携带 fileId，切文件瞬间的归档不会错位
  const handleViewStateChangeById = useCallback(
    (fileId: number, viewState: unknown) => {
      handleViewStateChange(fileId, viewState);
    },
    [handleViewStateChange],
  );

  const handleCursorChange = useCallback((line: number, column: number) => {
    setCursor({ line, column });
  }, []);

  if (!activeFile) {
    return (
      <section
        data-testid="editor-screen"
        className="flex h-full min-h-0 flex-col overflow-hidden bg-background"
      >
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 p-6 text-center select-none">
          <div className="flex size-14 items-center justify-center rounded-2xl border border-border/60 bg-surface/50 shadow-xs">
            <FileCode className="size-7 text-foreground-subtle" strokeWidth={1.5} />
          </div>
          {/* 空态文案用主字号（--text-ui-base），随「界面字号」设置缩放；
              此前是 text-ui-xs（主字号 -4px）明显偏小 */}
          <p className="max-w-sm text-ui-base leading-relaxed text-foreground-subtle">
            {t('editor.empty')}
          </p>
        </div>
      </section>
    );
  }

  const file = activeFile;
  const content = contents.get(file.id) ?? null;
  const viewState = viewStates.has(file.id) ? (viewStates.get(file.id) ?? null) : null;
  const diffRemoteContent =
    file.status === 'conflict' ? (conflictRemotes.get(file.id) ?? null) : null;
  const readOnly =
    file.status === 'offline' || file.status === 'orphaned' || file.status === 'conflict';
  const saveError = saveErrors.get(file.id) ?? null;
  // 加载失败（无权限 / 非文本 / 超大小 / 连接断开等）：显示错误面板而非让
  // content=null 的 Monaco 卡片永久转圈；reloadFile 内部会置回 loading 并重新拉取
  const loadError = loadErrors.get(file.id) ?? null;

  return (
    <section
      data-testid="editor-screen"
      className="flex h-full min-h-0 flex-col overflow-hidden bg-background"
    >
      <EditorTabBar
        files={files}
        activeFileId={activeFileId}
        onActivate={activateFile}
        onClose={closeFile}
        onFormat={handleFormat}
        isFormatting={isFormatting}
        canFormat={!readOnly && content !== null && loadError === null}
      />

      {content === null && loadError !== null ? (
        <div className="flex min-h-0 flex-1 items-center justify-center p-6">
          <div className="flex w-full max-w-md flex-col items-center gap-3 text-center">
            <div className="flex size-12 items-center justify-center rounded-2xl border border-border/60 bg-surface/50 shadow-xs">
              <AlertTriangle className="size-6 text-warning" strokeWidth={1.5} />
            </div>
            <h3 className="text-ui-base font-medium text-foreground">
              {t('editor.loadFailedTitle')}
            </h3>
            <p className="w-full truncate rounded-md border border-border/60 bg-surface/60 px-2.5 py-1.5 font-mono text-ui-xs text-foreground-subtle select-all">
              {file.remotePath}
            </p>
            <p className="text-ui-sm leading-relaxed text-foreground-subtle">{loadError}</p>
            <div className="mt-1 flex items-center gap-2">
              <Button type="button" variant="outline" onClick={() => void reloadFile(file.id)}>
                <RotateCw className="size-3.5" />
                <span>{t('editor.loadFailedRetry')}</span>
              </Button>
              <Button type="button" variant="ghost" onClick={() => closeFile(file.id)}>
                {t('common.close')}
              </Button>
            </div>
          </div>
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-hidden p-3 pt-2">
          <Suspense fallback={<div className="h-full w-full" />}>
            <MemoizedMonacoDocumentCard
              key={file.id}
              file={file}
              content={content}
              viewState={viewState}
              diffRemoteContent={diffRemoteContent}
              readOnly={readOnly}
              onContentChange={handleContentChangeForActive}
              onSaveRequest={handleSaveRequestForActive}
              onViewStateChange={handleViewStateChangeById}
              onCursorChange={handleCursorChange}
              onRegisterFormat={handleRegisterFormat}
            />
          </Suspense>
        </div>
      )}

      <footer className="flex h-6 shrink-0 items-center justify-between gap-2 border-t border-border/50 px-3 text-ui-xs text-foreground-subtle select-none">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate font-mono" title={file.remotePath}>
            {file.remotePath}
          </span>
          <span className="text-foreground-subtlest" aria-hidden>
            ·
          </span>
          <span className="shrink-0 uppercase">{file.languageId}</span>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <span className="font-mono">
            Ln {cursor.line}, Col {cursor.column}
          </span>
          <span className="text-foreground-subtlest" aria-hidden>
            ·
          </span>
          <EditorStatusText status={file.status} />
          {saveError ? <span className="truncate text-destructive">{t('editor.saveFailed')}</span> : null}
          {file.status === 'conflict' ? <EditorConflictActions fileId={file.id} /> : null}
        </div>
      </footer>
    </section>
  );
}
