import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Loader2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ImagePreview } from '@/components/files/ImagePreview';
import { downloadRemoteFile, type TransferProgress } from '@/services/sftpService';

export interface ImageRemotePreviewProps {
  /** SFTP 会话 id（下载远端文件用） */
  sessionId: string;
  /** 远端图片绝对路径 */
  remotePath: string;
}

type Stage = 'downloading' | 'ready' | 'error';

/**
 * 远端图片预览：先下载到本地临时目录（带进度），再交给 ImagePreview 展示。
 *
 * 与 PdfRemotePreview 同构（下载 → 渲染组件），差别只在于没有分页与渲染步骤。
 * 生命周期：下载的源文件与（可能的）转码 PNG 都登记在后端临时文件表，
 * 组件卸载时统一释放。
 */
export function ImageRemotePreview({ sessionId, remotePath }: ImageRemotePreviewProps) {
  const { t } = useTranslation();
  const [stage, setStage] = useState<Stage>('downloading');
  const [progress, setProgress] = useState<TransferProgress | null>(null);
  const [localPath, setLocalPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const localPathRef = useRef<string | null>(null);
  /** 后端转码产生的 PNG（卸载时一并删除） */
  const convertedFilesRef = useRef<Set<string>>(new Set());
  const disposedRef = useRef(false);

  /** ImagePreview 转码完成回调：收集路径供卸载时释放 */
  const handleConvertedFile = useCallback((path: string) => {
    convertedFilesRef.current.add(path);
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
    const baseName = remotePath.split('/').pop() ?? 'image';
    const local = await invoke<string>('pdf_temp_path', {
      fileName: `${hash.toString(16)}_${baseName}`,
      kind: 'img',
    });
    localPathRef.current = local;

    try {
      const handle = await downloadRemoteFile(sessionId, remotePath, local, (p) => {
        if (disposedRef.current) return;
        setProgress(p);
      });
      await handle.done;
      if (disposedRef.current) return;
      setLocalPath(local);
      setStage('ready');
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
      const paths: string[] = [];
      if (localPathRef.current) paths.push(localPathRef.current);
      paths.push(...convertedFilesRef.current);
      void invoke('release_pdf_temp_files', { paths }).catch(() => undefined);
    };
  }, [startDownload]);

  if (stage === 'downloading') {
    const percent = progress?.percent ?? 0;
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3">
        <Loader2 className="size-5 animate-spin text-foreground-subtle" />
        <span className="text-ui-sm text-foreground-subtle">
          {t('image.downloading', {
            percent: percent.toFixed(0),
            size: progress ? `${progress.transferred} / ${progress.total}` : '',
          })}
        </span>
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

  // 下载完成：交给 ImagePreview（它内部处理原生直传/后端转码、缩放与滚动）
  return localPath ? (
    <div className="flex h-full min-h-0 flex-col">
      <ImagePreview imagePath={localPath} onConvertedFile={handleConvertedFile} />
    </div>
  ) : null;
}
