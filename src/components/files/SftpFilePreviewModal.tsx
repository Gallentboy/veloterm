import { AlertCircle, Binary, BookOpen, Code2, Eye, FileJson, Pencil, PencilOff, Save, ScrollText, ShieldCheck, Table, X } from 'lucide-react';
import { Suspense, useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as monaco from 'monaco-editor';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { PdfRemotePreview } from '@/components/files/PdfRemotePreview';
import { ImageRemotePreview } from '@/components/files/ImageRemotePreview';
import { MarkdownPreview } from '@/components/files/MarkdownPreview';
import { CsvPreview } from '@/components/files/CsvPreview';
import { JsonPreview } from '@/components/files/JsonPreview';
import { HexPreview } from '@/components/files/HexPreview';
import { LogViewer } from '@/components/files/LogViewer';
import { ExcelRemotePreview } from '@/components/files/ExcelRemotePreview';
import { CertViewer } from '@/components/files/CertViewer';
import { SqliteRemotePreview } from '@/components/files/SqliteRemotePreview';
import { isPreviewableImage } from '@/services/imagePreviewService';
import { loadRemoteContent, loadRemoteContentSudo, saveRemoteContent, saveRemoteContentSudo } from '@/editor/contentLoader';
import { resolveLanguageIdWithContent } from '@/editor/languageMap';
import { setupMonacoEnvironment } from '@/editor/monacoEnvironment';
import { initMonacoTheme, VELOTERM_MONACO_THEME } from '@/editor/monacoTheme';
import { ensureExtendedLanguages } from '@/editor/languages';
import { ensureBashCompletion, registerBashHostModel } from '@/editor/bashLanguage';
import { attachHtmlAutoInsert } from '@/editor/htmlAutoInsert';
import { attachFindReplaceCommands, handleEditorKeyDownCapture } from '@/editor/findReplace';
import { cn } from '@/lib/utils';

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
  /** SFTP 会话 id（PDF 分支下载远端文件用；未传时 PDF 预览不可用） */
  sessionId?: string;
  fileName: string;
  remotePath: string;
  onClose: () => void;
  /** 保存成功后通知父层（刷新目录列表的 size/mtime 列） */
  onSaved?: () => void;
}

/** 后端 SFTP 权限错误文案判定（OpenSSH 状态码名「Permission denied」经 dedup 后的形态） */
function isPermissionDenied(message: string): boolean {
  return /permission denied/i.test(message);
}

