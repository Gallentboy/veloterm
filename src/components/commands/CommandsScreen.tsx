import { Command, Pencil, Pin, Play, Trash2, TriangleAlert } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { CommandFormDialog } from '@/components/commands/CommandFormDialog';
import { CommandParamDialog } from '@/components/commands/CommandParamDialog';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { commandStore, useCommands } from '@/services/commandStore';
import { extractPlaceholders, type CommandRecord } from '@/services/commandService';

/** 危险级别徽章 */
function DangerBadge({ level }: { level: CommandRecord['dangerLevel'] }) {
  const { t } = useTranslation();
  if (level === 'safe') {
    return (
      <span className="rounded-full bg-surface px-2 py-0.5 text-ui-xs text-success select-none">
        {t('commands.danger.safe')}
      </span>
    );
  }
  if (level === 'confirm') {
    return (
      <span className="rounded-full bg-warning/15 px-2 py-0.5 text-ui-xs text-warning select-none">
        ⚠ {t('commands.danger.confirm')}
      </span>
    );
  }
  return (
    <span className="rounded-full bg-destructive/15 px-2 py-0.5 text-ui-xs text-destructive select-none">
      <TriangleAlert className="mr-0.5 inline size-3 align-[-1px]" />
      {t('commands.danger.destructive')}
    </span>
  );
}

function formatRelativeTime(timestamp: number | null): string {
  if (!timestamp) return '';
  const diff = Date.now() - timestamp;
  if (diff < 60_000) return '刚刚';
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}分钟前`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}小时前`;
  return `${Math.floor(diff / 86_400_000)}天前`;
}

interface CommandCardProps {
  record: CommandRecord;
  onEdit: (record: CommandRecord) => void;
  onRun: (record: CommandRecord) => void;
}

function CommandCard({ record, onEdit, onRun }: CommandCardProps) {
  const { t } = useTranslation();
  const placeholders = extractPlaceholders(record.command);

  return (
    <div
      className={cn(
        'group flex flex-col gap-1.5 rounded-lg border border-border bg-background p-3 transition-colors',
        record.isPinned && 'border-border-hover bg-surface/30',
      )}
    >
      {/* 标题行 */}
      <div className="flex items-center gap-2">
        {record.isPinned ? <Pin className="size-3.5 shrink-0 text-primary" /> : null}
        <span className="min-w-0 flex-1 truncate text-ui-base font-medium text-foreground">
          {record.title}
        </span>
        <DangerBadge level={record.dangerLevel} />
      </div>

      {/* 命令体 */}
      <code className="block overflow-x-auto rounded-md border border-border/60 bg-surface px-2.5 py-1.5 font-mono text-ui-sm text-foreground whitespace-pre">
        {record.command}
      </code>

      {/* 描述 */}
      {record.description ? (
        <p className="text-ui-sm leading-relaxed text-foreground-subtle">{record.description}</p>
      ) : null}

      {/* 用途 */}
      {record.purpose ? (
        <p className="text-ui-xs text-foreground-subtlest">{record.purpose}</p>
      ) : null}

      {/* 标签 + 统计 + 操作 */}
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1.5">
          {record.tags.slice(0, 4).map((tag) => (
            <span
              key={tag}
              className="shrink-0 rounded-full bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle select-none"
            >
              {tag}
            </span>
          ))}
          {placeholders.length > 0 ? (
            <span className="shrink-0 text-ui-xs text-foreground-subtlest">
              {placeholders.length} 个参数
            </span>
          ) : null}
          <span className="shrink-0 text-ui-xs text-foreground-subtlest">
            {formatRelativeTime(record.lastUsedAt)}
            {record.usageCount > 0 ? ` · ${record.usageCount}次` : ''}
          </span>
        </div>

        <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => onRun(record)}
            aria-label={t('commands.run')}
            title={t('commands.run')}
          >
            <Play className="size-3.5 text-success" />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => onEdit(record)}
            aria-label={t('common.edit')}
          >
            <Pencil className="size-3.5" />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => void commandStore.remove(record.id)}
            aria-label={t('common.delete')}
          >
            <Trash2 className="size-3.5 text-destructive" />
          </Button>
        </div>
      </div>
    </div>
  );
}

/** 常用命令库页面：搜索 + 卡片列表 */
export function CommandsScreen() {
  const { t } = useTranslation();
  const commands = useCommands();
  const [keyword, setKeyword] = useState('');
  const [editRecord, setEditRecord] = useState<CommandRecord | null>(null);
  const [isFormOpen, setFormOpen] = useState(false);
  const [runRecord, setRunRecord] = useState<CommandRecord | null>(null);

  useEffect(() => {
    void commandStore.refresh();
  }, []);

  const filtered = useMemo(() => {
    const kw = keyword.trim().toLowerCase();
    if (!kw) return commands;
    return commands.filter(
      (cmd) =>
        cmd.title.toLowerCase().includes(kw) ||
        cmd.command.toLowerCase().includes(kw) ||
        cmd.description.toLowerCase().includes(kw) ||
        cmd.purpose.toLowerCase().includes(kw) ||
        cmd.category.toLowerCase().includes(kw) ||
        cmd.tags.some((tag) => tag.toLowerCase().includes(kw)),
    );
  }, [commands, keyword]);

  const handleEdit = (record: CommandRecord | null) => {
    setEditRecord(record);
    setFormOpen(true);
  };

  const handleRun = (record: CommandRecord) => {
    setRunRecord(record);
  };

  return (
    <div className="flex h-full flex-col overflow-hidden">
      {/* 工具栏 */}
      <div className="flex shrink-0 items-center gap-2 border-b border-border/50 px-4 py-2">
        <Command className="size-4 shrink-0 text-foreground-subtle" />
        <input
          value={keyword}
          onChange={(e) => setKeyword(e.target.value)}
          placeholder={t('commands.searchPlaceholder')}
          className="h-7 min-w-0 flex-1 rounded-md border border-border bg-input px-2.5 text-ui-sm text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover"
        />
        <Button
          variant="outline"
          size="sm"
          onClick={() => handleEdit(null)}
        >
          {t('commands.new')}
        </Button>
      </div>

      {/* 列表 */}
      <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-3">
        {filtered.length === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 select-none">
            <Command className="size-8 text-foreground-subtlest" />
            <span className="text-ui-sm text-foreground-subtlest">
              {keyword ? t('commands.noResults') : t('commands.empty')}
            </span>
            {!keyword ? (
              <Button variant="outline" onClick={() => handleEdit(null)}>
                {t('commands.new')}
              </Button>
            ) : null}
          </div>
        ) : (
          filtered.map((record) => (
            <CommandCard
              key={record.id}
              record={record}
              onEdit={handleEdit}
              onRun={handleRun}
            />
          ))
        )}
      </div>

      {/* 编辑弹窗 */}
      {isFormOpen ? (
        <CommandFormDialog
          node={editRecord}
          onClose={() => setFormOpen(false)}
          onSave={() => {
            setFormOpen(false);
            void commandStore.refresh();
          }}
        />
      ) : null}

      {/* 参数填写弹窗 */}
      {runRecord ? (
        <CommandParamDialog
          record={runRecord}
          onClose={() => setRunRecord(null)}
        />
      ) : null}
    </div>
  );
}
