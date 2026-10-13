import { getDbKv, setDbKv } from '@/services/dbStorage';
import { createSafeUsizeCounter, JS_MAX_SAFE_INTEGER } from '@/utils/safeUsize';

/**
 * SFTP 多会话 Tab 面板状态管理（对齐 TerminalPanelState 与 ZCode 风格）：
 *
 * - 每个 SFTP 会话对应一个 Tab，包含所属主机 ID、主机名称、当前路径 path 等；
 * - 切换左侧页面（如切到终端或主机）再切回时保持路径与文件状态，不重置回 '/'；
 * - 关闭 Tab 时就近激活相邻 Tab；
 * - 状态持久化到 SQLCipher 数据库 (`veloterm.db` 的 app_kv)，重启后恢复多 Tab 状态。
 */

const SFTP_PANEL_STATE_STORAGE_KEY = 'veloterm:sftp:panel-state:v1';

export interface SftpSessionDescriptor {
  id: number;
  hostId: number;
  hostName: string;
  currentPath: string;
  createdAt: number;
}

export interface SftpPanelState {
  sessions: Record<number, SftpSessionDescriptor>;
  sessionIds: number[];
  activeSessionId: number;
}

const nextSafeSftpSessionId = createSafeUsizeCounter(1);

function generateSessionId(existingIds?: readonly number[]): number {
  const occupied = existingIds ? new Set(existingIds) : null;
  let candidate = nextSafeSftpSessionId();
  while (occupied?.has(candidate)) {
    candidate = nextSafeSftpSessionId();
  }
  return candidate;
}

export function createInitialSftpPanelState(): SftpPanelState {
  return {
    sessions: {},
    sessionIds: [],
    activeSessionId: 0,
  };
}

export function addSftpSession(
  state: SftpPanelState,
  params: { hostId: number; hostName: string; initialPath?: string },
): { state: SftpPanelState; session: SftpSessionDescriptor } {
  const session: SftpSessionDescriptor = {
    id: generateSessionId(state.sessionIds),
    hostId: params.hostId,
    hostName: params.hostName,
    currentPath: params.initialPath || '/',
    createdAt: Date.now(),
  };

  return {
    session,
    state: {
      sessions: {
        ...state.sessions,
        [session.id]: session,
      },
      sessionIds: [...state.sessionIds, session.id],
      activeSessionId: session.id,
    },
  };
}

export function updateSftpSessionPath(
  state: SftpPanelState,
  sessionId: number,
  path: string,
): SftpPanelState {
  const session = state.sessions[sessionId];
  if (!session || session.currentPath === path) {
    return state;
  }
  return {
    ...state,
    sessions: {
      ...state.sessions,
      [sessionId]: {
        ...session,
        currentPath: path,
      },
    },
  };
}

export function removeSftpSession(
  state: SftpPanelState,
  sessionId: number,
): { state: SftpPanelState; removedSessionId: number | null } {
  const session = state.sessions[sessionId];
  if (!session || !state.sessionIds.includes(sessionId)) {
    return { state, removedSessionId: null };
  }

  const closingIndex = state.sessionIds.indexOf(sessionId);
  const nextSessionIds = state.sessionIds.filter((id) => id !== sessionId);
  const { [sessionId]: _closed, ...nextSessions } = state.sessions;

  if (nextSessionIds.length === 0) {
    return {
      removedSessionId: sessionId,
      state: {
        sessions: {},
        sessionIds: [],
        activeSessionId: 0,
      },
    };
  }

  const fallbackSessionId =
    nextSessionIds[Math.max(0, closingIndex - 1)] ?? nextSessionIds[0]!;

  return {
    removedSessionId: sessionId,
    state: {
      sessions: nextSessions,
      sessionIds: nextSessionIds,
      activeSessionId:
        state.activeSessionId === sessionId ? fallbackSessionId : state.activeSessionId,
    },
  };
}

export function activateSftpSession(
  state: SftpPanelState,
  sessionId: number,
): SftpPanelState {
  if (state.activeSessionId === sessionId || !state.sessions[sessionId]) {
    return state;
  }
  return {
    ...state,
    activeSessionId: sessionId,
  };
}

export function readStoredSftpPanelState(): SftpPanelState {
  if (typeof window === 'undefined') {
    return createInitialSftpPanelState();
  }

  const raw = getDbKv(SFTP_PANEL_STATE_STORAGE_KEY);
  if (!raw) {
    return createInitialSftpPanelState();
  }

  try {
    const parsed = JSON.parse(raw) as Partial<SftpPanelState>;
    if (
      !parsed ||
      typeof parsed !== 'object' ||
      !Array.isArray(parsed.sessionIds) ||
      !parsed.sessions ||
      typeof parsed.sessions !== 'object'
    ) {
      return createInitialSftpPanelState();
    }

    const sessions: Record<number, SftpSessionDescriptor> = {};
    const validSessionIds: number[] = [];

    for (const rawId of parsed.sessionIds) {
      const item =
        typeof rawId === 'number' || typeof rawId === 'string'
          ? parsed.sessions[Number(rawId)]
          : undefined;
      if (!item) continue;

      let numericId: number;
      if (
        typeof rawId === 'number' &&
        Number.isSafeInteger(rawId) &&
        rawId >= 0 &&
        rawId <= JS_MAX_SAFE_INTEGER
      ) {
        numericId = rawId;
      } else {
        numericId = generateSessionId(validSessionIds);
      }

      validSessionIds.push(numericId);
      sessions[numericId] = {
        id: numericId,
        hostId: typeof item.hostId === 'number' ? item.hostId : 0,
        hostName: typeof item.hostName === 'string' ? item.hostName : 'SFTP',
        currentPath: typeof item.currentPath === 'string' ? item.currentPath : '/',
        createdAt: typeof item.createdAt === 'number' ? item.createdAt : Date.now(),
      };
    }

    if (validSessionIds.length === 0) {
      return createInitialSftpPanelState();
    }

    const activeSessionId =
      typeof parsed.activeSessionId === 'number' && sessions[parsed.activeSessionId]
        ? parsed.activeSessionId
        : validSessionIds[0]!;

    return {
      sessions,
      sessionIds: validSessionIds,
      activeSessionId,
    };
  } catch {
    return createInitialSftpPanelState();
  }
}

export function persistSftpPanelState(state: SftpPanelState): void {
  if (typeof window === 'undefined') return;
  setDbKv(SFTP_PANEL_STATE_STORAGE_KEY, JSON.stringify(state));
}
