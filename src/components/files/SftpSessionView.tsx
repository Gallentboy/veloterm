import {
  AlertCircle,
  ArrowUp,
  Download,
  File,
  Folder,
  FolderOpen,
  Loader2,
  RefreshCw,
  Trash2,
  Upload,
  X,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { Button } from '@/components/ui/button';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Tooltip } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';
import {
  downloadRemoteFile,
  listRemoteDirectory,
  pickDownloadPath,
  pickUploadFile,
  removeRemoteItem,
  uploadLocalFile,
  type RemoteFileItem,
  type TransferProgress,
} from '@/services/sftpService';

interface ActiveTransfer {
  taskId: number;
  type: 'upload' | 'download';
  name: string;
  transferred: string;
  total: string;
  percent: number;
  speedBps: number;
  isFinished: boolean;
  cancel: () => void;
}

function formatBytes(bytesStr: string): string {
  const bytes = Number(bytesStr);
  if (!Number.isFinite(bytes) || bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  const val = bytes / Math.pow(k, i);
  return `${val.toFixed(i === 0 ? 0 : 1)} ${sizes[i]}`;
}

function formatSpeed(speedBps: number): string {
  if (speedBps <= 0) return '0 B/s';
  const k = 1024;
  const sizes = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
  const i = Math.floor(Math.log(speedBps) / Math.log(k));
  const val = speedBps / Math.pow(k, i);
  return `${val.toFixed(1)} ${sizes[i]}`;
}

function formatPermissions(mode: number, isDir?: boolean, isSymlink?: boolean): string {
  if (!mode && !isDir && !isSymlink) return '----------';
  const typeChar = isDir ? 'd' : isSymlink ? 'l' : '-';
  const r1 = mode & 0o400 ? 'r' : '-';
  const w1 = mode & 0o200 ? 'w' : '-';
  const x1 = mode & 0o4000 ? (mode & 0o100 ? 's' : 'S') : mode & 0o100 ? 'x' : '-';

  const r2 = mode & 0o040 ? 'r' : '-';
  const w2 = mode & 0o020 ? 'w' : '-';
  const x2 = mode & 0o2000 ? (mode & 0o010 ? 's' : 'S') : mode & 0o010 ? 'x' : '-';

  const r3 = mode & 0o004 ? 'r' : '-';
  const w3 = mode & 0o002 ? 'w' : '-';
  const x3 = mode & 0o1000 ? (mode & 0o001 ? 't' : 'T') : mode & 0o001 ? 'x' : '-';

  return `${typeChar}${r1}${w1}${x1}${r2}${w2}${x2}${r3}${w3}${x3}`;
}

const ARCHIVE_EXTS = new Set([
  'tar', 'gz', 'tgz', 'zip', 'zst', 'bz2', 'xz', 'rar', '7z', 'iso', 'deb', 'rpm', 'apk', 'jar', 'war',
]);
const MEDIA_EXTS = new Set([
  'png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'ico', 'mp4', 'mkv', 'avi', 'mov', 'webm', 'mp3', 'wav', 'flac', 'ogg',
]);
const CODE_CONFIG_EXTS = new Set([
  'json', 'yaml', 'yml', 'toml', 'xml', 'conf', 'cfg', 'ini', 'env', 'sh', 'bash', 'zsh', 'fish', 'py', 'rs', 'ts', 'js', 'go', 'c', 'cpp', 'h',
]);

function getFileItemVisuals(item: RemoteFileItem) {
  if (item.is_dir) {
    return {
      colorClass: 'text-blue-500 dark:text-blue-400 font-semibold',
      iconClass: 'text-blue-500 dark:text-blue-400',
    };
  }
  if (item.is_symlink) {
    return {
      colorClass: 'text-cyan-500 dark:text-cyan-400 font-normal',
      iconClass: 'text-cyan-500 dark:text-cyan-400',
    };
  }
  if ((item.permissions & 0o111) !== 0) {
    return {
      colorClass: 'text-emerald-500 dark:text-emerald-400 font-medium',
      iconClass: 'text-emerald-500 dark:text-emerald-400',
    };
  }

  const parts = item.name.split('.');
  const ext = parts.length > 1 ? parts.pop()?.toLowerCase() ?? '' : '';

  if (ARCHIVE_EXTS.has(ext)) {
    return {
      colorClass: 'text-red-500 dark:text-red-400 font-normal',
      iconClass: 'text-red-500 dark:text-red-400',
    };
  }
  if (MEDIA_EXTS.has(ext)) {
    return {
      colorClass: 'text-purple-500 dark:text-purple-400 font-normal',
      iconClass: 'text-purple-500 dark:text-purple-400',
    };
  }
  if (CODE_CONFIG_EXTS.has(ext)) {
    return {
      colorClass: 'text-amber-500 dark:text-amber-400 font-normal',
      iconClass: 'text-amber-500 dark:text-amber-400',
    };
  }

  return {
    colorClass: 'text-foreground font-normal',
    iconClass: 'text-foreground-subtle',
  };
}

function formatModifiedTime(timestampSec: number): string {
  if (!timestampSec) return '-';
  const d = new Date(timestampSec * 1000);
  return d.toLocaleString(undefined, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

export interface SftpSessionViewProps {
  sessionId: string;
  hostId: number;
  initialPath: string;
  isVisible: boolean;
  onPathChange?: (path: string) => void;
}

export function SftpSessionView({
  sessionId,
  hostId: _hostId,
  initialPath,
  isVisible,
  onPathChange,
}: SftpSessionViewProps) {
  const { t } = useTranslation();
  const [currentPath, setCurrentPath] = useState<string>(initialPath || '/');
  const [pathInput, setPathInput] = useState<string>(initialPath || '/');
  const [items, setItems] = useState<RemoteFileItem[]>([]);
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [activeTransfers, setActiveTransfers] = useState<Record<number, ActiveTransfer>>({});
  const [itemToDelete, setItemToDelete] = useState<RemoteFileItem | null>(null);
  const [isDeleting, setIsDeleting] = useState<boolean>(false);

  // 刷新当前路径目录
  const loadDirectory = useCallback(
    async (path: string) => {
      if (!sessionId) return;
      setIsLoading(true);
      setError(null);
      try {
        const files = await listRemoteDirectory(sessionId, path);
        setItems(files);
        setCurrentPath(path);
        setPathInput(path);
        onPathChange?.(path);
      } catch (err) {
        setError(String(err));
      } finally {
        setIsLoading(false);
      }
    },
    [sessionId, onPathChange],
  );

  // 初次加载挂载
  useEffect(() => {
    void loadDirectory(initialPath || '/');
  }, [initialPath, loadDirectory]);

  // 面包屑分段
  const breadcrumbs = useMemo(() => {
    if (currentPath === '/' || !currentPath) {
      return [{ name: '/', path: '/' }];
    }
    const parts = currentPath.split('/').filter(Boolean);
    const crumbs = [{ name: '/', path: '/' }];
    let acc = '';
    for (const part of parts) {
      acc += `/${part}`;
      crumbs.push({ name: part, path: acc });
    }
    return crumbs;
  }, [currentPath]);

  const handleNavigate = (targetPath: string) => {
    void loadDirectory(targetPath);
  };

  const handleUpDir = () => {
    if (currentPath === '/' || !currentPath) return;
    const parts = currentPath.split('/').filter(Boolean);
    parts.pop();
    const parentPath = parts.length === 0 ? '/' : `/${parts.join('/')}`;
    void loadDirectory(parentPath);
  };

  const handlePathSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (pathInput.trim()) {
      void loadDirectory(pathInput.trim());
    }
  };

  // 下载文件
  const handleDownload = async (item: RemoteFileItem) => {
    if (!sessionId) return;
    try {
      const localPath = await pickDownloadPath(item.name);
      if (!localPath) return;

      let taskId = 0;
      const handle = await downloadRemoteFile(
        sessionId,
        item.path,
        localPath,
        (progress: TransferProgress) => {
          setActiveTransfers((prev) => {
            const current = prev[progress.task_id];
            if (!current) return prev;
            return {
              ...prev,
              [progress.task_id]: {
                ...current,
                transferred: progress.transferred,
                total: progress.total,
                percent: progress.percent,
                speedBps: progress.speed_bps,
                isFinished: progress.is_finished,
              },
            };
          });

          if (progress.is_finished) {
            setTimeout(() => {
              setActiveTransfers((prev) => {
                const { [progress.task_id]: _done, ...rest } = prev;
                return rest;
              });
            }, 3000);
          }
        },
      );

      taskId = handle.taskId;
      setActiveTransfers((prev) => ({
        ...prev,
        [taskId]: {
          taskId,
          type: 'download',
          name: item.name,
          transferred: '0',
          total: item.size,
          percent: 0,
          speedBps: 0,
          isFinished: false,
          cancel: handle.cancel,
        },
      }));
    } catch (err) {
      setError(`下载失败: ${err}`);
    }
  };

  // 删除确认执行
  const handleDeleteConfirm = async () => {
    if (!itemToDelete || !sessionId) return;
    setIsDeleting(true);
    try {
      await removeRemoteItem(sessionId, itemToDelete.path, itemToDelete.is_dir);
      setItemToDelete(null);
      await loadDirectory(currentPath);
    } catch (err) {
      setError(`删除失败: ${err}`);
    } finally {
      setIsDeleting(false);
    }
  };

  // 批量上传文件
  const handleUploadFiles = useCallback(
    async (localPaths: string[], targetDir = currentPath) => {
      if (!sessionId || localPaths.length === 0) return;
      for (const localPath of localPaths) {
        const filename = localPath.split(/[/\\]/).pop() || 'file';
        const remoteTarget =
          targetDir === '/' ? `/${filename}` : `${targetDir}/${filename}`;

        try {
          const handle = await uploadLocalFile(
            sessionId,
            localPath,
            remoteTarget,
            (progress: TransferProgress) => {
              setActiveTransfers((prev) => {
                const current = prev[progress.task_id];
                if (!current) return prev;
                return {
                  ...prev,
                  [progress.task_id]: {
                    ...current,
                    transferred: progress.transferred,
                    total: progress.total,
                    percent: progress.percent,
                    speedBps: progress.speed_bps,
                    isFinished: progress.is_finished,
                  },
                };
              });

              if (progress.is_finished) {
                void loadDirectory(targetDir);
                setTimeout(() => {
                  setActiveTransfers((prev) => {
                    const { [progress.task_id]: _done, ...rest } = prev;
                    return rest;
                  });
                }, 3000);
              }
            },
          );

          const taskId = handle.taskId;
          setActiveTransfers((prev) => ({
            ...prev,
            [taskId]: {
              taskId,
              type: 'upload',
              name: filename,
              transferred: '0',
              total: '0',
              percent: 0,
              speedBps: 0,
              isFinished: false,
              cancel: handle.cancel,
            },
          }));
        } catch (err) {
          setError(`上传 ${filename} 失败: ${err}`);
        }
      }
    },
    [currentPath, loadDirectory, sessionId],
  );

  // 监听 Tauri 窗口原生拖拽事件（仅当当前会话激活时承接上传）
  useEffect(() => {
    if (!isVisible) return;
    let unlisten: (() => void) | undefined;
    let isSubscribed = true;

    void (async () => {
      try {
        const appWindow = getCurrentWebviewWindow();
        const unlistenFn = await appWindow.onDragDropEvent((event) => {
          if (!isSubscribed) return;
          if (event.payload.type === 'drop') {
            const droppedPaths = event.payload.paths;
            if (droppedPaths && droppedPaths.length > 0) {
              void handleUploadFiles(droppedPaths);
            }
          }
        });
        if (isSubscribed) {
          unlisten = unlistenFn;
        } else {
          unlistenFn();
        }
      } catch (err) {
        console.warn('Failed to register drag-drop event listener:', err);
      }
    })();

    return () => {
      isSubscribed = false;
      unlisten?.();
    };
  }, [handleUploadFiles, isVisible]);

  // 工具栏点击上传文件
  const handleUpload = async () => {
    if (!sessionId) return;
    try {
      const localPath = await pickUploadFile();
      if (!localPath) return;
      await handleUploadFiles([localPath]);
    } catch (err) {
      setError(`上传失败: ${err}`);
    }
  };

  const transfersList = Object.values(activeTransfers);

  return (
    <div className="relative flex h-full min-h-0 flex-col overflow-hidden bg-background">
      {/* 顶部工具栏：返回上级 + 刷新 + 路径输入 + 上传 */}
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-border/50 px-3 py-2">
        <div className="flex items-center gap-1.5">
          {/* 返回上一级 */}
          <ControlHintTooltip title={t('files.tooltips.upDir')} side="bottom">
            <Button
              type="button"
              size="icon-sm"
              variant="ghost"
              onClick={handleUpDir}
              disabled={currentPath === '/' || isLoading}
              className="size-7 rounded-lg text-foreground-subtle hover:text-foreground hover:bg-hover"
              aria-label={t('files.upDir')}
            >
              <ArrowUp className="size-3.5" />
            </Button>
          </ControlHintTooltip>

          {/* 刷新 */}
          <ControlHintTooltip title={t('files.tooltips.refresh')} side="bottom">
            <Button
              type="button"
              size="icon-sm"
              variant="ghost"
              onClick={() => loadDirectory(currentPath)}
              disabled={isLoading}
              className="size-7 rounded-lg text-foreground-subtle hover:text-foreground hover:bg-hover"
              aria-label={t('files.refresh')}
            >
              <RefreshCw className={cn('size-3.5', isLoading && 'animate-spin')} />
            </Button>
          </ControlHintTooltip>
        </div>

        {/* 路径输入框与上传按钮 */}
        <div className="flex flex-1 items-center gap-2 min-w-[200px] justify-end">
          <form onSubmit={handlePathSubmit} className="flex-1 max-w-md">
            <input
              type="text"
              value={pathInput}
              onChange={(e) => setPathInput(e.target.value)}
              placeholder="/"
              className="w-full rounded-md border border-border/60 bg-surface/30 px-2.5 py-1 font-mono text-ui-xs text-foreground outline-none focus:border-primary/60"
            />
          </form>

          {/* ZCode 规范操作按钮与向下避让提示 */}
          <ControlHintTooltip title={t('files.tooltips.upload')} side="bottom" align="end">
            <Button
              type="button"
              variant="outline"
              onClick={handleUpload}
              disabled={isLoading}
              className="h-7 gap-1.5 rounded-lg px-2.5 text-ui-xs font-medium border-border/70 bg-surface/40 hover:bg-hover active:bg-hover/80 text-foreground shrink-0 shadow-2xs transition-colors"
            >
              <Upload className="size-3.5 text-foreground-subtle group-hover/button:text-foreground" />
              <span>{t('files.upload')}</span>
            </Button>
          </ControlHintTooltip>
        </div>
      </div>

      {/* 面包屑导航栏 */}
      <div className="flex shrink-0 items-center gap-1 border-b border-border/40 bg-surface/10 px-3 py-1 overflow-x-auto text-ui-xs text-foreground-subtle">
        {breadcrumbs.map((crumb, idx) => (
          <div key={crumb.path} className="flex items-center gap-1 shrink-0">
            {idx > 0 && <span className="text-foreground-subtlest">/</span>}
            <button
              type="button"
              onClick={() => handleNavigate(crumb.path)}
              className={cn(
                'rounded px-1 py-0.5 hover:bg-surface/60 hover:text-foreground transition-colors',
                crumb.path === currentPath && 'font-semibold text-foreground',
              )}
            >
              {crumb.name}
            </button>
          </div>
        ))}
      </div>

      {/* 错误提示条 */}
      {error && (
        <div className="flex items-center justify-between gap-2 border-b border-destructive/20 bg-destructive/10 px-3 py-1.5 text-ui-xs text-destructive">
          <div className="flex items-center gap-1.5">
            <AlertCircle className="size-4 shrink-0" />
            <span>{error}</span>
          </div>
          <button type="button" onClick={() => setError(null)} className="opacity-70 hover:opacity-100">
            <X className="size-3.5" />
          </button>
        </div>
      )}

      {/* 远程文件列表 */}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {isLoading && items.length === 0 ? (
          <div className="flex h-48 items-center justify-center text-foreground-subtle text-xs gap-2">
            <Loader2 className="size-4 animate-spin text-primary" />
            <span>{t('files.connecting')}</span>
          </div>
        ) : items.length === 0 ? (
          <div className="flex h-48 items-center justify-center text-foreground-subtle text-xs">
            <span>{t('files.emptyDir')}</span>
          </div>
        ) : (
          <table className="w-full border-collapse text-left text-ui-xs">
            <thead className="sticky top-0 bg-background/95 backdrop-blur-xs border-b border-border/50 text-foreground-subtle select-none z-10">
              <tr>
                <th className="py-2 pl-4 pr-2 font-medium">{t('files.name')}</th>
                <th className="py-2 px-2 font-medium w-24">{t('files.size')}</th>
                <th className="py-2 px-2 font-medium w-28">{t('files.permissions')}</th>
                <th className="py-2 px-2 font-medium w-24">{t('files.owner')}</th>
                <th className="py-2 px-2 font-medium w-24">{t('files.group')}</th>
                <th className="py-2 px-2 font-medium w-36">{t('files.modified')}</th>
                <th className="py-2 pl-2 pr-4 font-medium w-28 text-right">{t('files.actions')}</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border/30">
              {items.map((item) => {
                const visuals = getFileItemVisuals(item);
                return (
                  <tr
                    key={item.path}
                    onDoubleClick={() => item.is_dir && handleNavigate(item.path)}
                    className="hover:bg-surface/40 transition-colors cursor-pointer group"
                  >
                    <td className="py-1.5 pl-4 pr-2 font-medium">
                      <div className="flex items-center gap-2">
                        {item.is_dir ? (
                          <Folder className={cn('size-4 shrink-0', visuals.iconClass)} />
                        ) : (
                          <File className={cn('size-4 shrink-0', visuals.iconClass)} />
                        )}
                        <span className={cn('truncate max-w-[260px] md:max-w-md', visuals.colorClass)}>
                          {item.name}
                        </span>
                      </div>
                    </td>
                    <td className="py-1.5 px-2 font-mono text-foreground-subtle">
                      {item.is_dir ? '-' : formatBytes(item.size)}
                    </td>
                    <td className="py-1.5 px-2 font-mono text-foreground-subtle">
                      {formatPermissions(item.permissions, item.is_dir, item.is_symlink)}
                    </td>
                    <td className="py-1.5 px-2 font-mono text-foreground-subtle truncate max-w-[100px]">
                      {item.owner || '-'}
                    </td>
                    <td className="py-1.5 px-2 font-mono text-foreground-subtle truncate max-w-[100px]">
                      {item.group || '-'}
                    </td>
                    <td className="py-1.5 px-2 text-foreground-subtle">
                      {formatModifiedTime(item.modified_at)}
                    </td>
                    <td className="py-1.5 pl-2 pr-4 text-right">
                      <div className="flex items-center justify-end gap-0.5 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 transition-opacity">
                        {item.is_dir ? (
                          <Tooltip content={t('files.tooltips.enterDir')} side="left">
                            <Button
                              type="button"
                              size="icon-xs"
                              variant="ghost"
                              onClick={(e) => {
                                e.stopPropagation();
                                handleNavigate(item.path);
                              }}
                              className="size-6 rounded-md text-foreground-subtle hover:text-foreground hover:bg-hover active:bg-hover/80"
                              aria-label={t('files.enterDir')}
                            >
                              <FolderOpen className="size-3.5" />
                            </Button>
                          </Tooltip>
                        ) : (
                          <Tooltip content={t('files.tooltips.download')} side="left">
                            <Button
                              type="button"
                              size="icon-xs"
                              variant="ghost"
                              onClick={(e) => {
                                e.stopPropagation();
                                void handleDownload(item);
                              }}
                              className="size-6 rounded-md text-foreground-subtle hover:text-foreground hover:bg-hover active:bg-hover/80"
                              aria-label={t('files.download')}
                            >
                              <Download className="size-3.5" />
                            </Button>
                          </Tooltip>
                        )}
                        <Tooltip content={t('files.tooltips.delete')} side="left">
                          <Button
                            type="button"
                            size="icon-xs"
                            variant="ghost"
                            onClick={(e) => {
                              e.stopPropagation();
                              setItemToDelete(item);
                            }}
                            className="size-6 rounded-md text-foreground-subtle hover:text-destructive hover:bg-destructive/10 active:bg-destructive/20"
                            aria-label={t('files.delete')}
                          >
                            <Trash2 className="size-3.5" />
                          </Button>
                        </Tooltip>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      {/* 底部传输任务进度卡片 */}
      {transfersList.length > 0 && (
        <div className="shrink-0 border-t border-border/60 bg-surface/30 p-2.5 space-y-2">
          {transfersList.map((tItem) => (
            <div
              key={tItem.taskId}
              className="flex items-center justify-between gap-3 rounded-md border border-border/50 bg-background/90 px-3 py-1.5 text-ui-xs shadow-xs"
            >
              <div className="flex items-center gap-2 min-w-0">
                {tItem.type === 'upload' ? (
                  <Upload className="size-3.5 text-primary shrink-0" />
                ) : (
                  <Download className="size-3.5 text-success shrink-0" />
                )}
                <span className="font-medium truncate max-w-[140px] md:max-w-[200px]">
                  {tItem.name}
                </span>
                <span className="text-foreground-subtle font-mono text-[11px]">
                  {formatBytes(tItem.transferred)} / {formatBytes(tItem.total)}
                </span>
                {tItem.speedBps > 0 && (
                  <span className="text-foreground-subtlest font-mono text-[10px]">
                    {formatSpeed(tItem.speedBps)}
                  </span>
                )}
              </div>

              <div className="flex items-center gap-3 shrink-0">
                <div className="w-24 bg-surface rounded-full h-1.5 overflow-hidden">
                  <div
                    className={cn(
                      'h-full transition-all duration-150',
                      tItem.isFinished ? 'bg-success' : 'bg-primary',
                    )}
                    style={{ width: `${tItem.percent}%` }}
                  />
                </div>
                <span className="w-10 text-right font-mono text-[11px] font-medium">
                  {tItem.percent.toFixed(0)}%
                </span>
                {!tItem.isFinished ? (
                  <Button
                    type="button"
                    size="icon-xs"
                    variant="ghost"
                    onClick={tItem.cancel}
                    aria-label={t('files.cancelTransfer')}
                  >
                    <X className="size-3 text-destructive" />
                  </Button>
                ) : (
                  <span className="text-success text-[10px]">✓</span>
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      {/* 删除确认弹窗（ZCode 桌面级精致对话框风格） */}
      {itemToDelete && (
        <div
          role="dialog"
          aria-modal="true"
          className="fixed inset-0 z-[70] flex items-center justify-center bg-black/25 p-4 animate-in fade-in duration-150"
        >
          <div className="flex w-[380px] max-w-full flex-col overflow-hidden rounded-2xl border border-border/80 bg-popover p-5 shadow-2xl animate-in zoom-in-95 duration-150">
            <h4 className="text-ui-base font-semibold text-foreground select-none">
              {t('files.confirmDeleteTitle', { defaultValue: '删除确认' })}
            </h4>
            <div className="mt-2 space-y-2 text-ui-sm text-foreground-subtle">
              <p>{t('files.confirmDelete', { name: itemToDelete.name })}</p>
              <p className="font-mono text-ui-xs text-foreground bg-surface/60 px-2.5 py-1.5 rounded-md border border-border/60 break-all select-all">
                {itemToDelete.path}
              </p>
            </div>
            <div className="mt-5 flex items-center justify-end gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => setItemToDelete(null)}
                disabled={isDeleting}
                className="h-8 px-3 rounded-lg"
              >
                {t('common.cancel', { defaultValue: '取消' })}
              </Button>
              <Button
                type="button"
                variant="destructive"
                size="sm"
                onClick={handleDeleteConfirm}
                disabled={isDeleting}
                className="h-8 gap-1.5 px-3 rounded-lg"
              >
                {isDeleting ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : (
                  <Trash2 className="size-3.5" />
                )}
                <span>{t('files.delete')}</span>
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
