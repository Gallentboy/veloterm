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
  addSftpSession,
  activateSftpSession,
  persistSftpPanelState,
  readStoredSftpPanelState,
  removeSftpSession,
  renameSftpSession,
  updateSftpSessionPath,
  type SftpPanelState,
  type SftpSessionDescriptor,
} from './sftpPanelState';

export interface SftpContextValue {
  sessions: SftpSessionDescriptor[];
  activeSession: SftpSessionDescriptor | null;
  activeSessionId: number;
  createSession: (params: { hostId: number; hostName: string; initialPath?: string }) => number;
  closeSession: (sessionId: number) => void;
  activateSession: (sessionId: number) => void;
  updateSessionPath: (sessionId: number, path: string) => void;
  /** 双击 Tab 重命名（customTitle 为空回退 hostName） */
  renameSession: (sessionId: number, customTitle: string | null) => void;
}

const SftpContext = createContext<SftpContextValue | null>(null);

export function SftpProvider({ children }: { children: ReactNode }) {
  const [panelState, setPanelState] = useState<SftpPanelState>(() =>
    readStoredSftpPanelState(),
  );

  useEffect(() => {
    persistSftpPanelState(panelState);
  }, [panelState]);

  const createSession = useCallback(
    (params: { hostId: number; hostName: string; initialPath?: string }) => {
      let createdId = 0;
      setPanelState((current) => {
        const { state, session } = addSftpSession(current, params);
        createdId = session.id;
        return state;
      });
      return createdId;
    },
    [],
  );

  const closeSession = useCallback((sessionId: number) => {
    setPanelState((current) => {
      const { state } = removeSftpSession(current, sessionId);
      return state;
    });
  }, []);

  const activateSession = useCallback((sessionId: number) => {
    setPanelState((current) => activateSftpSession(current, sessionId));
  }, []);

  const updateSessionPath = useCallback((sessionId: number, path: string) => {
    setPanelState((current) => updateSftpSessionPath(current, sessionId, path));
  }, []);

  const renameSession = useCallback((sessionId: number, customTitle: string | null) => {
    setPanelState((current) => renameSftpSession(current, sessionId, customTitle));
  }, []);

  const sessions = useMemo(
    () =>
      panelState.sessionIds
        .map((id) => panelState.sessions[id])
        .filter((s): s is SftpSessionDescriptor => Boolean(s)),
    [panelState.sessionIds, panelState.sessions],
  );

  const activeSession = useMemo(
    () => sessions.find((s) => s.id === panelState.activeSessionId) ?? sessions[0] ?? null,
    [panelState.activeSessionId, sessions],
  );

  const value = useMemo<SftpContextValue>(
    () => ({
      sessions,
      activeSession,
      activeSessionId: activeSession?.id ?? 0,
      createSession,
      closeSession,
      activateSession,
      updateSessionPath,
      renameSession,
    }),
    [
      activateSession,
      activeSession,
      closeSession,
      createSession,
      renameSession,
      sessions,
      updateSessionPath,
    ],
  );

  return <SftpContext value={value}>{children}</SftpContext>;
}

export function useSftpStore(): SftpContextValue {
  const ctx = useContext(SftpContext);
  if (!ctx) {
    throw new Error('useSftpStore 必须在 SftpProvider 内使用');
  }
  return ctx;
}
