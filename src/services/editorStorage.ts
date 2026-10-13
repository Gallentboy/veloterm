import type { EditorFileDescriptor, EditorPanelStateSnapshot, IEditorStorage } from '@/editor/types';

/**
 * 【桩件】编辑器持久化——由 Monaco 实现 agent 整体替换本文件内容。
 *
 * 目标形态：
 * - Tauri 环境：invoke 对应 Rust 命令（editor_load_state / editor_save_panel_state /
 *   editor_save_draft / editor_delete_draft；viewState 并入 panel state 即可）
 * - 浏览器开发环境：localStorage 降级（key 前缀 veloterm:editor:*）
 * - 草稿保存需在 Rust 端落 SQLCipher（editor_drafts 表）
 */

const PANEL_KEY = 'veloterm:editor:panel-state';
const DRAFT_KEY_PREFIX = 'veloterm:editor:draft:';

export const editorStorage: IEditorStorage = {
  loadPanelState(): EditorPanelStateSnapshot | null {
    try {
      const raw = localStorage.getItem(PANEL_KEY);
      return raw ? (JSON.parse(raw) as EditorPanelStateSnapshot) : null;
    } catch {
      return null;
    }
  },
  savePanelState(snapshot: EditorPanelStateSnapshot): void {
    try {
      localStorage.setItem(PANEL_KEY, JSON.stringify(snapshot));
    } catch {
      // 存储失败不阻断
    }
  },
  loadDraft(fileId: number) {
    try {
      const raw = localStorage.getItem(DRAFT_KEY_PREFIX + fileId);
      return raw ? (JSON.parse(raw) as { content: string; baseHash: string }) : null;
    } catch {
      return null;
    }
  },
  saveDraft(file: EditorFileDescriptor, content: string): void {
    void file;
    void content;
    // 桩件不落盘
  },
  deleteDraft(fileId: number): void {
    void fileId;
  },
  loadViewState(fileId: number): unknown {
    void fileId;
    return null;
  },
  saveViewState(fileId: number, viewState: unknown): void {
    void fileId;
    void viewState;
  },
};
