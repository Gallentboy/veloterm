import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';
import {
  addTerminalSession,
  markSessionDisconnected,
  markSessionReconnecting,
  persistTerminalPanelState,
  readStoredTerminalPanelState,
  removeTerminalSession,
  type TerminalPanelState,
  type TerminalSessionDescriptor,
} from './terminalPanelState';
import { terminalSessionRegistry } from './terminalSessionRegistry';
import { getDbKv, setDbKv } from '@/services/dbStorage';
import { defaultTerminalTransport } from './transport';
import type { ITerminalTransport } from './types';

export type TerminalLayoutMode = 'tabs' | 'grid';

export interface TerminalContextValue {
  transport: ITerminalTransport;
  sessions: TerminalSessionDescriptor[];
  activeSession: TerminalSessionDescriptor | null;
  activeSessionId: number;
  layoutMode: TerminalLayoutMode;
  setLayoutMode: (mode: TerminalLayoutMode) => void;
  toggleLayoutMode: () => void;
  maximizedSessionId: number | null;
  setMaximizedSessionId: (sessionId: number | null) => void;
  isBroadcastActive: boolean;
  setIsBroadcastActive: (active: boolean) => void;
  toggleBroadcast: () => void;
  createSession: (params?: { cwd?: string; hostId?: number; customTitle?: string }) => number;
  closeSession: (sessionId: number) => void;
  activateSession: (sessionId: number) => void;
  updateShellLabel: (sessionId: number, shellLabel: string | null) => void;
  /** 双击 Tab 重命名（customTitle 为空回退 shellLabel / 默认名） */
  renameSession: (sessionId: number, customTitle: string | null) => void;
  clearSession: (sessionId: number) => void;
  /** 标记会话为断开态（SSH 连接断开/启动失败时调用，Tab 保留；断开提示由 TerminalSession 写进 xterm） */
  markDisconnected: (sessionId: number, reason: string) => void;
  /** 重连：释放旧 xterm 与后端会话，remount 后走全新 SSH 连接 */
  reconnectSession: (sessionId: number) => void;
}

const LAYOUT_MODE_STORAGE_KEY = 'veloterm:terminal:layout-mode';

const TerminalContext = createContext<TerminalContextValue | null>(null);

