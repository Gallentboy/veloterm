import { CircleHelp, Info } from 'lucide-react';
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
      className="flex w-full cursor-default items-center gap-2 rounded-none px-2.5 py-1.5 text-left text-ui-sm text-foreground transition-colors hover:bg-menu-hover"
    >
      {icon}
      <span className="truncate">{children}</span>
    </button>
  );
}

interface WorkspaceHelpMenuButtonProps {
  onShowAbout: () => void;
}

/**
 * 头部帮助菜单（移植自 ZCode WorkspaceHelpMenuButton）：
 * ghost icon-md 问号按钮 + ControlHintTooltip，下拉仅保留「关于」项。
 */
export function WorkspaceHelpMenuButton({ onShowAbout }: WorkspaceHelpMenuButtonProps) {
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
        <div className="absolute right-0 top-full z-50 mt-1 min-w-40 rounded-md border border-border bg-menu py-1 shadow-lg">
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
