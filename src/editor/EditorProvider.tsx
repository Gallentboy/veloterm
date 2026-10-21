import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from 'react';
import { computeBaseHash, loadRemoteContent, saveRemoteContent } from '@/editor/contentLoader';
import { resolveLanguageIdWithContent } from '@/editor/languageMap';
import {
  activateFile as activateFileState,
  addFile,
  nextFileId,
  collectHostFileIds,
  findFileByPath,
  markHostOffline,
  removeFile,
  restoreEditorPanelState,
  setStatus,
  updateFileLanguage,
  type EditorPanelState,
} from '@/editor/editorPanelState';
import { editorStorage } from '@/services/editorStorage';
import { info as logInfo } from '@/utils/logger';
import type { EditorFileDescriptor } from '@/editor/types';

/** 编辑器链路轨迹日志（统一前缀，便于日志侧过滤定位懒加载问题） */
function logEditorTrace(message: string): void {
  void logInfo(`[Editor] ${message}`);
}

/** 草稿落盘防抖（onContentChange 后静默 800ms） */
const DRAFT_SAVE_DEBOUNCE_MS = 800;
/** 面板结构持久化防抖 */
const PANEL_SAVE_DEBOUNCE_MS = 500;

export type EditorConflictAction = 'overwrite' | 'take-remote';

export interface EditorContextValue {
  files: EditorFileDescriptor[];
  activeFile: EditorFileDescriptor | null;
  activeFileId: number | null;
  /** 每文件内容缓存（id → 已加载内容；缺失表示尚未加载） */
  contents: Map<number, string>;
  /** 每文件 Monaco viewState 缓存（id → viewState） */
  viewStates: Map<number, unknown>;
  /** conflict 状态文件暂存的远端内容（id → remote content） */
  conflictRemotes: Map<number, string>;
  /** 最近一次保存失败的错误信息（id → message，供状态栏显示） */
  saveErrors: Map<number, string>;
  /** 内容加载失败原因（id → message，非空时编辑页显示错误面板而非转圈） */
  loadErrors: Map<number, string>;
  /** 按 hostId + remotePath 去重打开；返回文件 id */
  openFile(params: { hostId: number; remotePath: string }): number;
  closeFile(id: number): void;
  activateFile(id: number): void;
  saveFile(id: number): Promise<void>;
  reloadFile(id: number): Promise<void>;
  resolveConflict(id: number, action: EditorConflictAction): Promise<void>;
  handleContentChange(id: number, content: string): void;
  handleViewStateChange(id: number, viewState: unknown): void;
}

const EditorContext = createContext<EditorContextValue | null>(null);

/** 断线/重连事件负载（当前无人派发，监听供将来 SFTP 层使用） */
interface SftpHostEventDetail {
  hostId: number;
}

function readHostEventId(event: Event): number | null {
  const detail = (event as CustomEvent<SftpHostEventDetail>).detail;
  return detail && typeof detail.hostId === 'number' ? detail.hostId : null;
}

function toErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return typeof error === 'string' ? error : String(error);
}

/**
 * 编辑器全局 Provider：
 * - 面板状态（多文件 Tab）+ 内容/viewState/冲突远端内容/保存错误四类缓存；
 * - 内容懒加载（激活且无缓存时拉取），草稿与面板结构防抖持久化；
 * - 启动恢复面板（status 一律 'loading'，激活时重新拉取校验）；
 * - 监听 veloterm:sftp-disconnected / veloterm:sftp-reconnected 维护 offline 状态。
 */