export function TerminalProvider({
  children,
  transport = defaultTerminalTransport,
}: {
  children: ReactNode;
  transport?: ITerminalTransport;
}) {
  const [panelState, setPanelState] = useState<TerminalPanelState>(() =>
    readStoredTerminalPanelState(),
  );

  const [layoutMode, setLayoutModeState] = useState<TerminalLayoutMode>(() => {
    const raw = getDbKv(LAYOUT_MODE_STORAGE_KEY);
    return raw === 'grid' ? 'grid' : 'tabs';
  });
  const [maximizedSessionId, setMaximizedSessionId] = useState<number | null>(null);
  const [isBroadcastActive, setIsBroadcastActive] = useState<boolean>(false);

  const setLayoutMode = useCallback((mode: TerminalLayoutMode) => {
    setLayoutModeState(mode);
    setDbKv(LAYOUT_MODE_STORAGE_KEY, mode);
  }, []);

  const toggleLayoutMode = useCallback(() => {
    setLayoutModeState((prev) => {
      const next = prev === 'grid' ? 'tabs' : 'grid';
      setDbKv(LAYOUT_MODE_STORAGE_KEY, next);
      return next;
    });
  }, []);

  const toggleBroadcast = useCallback(() => {
    setIsBroadcastActive((prev) => !prev);
  }, []);

  useEffect(() => {
    persistTerminalPanelState(panelState);
  }, [panelState]);

  const createSession = useCallback(
    (params?: { cwd?: string; hostId?: number; customTitle?: string }) => {
      let createdId = 0;
      setPanelState((current) => {
        const { state, session } = addTerminalSession(current, params);
        createdId = session.id;
        return state;
      });
      return createdId;
    },
    [],
  );

  const closeSession = useCallback((sessionId: number) => {
    // 先释放 xterm 实例与底层 Channel 会话，再更新 Tab 列表
    terminalSessionRegistry.release(sessionId);
    setMaximizedSessionId((prev) => (prev === sessionId ? null : prev));
    setPanelState((current) => {
      const { state } = removeTerminalSession(current, sessionId);
      return state;
    });
  }, []);

  const activateSession = useCallback((sessionId: number) => {
    setPanelState((current) => {
      if (!current.sessions[sessionId] || current.activeSessionId === sessionId) {
        return current;
      }
      return {
        ...current,
        activeSessionId: sessionId,
      };
    });
  }, []);

  const updateShellLabel = useCallback((sessionId: number, shellLabel: string | null) => {
    setPanelState((current) => {
      const session = current.sessions[sessionId];
      if (!session || session.shellLabel === shellLabel) {
        return current;
      }
      return {
        ...current,
        sessions: {
          ...current.sessions,
          [sessionId]: {
            ...session,
            shellLabel,
          },
        },
      };
    });
  }, []);

  // 双击 Tab 重命名：写入 customTitle（null 清除，回退 shellLabel / 默认名）
  const renameSession = useCallback((sessionId: number, customTitle: string | null) => {
    setPanelState((current) => {
      const session = current.sessions[sessionId];
      const nextTitle = customTitle ?? undefined;
      if (!session || session.customTitle === nextTitle) {
        return current;
      }
      return {
        ...current,
        sessions: {
          ...current.sessions,
          [sessionId]: {
            ...session,
            customTitle: nextTitle,
          },
        },
      };
    });
  }, []);

  const clearSession = useCallback((sessionId: number) => {
    terminalSessionRegistry.clear(sessionId);
  }, []);

  const markDisconnected = useCallback((sessionId: number, reason: string) => {
    setPanelState((current) => markSessionDisconnected(current, sessionId, reason));
  }, []);

  // 重连 = 释放旧 xterm 实例与后端会话（release 内部已处理 transport.dispose），
  // 再将状态置回 active 并 bump reconnectNonce，强制 TerminalSession remount 走全新创建路径
  const reconnectSession = useCallback((sessionId: number) => {
    terminalSessionRegistry.release(sessionId);
    setPanelState((current) => markSessionReconnecting(current, sessionId));
  }, []);

  const sessions = useMemo(
    () =>
      panelState.sessionIds
        .map((id) => panelState.sessions[id])
        .filter((s): s is TerminalSessionDescriptor => Boolean(s)),
    [panelState.sessionIds, panelState.sessions],
  );

  const activeSession = useMemo(
    () => sessions.find((s) => s.id === panelState.activeSessionId) ?? sessions[0] ?? null,
    [panelState.activeSessionId, sessions],
  );

  const value = useMemo<TerminalContextValue>(
    () => ({
      transport,
      sessions,
      activeSession,
      activeSessionId: activeSession?.id ?? 0,
      layoutMode,
      setLayoutMode,
      toggleLayoutMode,
      maximizedSessionId,
      setMaximizedSessionId,
      isBroadcastActive,
      setIsBroadcastActive,
      toggleBroadcast,
      createSession,
      closeSession,
      activateSession,
      updateShellLabel,
      clearSession,
      renameSession,
      markDisconnected,
      reconnectSession,
    }),
    [
      activateSession,
      activeSession,
      clearSession,
      closeSession,
      createSession,
      isBroadcastActive,
      layoutMode,
      markDisconnected,
      maximizedSessionId,
      reconnectSession,
      sessions,
      setLayoutMode,
      toggleBroadcast,
      toggleLayoutMode,
      transport,
      updateShellLabel,
      renameSession,
    ],
  );

  return <TerminalContext value={value}>{children}</TerminalContext>;
}

export function useTerminalStore(): TerminalContextValue {
  const ctx = useContext(TerminalContext);
  if (!ctx) {
    throw new Error('useTerminalStore 必须在 TerminalProvider 内使用');
  }
  return ctx;
}
