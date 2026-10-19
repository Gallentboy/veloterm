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
  /** 服务器资源监控开关（每会话独立、默认关）：开 = 该 SSH 会话底部展示监控条并后台采样；
   * 会话是运行期实体（不跨启动恢复），被控态不落库、关会话即清理 */
  resourceMonitorBySession: Record<number, boolean>;
  toggleResourceMonitor: (sessionId: number) => void;
  maximizedSessionId: number | null;
  setMaximizedSessionId: (sessionId: number | null) => void;
  isBroadcastActive: boolean;
  setIsBroadcastActive: (active: boolean) => void;
  toggleBroadcast: () => void;
  /** 终端联动目标 session id 集合：非空即联动生效（任一目标内的用户输入实时镜像到其余目标） */
  broadcastTargets: number[];
  setBroadcastTargets: (ids: number[]) => void;
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
  const [broadcastTargets, setBroadcastTargets] = useState<number[]>([]);

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

  // 服务器资源监控：每会话独立开关（默认关，仅显式开启才展示监控条并后台采样）
  const [resourceMonitorBySession, setResourceMonitorBySession] = useState<
    Record<number, boolean>
  >({});
  const toggleResourceMonitor = useCallback((sessionId: number) => {
    setResourceMonitorBySession((prev) => ({ ...prev, [sessionId]: !prev[sessionId] }));
  }, []);

  const toggleBroadcast = useCallback(() => {
    setIsBroadcastActive((prev) => !prev);
    // 关闭控制条时清空联动目标，镜像立即停止（重新打开时由控制条初始化为全选）
    setBroadcastTargets([]);
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
    setResourceMonitorBySession((prev) => {
      if (!(sessionId in prev)) return prev;
      const { [sessionId]: _closed, ...rest } = prev;
      return rest;
    });
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
      resourceMonitorBySession,
      toggleResourceMonitor,
      maximizedSessionId,
      setMaximizedSessionId,
      isBroadcastActive,
      setIsBroadcastActive,
      toggleBroadcast,
      broadcastTargets,
      setBroadcastTargets,
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
      broadcastTargets,
      layoutMode,
      markDisconnected,
      maximizedSessionId,
      reconnectSession,
      resourceMonitorBySession,
      sessions,
      setLayoutMode,
      toggleBroadcast,
      toggleLayoutMode,
      toggleResourceMonitor,
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
