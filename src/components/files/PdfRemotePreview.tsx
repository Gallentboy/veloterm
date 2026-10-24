import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { PdfPreview } from '@/components/files/PdfPreview';
import { downloadRemoteFile, type TransferProgress } from '@/services/sftpService';
import { Spinner } from '@/components/ui/spinner';

export interface PdfRemotePreviewProps {
  /** SFTP 会话 id（下载远端文件用） */
  sessionId: string;
  /** 远端 PDF 绝对路径 */
  remotePath: string;
}

/** 下载阶段（解析前）与渲染阶段的区分 */
type Stage = 'downloading' | 'parsing' | 'ready' | 'error';

/**
 * 远端 PDF 预览：先下载到本地临时目录（带进度），再交给 PdfPreview 渲染。
 *
 * 生命周期：
 * - 下载的文件与渲染出的 PNG 都登记在后端临时文件表
 * - 组件卸载时调 `release_pdf_temp_files` 删除本次涉及的文件
 * - 应用退出时后端兜底清理全部登记项
 */
export function PdfRemotePreview({ sessionId, remotePath }: PdfRemotePreviewProps) {
  const { t } = useTranslation();
  const [stage, setStage] = useState<Stage>('downloading');
  const [progress, setProgress] = useState<TransferProgress | null>(null);
  const [localPath, setLocalPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  /** 本次下载产生的临时文件（卸载时删除） */
  const localPathRef = useRef<string | null>(null);
  /** 渲染出的各页 PNG 临时文件（卸载时一并删除，否则只能等应用退出清理） */
  const renderedFilesRef = useRef<Set<string>>(new Set());
  /** 组件已卸载：丢弃在途下载的进度更新 */
  const disposedRef = useRef(false);

  /** PdfPreview 每渲染一页回调一次：收集路径供卸载时释放 */
  const handleRenderedFile = useCallback((path: string) => {
    renderedFilesRef.current.add(path);
  }, []);

  const startDownload = useCallback(async () => {
    setStage('downloading');
    setProgress(null);
    setError(null);

    // 本地临时路径：文件名带远程路径短哈希，避免同名不同目录互相覆盖
    const hash = Array.from(remotePath).reduce(
      (acc, ch) => ((acc * 31 + ch.charCodeAt(0)) >>> 0),
      0,
    );
    const baseName = remotePath.split('/').pop() ?? 'document.pdf';
    // 用 Tauri 的临时目录（与后端 std::env::temp_dir() 一致）
    const local = await invoke<string>('pdf_temp_path', {
      fileName: `${hash.toString(16)}_${baseName}`,
    });
    localPathRef.current = local;

    try {
      const handle = await downloadRemoteFile(
        sessionId,
        remotePath,
        local,
        (p) => {
          if (disposedRef.current) return;
          setProgress(p);
        },
      );
      await handle.done;
      if (disposedRef.current) return;
      // 下载完成 → 进入解析/渲染阶段（PdfPreview 内部渲染第一页）
      setStage('parsing');
      setLocalPath(local);
    } catch (e) {
      if (disposedRef.current) return;
      setError(e instanceof Error ? e.message : String(e));
      setStage('error');
    }
  }, [sessionId, remotePath]);

  useEffect(() => {
    disposedRef.current = false;
    void startDownload();
    return () => {
      disposedRef.current = true;
      // 卸载即释放本次全部临时文件：下载的源 PDF + 渲染出的各页 PNG
      const paths: string[] = [];
      if (localPathRef.current) paths.push(localPathRef.current);
      paths.push(...renderedFilesRef.current);
      void invoke('release_pdf_temp_files', { paths }).catch(() => undefined);
    };
  }, [startDownload]);

  // PdfPreview 渲染首帧后（stage 变 ready 由它内部处理），这里只负责下载阶段 UI
  if (stage === 'downloading') {
    const percent = progress?.percent ?? 0;
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3">
        <Spinner className="size-5 text-foreground-subtle" />
        <span className="text-ui-sm text-foreground-subtle">
          {t('pdf.downloading', {
            percent: percent.toFixed(0),
            size: progress ? `${progress.transferred} / ${progress.total}` : '',
          })}
        </span>
        {/* 进度条：下载大文件时给明确反馈，不让用户死等 */}
        <div className="h-1 w-56 overflow-hidden rounded-full bg-border/60">
          <div
            className="h-full rounded-full bg-primary transition-[width] duration-150"
            style={{ width: `${Math.min(100, Math.max(0, percent))}%` }}
          />
        </div>
      </div>
    );
  }

  if (stage === 'error') {
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2">
        <span className="max-w-md text-center text-ui-sm text-destructive">{error}</span>
        <button
          type="button"
          className="rounded-md border border-border px-2.5 py-1 text-ui-xs text-foreground-subtle hover:border-border-hover"
          onClick={() => void startDownload()}
        >
          {t('pdf.retry')}
        </button>
      </div>
    );
  }

  // parsing / ready：交给 PdfPreview（它内部有首帧 Loading 与翻页反馈）。
  // 必须包一层 h-full：PdfPreview 根元素是 flex-1，需要父级是定高 flex 容器
  // 才能拿到受限高度——否则高度随内容撑开，内部 overflow-auto 永不触发滚动条。
  return localPath ? (
    <div className="flex h-full min-h-0 flex-col">
      <PdfPreview pdfPath={localPath} onRenderedFile={handleRenderedFile} />
    </div>
  ) : null;
}
