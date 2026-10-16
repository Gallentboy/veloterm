import { Loader2, RefreshCw, TriangleAlert, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import type { EditorFileDescriptor, EditorFileStatus } from '@/editor/types';

export interface EditorTabBarProps {
  files: EditorFileDescriptor[];
  activeFileId: number | null;
  onActivate: (id: number) => void;
  onClose: (id: number) => void;
}

/**
 * 编辑器顶部 Tab 触发器（视觉规格复刻 TerminalTabTrigger）：
 * - 激活态 `bg-selected text-foreground`，非激活态悬停 `hover:bg-hover`；
 * - 状态标：dirty=圆点、offline=RefreshCw、conflict=TriangleAlert、saving=Loader2 旋转；
 * - 标题超长右侧渐隐；关闭按钮激活态常显、非激活态 hover/focus 浮现；中键快速关闭。
 */
function EditorTabTrigger({
  file,
  closeLabel,
  statusLabel,
  isActive,
  onActivate,
  onClose,
}: {
  file: EditorFileDescriptor;
  closeLabel: string;
  statusLabel: string;
  isActive: boolean;
  onActivate: (id: number) => void;
  onClose: (id: number) => void;
}) {
  return (
    <div
      role="tab"
      tabIndex={isActive ? 0 : -1}
      aria-selected={isActive}
      data-active={isActive ? '' : undefined}
      data-state={isActive ? 'active' : 'inactive'}
      onClick={() => onActivate(file.id)}
      onAuxClick={(event) => {
        // 鼠标中键关闭 Tab
        if (event.button === 1) {
          event.preventDefault();
          event.stopPropagation();
          onClose(file.id);
        }
      }}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          onActivate(file.id);
        }
      }}
      className={cn(
        'group relative inline-flex h-7 w-auto max-w-40 min-w-0 flex-none shrink-0 cursor-default items-center justify-start gap-1.5 overflow-hidden whitespace-nowrap rounded-lg border border-transparent bg-transparent pl-2 pr-1 text-ui-base font-medium text-foreground-subtle transition-all select-none outline-none',
        'hover:text-foreground',
        !isActive && 'hover:bg-hover',
        isActive && 'bg-selected text-foreground',
      )}
    >
      <EditorTabStatusMark status={file.status} label={statusLabel} />
      <span
        data-editor-tab-content=""
        className="flex min-w-0 flex-1 overflow-hidden whitespace-nowrap [mask-image:linear-gradient(to_right,black_calc(100%-0.5rem),transparent)]"
      >
        <span className="shrink-0 whitespace-nowrap">{file.fileName}</span>
      </span>
      <ControlHintTooltip title={closeLabel} side="bottom" align="end">
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label={closeLabel}
          className={cn(
            'shrink-0 rounded-md',
            !isActive &&
              'pointer-events-none opacity-0 group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100',
          )}
          onPointerDown={(event) => {
            event.stopPropagation();
          }}
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            onClose(file.id);
          }}
        >
          <X className="size-3" />
        </Button>
      </ControlHintTooltip>
    </div>
  );
}

/** 状态标：clean/loading 不显示；dirty=圆点、offline=RefreshCw、conflict=TriangleAlert、saving=Loader2 旋转 */
function EditorTabStatusMark({ status, label }: { status: EditorFileStatus; label: string }) {
  switch (status) {
    case 'dirty':
      return (
        <span className="flex size-3.5 shrink-0 items-center justify-center" title={label}>
          <span className="size-1.5 rounded-full bg-warning" />
        </span>
      );
    case 'saving':
      return <Loader2 className="size-3.5 shrink-0 animate-spin text-foreground-subtle" aria-label={label} />;
    case 'offline':
      return <RefreshCw className="size-3.5 shrink-0 text-foreground-subtlest" aria-label={label} />;
    case 'conflict':
      return <TriangleAlert className="size-3.5 shrink-0 text-warning" aria-label={label} />;
    default:
      return null;
  }
}

/** 编辑器顶部标签栏：结构对齐 TerminalScreen 顶栏（横向滚动、隐藏滚动条） */
export function EditorTabBar({ files, activeFileId, onActivate, onClose }: EditorTabBarProps) {
  const { t } = useTranslation();

  return (
    <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/50 px-3 py-1.5">
      <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto overflow-y-hidden [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        <div role="tablist" aria-label={t('editor.title')} className="flex h-7 w-max items-center gap-1">
          {files.map((file) => (
            <EditorTabTrigger
              key={file.id}
              file={file}
              closeLabel={t('editor.closeTab', { title: file.fileName })}
              statusLabel={t(`editor.status.${file.status}`)}
              isActive={file.id === activeFileId}
              onActivate={onActivate}
              onClose={onClose}
            />
          ))}
        </div>
      </div>
    </div>
  );
}
