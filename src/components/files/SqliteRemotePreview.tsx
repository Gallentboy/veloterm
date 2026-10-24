import { AlertCircle, DownloadCloud, RefreshCw } from 'lucide-react';
import { memo, useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { downloadRemoteFile, type TransferHandle, type TransferProgress } from '@/services/sftpService';
import { SqlitePreview } from '@/components/files/SqlitePreview';
import { cn } from '@/lib/utils';

export interface SqliteRemotePreviewProps {
  sessionId: string;
  remotePath: string;
  fileName: string;
  className?: string;
}

export const SqliteRemotePreview = memo(function SqliteRemotePreview({
  sessionId,
  remotePath,
  fileName,
  className,
}: SqliteRemotePreviewProps) {
  const { t } = useTranslation();

  const [downloading, setDownloading] = useState(true);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<TransferProgress | null>(null);
  const [localPath, setLocalPath] = useState<string | null>(null);

  const localPathRef = useRef<string | null>(null);
  const handleRef = useRef<TransferHandle | null>(null);
  const disposedRef = useRef(false);

  const startDownload = useCallback(async () => {
    setDownloading(true);
    setDownloadError(null);
    setDownloadProgress(null);
    setLocalPath(null);

    let temp: string;
    try {
      temp = await invoke<string>('pdf_temp_path', { fileName, kind: 'sqlite' });
      localPathRef.current = temp;
    } catch (err) {
      setDownloadError(err instanceof Error ? err.message : String(err));
      setDownloading(false);
      return;
    }

    try {
      const handle = await downloadRemoteFile(
        sessionId,
        remotePath,
        temp,
        (p) => {
          if (disposedRef.current) return;
          setDownloadProgress(p);
        },
      );
      handleRef.current = handle;
      await handle.done;
      if (disposedRef.current) return;
      setLocalPath(temp);
    } catch (err) {
      if (disposedRef.current) return;
      const msg = err instanceof Error ? err.message : String(err);
      if (msg.includes('canceled') || msg.includes('cancelled')) {
        return;
      }
      setDownloadError(msg);
    } finally {
      handleRef.current = null;
      if (!disposedRef.current) {
        setDownloading(false);
      }
    }
  }, [sessionId, remotePath, fileName]);

  useEffect(() => {
    disposedRef.current = false;
    void startDownload();
    return () => {
      disposedRef.current = true;
      handleRef.current?.cancel();
      const path = localPathRef.current;
      if (path) {
        invoke('release_pdf_temp_files', { paths: [path] }).catch((e) =>
          console.warn('Failed to release sqlite temp file', e),
        );
      }
    };
  }, [startDownload]);

  if (downloading) {
    const percent = downloadProgress?.percent ?? 0;

    return (
      <div className={cn('flex h-full w-full flex-col items-center justify-center gap-3 bg-popover p-6 text-center select-none', className)}>
        <div className="flex size-12 items-center justify-center rounded-2xl bg-surface/70 text-terminal-cyan shadow-sm">
          <DownloadCloud className="size-6 animate-pulse" />
        </div>
        <div className="flex flex-col gap-1">
          <span className="text-ui-sm font-medium text-foreground">
            正在下载 SQLite 数据库文件... {downloadProgress ? `${percent.toFixed(0)}%` : ''}
          </span>
          <span className="font-mono text-ui-xs text-foreground-subtle">
            {downloadProgress ? `${downloadProgress.transferred} / ${downloadProgress.total}` : '准备中...'}
          </span>
        </div>

        {downloadProgress ? (
          <div className="h-1.5 w-64 overflow-hidden rounded-full bg-surface">
            <div
              className="h-full rounded-full bg-primary transition-all duration-150"
              style={{ width: `${Math.min(100, Math.max(0, percent))}%` }}
            />
          </div>
        ) : (
          <Spinner className="size-4 text-foreground-subtle" />
        )}
      </div>
    );
  }

  if (downloadError) {
    return (
      <div className={cn('flex h-full w-full flex-col items-center justify-center gap-2.5 bg-popover p-6 text-center select-none', className)}>
        <AlertCircle className="size-6 text-warning" />
        <span className="text-ui-sm font-semibold text-foreground">下载 SQLite 数据库失败</span>
        <p className="max-w-md font-mono text-ui-xs text-foreground-subtle break-all">{downloadError}</p>
        <Button type="button" size="sm" variant="outline" onClick={() => void startDownload()}>
          <RefreshCw className="size-3 mr-1" />
          <span>{t('pdf.retry', '重试')}</span>
        </Button>
      </div>
    );
  }

  if (localPath) {
    return <SqlitePreview filePath={localPath} className={className} />;
  }

  return null;
});
