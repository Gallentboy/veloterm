import { ChevronDown, FileText } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FormField } from '@/components/hosts/formFields';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { cn } from '@/lib/utils';
import {
  applyImportCommands,
  type ImportApplyResult,
  type ImportStartResult,
  type ImportStrategy,
} from '@/services/commandService';

/** 冲突明细默认展开的条数上限（更多冲突时默认收起，避免对话框首屏被列表占满） */
const CONFLICTS_AUTO_OPEN_MAX = 5;

interface ImportPreviewDialogProps {
  open: boolean;
  result: ImportStartResult | null;
  onClose: () => void;
  /** 导入成功：父组件负责关闭对话框、刷新命令列表并反馈结果 */
  onImported: (result: ImportApplyResult) => void;
}

/** 折叠明细区：标题行按钮切换，内容限高滚动 */
function DetailSection({
  title,
  open,
  onToggle,
  children,
}: {
  title: string;
  open: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-lg border border-border bg-surface/30">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="flex w-full cursor-pointer items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-left transition-colors hover:bg-surface-hover"
      >
        <ChevronDown
          className={cn(
            'size-3.5 shrink-0 text-foreground-subtle transition-transform',
            !open && '-rotate-90',
          )}
        />
        <span className="text-ui-sm text-foreground-subtle select-none">{title}</span>
      </button>
      {open ? (
        <div className="scrollbar-slim flex max-h-40 flex-col gap-1.5 overflow-y-auto border-t border-border/50 px-2.5 py-2">
          {children}
        </div>
      ) : null}
    </div>
  );
}

/**
 * 导入命令预览：选定文件后的 dry-run 结果（新增/冲突/无效三分类汇总 +
 * 冲突处理三选一策略 + 可折叠明细），确认后按策略落地。
 *
 * 打开条件由父组件控制（open 为 false 返回 null）；result 换成新文件时
 * 各内部状态（策略/折叠/错误）经 effect 重置，避免残留上一次的中间态。
 */
