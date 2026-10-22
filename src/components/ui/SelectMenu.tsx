import { Check, ChevronDown } from 'lucide-react';
import { useEffect, useMemo, useState, type KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { useAnchoredPanel } from '@/hooks/useAnchoredPanel';
import { useDismissOnOutsidePointer } from '@/hooks/useDismissOnOutsidePointer';
import { cn } from '@/lib/utils';

export interface SelectMenuOption {
  value: string;
  label: string;
  /** 次要说明行（如命令正文）；搜索同时匹配 label 与 description */
  description?: string;
}

export interface SelectMenuProps {
  value: string;
  options: SelectMenuOption[];
  onChange: (value: string) => void;
  /** 面板顶部搜索框（选项较多时启用；搜索匹配 label + description） */
  searchable?: boolean;
  searchPlaceholder?: string;
  /** 搜索无匹配时的提示文案（仅 searchable 时需要） */
  emptyLabel?: string;
  /** 未选中任何项时的占位文案 */
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  /** 触发器样式覆写（tailwind-merge：同组类后者胜；如工具栏的 h-6 紧凑形态） */
  triggerClassName?: string;
  'aria-label'?: string;
}

/** 与 formFields 的 controlClassName 保持一致的触发器样式 */
const TRIGGER_CLASS =
  'h-8 w-full rounded-md border border-border bg-input px-2 text-ui-base text-foreground outline-none transition-colors focus:border-border-hover';

/**
 * 自绘下拉选择（替代原生 `<select>` 的表单场景）。
 *
 * 原生 select 的弹层由系统绘制：macOS 是圆角系统菜单、Windows 是直角系统列表框，
 * 两端观感割裂且深色主题不可控。本组件统一走工程菜单 token（bg-menu / menu-hover /
 * border / shadow），触发器样式与输入框一致（h-8、rounded-md、bg-input）。
 *
 * 弹层经 portal 挂到 `document.body`（定位/夹取见 `useAnchoredPanel`），z-[80] 高于
 * Modal 的 z-[70]——本组件主要用在 Modal 内的表单里。支持键盘：↑/↓ 移动高亮、
 * Enter 选中、Esc 关闭（由关闭钩子处理）。
 */
export function SelectMenu({
  value,
  options,
  onChange,
  searchable = false,
  searchPlaceholder,
  emptyLabel,
  placeholder,
  disabled = false,
  className,
  triggerClassName,
  'aria-label': ariaLabel,
}: SelectMenuProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [highlight, setHighlight] = useState(0);
  const { panelRef, position, measure, anchorRef } = useAnchoredPanel(open);
  useDismissOnOutsidePointer(open, [anchorRef, panelRef], () => setOpen(false));

  const selected = options.find((option) => option.value === value);

  const filtered = useMemo(() => {
    const keyword = query.trim().toLowerCase();
    if (!keyword) return options;
    return options.filter(
      (option) =>
        option.label.toLowerCase().includes(keyword) ||
        (option.description?.toLowerCase().includes(keyword) ?? false),
    );
  }, [options, query]);

  // 非搜索模式：把焦点移到面板容器，让 ↑/↓/Enter 的 keydown 落在面板上
  useEffect(() => {
    if (open && !searchable) {
      panelRef.current?.focus();
    }
  }, [open, searchable, panelRef]);

  // 高亮移动时滚动到可见区域（长列表键盘导航必需）
  useEffect(() => {
    if (!open) return;
    panelRef.current
      ?.querySelector<HTMLElement>(`[data-option-index="${highlight}"]`)
      ?.scrollIntoView({ block: 'nearest' });
  }, [highlight, open, panelRef]);

  // anchorRef 需挂在触发器外层容器上（hook 的测量锚点，rect 与按钮一致）
  const openPanel = () => {
    if (disabled) return;
    if (open) {
      setOpen(false);
      return;
    }
    setQuery('');
    const selectedIndex = options.findIndex((option) => option.value === value);
    setHighlight(selectedIndex >= 0 ? selectedIndex : 0);
    measure();
    setOpen(true);
  };

  const select = (next: string) => {
    setOpen(false);
    if (next !== value) {
      onChange(next);
    }
  };

  const handlePanelKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setHighlight((current) => Math.min(current + 1, filtered.length - 1));
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setHighlight((current) => Math.max(current - 1, 0));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      const option = filtered[highlight];
      if (option) {
        select(option.value);
      }
    }
  };

  return (
    <div ref={anchorRef} className={cn('relative', className)}>
      <button
        type="button"
        disabled={disabled}
        onClick={openPanel}
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        className={cn(
          TRIGGER_CLASS,
          'flex cursor-pointer items-center justify-between gap-1.5 text-left',
          disabled && 'cursor-not-allowed opacity-50',
          triggerClassName,
        )}
      >
        <span className={cn('truncate', !selected && 'text-foreground-subtlest')}>
          {selected?.label ?? placeholder ?? ''}
        </span>
        <ChevronDown
          className={cn(
            'size-3.5 shrink-0 text-foreground-subtle transition-transform',
            open && 'rotate-180',
          )}
        />
      </button>

      {open && position
        ? createPortal(
            <div
              ref={panelRef}
              role="listbox"
              tabIndex={-1}
              aria-label={ariaLabel}
              onKeyDown={handlePanelKeyDown}
              style={{
                position: 'fixed',
                top: position.top,
                left: position.left,
                minWidth: position.width,
              }}
              className="z-[80] flex flex-col rounded-lg border border-border bg-menu p-1 shadow-lg outline-none"
            >
              {searchable ? (
                <div className="p-1 pb-0.5">
                  <input
                    // 搜索框打开即聚焦，键入过滤
                    autoFocus
                    value={query}
                    onChange={(event) => {
                      setQuery(event.target.value);
                      setHighlight(0);
                    }}
                    placeholder={searchPlaceholder}
                    className="h-7 w-full rounded-md border border-border bg-input px-2 text-ui-sm text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover"
                  />
                </div>
              ) : null}
              <div className="scrollbar-slim max-h-64 overflow-y-auto p-0.5">
                {filtered.length > 0 ? (
                  filtered.map((option, index) => {
                    const isSelected = option.value === value;
                    return (
                      <button
                        key={option.value}
                        type="button"
                        role="option"
                        aria-selected={isSelected}
                        data-option-index={index}
                        onMouseEnter={() => setHighlight(index)}
                        onClick={() => select(option.value)}
                        className={cn(
                          'flex w-full cursor-default items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors',
                          index === highlight && 'bg-menu-hover',
                        )}
                      >
                        <span className="min-w-0 flex-1">
                          <span className="block truncate text-ui-sm text-foreground">
                            {option.label}
                          </span>
                          {option.description ? (
                            <span className="mt-0.5 block truncate font-mono text-ui-xs text-foreground-subtlest">
                              {option.description}
                            </span>
                          ) : null}
                        </span>
                        {isSelected ? (
                          <Check className="size-3.5 shrink-0 text-primary" />
                        ) : null}
                      </button>
                    );
                  })
                ) : (
                  <div className="px-2 py-2 text-ui-sm text-foreground-subtlest select-none">
                    {emptyLabel}
                  </div>
                )}
              </div>
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}
