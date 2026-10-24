import { BookOpen, CircleHelp, Info } from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';

function DropdownItem({
  icon,
  children,
  onSelect,
}: {
  icon: ReactNode;
  children: ReactNode;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      // 条目 hover 圆角与 ZCode 下拉菜单一致（rounded-md）；容器为 rounded-lg
      className="flex w-full cursor-default items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-ui-sm text-foreground transition-colors hover:bg-menu-hover"
    >
      {icon}
      <span className="flex-1 truncate">{children}</span>
    </button>
  );
}

interface WorkspaceHelpMenuButtonProps {
  onShowAbout: () => void;
  onOpenManual: () => void;
}

/**
 * 头部帮助菜单（移植自 ZCode WorkspaceHelpMenuButton）：
 * ghost icon-md 问号按钮 + ControlHintTooltip，下拉包含「使用说明」与「关于」。
 */
export function WorkspaceHelpMenuButton({ onShowAbout, onOpenManual }: WorkspaceHelpMenuButtonProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    const onPointerDown = (event: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  return (
    <div ref={rootRef} className="relative">
      <ControlHintTooltip title={t('help.menu')} side="bottom">
        <Button
          variant="ghost"
          size="icon-md"
          aria-label={t('help.menu')}
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
        >
          <CircleHelp />
        </Button>
      </ControlHintTooltip>

      {open ? (
        <div className="absolute right-0 top-full z-50 mt-1 rounded-lg border border-border bg-menu p-1 shadow-lg">
          <DropdownItem
            icon={<BookOpen className="size-4 shrink-0 text-primary" />}
            onSelect={() => {
              setOpen(false);
              onOpenManual();
            }}
          >
            <div className="flex items-center justify-between w-full">
              <span>{t('help.manual', '使用说明')}</span>
              <kbd className="rounded border border-border/60 bg-surface px-1 py-0.2 font-mono text-[10px] text-foreground-subtle">
                F1
              </kbd>
            </div>
          </DropdownItem>
          <DropdownItem
            icon={<Info className="size-4 shrink-0" />}
            onSelect={() => {
              setOpen(false);
              onShowAbout();
            }}
          >
            {t('help.about')}
          </DropdownItem>
        </div>
      ) : null}
    </div>
  );
}
