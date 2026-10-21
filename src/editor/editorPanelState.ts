import { resolveLanguageId } from '@/editor/languageMap';
import type {
  EditorFileDescriptor,
  EditorFileStatus,
  EditorPanelStateSnapshot,
} from '@/editor/types';
import { createSafeUsizeCounter } from '@/utils/safeUsize';

/**
 * 编辑器面板纯函数状态模块（对齐 `src/terminal/terminalPanelState.ts` 的模式）：
 * - 每个打开文件对应一个 `EditorFileDescriptor`，`id` 为 JS 安全整数自增（对齐终端会话 id 模式）；
 * - 业务唯一键为 `hostId + remotePath`（去重由 Provider 调用 `findFileByPath` 完成）；
 * - 关闭激活中的文件时自动激活相邻文件；
 * - 持久化快照的清洗/恢复也在此完成（恢复的文件 status 一律置 'loading'）。
 */

export interface EditorPanelState {
  files: Record<number, EditorFileDescriptor>;
  fileIds: number[];
  activeFileId: number | null;
}

const nextSafeFileId = createSafeUsizeCounter(1);

/** 数字自增 id；跳过已占用值（恢复持久化面板后避免与旧 id 冲突） */
export function nextFileId(existingIds?: readonly number[]): number {
  const occupied = existingIds ? new Set(existingIds) : null;
  let candidate = nextSafeFileId();
  while (occupied?.has(candidate)) {
    candidate = nextSafeFileId();
  }
  return candidate;
}

export function createInitialEditorPanelState(): EditorPanelState {
  return {
    files: {},
    fileIds: [],
    activeFileId: null,
  };
}

/** 远端绝对路径 → basename（远端为 Unix 路径，按 `/` 切分） */
export function getRemoteFileName(remotePath: string): string {
  const parts = remotePath.split('/').filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1]! : remotePath;
}

export function findFileByPath(
  state: EditorPanelState,
  hostId: number,
  remotePath: string,
): EditorFileDescriptor | null {
  for (const id of state.fileIds) {
    const file = state.files[id];
    if (file && file.hostId === hostId && file.remotePath === remotePath) {
      return file;
    }
  }
  return null;
}

/**
 * 新增文件条目。id 由调用方预先生成并传入——保持更新器纯函数：
 * StrictMode 会双调用 setState 更新器，若在更新器内部生成随机/递增 id，
 * 两次结果不一致会导致 React 采纳的 state 与 ref 镜像分裂（id 各归各的世界）。
 */
export function addFile(
  state: EditorPanelState,
  params: {
    id: number;
    hostId: number;
    remotePath: string;
  },
): { state: EditorPanelState; file: EditorFileDescriptor } {
  const fileName = getRemoteFileName(params.remotePath);
  const file: EditorFileDescriptor = {
    id: params.id,
    hostId: params.hostId,
    remotePath: params.remotePath,
    fileName,
    languageId: resolveLanguageId(fileName),
    status: 'loading',
    baseHash: null,
    openedAt: Date.now(),
  };

  return {
    file,
    state: {
      files: {
        ...state.files,
        [file.id]: file,
      },
      fileIds: [...state.fileIds, file.id],
      activeFileId: file.id,
    },
  };
}

/** 关闭指定文件：移除目标并就近回退激活项（对齐 removeTerminalSession 行为） */
export function removeFile(
  state: EditorPanelState,
  fileId: number,
): { state: EditorPanelState; removedFileId: number | null } {
  const file = state.files[fileId];
  if (!file || !state.fileIds.includes(fileId)) {
    return { state, removedFileId: null };
  }

  const closingIndex = state.fileIds.indexOf(fileId);
  const nextFileIds = state.fileIds.filter((id) => id !== fileId);
  const { [fileId]: _closed, ...nextFiles } = state.files;

  if (nextFileIds.length === 0) {
    return {
      removedFileId: fileId,
      state: createInitialEditorPanelState(),
    };
  }

  const fallbackFileId = nextFileIds[Math.max(0, closingIndex - 1)] ?? nextFileIds[0]!;

  return {
    removedFileId: fileId,
    state: {
      files: nextFiles,
      fileIds: nextFileIds,
      activeFileId:
        state.activeFileId === fileId ? fallbackFileId : state.activeFileId,
    },
  };
}

