import type { MonacoDocumentCardProps } from '@/editor/types';

/**
 * 【桩件】Monaco 文档卡片——由 Monaco 实现 agent 整体替换本文件内容。
 *
 * 目标形态（ZCode 文档范式）：
 * - 外层底座 bg-background p-2~3；文档本体是 rounded-xl border border-border 的卡片
 * - 单 Monaco editor 实例 + 每文件一个 ITextModel（模块级 model 缓存，切换文件 setModel）
 * - viewState 恢复/防抖上报；Cmd/Ctrl+S → onSaveRequest；光标位置 → onCursorChange
 * - diffRemoteContent 非 null 时切 Monaco DiffEditor（本地 vs 远端）
 * - 主题用 defineTheme 从 CSS 令牌生成（--color-terminal-* / background / foreground），跟随深浅切换
 * - MonacoEnvironment worker 走 new Worker(new URL(...))（Rsbuild 原生支持）
 */
export function MonacoDocumentCard(_props: MonacoDocumentCardProps) {
  return (
    <div className="flex h-full items-center justify-center text-ui-sm text-foreground-subtlest">
      MonacoDocumentCard stub
    </div>
  );
}