export function ImportPreviewDialog({ open, result, onClose, onImported }: ImportPreviewDialogProps) {
  const { t } = useTranslation();
  const [strategy, setStrategy] = useState<ImportStrategy>('skip');
  const [conflictsOpen, setConflictsOpen] = useState(false);
  const [invalidOpen, setInvalidOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [applying, setApplying] = useState(false);

  const preview = result?.preview ?? null;

  // 新文件到达时重置全部中间态（组件常驻挂载，靠 open 控制显隐）
  useEffect(() => {
    if (!result) return;
    setStrategy('skip');
    setError(null);
    setApplying(false);
    setConflictsOpen(
      result.preview.conflicts.length > 0 && result.preview.conflicts.length <= CONFLICTS_AUTO_OPEN_MAX,
    );
    setInvalidOpen(false);
  }, [result]);

  if (!open || !result || !preview) {
    return null;
  }

  const conflictCount = preview.conflicts.length;
  const invalidCount = preview.invalid.length;
  // 主按钮文案数字随策略动态变：skip 只导入新增，overwrite/duplicate 连冲突一起
  const importCount =
    strategy === 'skip' ? preview.newCount : preview.newCount + conflictCount;

  const fileName = result.path.split(/[\\/]/).pop() ?? result.path;

  const strategyOptions: Array<{ value: ImportStrategy; count: number }> = [
    { value: 'skip', count: preview.newCount },
    { value: 'overwrite', count: preview.newCount + conflictCount },
    { value: 'duplicate', count: preview.newCount + conflictCount },
  ];

  const handleApply = async () => {
    if (applying) return;
    setApplying(true);
    try {
      const applied = await applyImportCommands(result.path, strategy);
      onImported(applied);
    } catch (err) {
      // 文件已被移动/损坏等：对话框保持打开，顶部提示错误
      setError(t('commands.io.importFailed', { error: String(err) }));
      setApplying(false);
    }
  };

  return (
    <Modal
      title={t('commands.io.importTitle')}
      onClose={onClose}
      className="w-[520px]"
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={applying}>
            {t('common.cancel')}
          </Button>
          <Button onClick={() => void handleApply()} disabled={applying || importCount === 0}>
            {t('commands.io.importConfirm', { count: importCount })}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5">
        {error ? <p className="text-ui-sm text-destructive">{error}</p> : null}

        {/* 文件名（截断显示，title 悬浮全路径） */}
        <p className="flex min-w-0 items-center gap-1.5" title={result.path}>
          <FileText className="size-3.5 shrink-0 text-foreground-subtlest" />
          <span className="min-w-0 truncate text-ui-sm text-foreground-subtle">{fileName}</span>
        </p>

        {/* 汇总：新增 / 冲突 / 无效 */}
        <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-ui-sm select-none">
          <span className="text-foreground-subtle">
            {t('commands.io.statNewLabel')}
            <span className="ml-1 font-medium text-success">{preview.newCount}</span>
          </span>
          <span className="text-foreground-subtlest">·</span>
          <span className="text-foreground-subtle">
            {t('commands.io.statConflictLabel')}
            <span className="ml-1 font-medium text-warning">{conflictCount}</span>
          </span>
          <span className="text-foreground-subtlest">·</span>
          <span className="text-foreground-subtle">
            {t('commands.io.statInvalidLabel')}
            <span className="ml-1 font-medium text-destructive">{invalidCount}</span>
          </span>
        </p>

        {/* 冲突处理策略（默认跳过） */}
        <FormField label={t('commands.io.strategy')}>
          <div className="flex flex-col gap-1.5" role="radiogroup" aria-label={t('commands.io.strategy')}>
            {strategyOptions.map((option) => (
              <label
                key={option.value}
                className={cn(
                  'flex cursor-pointer items-center gap-2 rounded-lg border p-2 transition-colors',
                  strategy === option.value
                    ? 'border-border-hover bg-surface'
                    : 'border-border hover:bg-surface-hover',
                )}
              >
                <input
                  type="radio"
                  name="import-strategy"
                  checked={strategy === option.value}
                  onChange={() => setStrategy(option.value)}
                  className="accent-foreground"
                />
                <span className="text-ui-sm text-foreground select-none">
                  {t(`commands.io.strategy_${option.value}`, { count: option.count })}
                </span>
              </label>
            ))}
          </div>
        </FormField>

        {/* 冲突明细（≤5 条默认展开） */}
        {conflictCount > 0 ? (
          <DetailSection
            title={t('commands.io.conflictsTitle', { count: conflictCount })}
            open={conflictsOpen}
            onToggle={() => setConflictsOpen((value) => !value)}
          >
            {preview.conflicts.map((conflict, index) => (
              <div key={`${conflict.uuid ?? 'local'}-${index}`} className="flex items-baseline gap-2">
                <span className="max-w-[45%] shrink-0 truncate text-ui-sm text-foreground" title={conflict.title}>
                  {conflict.title}
                </span>
                <code
                  className="min-w-0 flex-1 truncate font-mono text-ui-xs text-foreground-subtle"
                  title={conflict.command}
                >
                  {conflict.command}
                </code>
              </div>
            ))}
          </DetailSection>
        ) : null}

        {/* 无效明细（默认收起） */}
        {invalidCount > 0 ? (
          <DetailSection
            title={t('commands.io.invalidTitle', { count: invalidCount })}
            open={invalidOpen}
            onToggle={() => setInvalidOpen((value) => !value)}
          >
            {preview.invalid.map((item, index) => (
              <div key={index} className="flex items-baseline gap-2">
                <span className="shrink-0 font-mono text-ui-xs text-foreground-subtlest">#{item.index}</span>
                <span className="max-w-[40%] shrink-0 truncate text-ui-sm text-foreground">
                  {item.title ?? t('commands.io.untitled')}
                </span>
                <span className="min-w-0 flex-1 truncate text-ui-xs text-destructive" title={item.reason}>
                  {item.reason}
                </span>
              </div>
            ))}
          </DetailSection>
        ) : null}

        {/* 安全提示 */}
        <p className="text-ui-xs text-foreground-subtlest select-none">
          {t('commands.io.securityHint')}
        </p>
      </div>
    </Modal>
  );
}
