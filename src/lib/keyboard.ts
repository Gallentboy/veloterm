import { isMacOS } from './platform';

/** 键盘事件的修饰键标记（KeyboardEvent 与 React.KeyboardEvent 结构兼容） */
export interface ModifierFlagBag {
  metaKey: boolean;
  ctrlKey: boolean;
}

/**
 * 是否按下「主修饰键」：macOS 为 ⌘ Command（metaKey），Windows/Linux 为 Ctrl（ctrlKey）。
 * 平台不参与的修饰键按下时返回 false（如 Windows 的 Win/Meta 键不触发快捷键，
 * macOS 的 Ctrl 留给终端控制字符），避免系统级组合误触。
 */
export function hasPrimaryModifier(event: ModifierFlagBag): boolean {
  return isMacOS ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
}

/** 展示用主修饰键符号：macOS 为 '⌘'，其余平台为 'Ctrl' */
export const PRIMARY_MODIFIER_LABEL = isMacOS ? '⌘' : 'Ctrl';

/**
 * 生成平台化快捷键展示文案：
 * - macOS: shortcutLabel('K') → '⌘K'；shift → '⌘⇧K'
 * - Windows/Linux: 'Ctrl+K'；shift → 'Ctrl+Shift+K'
 */
export function shortcutLabel(key: string, options?: { shift?: boolean; alt?: boolean }): string {
  const keyLabel = key.length === 1 ? key.toUpperCase() : key;
  if (isMacOS) {
    return `${options?.alt ? '⌥' : ''}${options?.shift ? '⇧' : ''}${PRIMARY_MODIFIER_LABEL}${keyLabel}`;
  }
  const parts = [PRIMARY_MODIFIER_LABEL];
  if (options?.shift) parts.push('Shift');
  if (options?.alt) parts.push('Alt');
  parts.push(keyLabel);
  return parts.join('+');
}
