import {
  AlertCircle,
  ArrowLeftRight,
  ArrowUp,
  Download,
  File,
  Folder,
  FolderOpen,
  HardDrive,
  Info,
  Loader2,
  Pencil,
  RefreshCw,
  RotateCw,
  SquareTerminal,
  Eye,
  Trash2,
  Upload,
  X,
} from 'lucide-react';
import { lazy, memo, Suspense, useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent as ReactMouseEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { MarqueeText } from '@/components/ui/MarqueeText';
import { Tooltip } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';
import { useDismissOnOutsidePointer } from '@/hooks/useDismissOnOutsidePointer';
import { useDropTarget } from '@/hooks/useDropTarget';
import {
  beginInternalDrag,
  endInternalDrag,
  type SftpItemDragPayload,
  type SftpItemEntry,
} from '@/services/dragDropService';
import {
  dirSizeRemote,
  execAvailableRemote,
  listRemoteDirectory,
  pickDownloadDir,
  pickDownloadPath,
  pickUploadFiles,
  previewFastRemove,
  removeRemoteFast,
  removeRemoteItem,
  removeRemoteTree,
  scanRemoteTree,
  type FastRemovePreview,
  type RemoteFileItem,
} from '@/services/sftpService';
import { loadRemoteContent } from '@/editor/contentLoader';
import { isPreviewableImage } from '@/services/imagePreviewService';
const SftpFilePreviewModal = lazy(() =>
  import('@/components/files/SftpFilePreviewModal').then((m) => ({
    default: m.SftpFilePreviewModal,
  })),
);
import { SftpFilePropsDialog } from '@/components/files/SftpFilePropsDialog';
import { sendToTerminal } from '@/services/aiTerminalTools';
import { formatPathForTerminal } from '@/lib/shellPath';
import { useTerminalStore } from '@/terminal/TerminalProvider';
import {
  transferQueue,
  useTransferSettings,
  useTransferTasks,
  type TransferTask,
} from '@/services/transferQueue';

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** i).toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

/** 传输行紧凑字节格式：无空格（"1.2MB"），窄面板下保证不折行 */
function formatBytesCompact(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** i).toFixed(i === 0 ? 0 : 1)}${units[i]}`;
}

/** 传输行速度格式：紧凑无空格（"2.1MB/s"），配合固定宽度槽位消除数字变化引起的宽度跳动 */
function formatSpeed(speedBps: number): string {
  if (speedBps <= 0) return '0B/s';
  const k = 1024;
  const sizes = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
  const i = Math.floor(Math.log(speedBps) / Math.log(k));
  const val = speedBps / Math.pow(k, i);
  return `${val.toFixed(1)}${sizes[i]}`;
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

/**
 * 是否有预览入口。
 *
 * 后端 `editable` 只判文本文件（二进制为 false），但 PDF 虽不可编辑也能预览
 * （走 PDFium 渲染分支），故这里额外放行 .pdf。
 */
function canPreview(item: { name: string; editable?: boolean }): boolean {
  return Boolean(item.editable) || /\.pdf$/i.test(item.name) || isPreviewableImage(item.name);
}

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

interface SftpFileRowProps {
  item: RemoteFileItem;
  index: number;
  isSelected: boolean;
  dirSizeLoading: boolean;
  dirSize: number | undefined;
  hostId: number;
  onSelect: (event: ReactMouseEvent<HTMLTableRowElement>, item: RemoteFileItem, index: number) => void;
  onDragStart: (item: RemoteFileItem, index: number, isSelected: boolean) => void;
  onDragEnd: () => void;
  onDoubleClick: (item: RemoteFileItem) => void;
  onContextMenu: (event: ReactMouseEvent, item: RemoteFileItem) => void;
  onNavigate: (path: string) => void;
  onPreview: (item: RemoteFileItem) => void;
  onOpenEditor?: (file: { hostId: number; remotePath: string }) => void;
  onDownload: (item: RemoteFileItem) => void;
  onDeleteItem: (item: RemoteFileItem) => void;
}

const SFTP_ROW_HEIGHT = 34;
const SFTP_OVERSCAN = 15;

const SftpFileRow = memo(function SftpFileRow({
  item,
  index,
  isSelected,
  dirSizeLoading,
  dirSize,
  hostId,
  onSelect,
  onDragStart,
  onDragEnd,
  onDoubleClick,
  onContextMenu,
  onNavigate,
  onPreview,
  onOpenEditor,
  onDownload,
  onDeleteItem,
}: SftpFileRowProps) {
  const { t } = useTranslation();
  const visuals = getFileItemVisuals(item);

  return (
    <tr
      key={item.path}
      draggable
      onClick={(event) => onSelect(event, item, index)}
      onDragStart={() => onDragStart(item, index, isSelected)}
      onDragEnd={onDragEnd}
      onDoubleClick={() => onDoubleClick(item)}
      onContextMenu={(event) => onContextMenu(event, item)}
      className={cn(
        'h-[34px] transition-colors cursor-pointer group',
        isSelected ? 'bg-selected' : 'hover:bg-surface/40',
      )}
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
        {item.is_dir
          ? dirSizeLoading
            ? (
                <Loader2 className="inline size-3.5 animate-spin text-foreground-subtlest" />
              )
            : dirSize != null
              ? formatBytes(dirSize)
              : '-'
          : formatBytes(Number(item.size))}
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
            <Tooltip content={t('files.tooltips.enterDir')} side="top">
              <Button
                type="button"
                size="icon-xs"
                variant="ghost"
                onClick={(e) => {
                  e.stopPropagation();
                  onNavigate(item.path);
                }}
                className="size-6 rounded-md text-foreground-subtle hover:text-foreground hover:bg-hover active:bg-hover/80"
                aria-label={t('files.enterDir')}
              >
                <FolderOpen className="size-3.5" />
              </Button>
            </Tooltip>
          ) : (
            <>
              {canPreview(item) ? (
                <Tooltip content={t('files.tooltips.preview')} side="top">
                  <Button
                    type="button"
                    size="icon-xs"
                    variant="ghost"
                    onClick={(e) => {
                      e.stopPropagation();
                      onPreview(item);
                    }}
                    className="size-6 rounded-md text-foreground-subtle hover:text-foreground hover:bg-hover active:bg-hover/80"
                    aria-label={t('files.preview')}
                  >
                    <Eye className="size-3.5" />
                  </Button>
                </Tooltip>
              ) : null}
              {item.editable && onOpenEditor ? (
                <Tooltip content={t('files.tooltips.edit')} side="top">
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
                    <Pencil className="size-3.5" />
                  </Button>
                </Tooltip>
              ) : null}
              <Tooltip content={t('files.tooltips.download')} side="top">
                <Button
                  type="button"
                  size="icon-xs"
                  variant="ghost"
                  onClick={(e) => {
                    e.stopPropagation();
                    void onDownload(item);
                  }}
                  className="size-6 rounded-md text-foreground-subtle hover:text-foreground hover:bg-hover active:bg-hover/80"
                  aria-label={t('files.download')}
                >
                  <Download className="size-3.5" />
                </Button>
              </Tooltip>
            </>
          )}
          <Tooltip content={t('files.tooltips.delete')} side="top">
            <Button
              type="button"
              size="icon-xs"
              variant="ghost"
              onClick={(e) => {
                e.stopPropagation();
                onDeleteItem(item);
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
});

/** SFTP 字体缩放范围（无级平滑，倍率作用于 --ui-font-size 基准 14px） */
const SFTP_FONT_SCALE_MIN = 0.5;
const SFTP_FONT_SCALE_MAX = 1.5;
/** 100% 磁吸阈值：捏合/滚轮滑过 1.0 附近时自动吸附，提升回正效率 */
const FONT_SCALE_SNAP_THRESHOLD = 0.04;
/** 与 styles.css :root 的 --ui-font-size 基准一致 */
const UI_FONT_BASE_PX = 14;

/**
 * 传输任务行（固定行高单行布局，永不折行）：
 * - 文件名为唯一弹性元素（吸收宽度变化），其余元素全部 shrink-0 + nowrap，消除逐帧 reflow；
 * - 容器查询按卡片实际宽度分级隐藏次要信息：速度（<30rem）→ 已传/总量（<24rem），进度条同步收窄；
 * - 状态互斥占位：「排队中」占速度槽、错误文本占尺寸+速度槽，避免元素出现/消失引起跳动；
 * - 文件名超出截断，悬停走马灯滚动展示全文（MarqueeText），重试次数折叠进 title 提示。
 */
function TransferTaskRow({ task }: { task: TransferTask }) {
  const { t } = useTranslation();

  return (
    <div
      className={cn(
        '@container flex h-8 items-center gap-3 rounded-md border border-border/50 bg-background/90 px-3 text-ui-xs shadow-xs',
        // 完成后 2s 由队列标记 dismissing：淡出 + 轻微缩小，动画播完由队列移除
        'transition-[opacity,transform] duration-300 ease-out',
        task.dismissing && 'pointer-events-none scale-[0.98] opacity-0',
      )}
    >
      <div className="flex min-w-0 flex-1 items-center gap-2">
        {task.direction === 'copy' ? (
          <ArrowLeftRight className="size-3.5 shrink-0 text-primary" />
        ) : task.direction === 'upload' ? (
          <Upload className="size-3.5 shrink-0 text-primary" />
        ) : (
          <Download className="size-3.5 shrink-0 text-success" />
        )}

        <MarqueeText
          text={task.fileName}
          titleSuffix={task.attempt > 1 ? `#${task.attempt}` : undefined}
          className="min-w-0 max-w-[240px] flex-1 font-medium text-foreground"
        />

        {task.status === 'error' ? (
          <ControlHintTooltip title={task.error ?? ''} side="top" className="min-w-0 flex-1">
            <span className="block min-w-0 truncate text-destructive">{task.error}</span>
          </ControlHintTooltip>
        ) : (
          <span className="hidden shrink-0 whitespace-nowrap font-mono text-ui-xs text-foreground-subtle @min-[24rem]:inline">
            {formatBytesCompact(task.transferred)}/{formatBytesCompact(task.totalBytes)}
          </span>
        )}

        {task.status === 'active' && task.speedBps > 0 && (
          <span className="hidden w-14 shrink-0 whitespace-nowrap text-right font-mono text-ui-xs text-foreground-subtlest @min-[30rem]:inline">
            {formatSpeed(task.speedBps)}
          </span>
        )}
        {task.status === 'queued' && (
          <span className="hidden w-14 shrink-0 whitespace-nowrap text-right text-ui-xs text-foreground-subtlest @min-[30rem]:inline">
            {t('files.transfer.queued')}
          </span>
        )}
      </div>

      <div className="flex shrink-0 items-center gap-2.5">
        <div className="h-1.5 w-14 overflow-hidden rounded-full bg-surface @min-[24rem]:w-20">
          <div
            className={cn(
              'h-full transition-all duration-150',
              task.status === 'done'
                ? 'bg-success'
                : task.status === 'error'
                  ? 'bg-destructive'
                  : task.status === 'canceled'
                    ? 'bg-foreground-subtlest'
                    : 'bg-primary',
            )}
            style={{ width: `${task.percent}%` }}
          />
        </div>
        <span className="w-10 text-right font-mono text-ui-xs font-medium">
          {task.percent.toFixed(0)}%
        </span>
        {task.status === 'active' || task.status === 'queued' ? (
          <Button
            type="button"
            size="icon-xs"
            variant="ghost"
            onClick={() => transferQueue.cancelTask(task.id)}
            aria-label={t('files.cancelTransfer')}
          >
            <X className="size-3 text-destructive" />
          </Button>
        ) : task.status === 'error' || task.status === 'canceled' ? (
          <Button
            type="button"
            size="icon-xs"
            variant="ghost"
            onClick={() => transferQueue.retryTask(task.id)}
            aria-label={t('files.transfer.retry')}
          >
            <RotateCw className="size-3 text-primary" />
          </Button>
        ) : (
          <span className="text-ui-xs text-success">✓</span>
        )}
      </div>
    </div>
  );
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
  /** 切到终端页（「在此目录打开终端」用；由 AppShell 注入 setPage('terminal')） */
  onNavigateTerminal?: () => void;
}

