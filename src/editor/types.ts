/**
 * 编辑器模块契约（多 agent 并行实现的公共接口，定义于基线，实现方不得更改签名）：
 * - EditorProvider / EditorScreen（骨架 agent）消费本文件 + 桩件
 * - MonacoDocumentCard / editorStorage（实现 agent）按本契约替换桩件
 * - Rust 端命令名见 IEditorStorage 注释
 */

/** 单个编辑文件的生命周期状态 */
export type EditorFileStatus =
  /** 正在从远端拉取内容 */
  | 'loading'
  /** 与远端一致 */
  | 'clean'
  /** 有未保存修改 */
  | 'dirty'
  /** 保存中 */
  | 'saving'
  /** 连接断开：允许继续编辑，禁用保存 */
  | 'offline'
  /** 远端已变化，等待三选（覆盖远端/取远端/对比合并） */
  | 'conflict'
  /** 主机已删除或不可达：只读 */
  | 'orphaned';

export interface EditorFileDescriptor {
  id: number;
  hostId: number;
  /** 远端绝对路径（唯一业务键：hostId + remotePath） */
  remotePath: string;
  /** basename，用于标签显示 */
  fileName: string;
  /** Monaco language id（由 languageMap 解析） */
  languageId: string;
  status: EditorFileStatus;
  /** 加载时远端内容的 hash（重连/恢复时判断远端漂移的锚点）；未加载为 null */
  baseHash: string | null;
  openedAt: number;
}

export interface EditorPanelStateSnapshot {
  files: EditorFileDescriptor[];
  activeFileId: number | null;
}

/** Monaco 宿主组件契约（实现 agent 替换桩件；骨架 agent 按此消费） */
export interface MonacoDocumentCardProps {
  file: EditorFileDescriptor;
  /** null = 内容尚未加载（显示 loading） */
  content: string | null;
  /** 恢复用的 Monaco viewState（cursor/selection/scroll），可为 null */
  viewState: unknown;
  /** 非 null 时进入冲突对比模式（DiffEditor，本地 vs 远端） */
  diffRemoteContent: string | null;
  /** offline / orphaned / conflict 期间为 true */
  readOnly: boolean;
  onContentChange(content: string): void;
  /** Cmd/Ctrl+S */
  onSaveRequest(): void;
  /** viewState 上报携带 fileId：切文件瞬间上报的是旧文件的 viewState，按 id 归档不会错位 */
  onViewStateChange(fileId: number, viewState: unknown): void;
  onCursorChange(line: number, column: number): void;
  /** 注册当前激活编辑器的格式化动作句柄 */
  onRegisterFormat?(formatFn: (() => Promise<void>) | null): void;
}

/** 编辑器持久化契约（实现 agent 替换桩件）。
 *  Tauri 环境走 invoke（命令名即方法对应的 Rust 命令）；
 *  浏览器开发环境桩件内部用 localStorage 降级。 */
export interface IEditorStorage {
  loadPanelState(): EditorPanelStateSnapshot | null;
  savePanelState(snapshot: EditorPanelStateSnapshot): void;
  loadDraft(fileId: number): { content: string; baseHash: string } | null;
  saveDraft(file: EditorFileDescriptor, content: string): void;
  deleteDraft(fileId: number): void;
  loadViewState(fileId: number): unknown;
  saveViewState(fileId: number, viewState: unknown): void;
}

/** 与 Rust 侧 text_detect::MAX_EDITABLE_FILE_SIZE_BYTES 保持一致（5MB） */
export const EDITOR_MAX_FILE_SIZE_BYTES = 5 * 1024 * 1024;

/** 内容 hash（baseHash 计算用，前后端一致口径：SHA-256 hex；浏览器与 Tauri 均可用） */
export async function hashContent(content: string): Promise<string> {
  const data = new TextEncoder().encode(content);
  const digest = await crypto.subtle.digest('SHA-256', data);
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}
