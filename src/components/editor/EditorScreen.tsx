import { FileCode } from 'lucide-react';
import { memo, useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { EditorTabBar } from '@/components/editor/EditorTabBar';
import { MonacoDocumentCard } from '@/components/editor/MonacoDocumentCard';
import { Button } from '@/components/ui/button';
import { useEditorStore } from '@/editor/EditorProvider';
import type { EditorFileDescriptor } from '@/editor/types';

const MemoizedMonacoDocumentCard = memo(MonacoDocumentCard);

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
    closeFile,
    activateFile,
    saveFile,
    handleContentChange,
    handleViewStateChange,
  } = useEditorStore();

  const [cursor, setCursor] = useState({ line: 1, column: 1 });

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

  const handleViewStateChangeForActive = useCallback(
    (viewState: unknown) => {
      if (activeFileId !== null) {
        handleViewStateChange(activeFileId, viewState);
      }
    },
    [activeFileId, handleViewStateChange],
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
          <p className="max-w-sm text-ui-xs leading-relaxed text-foreground-subtle">
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
      />

      <div className="min-h-0 flex-1 overflow-hidden p-3 pt-2">
        <MemoizedMonacoDocumentCard
          key={file.id}
          file={file}
          content={content}
          viewState={viewState}
          diffRemoteContent={diffRemoteContent}
          readOnly={readOnly}
          onContentChange={handleContentChangeForActive}
          onSaveRequest={handleSaveRequestForActive}
          onViewStateChange={handleViewStateChangeForActive}
          onCursorChange={handleCursorChange}
        />
      </div>

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
