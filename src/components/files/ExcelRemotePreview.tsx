import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Loader2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { cn } from '@/lib/utils';
import { ExcelPreview } from '@/components/files/ExcelPreview';
import { downloadRemoteFile, type TransferHandle, type TransferProgress } from '@/services/sftpService';

export interface ExcelRemotePreviewProps {
  /** SFTP 会话 id（下载远端文件用） */
  sessionId: string;
  /** 远端 Excel 绝对路径 */
  remotePath: string;
  /** 文件名（含后缀，如 data.xlsx） */
  fileName: string;
  className?: string;
}

/** 下载阶段与就绪渲染阶段区分 */
type Stage = 'downloading' | 'ready' | 'error';

/**
 * 远端 Excel 预览：先下载到本地临时目录（带进度与取消），再交给 ExcelPreview 渲染。
 *
 * 生命周期：
 * - 下载的源文件登记在后端临时文件表 (pdf_temp_path)
 * - 组件卸载时调 release_pdf_temp_files 删除本次下载的临时文件并取消在途传输
 * - 应用退出时后端兜底清理全部登记项
 */
export function ExcelRemotePreview({
  sessionId,
  remotePath,
  fileName,
  className,
}: ExcelRemotePreviewProps) {
  const { t } = useTranslation();
  const [stage, setStage] = useState<Stage>('downloading');
  const [progress, setProgress] = useState<TransferProgress | null>(null);
  const [localPath, setLocalPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  /** 本次下载产生的临时文件（卸载时删除） */
  const localPathRef = useRef<string | null>(null);
  /** 在途下载传输句柄（用于取消） */
  const handleRef = useRef<TransferHandle | null>(null);
  /** 用户主动取消标记 */
  const cancelledRef = useRef(false);
  /** 组件已卸载标记：丢弃后续进度更新 */
  const disposedRef = useRef(false);

  const startDownload = useCallback(async () => {
    setStage('downloading');
    setProgress(null);
    setError(null);
    cancelledRef.current = false;

    // 本地临时路径：文件名带远程路径短哈希，避免同名不同目录互相覆盖
    const hash = Array.from(remotePath).reduce(
      (acc, ch) => ((acc * 31 + ch.charCodeAt(0)) >>> 0),
      0,
    );
    const baseName = fileName || remotePath.split('/').pop() || 'workbook.xlsx';
    const local = await invoke<string>('pdf_temp_path', {
      fileName: `${hash.toString(16)}_${baseName}`,
      kind: 'excel',
    });
    localPathRef.current = local;

    try {
      const handle = await downloadRemoteFile(
        sessionId,
        remotePath,
        local,
        (p) => {
          if (disposedRef.current || cancelledRef.current) return;
          setProgress(p);
        },
      );
      handleRef.current = handle;
      await handle.done;
      if (disposedRef.current || cancelledRef.current) return;
      setLocalPath(local);
      setStage('ready');
    } catch (e) {
      if (disposedRef.current || cancelledRef.current) return;
      setError(e instanceof Error ? e.message : String(e));
      setStage('error');
    } finally {
      handleRef.current = null;
    }
  }, [sessionId, remotePath, fileName]);

  const handleCancel = useCallback(() => {
    cancelledRef.current = true;
    if (handleRef.current) {
      handleRef.current.cancel();
    }
    setError(t('excel.downloadCancelled', '下载已取消'));
    setStage('error');
  }, [t]);

  useEffect(() => {
    disposedRef.current = false;
    void startDownload();
    return () => {
      disposedRef.current = true;
      if (handleRef.current) {
        handleRef.current.cancel();
      }
      if (localPathRef.current) {
        void invoke('release_pdf_temp_files', {
          paths: [localPathRef.current],
        }).catch(() => undefined);
      }
    };
  }, [startDownload]);

  if (stage === 'downloading') {
    const percent = progress?.percent ?? 0;
    return (
      <div className={cn('flex min-h-0 flex-1 flex-col items-center justify-center gap-3', className)}>
        <Loader2 className="size-5 animate-spin text-foreground-subtle" />
        <span className="text-ui-sm text-foreground-subtle">
          {t('excel.downloading', '正在下载表格... {{percent}}% ({{size}})', {
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
        <button
          type="button"
          onClick={handleCancel}
          className="mt-1 rounded-md border border-border px-2.5 py-1 text-ui-xs text-foreground-subtle hover:border-border-hover hover:text-foreground"
        >
          {t('common.cancel', '取消')}
        </button>
      </div>
    );
  }

  if (stage === 'error') {
    return (
      <div className={cn('flex min-h-0 flex-1 flex-col items-center justify-center gap-2', className)}>
        <span className="max-w-md text-center text-ui-sm text-destructive">{error}</span>
        <button
          type="button"
          className="rounded-md border border-border px-2.5 py-1 text-ui-xs text-foreground-subtle hover:border-border-hover"
          onClick={() => void startDownload()}
        >
          {t('pdf.retry', '重试')}
        </button>
      </div>
    );
  }

  return localPath ? (
    <div className={cn('flex h-full min-h-0 flex-col', className)}>
      <ExcelPreview filePath={localPath} />
    </div>
  ) : null;
}
