import { AlertCircle, DownloadCloud, RefreshCw } from 'lucide-react';
import { memo, useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { downloadRemoteFile, type TransferHandle, type TransferProgress } from '@/services/sftpService';
import { JarPreview } from '@/components/files/JarPreview';
import { cn } from '@/lib/utils';

export interface JarRemotePreviewProps {
  sessionId: string;
  remotePath: string;
  fileName: string;
  className?: string;
}

export const JarRemotePreview = memo(function JarRemotePreview({
  sessionId,
  remotePath,
  fileName,
  className,
}: JarRemotePreviewProps) {
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
      temp = await invoke<string>('pdf_temp_path', { fileName, kind: 'jar' });
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
      setDownloading(false);
    } catch (err) {
      if (disposedRef.current) return;
      setDownloadError(err instanceof Error ? err.message : String(err));
      setDownloading(false);
    }
  }, [fileName, remotePath, sessionId]);

  useEffect(() => {
    disposedRef.current = false;
    void startDownload();

    return () => {
      disposedRef.current = true;
      if (handleRef.current) {
        handleRef.current.cancel();
        handleRef.current = null;
      }
      const p = localPathRef.current;
      if (p) {
        void invoke('clean_temp_file', { path: p }).catch(() => {});
        localPathRef.current = null;
      }
    };
  }, [startDownload]);

  if (downloading) {
    const pct = downloadProgress ? Math.min(100, Math.round(downloadProgress.percent)) : null;

    return (
      <div className={cn('flex flex-col items-center justify-center h-full gap-3 text-foreground-subtle', className)}>
        <Spinner className="size-6 text-foreground-subtle" />
        <div className="flex items-center gap-2 text-ui-base font-medium">
          <DownloadCloud className="size-4 animate-bounce text-foreground-subtle" />
          <span>{t('files.decompiler.downloadingArchive', { defaultValue: '正在下载远端归档文件…' })}</span>
        </div>
        {pct !== null ? (
          <div className="flex flex-col items-center gap-1.5 w-60">
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface">
              <div
                className="h-full rounded-full bg-foreground-subtle transition-all duration-150"
                style={{ width: `${pct}%` }}
              />
            </div>
            <span className="font-mono text-ui-xs text-foreground-subtlest">{pct}%</span>
          </div>
        ) : null}
      </div>
    );
  }

  if (downloadError) {
    return (
      <div className={cn('flex flex-col items-center justify-center h-full gap-3 p-6 text-center', className)}>
        <div className="flex size-10 items-center justify-center rounded-full bg-destructive/10 text-destructive">
          <AlertCircle className="size-5" />
        </div>
        <p className="text-ui-base font-medium text-foreground">{t('files.previewUnavailable', { defaultValue: '远程内容不可用' })}</p>
        <p className="max-w-md font-mono text-ui-xs text-destructive">{downloadError}</p>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => void startDownload()}
          className="mt-2 gap-1.5"
        >
          <RefreshCw className="size-3.5" />
          <span>{t('common.retry', { defaultValue: '重试' })}</span>
        </Button>
      </div>
    );
  }

  if (!localPath) return null;

  return <JarPreview filePath={localPath} fileName={fileName} className={className} />;
});
