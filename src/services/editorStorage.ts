import { invoke, isTauri } from '@tauri-apps/api/core';
import type { EditorFileDescriptor, EditorPanelStateSnapshot, IEditorStorage } from '@/editor/types';

/**
 * 编辑器持久化（Tauri 主路径 + localStorage 降级）。
 *
 * - 面板状态/草稿：Tauri 环境走 invoke（Rust 端落 SQLCipher：
 *   editor_load_state / editor_save_panel_state / editor_save_draft / editor_delete_draft）；
 *   invoke 失败（命令未就绪、webview 预览等）逐项 catch 后降级 localStorage，保证浏览器开发环境可用
 * - viewState：纯 UI 状态，不进数据库，全部环境一律 localStorage（veloterm:editor:viewstate:<fileId>）
 * - IEditorStorage 是同步签名：模块加载时自启 initEditorStorage() 预取（模块级 promise），
 *   同步读缓存；预取未就绪时 loadPanelState/loadDraft 降级 localStorage
 */

const PANEL_KEY = 'veloterm:editor:panel-state';
const DRAFT_KEY_PREFIX = 'veloterm:editor:draft:';
const VIEWSTATE_KEY_PREFIX = 'veloterm:editor:viewstate:';

/** editor_load_state 的返回契约（与 Rust 端 serde 命名一致） */
interface EditorLoadStatePayload {
  panelStateJson: string | null;
  drafts: EditorDraftRow[];
}

interface EditorDraftRow {
  fileId: number;
  hostId: number;
  remotePath: string;
  baseHash: string;
  content: string;
  updatedAt: number;
}

function isTauriEnv(): boolean {
  return typeof window !== 'undefined' && isTauri();
}

/* —— 模块级缓存 + 预取状态 —— */

const draftCache = new Map<number, { content: string; baseHash: string }>();
let panelStateCache: EditorPanelStateSnapshot | null = null;
/** 预取是否完成（Tauri 环境）；未完成前同步读走 localStorage 降级 */
let panelStateReady = false;
/** invoke 整体不可用（命令缺失/调用报错）后彻底转 localStorage */
let invokeBroken = false;

let initPromise: Promise<void> | null = null;

export function initEditorStorage(): Promise<void> {
  initPromise ??= (async () => {
    if (!isTauriEnv()) return;
    try {
      const payload = await invoke<EditorLoadStatePayload | null>('editor_load_state');
      if (payload && Array.isArray(payload.drafts)) {
        for (const row of payload.drafts) {
          if (
            row &&
            typeof row.fileId === 'number' &&
            typeof row.content === 'string' &&
            typeof row.baseHash === 'string'
          ) {
            draftCache.set(row.fileId, { content: row.content, baseHash: row.baseHash });
          }
        }
      }
      if (typeof payload?.panelStateJson === 'string' && payload.panelStateJson.length > 0) {
        try {
          panelStateCache = JSON.parse(payload.panelStateJson) as EditorPanelStateSnapshot;
        } catch {
          panelStateCache = null;
        }
      } else {
        panelStateCache = null;
      }
      panelStateReady = true;
    } catch (err) {
      console.error('[editorStorage] editor_load_state 失败，降级 localStorage:', err);
      invokeBroken = true;
    }
  })();
  return initPromise;
}

/* —— localStorage 原语（降级路径共用） —— */

function readLocalJson<T>(key: string): T | null {
  if (typeof window === 'undefined') return null;
  try {
    const raw = window.localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : null;
  } catch {
    return null;
  }
}

function writeLocalJson(key: string, value: unknown): void {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // 存储失败不阻断
  }
}

function removeLocalKey(key: string): void {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.removeItem(key);
  } catch {
    // 同上
  }
}

function invokeOrLocal(command: string, args: Record<string, unknown>, localFallback: () => void): void {
  if (!isTauriEnv() || invokeBroken) {
    localFallback();
    return;
  }
  void invoke(command, args).catch((err) => {
    console.error(`[editorStorage] ${command} 失败，降级 localStorage:`, err);
    invokeBroken = true;
    localFallback();
  });
}

export const editorStorage: IEditorStorage = {
  loadPanelState(): EditorPanelStateSnapshot | null {
    if (isTauriEnv() && !invokeBroken && !panelStateReady) {
      // 预取未就绪：降级 localStorage（正常时序下骨架在启动早期同步读，命中缓存）
      return readLocalJson<EditorPanelStateSnapshot>(PANEL_KEY);
    }
    if (isTauriEnv() && !invokeBroken) {
      return panelStateCache;
    }
    return readLocalJson<EditorPanelStateSnapshot>(PANEL_KEY);
  },

  savePanelState(snapshot: EditorPanelStateSnapshot): void {
    panelStateCache = snapshot;
    invokeOrLocal('editor_save_panel_state', { stateJson: JSON.stringify(snapshot) }, () => {
      writeLocalJson(PANEL_KEY, snapshot);
    });
  },

  loadDraft(fileId: number): { content: string; baseHash: string } | null {
    const cached = draftCache.get(fileId);
    if (cached) return cached;
    if (!isTauriEnv() || invokeBroken || !panelStateReady) {
      return readLocalJson<{ content: string; baseHash: string }>(DRAFT_KEY_PREFIX + fileId);
    }
    return null;
  },

  saveDraft(file: EditorFileDescriptor, content: string): void {
    const baseHash = file.baseHash ?? '';
    draftCache.set(file.id, { content, baseHash });
    invokeOrLocal(
      'editor_save_draft',
      {
        fileId: file.id,
        hostId: file.hostId,
        remotePath: file.remotePath,
        baseHash,
        content,
      },
      () => {
        writeLocalJson(DRAFT_KEY_PREFIX + file.id, { content, baseHash });
      },
    );
  },

  deleteDraft(fileId: number): void {
    draftCache.delete(fileId);
    invokeOrLocal('editor_delete_draft', { fileId }, () => {
      removeLocalKey(DRAFT_KEY_PREFIX + fileId);
    });
  },

  loadViewState(fileId: number): unknown {
    // viewState 为纯 UI 状态，无需进数据库，全环境 localStorage
    return readLocalJson(VIEWSTATE_KEY_PREFIX + fileId);
  },

  saveViewState(fileId: number, viewState: unknown): void {
    writeLocalJson(VIEWSTATE_KEY_PREFIX + fileId, viewState);
  },
};

// 模块加载即自启预取（骨架若显式调用 initEditorStorage() 则复用同一 promise）
if (typeof window !== 'undefined') {
  void initEditorStorage();
}
