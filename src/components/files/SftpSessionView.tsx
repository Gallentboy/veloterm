import {
  AlertCircle,
  ArrowLeftRight,
  ArrowUp,
  Download,
  File,
  FileCode,
  Folder,
  FolderOpen,
  Loader2,
  RefreshCw,
  RotateCw,
  Trash2,
  Upload,
  X,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Tooltip } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';
import {
  listRemoteDirectory,
  pickDownloadPath,
  pickUploadFile,
  removeRemoteItem,
  type RemoteFileItem,
} from '@/services/sftpService';
import {
  transferQueue,
  useTransferSettings,
  useTransferTasks,
} from '@/services/transferQueue';

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** i).toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
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
      colorClass: 'text-terminal-blue font-semibold',
      iconClass: 'text-terminal-blue',
    };
  }
  if (item.is_symlink) {
    return {
      colorClass: 'text-terminal-cyan font-normal',
      iconClass: 'text-terminal-cyan',
    };
  }
  if ((item.permissions & 0o111) !== 0) {
    return {
      colorClass: 'text-terminal-green font-medium',
      iconClass: 'text-terminal-green',
    };
  }

  const parts = item.name.split('.');
  const ext = parts.length > 1 ? parts.pop()?.toLowerCase() ?? '' : '';

  if (ARCHIVE_EXTS.has(ext)) {
    return {
      colorClass: 'text-terminal-red font-normal',
      iconClass: 'text-terminal-red',
    };
  }
  if (MEDIA_EXTS.has(ext)) {
    return {
      colorClass: 'text-terminal-magenta font-normal',
      iconClass: 'text-terminal-magenta',
    };
  }
  if (CODE_CONFIG_EXTS.has(ext)) {
    return {
      colorClass: 'text-terminal-yellow font-normal',
      iconClass: 'text-terminal-yellow',
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

/** 面板间拖拽复制的自定义 MIME 载荷标识 */
const SFTP_DRAG_MIME = 'application/x-veloterm-sftp';

/** 拖拽载荷：源面板会话 + 被拖拽条目信息 */
interface SftpDragPayload {
  sessionId: string;
  path: string;
  name: string;
  isDir: boolean;
}

export interface SftpSessionViewProps {
  sessionId: string;
  hostId: number;
  initialPath: string;
  isVisible: boolean;
  onPathChange?: (path: string) => void;
  /** 打开可编辑文件到编辑器页 */
  onOpenEditor?: (file: { hostId: number; remotePath: string }) => void;
  /** 面板获得焦点（双栏场景下用于标记激活面板，任意指针交互捕获触发） */
  onPaneFocus?: () => void;
}

export function SftpSessionView({
  sessionId,
  hostId,
  initialPath,
  isVisible: _isVisible,
  onPathChange,
  onOpenEditor,
  onPaneFocus,
}: SftpSessionViewProps) {
  const { t } = useTranslation();
  const [currentPath, setCurrentPath] = useState<string>(initialPath || '/');
  const [pathInput, setPathInput] = useState<string>(initialPath || '/');
  const [items, setItems] = useState<RemoteFileItem[]>([]);
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [itemToDelete, setItemToDelete] = useState<RemoteFileItem | null>(null);
  const [isDeleting, setIsDeleting] = useState<boolean>(false);
  // 是否正被拖拽悬停（作为放置目标时的高亮态）
  const [isDropTarget, setIsDropTarget] = useState<boolean>(false);

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
      transferQueue.enqueue({
        direction: 'download',
        sessionId,
        remotePath: item.path,
        localPath,
        fileName: item.name,
      });
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

  // 批量上传：全部入全局队列，由队列按并发数并行调度（支持失败重试）
  const handleUploadFiles = useCallback(
    async (localPaths: string[], targetDir = currentPath) => {
      if (!sessionId || localPaths.length === 0) return;
      for (const localPath of localPaths) {
        const filename = localPath.split(/[/\\]/).pop() || 'file';
        const remoteTarget =
          targetDir === '/' ? `/${filename}` : `${targetDir}/${filename}`;
        transferQueue.enqueue({
          direction: 'upload',
          sessionId,
          remotePath: remoteTarget,
          localPath,
          fileName: filename,
          onFinished: () => void loadDirectory(targetDir),
        });
      }
    },
    [currentPath, loadDirectory, sessionId],
  );

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

  // ===== 面板间拖拽复制：整个面板作为放置目标 =====

  /** 拖拽悬停：仅识别自定义 SFTP 载荷才允许放置 */
  const handleDragOver = (e: React.DragEvent<HTMLDivElement>) => {
    if (e.dataTransfer.types.includes(SFTP_DRAG_MIME)) {
      e.preventDefault();
      e.dataTransfer.dropEffect = 'copy';
      setIsDropTarget(true);
    }
  };

  /** 真正离开面板边界才取消高亮（在子元素间移动不取消） */
  const handleDragLeave = (e: React.DragEvent<HTMLDivElement>) => {
    if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
      setIsDropTarget(false);
    }
  };

  /** 放置：解析载荷并入队远端复制任务（目录拖拽/源目标相同均拒绝并提示） */
  const handleDrop = (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setIsDropTarget(false);
    const raw = e.dataTransfer.getData(SFTP_DRAG_MIME);
    if (!raw) return;
    let payload: SftpDragPayload;
    try {
      payload = JSON.parse(raw) as SftpDragPayload;
    } catch {
      return; // 非法载荷直接忽略
    }
    if (!payload?.sessionId || !payload.path || !payload.name) return;
    if (payload.isDir) {
      setError(t('files.drag.unsupportedDir'));
      return;
    }
    const targetPath =
      currentPath === '/' ? `/${payload.name}` : `${currentPath}/${payload.name}`;
    if (payload.sessionId === sessionId && payload.path === targetPath) {
      setError(t('files.drag.sameTarget'));
      return;
    }
    // 入队远端复制：源会话读取，本面板会话写入，完成后刷新目标面板列表
    transferQueue.enqueue({
      direction: 'copy',
      sessionId: payload.sessionId,
      remotePath: payload.path,
      targetSessionId: sessionId,
      targetRemotePath: targetPath,
      fileName: payload.name,
      onFinished: () => void loadDirectory(currentPath),
    });
  };

  const allTasks = useTransferTasks();
  // copy 任务双端可见：源端（sessionId）与目标端（targetSessionId）都展示
  const transfersList = allTasks.filter(
    (task) => task.sessionId === sessionId || task.targetSessionId === sessionId,
  );
  const transferSettings = useTransferSettings();

  return (
    <div
      onPointerDownCapture={() => onPaneFocus?.()}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
      className={cn(
        'relative flex h-full min-h-0 flex-col overflow-hidden bg-background',
        isDropTarget && 'ring-2 ring-primary/50 ring-inset',
      )}
    >
      {/* 拖拽悬停高亮层：绝对定位 + 不拦截指针，不影响布局与拖放事件 */}
      {isDropTarget && (
        <div className="pointer-events-none absolute inset-0 z-20 bg-primary/5" />
      )}
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
                    draggable
                    onDragStart={(e) => {
                      // 目录也允许拖起，由放置端校验拒绝并给出明确提示
                      e.dataTransfer.setData(
                        SFTP_DRAG_MIME,
                        JSON.stringify({
                          sessionId,
                          path: item.path,
                          name: item.name,
                          isDir: item.is_dir,
                        }),
                      );
                      e.dataTransfer.effectAllowed = 'copy';
                    }}
                    onDoubleClick={() => {
                      if (item.is_dir) {
                        handleNavigate(item.path);
                      } else if (item.editable && onOpenEditor) {
                        onOpenEditor({ hostId, remotePath: item.path });
                      }
                    }}
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
                      {item.is_dir ? '-' : formatBytes(Number(item.size))}
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
                          <>
                            {item.editable && onOpenEditor ? (
                              <Tooltip content={t('files.tooltips.edit')} side="left">
                                <Button
                                  type="button"
                                  size="icon-xs"
                                  variant="ghost"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    onOpenEditor({ hostId, remotePath: item.path });
                                  }}
                                  className="size-6 rounded-md text-foreground-subtle hover:text-foreground hover:bg-hover active:bg-hover/80"
                                  aria-label={t('files.edit')}
                                >
                                  <FileCode className="size-3.5" />
                                </Button>
                              </Tooltip>
                            ) : null}
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
                          </>
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

      {/* 底部传输任务进度卡片：全局队列（并行/重试/限速设置） */}
      {transfersList.length > 0 && (
        <div className="shrink-0 border-t border-border/60 bg-surface/30 p-2.5 space-y-2">
          {/* 队列工具栏：统计 + 设置 + 清除 */}
          <div className="flex items-center justify-between gap-2 text-ui-xs text-foreground-subtle select-none">
            <span>
              {t('files.transfer.queue', {
                active: transfersList.filter((task) => task.status === 'active').length,
                queued: transfersList.filter((task) => task.status === 'queued').length,
                maxConcurrent: transferSettings.maxConcurrent,
              })}
            </span>
            <div className="flex items-center gap-1.5">
              <label className="flex items-center gap-1">
                <span>{t('files.transfer.concurrency')}</span>
                <input
                  type="number"
                  min={1}
                  max={8}
                  value={transferSettings.maxConcurrent}
                  onChange={(e) =>
                    transferQueue.updateSettings({
                      maxConcurrent: Math.max(1, Math.min(8, Number(e.target.value) || 3)),
                    })
                  }
                  className="h-5 w-10 rounded border border-border bg-input px-1 text-center font-mono text-ui-xs outline-none focus:border-border-hover"
                />
              </label>
              <label className="flex items-center gap-1">
                <span>{t('files.transfer.speedLimit')}</span>
                <input
                  type="number"
                  min={0}
                  step={512}
                  value={transferSettings.speedLimitBps ? transferSettings.speedLimitBps / 1024 : 0}
                  onChange={(e) =>
                    transferQueue.updateSettings({
                      speedLimitBps: Math.max(0, Number(e.target.value) || 0) * 1024,
                    })
                  }
                  className="h-5 w-16 rounded border border-border bg-input px-1 text-center font-mono text-ui-xs outline-none focus:border-border-hover"
                />
                <span>KB/s</span>
              </label>
              <Button
                type="button"
                size="xs"
                variant="ghost"
                onClick={() => transferQueue.clearFinished()}
                aria-label={t('files.transfer.clearFinished')}
              >
                {t('files.transfer.clearFinished')}
              </Button>
            </div>
          </div>

          {transfersList.map((tItem) => (
            <div
              key={tItem.id}
              className="flex items-center justify-between gap-3 rounded-md border border-border/50 bg-background/90 px-3 py-1.5 text-ui-xs shadow-xs"
            >
              <div className="flex items-center gap-2 min-w-0">
                {tItem.direction === 'copy' ? (
                  <ArrowLeftRight className="size-3.5 text-primary shrink-0" />
                ) : tItem.direction === 'upload' ? (
                  <Upload className="size-3.5 text-primary shrink-0" />
                ) : (
                  <Download className="size-3.5 text-success shrink-0" />
                )}
                <span className="font-medium truncate max-w-[140px] md:max-w-[200px]">
                  {tItem.fileName}
                </span>
                <span className="text-foreground-subtle font-mono text-[11px]">
                  {formatBytes(tItem.transferred)} / {formatBytes(tItem.totalBytes)}
                </span>
                {tItem.speedBps > 0 && tItem.status === 'active' && (
                  <span className="text-foreground-subtlest font-mono text-[10px]">
                    {formatSpeed(tItem.speedBps)}
                  </span>
                )}
                {tItem.attempt > 1 && (
                  <span className="text-foreground-subtlest font-mono text-[10px]">
                    #{tItem.attempt}
                  </span>
                )}
                {tItem.status === 'queued' && (
                  <span className="text-foreground-subtlest text-[10px]">
                    {t('files.transfer.queued')}
                  </span>
                )}
                {tItem.status === 'error' && (
                  <span className="text-destructive truncate max-w-[160px]" title={tItem.error ?? undefined}>
                    {tItem.error}
                  </span>
                )}
              </div>

              <div className="flex items-center gap-3 shrink-0">
                <div className="w-24 bg-surface rounded-full h-1.5 overflow-hidden">
                  <div
                    className={cn(
                      'h-full transition-all duration-150',
                      tItem.status === 'done'
                        ? 'bg-success'
                        : tItem.status === 'error'
                          ? 'bg-destructive'
                          : tItem.status === 'canceled'
                            ? 'bg-foreground-subtlest'
                            : 'bg-primary',
                    )}
                    style={{ width: `${tItem.percent}%` }}
                  />
                </div>
                <span className="w-10 text-right font-mono text-[11px] font-medium">
                  {tItem.percent.toFixed(0)}%
                </span>
                {tItem.status === 'active' || tItem.status === 'queued' ? (
                  <Button
                    type="button"
                    size="icon-xs"
                    variant="ghost"
                    onClick={() => transferQueue.cancelTask(tItem.id)}
                    aria-label={t('files.cancelTransfer')}
                  >
                    <X className="size-3 text-destructive" />
                  </Button>
                ) : tItem.status === 'error' || tItem.status === 'canceled' ? (
                  <Button
                    type="button"
                    size="icon-xs"
                    variant="ghost"
                    onClick={() => transferQueue.retryTask(tItem.id)}
                    aria-label={t('files.transfer.retry')}
                    title={t('files.transfer.retry')}
                  >
                    <RotateCw className="size-3 text-primary" />
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