const PREVIEW_EDITOR_OPTIONS = {
  theme: VELOTERM_MONACO_THEME,
  automaticLayout: true,
  fontFamily: 'var(--font-mono)',
  fontLigatures: true,
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
  const { hostId, sessionId, fileName, remotePath, onClose, onSaved } = props;
  const { t } = useTranslation();

  /** PDF 文件走独立渲染分支（不加载 Monaco） */
  const isPdf = /\.pdf$/i.test(fileName);
  /** 图片走独立预览分支（原生直传 / 后端转码，不加载 Monaco） */
  const isImage = isPreviewableImage(fileName);
  /** Markdown 文件支持富文本渲染视图与源码视图切换 */
  const isMarkdown = /\.(md|markdown|mdown|mkd)$/i.test(fileName);
  /** CSV/TSV 表格文件支持结构化虚拟滚动表格与源码视图切换 */
  const isCsv = /\.(csv|tsv)$/i.test(fileName);
  /** JSON/JSONL 结构化数据支持树状展示与内置 jq 过滤 */
  const isJson = /\.(json|jsonl)$/i.test(fileName);
  /** 日志文件（.log/.out/.err）支持智能 Tail 模式、日志级别高亮与实时追随 */
  const isLog = /\.(log|out|err|trace)(\.\d+)?$/i.test(fileName);
  /** Excel 工作簿（.xlsx/.xls/.xlsb/.ods）走 calamine 解析与多 Sheet 预览 */
  const isExcel = /\.(xlsx|xls|xlsb|ods)$/i.test(fileName);
  /** SQLite 数据库文件（.db/.sqlite/.sqlite3）走只读元数据与 SQL 表格浏览 */
  const isSqlite = /\.(db|sqlite|sqlite3)$/i.test(fileName);
  /** X.509 SSL 证书文件（.crt/.cer/.pem/.ca-bundle）走结构化证书解析器 */
  const isCert = /\.(crt|cer|pem|ca-bundle)$/i.test(fileName);
  /** 具备专用预览渲染器的文本类型（Markdown / CSV / TSV / JSON / Log / Cert） */
  const hasCustomPreview = isMarkdown || isCsv || isJson || isLog || isCert;
  /** 非纯文本/独立渲染分支（PDF / 图片 / Excel / SQLite）：跳过 Monaco 内容加载与编辑按钮 */
  const isBinaryPreview = isPdf || isImage || isExcel || isSqlite;

  /** 检测到非文本文件时自动置为 true 并切到 Hex 模式 */
  const [isBinaryFile, setIsBinaryFile] = useState(false);

  /** 预览模式：结构化文本默认进入专用渲染，其余文件进入 Monaco 源码视图 */
  const [viewMode, setViewMode] = useState<'preview' | 'source' | 'hex' | 'log' | 'cert'>(
    isCert ? 'cert' : isLog ? 'log' : hasCustomPreview ? 'preview' : 'source',
  );

  /** 远端内容基线：加载时初始化、保存成功后推进（编辑器内容不回写此状态） */
  const [baseline, setBaseline] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  /** 脏态下的二次确认待执行动作：关闭弹窗 / 退出编辑（两者都会丢失未保存更改） */
  const [pendingDiscard, setPendingDiscard] = useState<'close' | 'exit-edit' | null>(null);

  /* —— sudo 提权态（Permission denied 的兜底）：sudoMode = 内容经 sudo 读入，保存走 sudo tee —— */
  const [sudoMode, setSudoMode] = useState(false);
  /** 最近一次验证通过的 sudo 密码（保存时复用；后端另有按主机的会话级缓存） */
  const [sudoPassword, setSudoPassword] = useState<string | null>(null);
  const [showSudoPrompt, setShowSudoPrompt] = useState(false);
  const [sudoInput, setSudoInput] = useState('');
  const [sudoBusy, setSudoBusy] = useState(false);
  const [sudoWrong, setSudoWrong] = useState(false);

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
    // PDF / 图片走独立渲染分支，不加载 Monaco 文本内容
    if (isBinaryPreview) return;
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
          const errMsg = err instanceof Error ? err.message : String(err);
          // 二进制文件嗅探拦截：自动降级切入 Hex 十六进制查看器，不报红报错
          if (/NOT_TEXT|不是可编辑的文本文件/i.test(errMsg)) {
            setIsBinaryFile(true);
            setViewMode('hex');
            setLoadError(null);
          } else {
            setLoadError(errMsg);
          }
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [hostId, remotePath, isBinaryPreview]);

  /* —— sudo 前缀码 → i18n 文案（REMOTE: 原样透出服务器输出尾部） —— */
  const mapSudoError = useCallback(
    (raw: string): string => {
      if (raw === 'SUDO_PASSWORD_REQUIRED') return t('files.sudoPasswordNeeded');
      if (raw === 'SUDO_PASSWORD_WRONG') return t('files.sudoWrong');
      if (raw === 'SUDO_NOT_ALLOWED') return t('files.sudoErrNotAllowed');
      if (raw === 'SUDO_NO_TTY') return t('files.sudoErrNoTty');
      if (raw === 'SUDO_MISSING') return t('files.sudoErrMissing');
      if (raw === 'TOO_LARGE') return t('files.sudoErrTooLarge');
      if (raw === 'NOT_TEXT') return t('files.sudoErrNotText');
      if (raw.startsWith('REMOTE:')) return raw.slice('REMOTE:'.length);
      return raw;
    },
    [t],
  );

  /** sudo 读取：null = 先走 NOPASSWD/会话缓存/主机密码凭证；需要密码时弹内联输入 */
  const handleSudoRead = useCallback(
    async (password: string | null) => {
      if (sudoBusy) return;
      setSudoBusy(true);
      setSudoWrong(false);
      try {
        const result = await loadRemoteContentSudo(hostId, remotePath, password);
        if (!result) throw new Error(tRef.current('files.previewUnavailable'));
        baselineRef.current = result.content;
        setBaseline(result.content);
        setLoadError(null);
        setShowSudoPrompt(false);
        setSudoMode(true);
        setSudoPassword(password);
      } catch (err) {
        const raw = err instanceof Error ? err.message : String(err);
        if (raw === 'SUDO_PASSWORD_REQUIRED') {
          setShowSudoPrompt(true);
        } else if (raw === 'SUDO_PASSWORD_WRONG') {
          setSudoWrong(true);
          setShowSudoPrompt(true);
        } else {
          setLoadError(mapSudoError(raw));
        }
      } finally {
        setSudoBusy(false);
      }
    },
    [hostId, remotePath, sudoBusy, mapSudoError],
  );

  /** 内联 sudo 密码输入（加载失败态与保存失败态复用） */
  const renderSudoPrompt = (onConfirm: () => void) => (
    <div className="flex w-full max-w-sm items-center gap-2">
      <input
        type="password"
        value={sudoInput}
        onChange={(event) => setSudoInput(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && sudoInput.trim()) onConfirm();
        }}
        placeholder={t('files.sudoPromptTitle')}
        autoFocus
        className="h-7 min-w-0 flex-1 rounded-md border border-border/70 bg-background px-2 text-ui-xs text-foreground outline-none focus:border-border"
      />
      <Button
        type="button"
        size="sm"
        variant="outline"
        disabled={!sudoInput.trim()}
        onClick={onConfirm}
      >
        {t('files.sudoConfirm')}
      </Button>
      {sudoWrong ? (
        <span className="shrink-0 text-ui-xs text-warning">{t('files.sudoWrong')}</span>
      ) : null}
    </div>
  );

  /* —— 保存（编辑态 + 脏态才可触发；sudo 模式走 sudo tee） —— */
  const save = useCallback(
    async (passwordOverride?: string | null) => {
      const editor = editorRef.current;
      if (!editor || saving || !dirty) return;
      const value = editor.getModel()?.getValue();
      if (value == null) return;
      setSaving(true);
      setSaveError(null);
      try {
        const password = passwordOverride !== undefined ? passwordOverride : sudoPassword;
        if (sudoMode) {
          await saveRemoteContentSudo(hostId, remotePath, value, password);
        } else {
          await saveRemoteContent(hostId, remotePath, value);
        }
        baselineRef.current = value;
        setBaseline(value);
        setDirty(false);
        setPendingDiscard(null);
        onSavedRef.current?.();
      } catch (err) {
        const raw = err instanceof Error ? err.message : String(err);
        if (sudoMode && (raw === 'SUDO_PASSWORD_REQUIRED' || raw === 'SUDO_PASSWORD_WRONG')) {
          // 提权密码失效（远端改密）：就地弹密码输入重试
          if (raw === 'SUDO_PASSWORD_WRONG') setSudoWrong(true);
          setShowSudoPrompt(true);
          setSaveError(mapSudoError(raw));
        } else {
          setSaveError(sudoMode ? mapSudoError(raw) : raw);
        }
      } finally {
        setSaving(false);
      }
    },
    [dirty, hostId, remotePath, saving, sudoMode, sudoPassword, mapSudoError],
  );

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
    ensureExtendedLanguages(monaco);
    ensureBashCompletion(monaco);

    // 内容/路径嗅探：conf 族或无扩展名文件实为 nginx 时切 nginx（/nginx/ 路径
    // 直判 + 顶层块/指令形态嗅探，见 languageMap.ts）
    const model = monaco.editor.createModel(
      initial,
      resolveLanguageIdWithContent(fileName, initial, remotePath),
    );
    registerBashHostModel(model, hostId);
    const editor = monaco.editor.create(container, {
      ...PREVIEW_EDITOR_OPTIONS,
      model,
      readOnly: true,
    });
    editorRef.current = editor;

    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
      saveRef.current();
    });
    // 跨平台查找 (Ctrl+F / ⌘F) 与替换 (Ctrl+R / ⌘R / Ctrl+H)
    attachFindReplaceCommands(editor);
    // HTML 键入辅助（编辑态可用；只读时无键入不触发）
    attachHtmlAutoInsert(editor);
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

  /* —— 视图切换或进入编辑态时触发 Monaco 自适应重新排版 —— */
  useEffect(() => {
    if (viewMode === 'source' || editing) {
      requestAnimationFrame(() => {
        editorRef.current?.layout();
      });
    }
  }, [viewMode, editing]);

  const switchToEdit = useCallback(() => {
    setPendingDiscard(null);
    setViewMode('source');
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
            {sudoMode ? (
              <span className="shrink-0 rounded-full bg-warning/15 px-2 py-0.5 font-mono text-ui-xs text-warning select-none">
                sudo
              </span>
            ) : null}
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
            {/* 只读模式下的多视图切换器（专用结构视图 / Monaco 源码 / Hex 十六进制） */}
            {!editing && !isBinaryPreview && (baseline !== null || isBinaryFile || isLog) ? (
              <div className="flex items-center rounded-lg border border-border/70 bg-surface/50 p-0.5 text-ui-xs mr-1">
                {isCert ? (
                  <button
                    type="button"
                    onClick={() => setViewMode('cert')}
                    className={cn(
                      'flex items-center gap-1 rounded-md px-2 py-0.5 transition-colors',
                      viewMode === 'cert'
                        ? 'bg-selected text-foreground font-medium shadow-xs'
                        : 'text-foreground-subtle hover:text-foreground',
                    )}
                    title={t('files.previewCert')}
                  >
                    <ShieldCheck className="size-3 text-terminal-blue" />
                    <span>{t('files.previewCert')}</span>
                  </button>
                ) : isLog ? (
                  <button
                    type="button"
                    onClick={() => setViewMode('log')}
                    className={cn(
                      'flex items-center gap-1 rounded-md px-2 py-0.5 transition-colors',
                      viewMode === 'log'
                        ? 'bg-selected text-foreground font-medium shadow-xs'
                        : 'text-foreground-subtle hover:text-foreground',
                    )}
                    title={t('files.previewLog')}
                  >
                    <ScrollText className="size-3 text-terminal-green" />
                    <span>{t('files.previewLog')}</span>
                  </button>
                ) : hasCustomPreview && !isBinaryFile ? (
                  <button
                    type="button"
                    onClick={() => setViewMode('preview')}
                    className={cn(
                      'flex items-center gap-1 rounded-md px-2 py-0.5 transition-colors',
                      viewMode === 'preview'
                        ? 'bg-selected text-foreground font-medium shadow-xs'
                        : 'text-foreground-subtle hover:text-foreground',
                    )}
                    title={
                      isCsv
                        ? t('files.previewTable')
                        : isJson
                        ? t('files.previewJson')
                        : t('files.previewRendered')
                    }
                  >
                    {isCsv ? (
                      <Table className="size-3" />
                    ) : isJson ? (
                      <FileJson className="size-3" />
                    ) : (
                      <BookOpen className="size-3" />
                    )}
                    <span>
                      {isCsv
                        ? t('files.previewTable')
                        : isJson
                        ? t('files.previewJson')
                        : t('files.previewRendered')}
                    </span>
                  </button>
                ) : null}

                {!isBinaryFile ? (
                  <button
                    type="button"
                    onClick={() => setViewMode('source')}
                    className={cn(
                      'flex items-center gap-1 rounded-md px-2 py-0.5 transition-colors',
                      viewMode === 'source'
                        ? 'bg-selected text-foreground font-medium shadow-xs'
                        : 'text-foreground-subtle hover:text-foreground',
                    )}
                    title={t('files.previewSource')}
                  >
                    <Code2 className="size-3" />
                    <span>{t('files.previewSource')}</span>
                  </button>
                ) : null}

                <button
                  type="button"
                  onClick={() => setViewMode('hex')}
                  className={cn(
                    'flex items-center gap-1 rounded-md px-2 py-0.5 transition-colors',
                    viewMode === 'hex'
                      ? 'bg-selected text-foreground font-medium shadow-xs'
                      : 'text-foreground-subtle hover:text-foreground',
                  )}
                  title={t('files.previewHex')}
                >
                  <Binary className="size-3" />
                  <span>{t('files.previewHex')}</span>
                </button>
              </div>
            ) : null}

            {isBinaryPreview || isBinaryFile ? null : !editing ? (
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
                    <Spinner className="size-3" />
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
          {isPdf && sessionId ? (
            // PDF：后端 PDFium 渲染成图，不走 Monaco（下载 → 渲染 → 翻页全链路
            // 自带进度与 Loading 反馈，见 PdfRemotePreview）
            <Suspense fallback={null}>
              <PdfRemotePreview sessionId={sessionId} remotePath={remotePath} />
            </Suspense>
          ) : isImage && sessionId ? (
            // 图片：下载 → 原生直传或后端转码 → <img>（缩放与滚动见 ImagePreview）
            <Suspense fallback={null}>
              <ImageRemotePreview sessionId={sessionId} remotePath={remotePath} />
            </Suspense>
          ) : isExcel && sessionId ? (
            // Excel：后端 calamine 解析，多 Sheet 切换 + 虚拟滚动表格（见 ExcelRemotePreview）
            <Suspense fallback={null}>
              <ExcelRemotePreview sessionId={sessionId} remotePath={remotePath} fileName={fileName} />
            </Suspense>
          ) : isSqlite && sessionId ? (
            // SQLite：只读模式打开，表清单 + 虚拟滚动表格 + 只读 SQL 查询（见 SqliteRemotePreview）
            <Suspense fallback={null}>
              <SqliteRemotePreview sessionId={sessionId} remotePath={remotePath} fileName={fileName} />
            </Suspense>
          ) : (
            <>
              {isMarkdown && viewMode === 'preview' && !editing && baseline !== null ? (
                <MarkdownPreview content={baseline} />
              ) : null}
              {isCsv && viewMode === 'preview' && !editing && baseline !== null ? (
                <CsvPreview content={baseline} fileName={fileName} />
              ) : null}
              {isJson && viewMode === 'preview' && !editing && baseline !== null ? (
                <JsonPreview content={baseline} fileName={fileName} />
              ) : null}
              {viewMode === 'cert' && !editing && baseline !== null ? (
                <CertViewer content={baseline} />
              ) : null}
              {viewMode === 'log' && !editing ? (
                <LogViewer
                  initialContent={baseline ?? undefined}
                  hostId={hostId}
                  remotePath={remotePath}
                />
              ) : null}
              {viewMode === 'hex' && !editing ? (
                <HexPreview
                  initialContent={baseline ?? undefined}
                  hostId={hostId}
                  remotePath={remotePath}
                />
              ) : null}
              <div
                ref={containerRef}
                onKeyDownCapture={(e) => handleEditorKeyDownCapture(e, editorRef.current)}
                className={cn(
                  'h-full w-full',
                  (hasCustomPreview && viewMode === 'preview') ||
                    viewMode === 'hex' ||
                    viewMode === 'log' ||
                    viewMode === 'cert'
                    ? 'hidden'
                    : 'block',
                )}
              />
              {baseline === null && !loadError && !isBinaryFile && !isLog ? (
                <div className="absolute inset-0 flex items-center justify-center bg-popover">
                  <Spinner className="size-5 text-foreground-subtle" />
                </div>
              ) : null}
            </>
          )}
          {!isBinaryPreview && loadError ? (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-2.5 bg-popover px-6 text-center">
              <AlertCircle className="size-6 text-warning" />
              <p className="max-w-full break-all text-ui-sm text-foreground-subtle">{loadError}</p>
              {/* 权限拒绝 → sudo 提权读取入口：NOPASSWD/缓存/主机凭证先行，需要密码时内联输入 */}
              {isPermissionDenied(loadError) ? (
                sudoBusy ? (
                  <Spinner className="size-5 text-foreground-subtle" />
                ) : showSudoPrompt ? (
                  renderSudoPrompt(() => {
                    const pw = sudoInput.trim() || null;
                    setSudoInput('');
                    void handleSudoRead(pw);
                  })
                ) : (
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    onClick={() => void handleSudoRead(null)}
                  >
                    {t('files.sudoRead')}
                  </Button>
                )
              ) : null}
            </div>
          ) : null}
        </div>

        {saveError != null || pendingDiscard != null || (sudoMode && showSudoPrompt) ? (
          <div className="flex shrink-0 flex-col gap-0 border-t border-border/60">
            {saveError != null ? (
              <p className="flex items-center gap-1.5 px-4 py-1.5 text-ui-xs text-warning">
                <AlertCircle className="size-3.5 shrink-0" />
                <span className="break-all">{t('files.previewSaveFailed', { error: saveError })}</span>
              </p>
            ) : null}
            {/* sudo 保存时密码失效（远端改密）：就地重输 */}
            {sudoMode && showSudoPrompt ? (
              <div className="flex justify-center px-4 py-1.5">
                {renderSudoPrompt(() => {
                  const pw = sudoInput.trim() || null;
                  setSudoInput('');
                  void save(pw);
                })}
              </div>
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
