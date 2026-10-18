import { ChevronDown, Command, Download, Pencil, Pin, Play, Trash2, TriangleAlert, Upload } from 'lucide-react';
import { createPortal } from 'react-dom';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { CommandFormDialog } from '@/components/commands/CommandFormDialog';
import { CommandParamDialog } from '@/components/commands/CommandParamDialog';
import { ImportPreviewDialog } from '@/components/commands/ImportPreviewDialog';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/ConfirmDialog';
import { useAnchoredPanel } from '@/hooks/useAnchoredPanel';
import { useDismissOnOutsidePointer } from '@/hooks/useDismissOnOutsidePointer';
import { cn } from '@/lib/utils';
import { commandStore, useCommands } from '@/services/commandStore';
import {
  exportCommandsToFile,
  extractPlaceholders,
  startImportCommands,
  type CommandRecord,
  type ExportFileFormat,
  type ImportApplyResult,
  type ImportStartResult,
} from '@/services/commandService';

/** 适配系统徽章：仅受限（非全平台）时展示，如「Win·Linux」；通用（空）不占位 */
function PlatformsBadge({ platforms }: { platforms: string[] }) {
  if (platforms.length === 0 || platforms.length >= 3) {
    return null;
  }
  const labelOf: Record<string, string> = { win: 'Win', linux: 'Linux', mac: 'Mac' };
  const label = platforms.map((p) => labelOf[p] ?? p).join('·');
  return (
    <span className="shrink-0 rounded-full bg-surface px-2 py-0.5 text-ui-xs text-foreground-subtle select-none">
      {label}
    </span>
  );
}

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


interface CommandCardProps {
  record: CommandRecord;
  onEdit: (record: CommandRecord) => void;
  onRun: (record: CommandRecord) => void;
  /** 请求删除（由父屏弹窗确认，工程约定：删除必须弹窗） */
  onDelete: (record: CommandRecord) => void;
}

function CommandCard({ record, onEdit, onRun, onDelete }: CommandCardProps) {
  const { t } = useTranslation();
  const placeholders = extractPlaceholders(record.command);

  return (
    <div
      className={cn(
        'group flex h-8 items-center gap-1.5 rounded-lg pr-1 transition-colors hover:bg-surface-hover',
        record.isPinned && 'bg-selected',
      )}
      style={{ paddingLeft: 10 }}
    >
      {record.isPinned ? <Pin className="size-3.5 shrink-0 text-primary" /> : <Command className="size-4 shrink-0 text-foreground-subtle" />}
      <span className="min-w-0 w-40 shrink-0 truncate text-ui-base text-foreground select-none">
        {record.title}
      </span>
      <code className="min-w-0 flex-1 truncate font-mono text-ui-sm text-foreground-subtle">
        {record.command}
      </code>
      <DangerBadge level={record.dangerLevel} />
      <PlatformsBadge platforms={record.platforms} />
      {placeholders.length > 0 ? (
        <span className="shrink-0 text-ui-xs text-foreground-subtlest">
          {placeholders.length}参
        </span>
      ) : null}
      <span className="shrink-0 text-ui-xs text-foreground-subtlest select-none">
        {record.usageCount > 0 ? `${record.usageCount}次` : ''}
      </span>

      <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
        <ControlHintTooltip title={t('commands.run')} side="top">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => onRun(record)}
            aria-label={t('commands.run')}
          >
            <Play className="size-3.5 text-success" />
          </Button>
        </ControlHintTooltip>
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
          onClick={() => onDelete(record)}
          aria-label={t('common.delete')}
        >
          <Trash2 className="size-3.5 text-destructive" />
        </Button>
      </span>
    </div>
  );
}

/** 工具栏内联反馈（无全局 toast，成功/失败都在按钮旁短暂显示后自动消失） */
interface ToolbarFeedback {
  kind: 'success' | 'error';
  text: string;
}

