import { CornerDownLeft, Pin, Search } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { createPortal } from 'react-dom';
import { cn } from '@/lib/utils';
import { useCommands } from '@/services/commandStore';
import {
  extractPlaceholders,
  type CommandRecord,
} from '@/services/commandService';
import { sendToTerminal } from '@/services/aiTerminalTools';
import { useTerminalStore } from '@/terminal/TerminalProvider';
import { commandStore } from '@/services/commandStore';

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
}

/**
 * 全局命令面板（主修饰键+K）：模糊搜索 + 键盘导航 + Enter 直接发送到激活终端。
 * 从任何页面打开，不依赖命令页。
 */
export function CommandPalette({ open, onClose }: CommandPaletteProps) {
  const { t } = useTranslation();
  const commands = useCommands();
  const terminalStore = useTerminalStore();
  const activeTerminalKey = terminalStore.activeSessionId;
  const [keyword, setKeyword] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (open) {
      setKeyword('');
      setSelectedIndex(0);
      // 聚焦搜索框
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  const filtered = useMemo(() => {
    const kw = keyword.trim().toLowerCase();
    if (!kw) return commands;
    return commands.filter(
      (cmd) =>
        cmd.title.toLowerCase().includes(kw) ||
        cmd.command.toLowerCase().includes(kw) ||
        cmd.description.toLowerCase().includes(kw) ||
        cmd.tags.some((tag) => tag.toLowerCase().includes(kw)),
    );
  }, [commands, keyword]);

  // 选中项滚动到可视区
  useEffect(() => {
    const el = listRef.current?.children[selectedIndex] as HTMLElement | undefined;
    el?.scrollIntoView({ block: 'nearest' });
  }, [selectedIndex]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const selected = filtered[selectedIndex];
      if (selected && activeTerminalKey != null) {
        void execute(selected);
      }
    } else if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    }
  };

  const execute = async (record: CommandRecord) => {
    if (activeTerminalKey == null) return;
    const placeholders = extractPlaceholders(record.command);
    // 命令面板直接执行：无占位符且 safe 才直接发；有占位符或需确认的跳到命令页
    if (placeholders.length === 0 && record.dangerLevel === 'safe') {
      const result = await sendToTerminal(activeTerminalKey, `${record.command}\n`);
      if (result.ok) {
        void commandStore.used(record.id);
      }
      onClose();
    } else {
      // 有占位符或危险命令：关闭面板，切到命令页让用户填写参数
      onClose();
    }
  };

  if (!open) return null;

  // 容器化定位（与 Modal 的 containerSelector 同机制、DbHostsScreen 先例）：
  // 遮罩 portal 到主内容区 <main>（AppShell 已挂 relative）内用 absolute，
  // 只盖主内容区并在其视窗内居中；拿不到容器（测试环境等）退全屏 fixed
  const [mainContainer] = useState(() => document.querySelector<HTMLElement>('main'));
  const overlay = (
    <div
      className={cn(
        'z-[80] flex items-start justify-center bg-black/25 pt-[15vh]',
        mainContainer ? 'absolute inset-0' : 'fixed inset-0',
      )}
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-label={t('commands.palette.title')}
    >
      <div
        className="flex w-[520px] max-w-full flex-col overflow-hidden rounded-xl border border-border bg-popover shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 搜索框 */}
        <div className="flex items-center gap-2 border-b border-border/50 px-3 py-2.5">
          <Search className="size-4 shrink-0 text-foreground-subtle" />
          <input
            ref={inputRef}
            value={keyword}
            onChange={(e) => {
              setKeyword(e.target.value);
              setSelectedIndex(0);
            }}
            onKeyDown={handleKeyDown}
            placeholder={t('commands.palette.placeholder')}
            className="min-w-0 flex-1 bg-transparent text-ui-base text-foreground outline-none placeholder:text-foreground-subtlest"
          />
          <kbd className="rounded border border-border bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle select-none">
            ESC
          </kbd>
        </div>

        {/* 结果列表 */}
        <div
          ref={listRef}
          className="flex max-h-[320px] min-h-[80px] flex-col overflow-y-auto p-1.5"
        >
          {filtered.length === 0 ? (
            <div className="flex flex-1 items-center justify-center py-8 select-none">
              <span className="text-ui-sm text-foreground-subtlest">
                {t('commands.palette.noResults')}
              </span>
            </div>
          ) : (
            filtered.map((record, index) => (
              <button
                key={record.id}
                type="button"
                onClick={() => void execute(record)}
                onMouseEnter={() => setSelectedIndex(index)}
                className={cn(
                  'flex flex-col gap-0.5 rounded-lg px-3 py-2 text-left transition-colors',
                  index === selectedIndex
                    ? 'bg-selected'
                    : 'hover:bg-surface-hover',
                )}
              >
                <div className="flex items-center gap-1.5">
                  {record.isPinned ? <Pin className="size-3 shrink-0 text-primary" /> : null}
                  <span className="min-w-0 flex-1 truncate text-ui-sm font-medium text-foreground">
                    {record.title}
                  </span>
                  {record.dangerLevel !== 'safe' ? (
                    <span
                      className={cn(
                        'shrink-0 rounded px-1 py-0.5 text-ui-xs select-none',
                        record.dangerLevel === 'destructive'
                          ? 'bg-destructive/15 text-destructive'
                          : 'bg-warning/15 text-warning',
                      )}
                    >
                      {record.dangerLevel === 'destructive' ? '⚠' : '⚠'}
                    </span>
                  ) : null}
                  {index === selectedIndex ? (
                    <CornerDownLeft className="size-3 shrink-0 text-foreground-subtlest" />
                  ) : null}
                </div>
                <code className="truncate font-mono text-ui-xs text-foreground-subtle">
                  {record.command}
                </code>
              </button>
            ))
          )}
        </div>

        {/* 底部提示 */}
        <div className="flex items-center gap-3 border-t border-border/50 px-3 py-1.5 text-ui-xs text-foreground-subtlest select-none">
          <span>↑↓ {t('commands.palette.navigate')}</span>
          <span>
            <CornerDownLeft className="inline size-3 align-[-1px]" />{' '}
            {t('commands.palette.send')}
          </span>
          <span>ESC {t('commands.palette.close')}</span>
        </div>
      </div>
    </div>
  );

  return mainContainer ? createPortal(overlay, mainContainer) : overlay;
}
