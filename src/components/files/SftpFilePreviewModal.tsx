import { AlertCircle, Eye, Loader2, Pencil, PencilOff, Save, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as monaco from 'monaco-editor';
import { Button } from '@/components/ui/button';
import { loadRemoteContent, saveRemoteContent } from '@/editor/contentLoader';
import { resolveLanguageId } from '@/editor/languageMap';
import { setupMonacoEnvironment } from '@/editor/monacoEnvironment';
import { initMonacoTheme, VELOTERM_MONACO_THEME } from '@/editor/monacoTheme';

/**
 * SFTP 文件预览弹窗：Monaco 承载，默认只读，可一键切换编辑并保存回远端。
 *
 * - 内容加载在组件内完成（loadRemoteContent：>5MB / 非文本由后端拒绝并展示原因）；
 * - 默认 readOnly，头部「编辑」/「退出编辑」按钮切换编辑态（退出时脏内容回滚到基线）；
 *   ⌘S/Ctrl+S 或「保存」按钮写回远端；
 * - 脏态（未保存更改）时点遮罩/X 不直接关闭，先弹「放弃更改 / 继续编辑」确认条；
 * - 每次打开新建临时 model，关闭即 dispose（不复用编辑器面板的 model 缓存，
 *   预览的时效性优先：下次打开重新拉远端内容）；
 * - worker 环境 / 主题跟随走共享模块（monacoEnvironment / monacoTheme），
 *   编辑器面板从未打开过也能独立工作。
 */

interface SftpFilePreviewModalProps {
  hostId: number;
  fileName: string;
  remotePath: string;
  onClose: () => void;
  /** 保存成功后通知父层（刷新目录列表的 size/mtime 列） */
  onSaved?: () => void;
}

const PREVIEW_EDITOR_OPTIONS = {
  theme: VELOTERM_MONACO_THEME,
  automaticLayout: true,
  fontFamily: 'var(--font-mono)',
  fontSize: 13,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  // 预览场景对齐旧 <pre> 的 whitespace-pre-wrap 行为：长行折行
  wordWrap: 'on',
  renderWhitespace: 'selection',
  smoothScrolling: true,
  fixedOverflowWidgets: true,
  padding: { top: 8, bottom: 8 },
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

export function SftpFilePreviewModal(props: SftpFilePreviewModalProps) {
  const { hostId, fileName, remotePath, onClose, onSaved } = props;
  const { t } = useTranslation();

  /** 远端内容基线：加载时初始化、保存成功后推进（编辑器内容不回写此状态） */
  const [baseline, setBaseline] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  /** 脏态下的二次确认待执行动作：关闭弹窗 / 退出编辑（两者都会丢失未保存更改） */
  const [pendingDiscard, setPendingDiscard] = useState<'close' | 'exit-edit' | null>(null);

  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const baselineRef = useRef<string | null>(null);
  /** ⌘S 命令回调取最新闭包（命令绑定在编辑器实例上，仅注册一次） */
  const saveRef = useRef<() => void>(() => {});
  const onSavedRef = useRef(onSaved);
  useEffect(() => {
    onSavedRef.current = onSaved;
  });
  /** 翻译函数经 ref 取最新（避免 t 进加载 effect 依赖：语言切换不应重拉远端内容） */
  const tRef = useRef(t);
  useEffect(() => {
    tRef.current = t;
  });

  /* —— 内容加载（打开时一次） —— */
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const result = await loadRemoteContent(hostId, remotePath);
        if (!result) throw new Error(tRef.current('files.previewUnavailable'));
        if (!cancelled) {
          baselineRef.current = result.content;
          setBaseline(result.content);
        }
      } catch (err) {
        if (!cancelled) {
          setLoadError(err instanceof Error ? err.message : String(err));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [hostId, remotePath]);

  /* —— 保存（编辑态 + 脏态才可触发） —— */
  const save = useCallback(async () => {
    const editor = editorRef.current;
    if (!editor || saving || !dirty) return;
    const value = editor.getModel()?.getValue();
    if (value == null) return;
    setSaving(true);
    setSaveError(null);
    try {
      await saveRemoteContent(hostId, remotePath, value);
      baselineRef.current = value;
      setBaseline(value);
      setDirty(false);
      setPendingDiscard(null);
      onSavedRef.current?.();
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }, [dirty, hostId, remotePath, saving]);

  useEffect(() => {
    saveRef.current = () => void save();
  }, [save]);

  /* —— 编辑器生命周期：内容就绪后建一次，卸载即销毁（保存不重建，光标不丢） —— */
  const hasContent = baseline !== null;
  useEffect(() => {
    if (!hasContent) return;
    const container = containerRef.current;
    const initial = baselineRef.current;
    if (!container || initial == null) return;

    initMonacoTheme();
    setupMonacoEnvironment();

    const model = monaco.editor.createModel(initial, resolveLanguageId(fileName));
    const editor = monaco.editor.create(container, {
      ...PREVIEW_EDITOR_OPTIONS,
      model,
      readOnly: true,
    });
    editorRef.current = editor;

    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
      saveRef.current();
    });
    const listener = model.onDidChangeContent(() => {
      setDirty(model.getValue() !== baselineRef.current);
      setPendingDiscard(null);
    });

    return () => {
      listener.dispose();
      editorRef.current = null;
      editor.dispose();
      model.dispose();
    };
  }, [hasContent, fileName]);

  /* —— 只读开关：非编辑态 / 保存中均锁编辑 —— */
  useEffect(() => {
    editorRef.current?.updateOptions({ readOnly: !editing || saving });
  }, [editing, saving]);

  const switchToEdit = useCallback(() => {
    setPendingDiscard(null);
    setEditing(true);
    // 切编辑态后聚焦（readOnly 解除在 effect 中，晚一帧再聚焦）
    requestAnimationFrame(() => editorRef.current?.focus());
  }, []);

  /** 退出编辑：回只读；脏内容回滚到基线（readOnly 视图必须展示真实远端内容） */
  const exitEdit = useCallback(() => {
    setPendingDiscard(null);
    const model = editorRef.current?.getModel();
    if (model && baselineRef.current != null && model.getValue() !== baselineRef.current) {
      model.setValue(baselineRef.current); // 触发内容监听 → dirty 归零
    }
    setEditing(false);
  }, []);

  /** 退出编辑请求：干净态直接退，脏态先走放弃确认 */
  const requestExitEdit = useCallback(() => {
    if (saving) return;
    if (dirty) {
      setPendingDiscard('exit-edit');
      return;
    }
    exitEdit();
  }, [dirty, saving, exitEdit]);

  /** 关闭请求：脏态先确认，干净态直接关 */
  const requestClose = useCallback(() => {
    if (dirty) {
      setPendingDiscard('close');
      return;
    }
    onClose();
  }, [dirty, onClose]);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t('files.preview')}
      onClick={requestClose}
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/25 p-6 animate-in fade-in duration-150"
    >
      <div
        onClick={(event) => event.stopPropagation()}
        className="flex h-[min(78vh,720px)] w-[min(80vw,960px)] max-w-full flex-col overflow-hidden rounded-2xl border border-border/80 bg-popover shadow-2xl animate-in zoom-in-95 duration-150"
      >
        <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/60 px-4 py-2.5">
          <div className="flex min-w-0 items-center gap-2">
            <Eye className="size-3.5 shrink-0 text-foreground-subtle" />
            <span className="truncate font-mono text-ui-sm text-foreground select-none">
              {fileName}
            </span>
            {!editing ? (
              <span className="shrink-0 rounded-full bg-surface px-2 py-0.5 text-ui-xs text-foreground-subtle select-none">
                {t('files.previewReadonly')}
              </span>
            ) : dirty ? (
              <span className="shrink-0 rounded-full bg-warning/15 px-2 py-0.5 text-ui-xs text-warning select-none">
                {t('files.previewUnsaved')}
              </span>
            ) : (
              <span className="shrink-0 rounded-full bg-surface px-2 py-0.5 text-ui-xs text-foreground-subtle select-none">
                {t('files.previewEditing')}
              </span>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            {!editing ? (
              <Button type="button" size="sm" variant="ghost" onClick={switchToEdit}>
                <Pencil className="size-3" />
                <span>{t('common.edit')}</span>
              </Button>
            ) : (
              <>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  disabled={saving}
                  onClick={requestExitEdit}
                >
                  <PencilOff className="size-3" />
                  <span>{t('files.previewExitEdit')}</span>
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  disabled={!dirty || saving}
                  onClick={() => void save()}
                >
                  {saving ? (
                    <Loader2 className="size-3 animate-spin" />
                  ) : (
                    <Save className="size-3" />
                  )}
                  <span>{t('common.save')}</span>
                </Button>
              </>
            )}
            <button
              type="button"
              onClick={requestClose}
              aria-label={t('common.cancel')}
              className="shrink-0 rounded-md p-1 text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
            >
              <X className="size-4" />
            </button>
          </div>
        </div>

        <div className="relative min-h-0 flex-1">
          <div ref={containerRef} className="h-full w-full" />
          {baseline === null && !loadError ? (
            <div className="absolute inset-0 flex items-center justify-center bg-popover">
              <Loader2 className="size-5 animate-spin text-foreground-subtle" />
            </div>
          ) : null}
          {loadError ? (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-popover px-6 text-center">
              <AlertCircle className="size-6 text-warning" />
              <p className="break-all text-ui-sm text-foreground-subtle">{loadError}</p>
            </div>
          ) : null}
        </div>

        {saveError != null || pendingDiscard != null ? (
          <div className="flex shrink-0 flex-col gap-0 border-t border-border/60">
            {saveError != null ? (
              <p className="flex items-center gap-1.5 px-4 py-1.5 text-ui-xs text-warning">
                <AlertCircle className="size-3.5 shrink-0" />
                <span className="break-all">{t('files.previewSaveFailed', { error: saveError })}</span>
              </p>
            ) : null}
            {pendingDiscard != null ? (
              <div className="flex items-center justify-between gap-3 px-4 py-2">
                <p className="text-ui-sm text-foreground-subtle">
                  {t('files.previewUnsavedHint')}
                </p>
                <div className="flex shrink-0 items-center gap-1.5">
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    onClick={() => setPendingDiscard(null)}
                  >
                    {t('files.previewKeepEditing')}
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    className="text-destructive hover:bg-destructive/10"
                    onClick={() => {
                      if (pendingDiscard === 'close') {
                        onClose();
                      } else {
                        exitEdit();
                      }
                    }}
                  >
                    {t('files.previewDiscard')}
                  </Button>
                </div>
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  );
}