/** 常用命令库页面：搜索 + 卡片列表 */
export function CommandsScreen() {
  const { t } = useTranslation();
  const commands = useCommands();
  const [keyword, setKeyword] = useState('');
  const [editRecord, setEditRecord] = useState<CommandRecord | null>(null);
  const [isFormOpen, setFormOpen] = useState(false);
  const [runRecord, setRunRecord] = useState<CommandRecord | null>(null);
  // 删除确认弹窗目标（工程约定：删除必须弹窗确认，严禁无确认直接删）
  const [pendingDelete, setPendingDelete] = useState<CommandRecord | null>(null);
  const [feedback, setFeedback] = useState<ToolbarFeedback | null>(null);
  const feedbackTimer = useRef<number | null>(null);
  const [exportOpen, setExportOpen] = useState(false);
  const [importResult, setImportResult] = useState<ImportStartResult | null>(null);
  const { anchorRef, panelRef, position, measure } = useAnchoredPanel(exportOpen);
  useDismissOnOutsidePointer(exportOpen, [anchorRef, panelRef], () => setExportOpen(false));

  useEffect(() => {
    void commandStore.refresh();
  }, []);

  // 卸载时清理反馈定时器，避免 setState 打到已卸载组件
  useEffect(() => {
    return () => {
      if (feedbackTimer.current !== null) {
        window.clearTimeout(feedbackTimer.current);
      }
    };
  }, []);

  const showFeedback = (kind: ToolbarFeedback['kind'], text: string, durationMs = 2000) => {
    if (feedbackTimer.current !== null) {
      window.clearTimeout(feedbackTimer.current);
    }
    setFeedback({ kind, text });
    feedbackTimer.current = window.setTimeout(() => setFeedback(null), durationMs);
  };

  const toggleExportMenu = () => {
    if (exportOpen) {
      setExportOpen(false);
      return;
    }
    measure();
    setExportOpen(true);
  };

  const handleExport = async (format: ExportFileFormat) => {
    setExportOpen(false);
    try {
      const result = await exportCommandsToFile(format);
      if (!result) return; // 用户在保存对话框取消，静默
      showFeedback('success', t('commands.io.exportSuccess', { count: result.count }));
    } catch (err) {
      showFeedback('error', t('commands.io.exportFailed', { error: String(err) }), 3000);
    }
  };

  const handleImportStart = async () => {
    try {
      const result = await startImportCommands();
      if (!result) return; // 用户在打开对话框取消，静默
      setImportResult(result);
    } catch (err) {
      showFeedback('error', t('commands.io.importStartFailed', { error: String(err) }), 3000);
    }
  };

  const handleImported = (applied: ImportApplyResult) => {
    setImportResult(null);
    void commandStore.refresh();
    showFeedback(
      'success',
      t('commands.io.importResult', {
        inserted: applied.inserted,
        updated: applied.updated,
        skipped: applied.skipped,
        invalid: applied.invalid,
      }),
      4000,
    );
  };

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

  /** 折叠的类型组（按组名记录；搜索时忽略——结果恒全展开） */
  const [collapsedGroups, setCollapsedGroups] = useState<ReadonlySet<string>>(() => new Set());
  const toggleGroup = (group: string) => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(group)) {
        next.delete(group);
      } else {
        next.add(group);
      }
      return next;
    });
  };

  /** 按类型分组：键用归一化后的原始值（空/other 同归 other），展示名走 i18n
   *（导入数据里的自定义类型回退原值）；other 组排末尾，其余按本地化标签排序 */
  const groups = useMemo(() => {
    const byGroup = new Map<string, CommandRecord[]>();
    for (const record of filtered) {
      const key = record.category.trim() || 'other';
      const list = byGroup.get(key);
      if (list) {
        list.push(record);
      } else {
        byGroup.set(key, [record]);
      }
    }
    const labelOf = (key: string) =>
      t(`commands.categories.${key}`, { defaultValue: key === 'other' ? t('commands.categoryOther') : key });
    return [...byGroup.entries()]
      .map(([key, records]) => ({ key, label: labelOf(key), records }))
      .sort((a, b) => {
        if (a.key === 'other') return 1;
        if (b.key === 'other') return -1;
        return a.label.localeCompare(b.label, 'zh-Hans-CN');
      });
  }, [filtered, t]);

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
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/50 px-4 py-2">
        <span className="text-ui-sm text-foreground-subtle select-none">
          {t('commands.title')} · {filtered.length}
        </span>
        <div className="flex items-center gap-2">
          {feedback ? (
            <span
              role="status"
              className={cn(
                'min-w-0 truncate text-ui-sm',
                feedback.kind === 'success' ? 'text-success' : 'text-destructive',
              )}
            >
              {feedback.text}
            </span>
          ) : null}
          <input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            placeholder={t('commands.searchPlaceholder')}
            className="h-7 w-48 rounded-md border border-border bg-input px-2.5 text-ui-sm text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover"
          />
          <Button
            variant="outline"
            size="lg"
            onClick={() => void handleImportStart()}
          >
            <Download className="size-3.5" />
            {t('commands.io.importButton')}
          </Button>
          <div ref={anchorRef} className="relative">
            <Button
              variant="outline"
              size="lg"
              onClick={toggleExportMenu}
              aria-haspopup="menu"
              aria-expanded={exportOpen}
            >
              <Upload className="size-3.5" />
              {t('commands.io.exportButton')}
              <ChevronDown
                className={cn('size-3.5 text-foreground-subtle transition-transform', exportOpen && 'rotate-180')}
              />
            </Button>
          </div>
          <Button
            variant="outline"
            size="lg"
            onClick={() => handleEdit(null)}
          >
            {t('commands.new')}
          </Button>
        </div>
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
              <Button variant="outline" size="lg" onClick={() => handleEdit(null)}>
                {t('commands.new')}
              </Button>
            ) : null}
          </div>
        ) : (
          groups.map((group) => {
            // 搜索时忽略折叠：结果恒全展开，避免命中项藏在收起的组里
            const collapsed = !keyword && collapsedGroups.has(group.key);
            return (
              <div key={group.key} className="flex flex-col gap-1">
                <button
                  type="button"
                  onClick={() => toggleGroup(group.key)}
                  aria-expanded={!collapsed}
                  className="flex h-8 w-full cursor-default items-center gap-1.5 rounded-md px-1 text-left transition-colors hover:bg-surface select-none"
                >
                  <ChevronDown
                    className={cn(
                      'size-3.5 shrink-0 text-foreground-subtle transition-transform',
                      collapsed && '-rotate-90',
                    )}
                  />
                  <span className="min-w-0 truncate text-ui-base font-medium text-foreground">
                    {group.label}
                  </span>
                  <span className="shrink-0 text-ui-xs text-foreground-subtlest">
                    {group.records.length}
                  </span>
                </button>
                {collapsed
                  ? null
                  : group.records.map((record) => (
                      <CommandCard
                        key={record.id}
                        record={record}
                        onEdit={handleEdit}
                        onRun={handleRun}
                        onDelete={setPendingDelete}
                      />
                    ))}
              </div>
            );
          })
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

      {/* 导出格式下拉（portal 到 body + fixed，规格同 SelectMenu 弹层） */}
      {exportOpen && position
        ? createPortal(
            <div
              ref={panelRef}
              role="menu"
              aria-label={t('commands.io.exportButton')}
              style={{ position: 'fixed', top: position.top, left: position.left }}
              className="z-[80] flex w-max flex-col rounded-lg border border-border bg-menu p-1 shadow-lg"
            >
              {(['json', 'vtcmd'] as const).map((format) => (
                <button
                  key={format}
                  type="button"
                  role="menuitem"
                  onClick={() => void handleExport(format)}
                  className="flex w-full cursor-default items-center rounded-md px-2.5 py-1.5 text-left text-ui-sm whitespace-nowrap text-foreground transition-colors hover:bg-menu-hover"
                >
                  {t(`commands.io.export_${format}`)}
                </button>
              ))}
            </div>,
            document.body,
          )
        : null}

      {/* 导入预览弹窗 */}
      <ImportPreviewDialog
        open={importResult !== null}
        result={importResult}
        onClose={() => setImportResult(null)}
        onImported={handleImported}
      />

      {/* 删除确认弹窗（工程约定：删除必须弹窗确认） */}
      <ConfirmDialog
        open={pendingDelete !== null}
        title={t('common.confirmDeleteTitle')}
        description={t('common.confirmDelete', { name: pendingDelete?.title ?? '' })}
        detail={pendingDelete?.command}
        onConfirm={() => {
          const target = pendingDelete;
          setPendingDelete(null);
          if (target) {
            void commandStore.remove(target.id);
          }
        }}
        onCancel={() => setPendingDelete(null)}
      />
    </div>
  );
}
