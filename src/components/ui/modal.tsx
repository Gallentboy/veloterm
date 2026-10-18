import { X } from 'lucide-react';
import { useEffect, useRef, type ReactNode } from 'react';
import { cn } from '@/lib/utils';

interface ModalProps {
  title: string;
  children: ReactNode;
  footer?: ReactNode;
  onClose: () => void;
  className?: string;
}

/** 通用模态框：遮罩 + 面板卡片，Escape/关闭按钮关闭。 */
export function Modal({ title, children, footer, onClose, className }: ModalProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  // 最新回调存 ref：两个 effect 都只在挂载时跑，重渲染不重跑
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  // 挂载时聚焦第一个可交互元素。**依赖必须为空**：调用方传的 onClose 多为 inline 箭头
  // （每次渲染新身份），若进了依赖数组，受控输入每敲一键 effect 就重跑、焦点被
  // 抢回首元素——症状是「输一个字符就得重新点一次输入框」
  useEffect(() => {
    rootRef.current?.querySelector<HTMLElement>('input, textarea, select, button')?.focus();
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onCloseRef.current();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  return (
    <div
      ref={rootRef}
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/25 p-6"
      role="dialog"
      aria-modal="true"
      aria-label={title}
    >
      <div
        className={cn(
          'flex max-h-full w-[420px] max-w-full flex-col overflow-hidden rounded-lg border border-border bg-panel shadow-2xl',
          className,
        )}
      >
        <header className="flex h-11 shrink-0 items-center justify-between border-b border-border/50 pl-4 pr-2">
          <span className="text-ui-base font-medium text-foreground select-none">{title}</span>
          <button
            type="button"
            aria-label="close"
            onClick={onClose}
            className="inline-flex size-7 items-center justify-center rounded-lg text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
          >
            <X className="size-4" />
          </button>
        </header>
        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-4">{children}</div>
        {footer ? (
          <footer className="flex shrink-0 items-center justify-end gap-2 border-t border-border/50 p-3">
            {footer}
          </footer>
        ) : null}
      </div>
    </div>
  );
}