export function activateFile(state: EditorPanelState, fileId: number): EditorPanelState {
  if (!state.files[fileId] || state.activeFileId === fileId) {
    return state;
  }
  return {
    ...state,
    activeFileId: fileId,
  };
}

/** 更新单个文件状态；`baseHash` 省略时保持原值（传 null 可显式清空） */
export function setStatus(
  state: EditorPanelState,
  fileId: number,
  status: EditorFileStatus,
  baseHash?: string | null,
): EditorPanelState {
  const file = state.files[fileId];
  if (!file) {
    return state;
  }
  const nextBaseHash = baseHash === undefined ? file.baseHash : baseHash;
  if (file.status === status && file.baseHash === nextBaseHash) {
    return state;
  }
  return {
    ...state,
    files: {
      ...state.files,
      [fileId]: {
        ...file,
        status,
        baseHash: nextBaseHash,
      },
    },
  };
}

/**
 * 内容就绪后的语言升级（文件名判不准、内容/路径判得准的场景——如 yh.conf
 * 实为 nginx 配置）：addFile 按文件名一锤定音，拉取到内容后由 Provider 调
 * resolveLanguageIdWithContent 重判并落回 descriptor（跨会话恢复一致，
 * MonacoDocumentCard.getFileModel 已有 languageId 变化即 setModelLanguage
 * 的同步逻辑）。幂等：语言相同则原样返回。
 */
export function updateFileLanguage(
  state: EditorPanelState,
  fileId: number,
  languageId: string,
): EditorPanelState {
  const file = state.files[fileId];
  if (!file || file.languageId === languageId) {
    return state;
  }
  return {
    ...state,
    files: {
      ...state.files,
      [fileId]: { ...file, languageId },
    },
  };
}

/** 把指定主机的全部非 loading 文件置为 offline（断线事件用） */
export function markHostOffline(state: EditorPanelState, hostId: number): EditorPanelState {
  let next = state;
  for (const id of state.fileIds) {
    const file = state.files[id];
    if (file && file.hostId === hostId && file.status !== 'loading') {
      next = setStatus(next, id, 'offline');
    }
  }
  return next;
}

/** 收集指定主机的文件 id（重连校验用） */
export function collectHostFileIds(state: EditorPanelState, hostId: number): number[] {
  return state.fileIds.filter((id) => state.files[id]?.hostId === hostId);
}

/**
 * 清洗持久化快照并恢复面板状态：恢复的文件 status 一律置 'loading'（内容待激活时拉取）、
 * baseHash 置 null；非法快照回退空白状态。
 */
export function restoreEditorPanelState(
  snapshot: EditorPanelStateSnapshot | null,
): EditorPanelState {
  if (!snapshot || !Array.isArray(snapshot.files) || snapshot.files.length === 0) {
    return createInitialEditorPanelState();
  }

  const files: Record<number, EditorFileDescriptor> = {};
  const fileIds: number[] = [];

  for (const raw of snapshot.files) {
    if (
      !raw ||
      typeof raw.id !== 'number' ||
      !Number.isSafeInteger(raw.id) ||
      typeof raw.hostId !== 'number' ||
      typeof raw.remotePath !== 'string' ||
      raw.remotePath.length === 0
    ) {
      continue;
    }
    if (fileIds.includes(raw.id)) {
      continue;
    }
    const fileName =
      typeof raw.fileName === 'string' && raw.fileName.length > 0
        ? raw.fileName
        : getRemoteFileName(raw.remotePath);
    files[raw.id] = {
      id: raw.id,
      hostId: raw.hostId,
      remotePath: raw.remotePath,
      fileName,
      languageId:
        typeof raw.languageId === 'string' && raw.languageId.length > 0
          ? raw.languageId
          : resolveLanguageId(fileName),
      status: 'loading',
      baseHash: null,
      openedAt: typeof raw.openedAt === 'number' ? raw.openedAt : Date.now(),
    };
    fileIds.push(raw.id);
  }

  if (fileIds.length === 0) {
    return createInitialEditorPanelState();
  }

  const activeFileId =
    typeof snapshot.activeFileId === 'number' && files[snapshot.activeFileId]
      ? snapshot.activeFileId
      : fileIds[0]!;

  return { files, fileIds, activeFileId };
}
