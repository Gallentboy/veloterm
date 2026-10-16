import { Loader2, Trash2 } from 'lucide-react';
import { useEffect, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';

export interface ConfirmDialogProps {
  /** 打开状态；null/关闭时组件不渲染 */
  open: boolean;
  /** 弹窗标题（如「删除确认」） */
  title: string;
  /** 正文说明（删除对象、后果提示） */
  description?: ReactNode;
  /** 等宽细节块（路径 / 命令正文等，可全选复制） */
  detail?: ReactNode;
  /** 确认按钮文案；缺省「删除」 */
  confirmLabel?: string;
  /** 取消按钮文案；缺省 common.cancel */
  cancelLabel?: string;
  /** 确认中：按钮转菊花并禁用（异步删除防重入） */
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * 通用危险操作确认弹窗：删除类按钮的标准配套（工程约定：**严禁无确认直接删除**）。
 *
 * 视觉规格对齐 SftpSessionView 的删除确认弹窗（遮罩 + 380px popover 卡片 +
 * destructive 确认按钮）。Esc 关闭；遮罩点击不关闭（防误触）。
 */
export function ConfirmDialog({
  open,
  title,
  description,
  detail,
  confirmLabel,
  cancelLabel,
  busy = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const { t } = useTranslation();

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !busy) {
        onCancel();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [open, busy, onCancel]);

  if (!open) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/25 p-4 animate-in fade-in duration-150"
    >
      <div className="flex w-[380px] max-w-full flex-col overflow-hidden rounded-2xl border border-border/80 bg-popover p-5 shadow-2xl animate-in zoom-in-95 duration-150">
        <h4 className="text-ui-base font-semibold text-foreground select-none">{title}</h4>
        {description ? (
          <div className="mt-2 text-ui-sm text-foreground-subtle">{description}</div>
        ) : null}
        {detail ? (
          <p className="mt-2 rounded-md border border-border/60 bg-surface/60 px-2.5 py-1.5 font-mono text-ui-xs text-foreground break-all select-all">
            {detail}
          </p>
        ) : null}
        <div className="mt-5 flex items-center justify-end gap-2">
          <Button
            type="button"
            variant="outline"
            size="lg"
            onClick={onCancel}
            disabled={busy}
            className="h-8 px-3 text-ui-base"
          >
            {cancelLabel ?? t('common.cancel')}
          </Button>
          <Button
            type="button"
            variant="destructive"
            size="lg"
            onClick={onConfirm}
            disabled={busy}
            className="h-8 gap-1.5 px-3 text-ui-base"
          >
            {busy ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <Trash2 className="size-4" />
            )}
            <span>{confirmLabel ?? t('common.delete')}</span>
          </Button>
        </div>
      </div>
    </div>
  );
}