export function EditorProvider({ children }: { children: ReactNode }) {
  const [panelState, setPanelState] = useState<EditorPanelState>(() =>
    restoreEditorPanelState(editorStorage.loadPanelState()),
  );
  const panelStateRef = useRef(panelState);

  const [contents, setContents] = useState<Map<number, string>>(() => new Map());
  const contentsRef = useRef(contents);
  const [viewStates, setViewStates] = useState<Map<number, unknown>>(() => new Map());
  const viewStatesRef = useRef(viewStates);
  const [conflictRemotes, setConflictRemotes] = useState<Map<number, string>>(() => new Map());
  const conflictRemotesRef = useRef(conflictRemotes);
  const [saveErrors, setSaveErrors] = useState<Map<number, string>>(() => new Map());
  const saveErrorsRef = useRef(saveErrors);
  /** 内容加载失败原因（id → message；供编辑页错误面板展示，区别于加载中的转圈） */
  const [loadErrors, setLoadErrors] = useState<Map<number, string>>(() => new Map());
  const loadErrorsRef = useRef(loadErrors);

  const draftTimersRef = useRef(new Map<number, ReturnType<typeof setTimeout>>());
  const fetchInFlightRef = useRef(new Set<number>());

  /** setPanelState + 同步刷新镜像 ref（异步流程读取最新值，避免闭包过期） */
  const applyPanelState = useCallback(
    (updater: (current: EditorPanelState) => EditorPanelState) => {
      setPanelState((current) => {
        const next = updater(current);
        panelStateRef.current = next;
        return next;
      });
    },
    [],
  );

  /** 拷贝式更新缓存 Map（同步刷新镜像 ref 并触发重渲）；function 表达式避免 tsx 泛型歧义 */
  const mutateMap = useCallback(
    function <T>(
      ref: RefObject<Map<number, T>>,
      setState: (next: Map<number, T>) => void,
      mutate: (map: Map<number, T>) => void,
    ) {
      const next = new Map(ref.current);
      mutate(next);
      ref.current = next;
      setState(next);
    },
    [],
  );

  const updateContents = useCallback(
    (mutate: (map: Map<number, string>) => void) =>
      mutateMap(contentsRef, setContents, mutate),
    [mutateMap],
  );
  const updateViewStates = useCallback(
    (mutate: (map: Map<number, unknown>) => void) =>
      mutateMap(viewStatesRef, setViewStates, mutate),
    [mutateMap],
  );
  const updateConflictRemotes = useCallback(
    (mutate: (map: Map<number, string>) => void) =>
      mutateMap(conflictRemotesRef, setConflictRemotes, mutate),
    [mutateMap],
  );
  const updateSaveErrors = useCallback(
    (mutate: (map: Map<number, string>) => void) =>
      mutateMap(saveErrorsRef, setSaveErrors, mutate),
    [mutateMap],
  );
  const updateLoadErrors = useCallback(
    (mutate: (map: Map<number, string>) => void) =>
      mutateMap(loadErrorsRef, setLoadErrors, mutate),
    [mutateMap],
  );

  /** 草稿防抖落盘（每文件一个定时器，写入前再校验文件与内容仍存在） */
  const scheduleDraftSave = useCallback((id: number) => {
    const timers = draftTimersRef.current;
    const existing = timers.get(id);
    if (existing) {
      clearTimeout(existing);
    }
    timers.set(
      id,
      setTimeout(() => {
        timers.delete(id);
        const file = panelStateRef.current.files[id];
        const content = contentsRef.current.get(id);
        if (!file || content === undefined) {
          return;
        }
        editorStorage.saveDraft(file, content);
      }, DRAFT_SAVE_DEBOUNCE_MS),
    );
  }, []);

  /**
   * 内容就绪后的一次性语言升级：addFile 按文件名判语言（yh.conf → ini），
   * 内容到位后按内容+路径重判（实为 nginx 则升级 descriptor——getFileModel
   * 监听 languageId 变化自动 setModelLanguage）。幂等：结果相同不更新。
   */
  const maybeUpgradeLanguage = useCallback(
    (id: number, content: string) => {
      const file = panelStateRef.current.files[id];
      if (!file) {
        return;
      }
      const upgraded = resolveLanguageIdWithContent(file.fileName, content, file.remotePath);
      if (upgraded !== file.languageId) {
        logEditorTrace(
          `language upgrade id=${id} ${file.languageId} -> ${upgraded} (${file.remotePath})`,
        );
        applyPanelState((current) => updateFileLanguage(current, id, upgraded));
      }
    },
    [applyPanelState],
  );

  /**
   * 拉取远端内容并按草稿情况落状态：
   * - 草稿存在且 baseHash 与远端一致 → 用草稿，dirty；
   * - 草稿存在但 hash 不符 → 本地草稿为当前内容、远端内容暂存，conflict；
   * - 无草稿 → clean，baseHash = hash(远端内容)；
   * - 拉取失败/浏览器环境 → orphaned（只读）。
   */
  const fetchFileContent = useCallback(
    async (id: number) => {
      const file = panelStateRef.current.files[id];
      if (!file || fetchInFlightRef.current.has(id)) {
        logEditorTrace(
          `fetch skipped id=${id} fileMissing=${!file} inFlight=${fetchInFlightRef.current.has(id)} stateIds=${Object.keys(panelStateRef.current.files).join(',')} refActive=${panelStateRef.current.activeFileId}`,
        );
        return;
      }
      fetchInFlightRef.current.add(id);
      logEditorTrace(`fetch start id=${id} path=${file.remotePath}`);
      applyPanelState((current) => setStatus(current, id, 'loading'));
      try {
        const result = await loadRemoteContent(file.hostId, file.remotePath);
        if (!result) {
          throw new Error('remote content unavailable');
        }
        const remoteHash = await computeBaseHash(result.content);
        if (!panelStateRef.current.files[id]) {
          return;
        }
        const draft = editorStorage.loadDraft(id);
        if (draft && typeof draft.content === 'string' && draft.baseHash === remoteHash) {
          updateContents((map) => map.set(id, draft.content));
          applyPanelState((current) => setStatus(current, id, 'dirty', remoteHash));
        } else if (draft && typeof draft.content === 'string') {
          updateContents((map) => map.set(id, draft.content));
          updateConflictRemotes((map) => map.set(id, result.content));
          applyPanelState((current) => setStatus(current, id, 'conflict', remoteHash));
        } else {
          updateContents((map) => map.set(id, result.content));
          updateConflictRemotes((map) => map.delete(id));
          applyPanelState((current) => setStatus(current, id, 'clean', remoteHash));
        }
        // 内容就绪后的语言升级（三分支同接：草稿/冲突分支的内容同样代表文件形态）
        maybeUpgradeLanguage(id, contentsRef.current.get(id) ?? result.content);
        updateLoadErrors((map) => map.delete(id));
      } catch (error) {
        logEditorTrace(`fetch failed id=${id}: ${String(error)}`);
        if (!panelStateRef.current.files[id]) {
          return;
        }
        updateContents((map) => map.delete(id));
        updateConflictRemotes((map) => map.delete(id));
        // 记录失败原因供错误面板展示（无权限 / 非文本 / 超大小 / 网络断开等），
        // 避免 content=null 的文件在编辑页永久转圈且无从得知原因
        updateLoadErrors((map) => map.set(id, toErrorMessage(error)));
        applyPanelState((current) => setStatus(current, id, 'orphaned'));
      } finally {
        fetchInFlightRef.current.delete(id);
      }
    },
    [applyPanelState, maybeUpgradeLanguage, updateConflictRemotes, updateContents],
  );

  // 面板结构持久化（防抖）：files / activeFileId 变化后落快照
  useEffect(() => {
    const timer = setTimeout(() => {
      editorStorage.savePanelState({
        files: panelState.fileIds
          .map((id) => panelState.files[id])
          .filter((file): file is EditorFileDescriptor => Boolean(file)),
        activeFileId: panelState.activeFileId,
      });
    }, PANEL_SAVE_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [panelState]);

  // 激活文件时：懒加载内容 + 从持久层补 viewState
  useEffect(() => {
    const id = panelState.activeFileId;
    if (id === null) {
      return;
    }
    const file = panelState.files[id];
    if (!file) {
      return;
    }
    if (!viewStatesRef.current.has(id)) {
      const stored = editorStorage.loadViewState(id);
      updateViewStates((map) => map.set(id, stored));
    }
    logEditorTrace(
      `activate-effect id=${id} status=${file.status} hasContent=${contentsRef.current.has(id)} inFlight=${fetchInFlightRef.current.has(id)} refHasFile=${panelStateRef.current.files[id] !== undefined}`,
    );
    if (file.status === 'loading' && !contentsRef.current.has(id)) {
      void fetchFileContent(id);
    }
  }, [panelState, fetchFileContent, updateViewStates]);

  // 断线：该主机非 loading 文件置 offline；重连：该主机文件逐个 reload 校验
  useEffect(() => {
    const onDisconnected = (event: Event) => {
      const hostId = readHostEventId(event);
      if (hostId === null) {
        return;
      }
      applyPanelState((current) => markHostOffline(current, hostId));
    };
    const onReconnected = (event: Event) => {
      const hostId = readHostEventId(event);
      if (hostId === null) {
        return;
      }
      for (const id of collectHostFileIds(panelStateRef.current, hostId)) {
        void fetchFileContent(id);
      }
    };
    window.addEventListener('veloterm:sftp-disconnected', onDisconnected);
    window.addEventListener('veloterm:sftp-reconnected', onReconnected);
    return () => {
      window.removeEventListener('veloterm:sftp-disconnected', onDisconnected);
      window.removeEventListener('veloterm:sftp-reconnected', onReconnected);
    };
  }, [applyPanelState, fetchFileContent]);

  // 卸载时清空草稿防抖定时器
  useEffect(() => {
    const timers = draftTimersRef.current;
    return () => {
      for (const timer of timers.values()) {
        clearTimeout(timer);
      }
      timers.clear();
    };
  }, []);

  const openFile = useCallback(
    (params: { hostId: number; remotePath: string }): number => {
      logEditorTrace(`openFile host=${params.hostId} path=${params.remotePath}`);
      const existing = findFileByPath(panelStateRef.current, params.hostId, params.remotePath);
      if (existing) {
        applyPanelState((current) => activateFileState(current, existing.id));
        return existing.id;
      }
      // id 在更新器外预生成：保持更新器纯函数（StrictMode 双调用下结果一致，
      // 避免 state 与 ref 镜像按不同 id 分裂——曾致点编辑不触发读取、点标签才读）
      const newId = nextFileId(panelStateRef.current.fileIds);
      applyPanelState((current) => {
        const { state } = addFile(current, {
          id: newId,
          hostId: params.hostId,
          remotePath: params.remotePath,
        });
        return state;
      });
      return newId;
    },
    [applyPanelState],
  );

  const closeFile = useCallback(
    (id: number) => {
      const file = panelStateRef.current.files[id];
      if (!file) {
        return;
      }
      const content = contentsRef.current.get(id);
      // 干净文件清草稿；脏文件立即落草稿（不等防抖）
      if (content !== undefined) {
        if (file.status === 'clean') {
          editorStorage.deleteDraft(id);
        } else {
          editorStorage.saveDraft(file, content);
        }
      }
      const timer = draftTimersRef.current.get(id);
      if (timer) {
        clearTimeout(timer);
        draftTimersRef.current.delete(id);
      }
      updateContents((map) => map.delete(id));
      updateViewStates((map) => map.delete(id));
      updateConflictRemotes((map) => map.delete(id));
      updateSaveErrors((map) => map.delete(id));
      updateLoadErrors((map) => map.delete(id));
      applyPanelState((current) => removeFile(current, id).state);
    },
    [
      applyPanelState,
      updateConflictRemotes,
      updateContents,
      updateLoadErrors,
      updateSaveErrors,
      updateViewStates,
    ],
  );

  const activateFile = useCallback(
    (id: number) => {
      applyPanelState((current) => activateFileState(current, id));
    },
    [applyPanelState],
  );

  const saveFile = useCallback(
    async (id: number) => {
      const file = panelStateRef.current.files[id];
      if (!file) {
        return;
      }
      const content = contentsRef.current.get(id);
      if (content === undefined) {
        return;
      }
      if (file.status !== 'clean' && file.status !== 'dirty') {
        // loading / saving / offline / orphaned / conflict 不直接保存
        return;
      }
      applyPanelState((current) => setStatus(current, id, 'saving'));
      try {
        await saveRemoteContent(file.hostId, file.remotePath, content);
        const savedHash = await computeBaseHash(content);
        if (!panelStateRef.current.files[id]) {
          return;
        }
        const currentContent = contentsRef.current.get(id);
        const isStillSnapshot = currentContent === content;
        editorStorage.deleteDraft(id);
        updateSaveErrors((map) => map.delete(id));
        applyPanelState((current) =>
          setStatus(current, id, isStillSnapshot ? 'clean' : 'dirty', savedHash),
        );
        if (!isStillSnapshot && currentContent !== undefined) {
          // 保存期间又有编辑：立即补一次草稿，避免丢改动
          editorStorage.saveDraft({ ...file, baseHash: savedHash }, currentContent);
        }
      } catch (error) {
        if (!panelStateRef.current.files[id]) {
          return;
        }
        applyPanelState((current) => setStatus(current, id, 'dirty'));
        updateSaveErrors((map) => map.set(id, toErrorMessage(error)));
      }
    },
    [applyPanelState, updateSaveErrors],
  );

  const reloadFile = useCallback(
    async (id: number) => {
      const file = panelStateRef.current.files[id];
      if (!file || file.status === 'saving') {
        return;
      }
      await fetchFileContent(id);
    },
    [fetchFileContent],
  );

  const resolveConflict = useCallback(
    async (id: number, action: EditorConflictAction) => {
      const file = panelStateRef.current.files[id];
      if (!file || file.status !== 'conflict') {
        return;
      }
      if (action === 'overwrite') {
        const content = contentsRef.current.get(id);
        if (content === undefined) {
          return;
        }
        applyPanelState((current) => setStatus(current, id, 'saving'));
        try {
          await saveRemoteContent(file.hostId, file.remotePath, content);
          if (!panelStateRef.current.files[id]) {
            return;
          }
          editorStorage.deleteDraft(id);
          updateSaveErrors((map) => map.delete(id));
          await fetchFileContent(id);
        } catch (error) {
          if (!panelStateRef.current.files[id]) {
            return;
          }
          applyPanelState((current) => setStatus(current, id, 'conflict'));
          updateSaveErrors((map) => map.set(id, toErrorMessage(error)));
        }
      } else {
        // take-remote：丢弃本地草稿后重载
        editorStorage.deleteDraft(id);
        updateConflictRemotes((map) => map.delete(id));
        updateSaveErrors((map) => map.delete(id));
        await fetchFileContent(id);
      }
    },
    [applyPanelState, fetchFileContent, updateConflictRemotes, updateSaveErrors],
  );

  const handleContentChange = useCallback(
    (id: number, content: string) => {
      const file = panelStateRef.current.files[id];
      if (!file) {
        return;
      }
      updateContents((map) => map.set(id, content));
      updateSaveErrors((map) => map.delete(id));
      if (file.status === 'clean') {
        applyPanelState((current) => setStatus(current, id, 'dirty'));
      }
      scheduleDraftSave(id);
    },
    [applyPanelState, scheduleDraftSave, updateContents, updateSaveErrors],
  );

  const handleViewStateChange = useCallback(
    (id: number, viewState: unknown) => {
      if (!panelStateRef.current.files[id]) {
        return;
      }
      updateViewStates((map) => map.set(id, viewState));
      editorStorage.saveViewState(id, viewState);
    },
    [updateViewStates],
  );

  const files = useMemo(
    () =>
      panelState.fileIds
        .map((id) => panelState.files[id])
        .filter((file): file is EditorFileDescriptor => Boolean(file)),
    [panelState.fileIds, panelState.files],
  );

  const activeFile = useMemo(
    () => files.find((file) => file.id === panelState.activeFileId) ?? null,
    [files, panelState.activeFileId],
  );

  const value = useMemo<EditorContextValue>(
    () => ({
      files,
      activeFile,
      activeFileId: activeFile?.id ?? null,
      contents,
      viewStates,
      conflictRemotes,
      saveErrors,
      loadErrors,
      openFile,
      closeFile,
      activateFile,
      saveFile,
      reloadFile,
      resolveConflict,
      handleContentChange,
      handleViewStateChange,
    }),
    [
      activateFile,
      activeFile,
      closeFile,
      conflictRemotes,
      contents,
      files,
      handleContentChange,
      handleViewStateChange,
      loadErrors,
      openFile,
      reloadFile,
      resolveConflict,
      saveErrors,
      saveFile,
      viewStates,
    ],
  );

  return <EditorContext value={value}>{children}</EditorContext>;
}

export function useEditorStore(): EditorContextValue {
  const ctx = useContext(EditorContext);
  if (!ctx) {
    throw new Error('useEditorStore 必须在 EditorProvider 内使用');
  }
  return ctx;
}