export function SftpSessionView({
  sessionId,
  hostId,
  initialPath,
  isVisible: _isVisible,
  onPathChange,
  onOpenEditor,
  onPaneFocus,
  onNavigateTerminal,
}: SftpSessionViewProps) {
  const { t } = useTranslation();
  const { createSession } = useTerminalStore();
  const [currentPath, setCurrentPath] = useState<string>(initialPath || '/');
  const [pathInput, setPathInput] = useState<string>(initialPath || '/');
  const [items, setItems] = useState<RemoteFileItem[]>([]);
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [itemToDelete, setItemToDelete] = useState<RemoteFileItem | null>(null);
  const [isDeleting, setIsDeleting] = useState<boolean>(false);

  // ── 目录递归删除流程（纯 SFTP 逐文件 + Shell 极速两路）──
  // 与后端 FAST_ASK_THRESHOLD 同值的客户端阈值：扫描计数超过即弹极速询问
  const FAST_ASK_THRESHOLD_CLIENT = 500;
  type DeleteFlowPhase = 'counting' | 'ask-fast' | 'deleting' | 'fast';
  interface DeleteFlowState {
    phase: DeleteFlowPhase;
    item: RemoteFileItem;
    count: number | null;
    truncated: boolean;
    preview: FastRemovePreview | null;
    progress: { deleted: number; failed: number; total: number | null } | null;
    fallbackNote: string | null;
  }
  const [deleteFlow, setDeleteFlow] = useState<DeleteFlowState | null>(null);
  const [fastDontAsk, setFastDontAsk] = useState(false);
  const deleteCancelRef = useRef<(() => void) | null>(null);
  // 「本会话不再询问」（会话内存态：跨会话环境可能变化，保守不落库）
  const fastDeleteSkipRef = useRef<Set<string>>(new Set());
  // 目录大小（远端 du 经独立 exec 通道）：算过的目录填到大小说明列；同目录防重入
  const [dirSizes, setDirSizes] = useState<Record<string, number>>({});
  const [dirSizePath, setDirSizePath] = useState<string | null>(null);
  // 文件/目录行右键菜单（与面板空白菜单同款定位与dismiss模式）
  const [fileMenu, setFileMenu] = useState<
    { x: number; y: number; item: RemoteFileItem } | null
  >(null);
  const fileMenuRef = useRef<HTMLDivElement | null>(null);
  // 「尝试编辑」失败弹窗（后端拒绝原因直接展示给用户）
  const [editAttempt, setEditAttempt] = useState<{ name: string; message: string } | null>(null);
  const [isTryingEdit, setIsTryingEdit] = useState<boolean>(false);
  // 文件预览弹窗（Monaco：默认只读、可切换编辑保存；内容加载/保存由弹窗组件自理）
  const [preview, setPreview] = useState<{ name: string; path: string } | null>(null);
  // 是否正被拖拽悬停（作为放置目标时的高亮态）
  const [isDropTarget, setIsDropTarget] = useState<boolean>(false);

  // ── 远端列表多选（Ctrl/⌘ 点选、Shift 范围、单击单选；双击打开不受影响）──
  // 导航到其他目录/刷新时清空；选中集合是拖拽互拷与批量下载的载荷源
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const selectionAnchorRef = useRef<number | null>(null);
  useEffect(() => {
    setSelectedPaths(new Set());
    selectionAnchorRef.current = null;
  }, [currentPath]);

  // ── 文件列表固定行高虚拟滚动（无论上千或上万文件，视口内仅保持 30~50 行 DOM）──
  const listScrollRef = useRef<HTMLDivElement>(null);
  const [listScrollTop, setListScrollTop] = useState(0);
  const [listContainerHeight, setListContainerHeight] = useState(600);

  const handleListScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    setListScrollTop(e.currentTarget.scrollTop);
  }, []);

  useEffect(() => {
    const el = listScrollRef.current;
    if (!el) return;
    setListContainerHeight(el.clientHeight || 600);
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        if (entry.contentRect.height > 0) {
          setListContainerHeight(entry.contentRect.height);
        }
      }
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // 切换目录时滚动位置归零
  useEffect(() => {
    if (listScrollRef.current) {
      listScrollRef.current.scrollTop = 0;
      setListScrollTop(0);
    }
  }, [currentPath]);

  const { startIndex, endIndex, topSpacerHeight, bottomSpacerHeight } = useMemo(() => {
    const count = items.length;
    if (count === 0) {
      return { startIndex: 0, endIndex: 0, topSpacerHeight: 0, bottomSpacerHeight: 0 };
    }
    const start = Math.max(0, Math.floor(listScrollTop / SFTP_ROW_HEIGHT) - SFTP_OVERSCAN);
    const end = Math.min(count, Math.ceil((listScrollTop + listContainerHeight) / SFTP_ROW_HEIGHT) + SFTP_OVERSCAN);
    const topSpacer = start * SFTP_ROW_HEIGHT;
    const bottomSpacer = Math.max(0, (count - end) * SFTP_ROW_HEIGHT);
    return { startIndex: start, endIndex: end, topSpacerHeight: topSpacer, bottomSpacerHeight: bottomSpacer };
  }, [items.length, listScrollTop, listContainerHeight]);

  const visibleItems = useMemo(() => {
    return items.slice(startIndex, endIndex);
  }, [items, startIndex, endIndex]);

  // ── 字体大小调节（每个 SFTP 会话独立，关闭 Tab 即复位，不持久化）──
  // 触发方式借用 Chrome 两指捏合手势：WebKit(WKWebView) 的 gesture 事件 + Chrome 合成的 ctrl+滚轮；
  // 应用方式为在面板根覆盖 --ui-font-size，面板内全部 text-ui-* 文字随动（图标/间距不变）
  const rootRef = useRef<HTMLDivElement>(null);
  const [fontScale, setFontScale] = useState(1);
  const fontScaleRef = useRef(1);
  const gestureStartScaleRef = useRef(1);
  const [fontBadgeVisible, setFontBadgeVisible] = useState(false);
  const fontBadgeTimerRef = useRef<number | null>(null);
  // 右键菜单：提供「重置缩放」（与 Tab 右键菜单同款 ZCode 风格）
  const [paneMenu, setPaneMenu] = useState<{ x: number; y: number } | null>(null);
  const paneMenuRef = useRef<HTMLDivElement | null>(null);
  /** 打开菜单的锚点元素：再次右键同一锚点时视为「关闭」而非原地重开 */
  const paneMenuAnchorRef = useRef<Element | null>(null);

  const applyFontScale = useCallback((nextScale: number) => {
    const clamped = Math.min(SFTP_FONT_SCALE_MAX, Math.max(SFTP_FONT_SCALE_MIN, nextScale));
    // 100% 磁吸：滑入 1.0 ± 4% 区间时吸附到精确 1.0，松手即回正
    const snapped = Math.abs(clamped - 1) < FONT_SCALE_SNAP_THRESHOLD ? 1 : clamped;
    fontScaleRef.current = snapped;
    setFontScale(snapped);
    setFontBadgeVisible(true);
    if (fontBadgeTimerRef.current !== null) {
      window.clearTimeout(fontBadgeTimerRef.current);
    }
    fontBadgeTimerRef.current = window.setTimeout(() => setFontBadgeVisible(false), 1000);
  }, []);

  useEffect(() => {
    const element = rootRef.current;
    if (!element) return;
    // Chrome 两指捏合/外接鼠标 ctrl+滚轮：合成 ctrlKey wheel，指数累积平滑无级
    const onWheel = (event: WheelEvent) => {
      if (!event.ctrlKey && !event.metaKey) return;
      event.preventDefault();
      applyFontScale(fontScaleRef.current * Math.exp(-event.deltaY * 0.002));
    };
    // WebKit(WKWebView) 两指捏合：gesture 事件携带累计 scale，以手势起点为基准缩放
    const onGestureStart = (event: Event) => {
      event.preventDefault();
      gestureStartScaleRef.current = fontScaleRef.current;
    };
    const onGestureChange = (event: Event) => {
      event.preventDefault();
      const scale = (event as unknown as { scale?: number }).scale ?? 1;
      applyFontScale(gestureStartScaleRef.current * scale);
    };
    element.addEventListener('wheel', onWheel, { passive: false });
    element.addEventListener('gesturestart', onGestureStart);
    element.addEventListener('gesturechange', onGestureChange);
    return () => {
      element.removeEventListener('wheel', onWheel);
      element.removeEventListener('gesturestart', onGestureStart);
      element.removeEventListener('gesturechange', onGestureChange);
      if (fontBadgeTimerRef.current !== null) {
        window.clearTimeout(fontBadgeTimerRef.current);
      }
    };
  }, [applyFontScale]);

  // 右键菜单关闭：点击外部 / Escape
  const closePaneMenu = useCallback(() => {
    paneMenuAnchorRef.current = null;
    setPaneMenu(null);
  }, []);

  // document 捕获阶段监听，不受子树 stopPropagation / drag-region 影响
  useDismissOnOutsidePointer(Boolean(paneMenu), paneMenuRef, closePaneMenu);
  useDismissOnOutsidePointer(Boolean(fileMenu), fileMenuRef, () => setFileMenu(null));

  /**
   * 尝试编辑：对白名单文件直接进编辑器；非白名单（未知后缀等）也先走一次
   * sftp_read_file —— 后端 stat 大小上限 + 512B 探针嗅探做真实判定，
   * 内容确是文本（如无后缀的 runbook）则直接打开编辑，否则把拒绝原因弹窗告知用户。
   */
  const handleTryEdit = useCallback(
    async (item: RemoteFileItem) => {
      if (!onOpenEditor || isTryingEdit) return;
      if (item.editable) {
        setFileMenu(null);
        onOpenEditor({ hostId, remotePath: item.path });
        return;
      }
      setIsTryingEdit(true);
      try {
        const result = await loadRemoteContent(hostId, item.path);
        if (!result) {
          throw new Error('远程内容不可用');
        }
        setFileMenu(null);
        onOpenEditor({ hostId, remotePath: item.path });
      } catch (err) {
        setFileMenu(null);
        setEditAttempt({
          name: item.name,
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setIsTryingEdit(false);
      }
    },
    [hostId, isTryingEdit, onOpenEditor],
  );

  /** 文件预览：打开 Monaco 弹窗（默认只读，弹窗内可切换编辑并保存回远端） */
  const handlePreview = useCallback((item: RemoteFileItem) => {
    setPreview({ name: item.name, path: item.path });
  }, []);

  // 文件属性弹窗（权限/属主/属组；应用成功后刷新目录）
  const [propsItem, setPropsItem] = useState<RemoteFileItem | null>(null);
  // 旁路 exec 通道可用性（属性/目录大小等 shell 能力的入口门控）：
  // 目录切换时同步（getent 已在列表阶段探过路，堡垒机场景此时已标禁）
  const [execAvailable, setExecAvailable] = useState(true);
  useEffect(() => {
    if (!sessionId) return;
    execAvailableRemote(sessionId)
      .then(setExecAvailable)
      .catch(() => setExecAvailable(true));
  }, [sessionId, currentPath]);

  // onPathChange 经 ref 稳定引用：父组件（FilesScreen）每次重渲染传入的都是新的内联回调，
  // 若进入 loadDirectory 依赖数组，拖分栏/聚焦切换/任何重渲染都会重建 loadDirectory，
  // 触发下方加载 effect 重跑并真实刷新远端目录列表（表现为文件列表闪刷）
  const onPathChangeRef = useRef(onPathChange);
  onPathChangeRef.current = onPathChange;

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
        onPathChangeRef.current?.(path);
      } catch (err) {
        setError(String(err));
      } finally {
        setIsLoading(false);
      }
    },
    [sessionId],
  );

  // 仅挂载（会话实例首次出现）时按初始路径加载一次：
  // initialPath 的后续变化都是本组件 onPathChange 回写到 store 的回声，不应触发重新拉取
  useEffect(() => {
    void loadDirectory(initialPath || '/');
    // eslint-disable-next-line react-hooks/exhaustive-deps -- 仅按会话挂载加载一次；导航由内部 handleNavigate/refresh 驱动
  }, [sessionId]);

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

  /** 在此目录打开终端：为该主机建终端会话，就绪后注入 cd（sendToTerminal
   *  内部等待 terminalId 就绪，连接慢也不会丢命令）；Tab 标题取目录名 */
  const handleOpenTerminalHere = (dirPath: string) => {
    const title = dirPath.split('/').filter(Boolean).pop() ?? dirPath;
    const sessionKey = createSession({ hostId, customTitle: title || dirPath });
    onNavigateTerminal?.();
    void sendToTerminal(sessionKey, `cd ${formatPathForTerminal(dirPath, 'posix')}\n`);
  };

  /** 计算目录大小（远端 du；同一目录防重入，结果填大小说明列） */
  const handleCalcDirSize = async (item: RemoteFileItem) => {
    if (dirSizePath) return;
    setDirSizePath(item.path);
    try {
      const bytes = await dirSizeRemote(sessionId, item.path);
      setDirSizes((prev) => ({ ...prev, [item.path]: bytes }));
    } catch (err) {
      setError(`${item.name}: ${err}`);
    } finally {
      setDirSizePath(null);
    }
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

  // 纯 SFTP 递归删除执行（进度 + 可取消）；fallbackNote 用于极速失败回退时的提示
  const runSftpTreeDelete = async (
    item: RemoteFileItem,
    total: number | null,
    fallbackNote?: string,
  ) => {
    if (!sessionId) return;
    setDeleteFlow({
      phase: 'deleting',
      item,
      count: total,
      truncated: false,
      preview: null,
      progress: { deleted: 0, failed: 0, total },
      fallbackNote: fallbackNote ?? null,
    });
    try {
      const handle = await removeRemoteTree(sessionId, item.path, total, (p) => {
        if (!p.isFinished) {
          setDeleteFlow(
            (f) =>
              f && {
                ...f,
                progress: { deleted: p.deleted, failed: p.failed, total: p.total },
              },
          );
        }
      });
      deleteCancelRef.current = handle.cancel;
      await handle.done;
      setDeleteFlow(null);
      deleteCancelRef.current = null;
      await loadDirectory(currentPath);
    } catch (err) {
      const msg = String(err);
      setDeleteFlow(null);
      deleteCancelRef.current = null;
      if (msg.startsWith('CANCELLED:')) {
        setError(t('files.deleteCancelled', { done: msg.split(':')[1] ?? '0' }));
      } else {
        setError(`删除失败: ${msg}`);
      }
      // 部分删除/取消后刷新列表，如实反映远端现状
      await loadDirectory(currentPath);
    }
  };

  // 极速删除执行：失败自动回退纯 SFTP（用户意图是「删掉」，不弹第三个框）
  const handleFastDelete = async () => {
    if (!deleteFlow || !sessionId) return;
    const { item, count, truncated } = deleteFlow;
    if (fastDontAsk) fastDeleteSkipRef.current.add(sessionId);
    setDeleteFlow((f) => f && { ...f, phase: 'fast' });
    try {
      await removeRemoteFast(sessionId, item.path, truncated ? null : count);
      setDeleteFlow(null);
      await loadDirectory(currentPath);
    } catch (err) {
      await runSftpTreeDelete(
        item,
        truncated ? null : count,
        t('files.fastDeleteFallback', { reason: String(err) }),
      );
    }
  };

  // 删除确认执行：文件走原单层路径；目录进入扫描→阈值分流→（极速询问）→递归删除
  const handleDeleteConfirm = async () => {
    if (!itemToDelete || !sessionId) return;
    if (!itemToDelete.is_dir) {
      setIsDeleting(true);
      try {
        await removeRemoteItem(sessionId, itemToDelete.path, false);
        setItemToDelete(null);
        await loadDirectory(currentPath);
      } catch (err) {
        setError(`删除失败: ${err}`);
      } finally {
        setIsDeleting(false);
      }
      return;
    }
    const item = itemToDelete;
    setItemToDelete(null);
    setFastDontAsk(false);
    setDeleteFlow({
      phase: 'counting', item, count: null, truncated: false,
      preview: null, progress: null, fallbackNote: null,
    });
    try {
      const scan = await scanRemoteTree(sessionId, item.path);
      const total = scan.truncated ? null : scan.nodeCount;
      const big = scan.truncated || scan.nodeCount > FAST_ASK_THRESHOLD_CLIENT;
      if (!big || fastDeleteSkipRef.current.has(sessionId)) {
        await runSftpTreeDelete(item, total);
        return;
      }
      // 取极速命令预览（无 POSIX shell/无句柄时预览报错——静默转纯 SFTP）
      try {
        const preview = await previewFastRemove(sessionId, item.path, total);
        setDeleteFlow(
          (f) => f && { ...f, phase: 'ask-fast', count: scan.nodeCount, truncated: scan.truncated, preview },
        );
      } catch {
        await runSftpTreeDelete(item, total);
      }
    } catch (err) {
      setDeleteFlow(null);
      setError(`删除失败: ${err}`);
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
      // 多选上传：一次选起多个文件批量入队
      const paths = await pickUploadFiles();
      if (!paths || paths.length === 0) return;
      await handleUploadFiles(paths);
    } catch (err) {
      setError(`上传失败: ${err}`);
    }
  };

  // 多选批量下载：选目标目录后逐个入队（文件名取远程同名；目录暂不支持，跳过计数提示）
  const handleBatchDownload = async () => {
    if (!sessionId) return;
    const selected = items.filter((item) => selectedPaths.has(item.path));
    const files = selected.filter((item) => !item.is_dir);
    const dirCount = selected.length - files.length;
    if (files.length === 0 && dirCount > 0) {
      setError(t('files.drag.unsupportedDir'));
      return;
    }
    try {
      const dir = await pickDownloadDir();
      if (!dir) return;
      for (const file of files) {
        const sep = dir.endsWith('/') || dir.endsWith('\\') ? '' : '/';
        transferQueue.enqueue({
          direction: 'download',
          sessionId,
          remotePath: file.path,
          localPath: `${dir}${sep}${file.name}`,
          fileName: file.name,
        });
      }
      if (dirCount > 0) {
        setError(t('files.selection.skippedDirs', { count: dirCount }));
      }
    } catch (err) {
      setError(`下载失败: ${err}`);
    }
  };

  // 行点击：多选状态机（Ctrl/⌘ 切换、Shift 范围、单击单选）；双击打开不受影响
  const handleRowSelect = useCallback(
    (
      event: ReactMouseEvent<HTMLTableRowElement>,
      item: RemoteFileItem,
      index: number,
    ) => {
      if (event.button !== 0) return;
      if (event.metaKey || event.ctrlKey) {
        setSelectedPaths((prev) => {
          const next = new Set(prev);
          if (next.has(item.path)) {
            next.delete(item.path);
          } else {
            next.add(item.path);
          }
          return next;
        });
        selectionAnchorRef.current = index;
        return;
      }
      if (event.shiftKey && selectionAnchorRef.current != null) {
        const from = Math.min(selectionAnchorRef.current, index);
        const to = Math.max(selectionAnchorRef.current, index);
        setSelectedPaths(new Set(items.slice(from, to + 1).map((row) => row.path)));
        return;
      }
      setSelectedPaths(new Set([item.path]));
      selectionAnchorRef.current = index;
    },
    [items],
  );

  const handleRowDragStart = useCallback(
    (item: RemoteFileItem, index: number, isSelected: boolean) => {
      const dragged: SftpItemEntry[] = isSelected
        ? items
            .filter((row) => selectedPaths.has(row.path))
            .map((row) => ({
              path: row.path,
              name: row.name,
              isDir: row.is_dir,
            }))
        : [
            {
              path: item.path,
              name: item.name,
              isDir: item.is_dir,
            },
          ];
      if (!isSelected) {
        setSelectedPaths(new Set([item.path]));
        selectionAnchorRef.current = index;
      }
      beginInternalDrag('sftp-item', { sessionId, items: dragged });
    },
    [items, selectedPaths, sessionId],
  );

  const handleRowDoubleClick = useCallback(
    (item: RemoteFileItem) => {
      if (item.is_dir) {
        handleNavigate(item.path);
      } else if (item.editable && onOpenEditor) {
        onOpenEditor({ hostId, remotePath: item.path });
      }
    },
    [handleNavigate, hostId, onOpenEditor],
  );

  const handleRowContextMenu = useCallback(
    (event: ReactMouseEvent, item: RemoteFileItem) => {
      event.preventDefault();
      event.stopPropagation();
      setFileMenu({
        x: Math.min(event.clientX, window.innerWidth - 180),
        y: Math.min(event.clientY, window.innerHeight - 150),
        item,
      });
    },
    [],
  );

  // ===== 面板间拖拽复制：整个面板作为放置目标 =====
  // 拖放事件流见 services/dragDropService（原生接管后页内 HTML5 drop 不再触发，
  // 载荷在 dragstart 时经 beginInternalDrag 登记，Drop 按命中元素分发）
  useDropTarget({
    ref: rootRef,
    // 两种落点：远端条目（面板互拷）与 OS 文件（拖入上传——MobaXterm 式，
    // 原生接管后经统一管线拿绝对路径，复用批量上传入队）
    accepts: ['sftp-item', 'files'],
    onHoverChange: (hovered) => setIsDropTarget(hovered),
    onDrop: (event) => {
      if (event.kind === 'files') {
        if (event.paths.length > 0) {
          void handleUploadFiles(event.paths);
        }
        return;
      }
      if (event.kind !== 'sftp-item') return;
      const payload = event.payload as SftpItemDragPayload;
      // 多条目逐个入队远端复制：目录跳过并提示，源目标同名跳过；
      // 全部被跳过时才报错（部分成功不弹错）
      if (!payload?.sessionId || !Array.isArray(payload.items) || payload.items.length === 0) {
        return;
      }
      let enqueued = 0;
      let dirSkipped = 0;
      for (const entry of payload.items as SftpItemEntry[]) {
        if (entry.isDir) {
          dirSkipped += 1;
          continue;
        }
        const targetPath =
          currentPath === '/' ? `/${entry.name}` : `${currentPath}/${entry.name}`;
        if (payload.sessionId === sessionId && entry.path === targetPath) {
          continue;
        }
        transferQueue.enqueue({
          direction: 'copy',
          sessionId: payload.sessionId,
          remotePath: entry.path,
          targetSessionId: sessionId,
          targetRemotePath: targetPath,
          fileName: entry.name,
          onFinished: () => void loadDirectory(currentPath),
        });
        enqueued += 1;
      }
      if (dirSkipped > 0) {
        setError(t('files.drag.unsupportedDir'));
      } else if (enqueued === 0) {
        setError(t('files.drag.sameTarget'));
      }
    },
  });

  const allTasks = useTransferTasks();
  // copy 任务双端可见：源端（sessionId）与目标端（targetSessionId）都展示
  const transfersList = allTasks.filter(
    (task) => task.sessionId === sessionId || task.targetSessionId === sessionId,
  );
  const transferSettings = useTransferSettings();

  // 缩放后的字号 token 覆盖表：--text-ui-* 在 :root(@theme) 已把 calc 解析为定值并按定值继承，
  // 子树覆盖 --ui-font-size 无效；必须直接覆盖最终 token，面板内全部 text-ui-* 文字才会随倍率变化
  const scaledFontSize = (deltaPx: number) =>
    `${(UI_FONT_BASE_PX * fontScale + deltaPx).toFixed(2)}px`;
  const fontScaleStyle = {
    '--text-ui-xl': scaledFontSize(4),
    '--text-ui-lg': scaledFontSize(2),
    '--text-ui-base': scaledFontSize(0),
    '--text-ui-caption': scaledFontSize(-1),
    '--text-ui-sm': scaledFontSize(-2),
    '--text-ui-xs': scaledFontSize(-4),
  } as CSSProperties;

  return (
    <div
      ref={rootRef}
      onPointerDownCapture={() => onPaneFocus?.()}
      onContextMenu={(event) => {
        event.preventDefault();
        // 菜单已打开时再次右键同一锚点 → 直接关闭（否则 pointerdown 先关、contextmenu
        // 又原地重开，视觉上等于「第一次点了没反应」）
        if (paneMenu && paneMenuAnchorRef.current === event.currentTarget) {
          closePaneMenu();
          return;
        }
        paneMenuAnchorRef.current = event.currentTarget;
        // 视口边缘锥制，避免菜单溢出屏幕
        setPaneMenu({
          x: Math.min(event.clientX, window.innerWidth - 180),
          y: Math.min(event.clientY, window.innerHeight - 120),
        });
      }}
      style={fontScaleStyle}
      className={cn(
        // 面板底色与页面/顶栏同源（bg-background，与终端页内容区一致；
        // 不再用偏亮的 --color-input——深色下会与顶部 Tab 栏形成色差）
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

          {/* 多选操作（内联在刷新右侧）：选中非空时显示（批量下载 / 清除；
              多选互拷无需按钮，直接拖选区即可） */}
          {selectedPaths.size > 0 ? (
            <div className="flex h-7 items-center gap-1.5 rounded-lg bg-selected/40 px-2 text-ui-xs">
              <span className="text-foreground-subtle select-none">
                {t('files.selection.selected', { count: selectedPaths.size })}
              </span>
              <Button
                type="button"
                variant="outline"
                onClick={handleBatchDownload}
                className="h-6 gap-1 rounded-md px-2 text-ui-xs font-medium border-border/70 bg-surface/40 hover:bg-hover active:bg-hover/80 text-foreground shadow-2xs transition-colors"
              >
                <Download className="size-3 text-foreground-subtle" />
                <span>{t('files.selection.download')}</span>
              </Button>
              <Button
                type="button"
                variant="ghost"
                onClick={() => {
                  setSelectedPaths(new Set());
                  selectionAnchorRef.current = null;
                }}
                className="h-6 gap-1 rounded-md px-2 text-ui-xs font-medium text-foreground-subtle hover:text-foreground hover:bg-hover"
              >
                <span>{t('files.selection.clear')}</span>
              </Button>
            </div>
          ) : null}
        </div>

        {/* 路径输入框与上传按钮 */}
        <div className="flex flex-1 items-center gap-2 min-w-[200px] justify-end">
          <form onSubmit={handlePathSubmit} className="flex-1 max-w-md">
            <input
              type="text"
              value={pathInput}
              onChange={(e) => setPathInput(e.target.value)}
              placeholder="/"
              className="select-text w-full rounded-md border border-border/60 bg-surface/30 px-2.5 py-1 font-mono text-ui-xs text-foreground outline-none focus:border-primary/60"
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

      {/* 面包屑导航栏：底色与面板统一（bg-background）——主内容与
          上方 Header 各层同色，仅以分隔线区分结构 */}
      <div className="flex shrink-0 items-center gap-1 border-b border-border/40 px-3 py-1 overflow-x-auto text-ui-xs text-foreground-subtle">
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
      <div
        ref={listScrollRef}
        onScroll={handleListScroll}
        className="min-h-0 flex-1 overflow-y-auto"
      >
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
            <thead className="sticky top-0 bg-background border-b border-border/50 text-foreground-subtle select-none z-10">
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
              {topSpacerHeight > 0 && (
                <tr style={{ height: `${topSpacerHeight}px` }} aria-hidden>
                  <td colSpan={7} className="p-0 border-0" />
                </tr>
              )}
              {visibleItems.map((item, localIdx) => {
                const index = startIndex + localIdx;
                return (
                  <SftpFileRow
                    key={item.path}
                    item={item}
                    index={index}
                    isSelected={selectedPaths.has(item.path)}
                    dirSizeLoading={dirSizePath === item.path}
                    dirSize={dirSizes[item.path]}
                    hostId={hostId}
                    onSelect={handleRowSelect}
                    onDragStart={handleRowDragStart}
                    onDragEnd={endInternalDrag}
                    onDoubleClick={handleRowDoubleClick}
                    onContextMenu={handleRowContextMenu}
                    onNavigate={handleNavigate}
                    onPreview={handlePreview}
                    onOpenEditor={onOpenEditor}
                    onDownload={handleDownload}
                    onDeleteItem={setItemToDelete}
                  />
                );
              })}
              {bottomSpacerHeight > 0 && (
                <tr style={{ height: `${bottomSpacerHeight}px` }} aria-hidden>
                  <td colSpan={7} className="p-0 border-0" />
                </tr>
              )}
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
                  className="select-text h-5 w-10 rounded border border-border bg-input px-1 text-center font-mono text-ui-xs outline-none focus:border-border-hover"
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
                  className="select-text h-5 w-16 rounded border border-border bg-input px-1 text-center font-mono text-ui-xs outline-none focus:border-border-hover"
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
            <TransferTaskRow key={tItem.id} task={tItem} />
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
              {itemToDelete.is_dir && (
                <p className="text-foreground">
                  {t('files.deleteDirNote', { defaultValue: '将递归删除整个目录及其全部内容。' })}
                </p>
              )}
              <p className="font-mono text-ui-xs text-foreground bg-surface/60 px-2.5 py-1.5 rounded-md border border-border/60 break-all select-all">
                {itemToDelete.path}
              </p>
            </div>
            <div className="mt-5 flex items-center justify-end gap-2">
              <Button
                type="button"
                variant="outline"
                size="lg"
                onClick={() => setItemToDelete(null)}
                disabled={isDeleting}
                className="h-8 px-3 text-ui-base"
              >
                {t('common.cancel', { defaultValue: '取消' })}
              </Button>
              <Button
                type="button"
                variant="destructive"
                size="lg"
                onClick={handleDeleteConfirm}
                disabled={isDeleting}
                className="h-8 gap-1.5 px-3 text-ui-base"
              >
                {isDeleting ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : (
                  <Trash2 className="size-4" />
                )}
                <span>{t('files.delete')}</span>
              </Button>
            </div>
          </div>
        </div>
      )}

      {/* 目录递归删除流程弹窗：统计计数 → 极速询问（红字命令审批）→ 执行（极速/进度） */}
      {deleteFlow && (
        <div
          role="dialog"
          aria-modal="true"
          className="fixed inset-0 z-[70] flex items-center justify-center bg-black/25 p-4 animate-in fade-in duration-150"
        >
          <div className="flex w-[440px] max-w-full flex-col overflow-hidden rounded-2xl border border-border/80 bg-popover p-5 shadow-2xl animate-in zoom-in-95 duration-150">
            {/* ── 统计中 ── */}
            {deleteFlow.phase === 'counting' && (
              <>
                <h4 className="flex items-center gap-2 text-ui-base font-semibold text-foreground select-none">
                  <Loader2 className="size-4 animate-spin" />
                  {t('files.deleteCounting', { defaultValue: '统计文件数…' })}
                </h4>
                <p className="mt-2 font-mono text-ui-xs text-foreground-subtle bg-surface/60 px-2.5 py-1.5 rounded-md border border-border/60 break-all">
                  {deleteFlow.item.path}
                </p>
                <div className="mt-5 flex justify-end">
                  <Button
                    type="button"
                    variant="outline"
                    size="lg"
                    onClick={() => setDeleteFlow(null)}
                    className="h-8 px-3 text-ui-base"
                  >
                    {t('common.cancel', { defaultValue: '取消' })}
                  </Button>
                </div>
              </>
            )}

            {/* ── 极速删除审批（红字命令展示）── */}
            {deleteFlow.phase === 'ask-fast' && deleteFlow.preview && (
              <>
                <h4 className="text-ui-base font-semibold text-foreground select-none">
                  {t('files.fastDeleteTitle', { defaultValue: '使用服务器 Shell 极速删除？' })}
                </h4>
                <div className="mt-2 space-y-2 text-ui-sm text-foreground-subtle">
                  <p>
                    {t('files.fastDeleteBody', {
                      count: deleteFlow.truncated
                        ? `${deleteFlow.count ?? ''}+`
                        : deleteFlow.count,
                    })}
                  </p>
                  <p className="font-mono text-ui-xs text-foreground-subtle bg-surface/60 px-2.5 py-1.5 rounded-md border border-border/60 break-all select-all">
                    {deleteFlow.item.path}
                  </p>
                  <p className="text-foreground">
                    {t('files.fastDeleteCommandLabel', { defaultValue: '将在服务器执行：' })}
                  </p>
                  <pre className="font-mono text-ui-xs leading-relaxed text-destructive bg-surface/60 px-2.5 py-1.5 rounded-md border border-border/60 break-all whitespace-pre-wrap select-all">
                    {deleteFlow.preview.command}
                  </pre>
                </div>
                <label className="mt-3 flex items-center gap-2 text-ui-xs text-foreground-subtle select-none">
                  <input
                    type="checkbox"
                    checked={fastDontAsk}
                    onChange={(e) => setFastDontAsk(e.target.checked)}
                    className="size-3.5 accent-foreground"
                  />
                  {t('files.fastDeleteDontAsk', { defaultValue: '本会话不再询问' })}
                </label>
                <div className="mt-4 flex items-center justify-end gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="lg"
                    onClick={() => {
                      if (sessionId && fastDontAsk) fastDeleteSkipRef.current.add(sessionId);
                      void runSftpTreeDelete(
                        deleteFlow.item,
                        deleteFlow.truncated ? null : deleteFlow.count,
                      );
                    }}
                    className="h-8 px-3 text-ui-base"
                  >
                    {t('files.fastDeleteKeepSftp', { defaultValue: '仍用 SFTP' })}
                  </Button>
                  <Button
                    type="button"
                    variant="destructive"
                    size="lg"
                    onClick={() => void handleFastDelete()}
                    className="h-8 px-3 text-ui-base"
                  >
                    {t('files.fastDeleteExecute', { defaultValue: '确认执行' })}
                  </Button>
                </div>
              </>
            )}

            {/* ── 极速执行中（无进度、不可中断——语义已在审批框提示）── */}
            {deleteFlow.phase === 'fast' && (
              <>
                <h4 className="flex items-center gap-2 text-ui-base font-semibold text-foreground select-none">
                  <Loader2 className="size-4 animate-spin" />
                  {t('files.fastDeleteRunning', { defaultValue: '服务器 Shell 删除中…' })}
                </h4>
                <p className="mt-2 font-mono text-ui-xs text-destructive bg-surface/60 px-2.5 py-1.5 rounded-md border border-border/60 break-all">
                  {deleteFlow.preview?.command}
                </p>
              </>
            )}

            {/* ── 纯 SFTP 逐文件删除（进度 + 可取消）── */}
            {deleteFlow.phase === 'deleting' && (
              <>
                <h4 className="flex items-center gap-2 text-ui-base font-semibold text-foreground select-none">
                  <Loader2 className="size-4 animate-spin" />
                  {t('files.deleteProgressTitle', { defaultValue: '正在删除…' })}
                </h4>
                {deleteFlow.fallbackNote && (
                  <p className="mt-2 rounded-md border border-border/60 bg-surface/60 px-2.5 py-1.5 text-ui-xs text-foreground-subtle">
                    {deleteFlow.fallbackNote}
                  </p>
                )}
                <p className="mt-2 font-mono text-ui-xs text-foreground-subtle bg-surface/60 px-2.5 py-1.5 rounded-md border border-border/60 break-all">
                  {deleteFlow.item.path}
                </p>
                <div className="mt-3">
                  {deleteFlow.progress?.total ? (
                    <>
                      <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface">
                        <div
                          className="h-full rounded-full bg-foreground transition-[width] duration-200"
                          style={{
                            width: `${Math.min(100, (deleteFlow.progress.deleted / deleteFlow.progress.total) * 100)}%`,
                          }}
                        />
                      </div>
                      <p className="mt-1.5 text-right font-mono text-ui-xs text-foreground-subtle">
                        {deleteFlow.progress.deleted} / {deleteFlow.progress.total}
                      </p>
                    </>
                  ) : (
                    <p className="text-right font-mono text-ui-xs text-foreground-subtle">
                      {t('files.deleteProgressCount', {
                        done: deleteFlow.progress?.deleted ?? 0,
                      })}
                    </p>
                  )}
                </div>
                <div className="mt-4 flex justify-end">
                  <Button
                    type="button"
                    variant="outline"
                    size="lg"
                    onClick={() => deleteCancelRef.current?.()}
                    className="h-8 px-3 text-ui-base"
                  >
                    {t('files.deleteCancel', { defaultValue: '取消删除' })}
                  </Button>
                </div>
              </>
            )}
          </div>
        </div>
      )}

      {/* 字体倍率气泡（Chrome 同款反馈）：捏合/ctrl+滚轮时短暂显示当前百分比 */}
      <div
        aria-hidden={!fontBadgeVisible}
        className={cn(
          'pointer-events-none absolute bottom-4 left-1/2 z-30 -translate-x-1/2 rounded-full border border-border bg-menu px-2.5 py-0.5 font-mono text-ui-xs text-foreground shadow-md transition-opacity duration-200',
          fontBadgeVisible ? 'opacity-100' : 'opacity-0',
        )}
      >
        {Math.round(fontScale * 100)}%
      </div>

      {/* 文件面板右键菜单（ZCode 风格）：重置缩放 */}
      {paneMenu ? (
        <div
          ref={paneMenuRef}
          role="menu"
          style={{ left: paneMenu.x, top: paneMenu.y }}
          onPointerDown={(event) => event.stopPropagation()}
          className="fixed z-50 min-w-36 overflow-hidden rounded-lg border border-border bg-menu p-1 text-ui-sm text-foreground shadow-lg"
        >
          <button
            type="button"
            role="menuitem"
            disabled={fontScale === 1}
            onClick={() => {
              closePaneMenu();
              applyFontScale(1);
            }}
            className={cn(
              'flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors',
              fontScale === 1 ? 'cursor-not-allowed opacity-40' : 'hover:bg-menu-hover',
            )}
          >
            <RotateCw className="size-3.5" />
            <span>{t('files.pane.resetZoom')}</span>
            <span className="ml-auto font-mono text-ui-xs text-foreground-subtlest">
              {Math.round(fontScale * 100)}%
            </span>
          </button>
        </div>
      ) : null}

      {/* 文件/目录行右键菜单：文件（预览/编辑[尝试]/下载/属性/删除）、目录（进入/开终端/算大小/属性/删除） */}
      {fileMenu ? (
        <div
          ref={fileMenuRef}
          role="menu"
          style={{ left: fileMenu.x, top: fileMenu.y }}
          onPointerDown={(event) => event.stopPropagation()}
          className="fixed z-50 min-w-36 overflow-hidden rounded-lg border border-border bg-menu p-1 text-ui-sm text-foreground shadow-lg"
        >
          {fileMenu.item.is_dir ? (
            <>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  const target = fileMenu.item;
                  setFileMenu(null);
                  handleNavigate(target.path);
                }}
                className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-menu-hover"
              >
                <FolderOpen className="size-3.5 text-foreground-subtle" />
                <span>{t('files.enterDir')}</span>
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  const target = fileMenu.item;
                  setFileMenu(null);
                  handleOpenTerminalHere(target.path);
                }}
                className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-menu-hover"
              >
                <SquareTerminal className="size-3.5 text-foreground-subtle" />
                <span>{t('files.openTerminalHere')}</span>
              </button>
              <button
                type="button"
                role="menuitem"
                disabled={dirSizePath != null}
                onClick={() => {
                  const target = fileMenu.item;
                  setFileMenu(null);
                  void handleCalcDirSize(target);
                }}
                className={cn(
                  'flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors',
                  dirSizePath != null ? 'cursor-not-allowed opacity-40' : 'hover:bg-menu-hover',
                )}
              >
                <HardDrive className="size-3.5 text-foreground-subtle" />
                <span>{t('files.calcDirSize')}</span>
              </button>
            </>
          ) : (
            <>
              {/* 预览与编辑同门槛：后端判非可编辑（非文本后缀 / 超 5MB）的内容拉下来也无法展示 */}
              {canPreview(fileMenu.item) ? (
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    const target = fileMenu.item;
                    setFileMenu(null);
                    handlePreview(target);
                  }}
                  className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-menu-hover"
                >
                  <Eye className="size-3.5 text-foreground-subtle" />
                  <span>{t('files.preview')}</span>
                </button>
              ) : null}
              <button
                type="button"
                role="menuitem"
                disabled={!onOpenEditor || isTryingEdit}
                onClick={() => void handleTryEdit(fileMenu.item)}
                className={cn(
                  'flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors',
                  !onOpenEditor || isTryingEdit
                    ? 'cursor-not-allowed opacity-40'
                    : 'hover:bg-menu-hover',
                )}
              >
                {isTryingEdit ? (
                  <Loader2 className="size-3.5 animate-spin text-foreground-subtle" />
                ) : (
                  <Pencil className="size-3.5 text-foreground-subtle" />
                )}
                <span>{t('files.edit')}</span>
                {/* 非白名单文件：编辑是「尝试」——后端探针嗅探，文本则开，否则弹窗说明 */}
                {!fileMenu.item.editable ? (
                  <span className="ml-auto font-mono text-ui-xs text-foreground-subtlest">
                    {t('files.menu.tryHint')}
                  </span>
                ) : null}
              </button>
            </>
          )}
          {!fileMenu.item.is_dir ? (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                const target = fileMenu.item;
                setFileMenu(null);
                void handleDownload(target);
              }}
              className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-menu-hover"
            >
              <Download className="size-3.5 text-foreground-subtle" />
              <span>{t('files.download')}</span>
            </button>
          ) : null}
          {/* 属性（权限/属主/属组）：依赖旁路 exec 通道，禁用的会话置灰并提示 */}
          <button
            type="button"
            role="menuitem"
            disabled={!execAvailable}
            onClick={() => {
              const target = fileMenu.item;
              setFileMenu(null);
              setPropsItem(target);
            }}
            className={cn(
              'flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors',
              !execAvailable ? 'cursor-not-allowed opacity-40' : 'hover:bg-menu-hover',
            )}
          >
            <Info className="size-3.5 text-foreground-subtle" />
            <span>{t('files.props')}</span>
            {!execAvailable ? (
              <span className="ml-auto font-mono text-ui-xs text-foreground-subtlest">
                {t('files.propsMenuDisabledHint')}
              </span>
            ) : null}
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              const target = fileMenu.item;
              setFileMenu(null);
              setItemToDelete(target);
            }}
            className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-menu-hover hover:text-destructive"
          >
            <Trash2 className="size-3.5 text-foreground-subtle" />
            <span>{t('files.delete')}</span>
          </button>
        </div>
      ) : null}

      {/* 文件预览弹窗：Monaco 承载，默认只读，可切换编辑并保存（脏态关闭需确认） */}
      {preview ? (
        <Suspense fallback={null}>
          <SftpFilePreviewModal
            hostId={hostId}
            sessionId={sessionId}
            fileName={preview.name}
            remotePath={preview.path}
            onClose={() => setPreview(null)}
            onSaved={() => {
              // 保存成功：刷新目录列表（size/mtime 列随之更新）
              void loadDirectory(currentPath);
            }}
          />
        </Suspense>
      ) : null}

      {/* 文件属性弹窗：权限九宫格/预设/八进制联动/特殊位/属主属组/递归；应用后刷新目录 */}
      {propsItem ? (
        <SftpFilePropsDialog
          sessionId={sessionId}
          item={propsItem}
          onClose={() => setPropsItem(null)}
          onApplied={() => {
            void loadDirectory(currentPath);
          }}
          onExecUnavailable={() => setExecAvailable(false)}
        />
      ) : null}

      {/* 尝试编辑失败弹窗：展示后端真实拒绝原因（非文本 / 超大小上限等） */}
      {editAttempt ? (
        <div
          role="dialog"
          aria-modal="true"
          className="fixed inset-0 z-[70] flex items-center justify-center bg-black/25 p-4 animate-in fade-in duration-150"
        >
          <div className="flex w-[420px] max-w-full flex-col overflow-hidden rounded-2xl border border-border/80 bg-popover p-5 shadow-2xl animate-in zoom-in-95 duration-150">
            <div className="flex items-start gap-2.5">
              <AlertCircle className="mt-0.5 size-4 shrink-0 text-warning" />
              <h4 className="text-ui-base font-semibold text-foreground select-none">
                {t('files.editFailedTitle')}
              </h4>
            </div>
            <div className="mt-2 space-y-2 text-ui-sm text-foreground-subtle">
              <p>{t('files.editFailedDesc', { name: editAttempt.name })}</p>
              <p className="rounded-md border border-border/60 bg-surface/60 px-2.5 py-1.5 font-mono text-ui-xs text-foreground break-all select-all">
                {editAttempt.message}
              </p>
            </div>
            <div className="mt-5 flex items-center justify-end gap-2">
              <Button
                type="button"
                size="lg"
                onClick={() => setEditAttempt(null)}
                className="h-8 px-3 text-ui-base"
              >
                {t('files.editFailedOk')}
              </Button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
