import { ChevronRight, Copy, Folder, FolderOpen, FolderSync, Pencil, Plus, Server, Terminal, Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import type { GroupNode, HostNode } from '@/hosts/types';
import { cn } from '@/lib/utils';

const INDENT_PER_LEVEL_PX = 14;

function ActionButton({
  label,
  onClick,
  destructive,
  children,
}: {
  label: string;
  onClick: () => void;
  destructive?: boolean;
  children: React.ReactNode;
}) {
  return (
    // 原生 title 在 Windows/WKWebView 下延迟一两秒甚至不弹（工程统一用即时 CSS tooltip）
    <ControlHintTooltip title={label} side="bottom">
      <button
        type="button"
        aria-label={label}
        onClick={(event) => {
          event.stopPropagation();
          onClick();
        }}
        className={cn(
          'inline-flex size-6 shrink-0 items-center justify-center rounded-md transition-colors',
          destructive
            ? 'text-destructive hover:bg-destructive/10'
            : 'text-foreground-subtle hover:bg-hover hover:text-foreground',
        )}
      >
        {children}
      </button>
    </ControlHintTooltip>
  );
}

interface GroupTreeRowProps {
  group: GroupNode;
  depth: number;
  expanded: boolean;
  /** 分组行上的附加动作（如组内新建子分组） */
  showAddChild?: boolean;
  onToggle: (id: number) => void;
  onEdit: (group: GroupNode) => void;
  onDelete: (id: number) => void;
  onAddChild?: (parentId: number) => void;
}

/** 分组树节点行：箭头展开/收起 + 文件夹图标 + hover 操作（编辑/删除，可选加子组）；
 *  删除由父屏弹窗确认（工程约定：删除必须弹窗，严禁无确认直接删） */
export function GroupTreeRow({
  group,
  depth,
  expanded,
  showAddChild,
  onToggle,
  onEdit,
  onDelete,
  onAddChild,
}: GroupTreeRowProps) {
  const { t } = useTranslation();

  return (
    <div
      className={cn(
        'group flex h-8 items-center gap-1 rounded-lg pr-1 transition-colors hover:bg-surface-hover',
      )}
      style={{ paddingLeft: 8 + depth * INDENT_PER_LEVEL_PX }}
    >
      <button
        type="button"
        aria-label={t('hosts.toggleFolder')}
        aria-expanded={expanded}
        onClick={() => onToggle(group.id)}
        className="inline-flex size-5 shrink-0 items-center justify-center text-foreground-subtle transition-colors hover:text-foreground"
      >
        <ChevronRight
          className={cn('size-3.5 transition-transform duration-150', expanded && 'rotate-90')}
        />
      </button>
      {expanded ? (
        <FolderOpen className="size-4 shrink-0 text-foreground-subtle" />
      ) : (
        <Folder className="size-4 shrink-0 text-foreground-subtle" />
      )}
      <span className="min-w-0 truncate text-ui-base text-foreground select-none">
        {group.name}
      </span>
      {/* 操作紧跟分组名：全宽页面下贴右缘会让名称与 hover 按钮隔一整行空白，
          尾部占位吸收剩余宽度（主机行同构） */}
      <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
        {showAddChild && onAddChild ? (
          <ActionButton label={t('groups.addChild')} onClick={() => onAddChild(group.id)}>
            <Plus className="size-3.5" />
          </ActionButton>
        ) : null}
        <ActionButton label={t('common.edit')} onClick={() => onEdit(group)}>
          <Pencil className="size-3.5" />
        </ActionButton>
        <ActionButton
          label={t('common.delete')}
          destructive
          onClick={() => onDelete(group.id)}
        >
          <Trash2 className="size-3.5" />
        </ActionButton>
      </span>
      <span className="min-w-0 flex-1" aria-hidden />
    </div>
  );
}

interface HostTreeRowProps {
  host: HostNode;
  jumpHostName?: string;
  depth: number;
  onConnect?: (host: HostNode) => void;
  onOpenSftp?: (host: HostNode) => void;
  /** 复制主机：以预填态打开新建对话框，保存后落库为新主机（不影响原主机） */
  onDuplicate?: (host: HostNode) => void;
  onEdit: (host: HostNode) => void;
  onDelete: (id: number) => void;
}

/** 主机行：Server 图标 + 名称 + user@host:port 副标题，支持双击或点击终端图标发起 SSH 连接，hover 编辑/删除；
 *  删除由父屏弹窗确认（工程约定：删除必须弹窗，严禁无确认直接删） */
export function HostTreeRow({
  host,
  jumpHostName,
  depth,
  onConnect,
  onOpenSftp,
  onDuplicate,
  onEdit,
  onDelete,
}: HostTreeRowProps) {
  const { t } = useTranslation();
  const subtitle = `${host.username || '—'}@${host.host}:${host.port}`;

  return (
    <div
      onDoubleClick={() => onConnect?.(host)}
      className={cn(
        'group flex h-8 items-center gap-1 rounded-lg pr-1 pl-2 transition-colors hover:bg-surface-hover',
        onConnect && 'cursor-pointer',
      )}
      style={{ paddingLeft: 8 + depth * INDENT_PER_LEVEL_PX }}
    >
      <span className="size-5 shrink-0" />
      <Server className="size-4 shrink-0 text-foreground-subtle" />
      {/* 名称块与操作组相邻，尾部占位吸收剩余宽度：全宽页面下操作若贴右缘，
          名称与 hover 按钮之间会隔一整行空白，视线动线过长；空间不足时名称块
          内部 truncate，副标题随块收缩 */}
      <span className="min-w-0 truncate text-ui-base text-foreground select-none">
        {host.name}
        <span className="ml-2 text-ui-xs text-foreground-subtlest">{subtitle}</span>
        {jumpHostName ? (
          <span className="ml-1.5 rounded bg-surface px-1.5 py-0.5 text-ui-xs text-foreground-subtle">
            {t('hosts.form.viaJump', { name: jumpHostName })}
          </span>
        ) : null}
      </span>
      <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
        {onConnect ? (
          <ActionButton label={t('terminal.new')} onClick={() => onConnect(host)}>
            <Terminal className="size-3.5" />
          </ActionButton>
        ) : null}
        {onOpenSftp ? (
          <ActionButton label={t('files.title')} onClick={() => onOpenSftp(host)}>
            <FolderSync className="size-3.5" />
          </ActionButton>
        ) : null}
        {onDuplicate ? (
          <ActionButton label={t('hosts.duplicate')} onClick={() => onDuplicate(host)}>
            <Copy className="size-3.5" />
          </ActionButton>
        ) : null}
        <ActionButton label={t('common.edit')} onClick={() => onEdit(host)}>
          <Pencil className="size-3.5" />
        </ActionButton>
        <ActionButton
          label={t('common.delete')}
          destructive
          onClick={() => onDelete(host.id)}
        >
          <Trash2 className="size-3.5" />
        </ActionButton>
      </span>
      <span className="min-w-0 flex-1" aria-hidden />
    </div>
  );
}
