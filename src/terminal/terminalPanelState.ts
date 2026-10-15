import { getDbKv, setDbKv } from '@/services/dbStorage';
import { createSafeUsizeCounter, JS_MAX_SAFE_INTEGER } from '@/utils/safeUsize';

/**
 * 多 Tab 终端面板状态管理（移植自 ZCode `terminalPanelState.ts`，并增加 SQLCipher 数据库状态持久化）。
 *
 * - 每个 Tab 对应一个 `TerminalSessionDescriptor`，`id` 为 JS 安全整数范围（`[0, Number.MAX_SAFE_INTEGER]`，对应后端 `usize`）；
 * - 新建 Tab 时按现存会话的 `index` 计算最小未占用正整数（如关掉 2 再新建仍得 2）；
 * - 关闭激活中的 Tab 时自动激活相邻 Tab；
 * - 当最后一个 Tab 被关闭时，自动创建一个新会话保持终端页始终可用；
 * - 会话 Tab 元数据同步保存到 `veloterm.db` (`app_kv`)，重启/刷新后恢复多 Tab 结构。
 */

const TERMINAL_PANEL_STATE_STORAGE_KEY = 'veloterm:terminal:panel-state:v1';

/** 会话连接状态：active = 正常运行；disconnected = SSH 连接断开，等待用户重连 */
export type TerminalSessionStatus = 'active' | 'disconnected';

export interface TerminalSessionDescriptor {
  id: number;
  index: number;
  cwd?: string;
  hostId?: number;
  customTitle?: string;
  shellLabel: string | null;
  createdAt: number;
  status: TerminalSessionStatus;
  /** 断开原因（展示用，仅 disconnected 态有值） */
  disconnectReason?: string;
  /** 断开时间戳（ms，仅 disconnected 态有值） */
  disconnectedAt?: number;
  /** 重连次数（仅用于强制 remount TerminalSession，非业务语义） */
  reconnectNonce?: number;
}

export interface TerminalPanelState {
  sessions: Record<number, TerminalSessionDescriptor>;
  sessionIds: number[];
  activeSessionId: number;
}

const nextSafeSessionId = createSafeUsizeCounter(1);

function generateSessionId(existingIds?: readonly number[]): number {
  const occupied = existingIds ? new Set(existingIds) : null;
  let candidate = nextSafeSessionId();
  while (occupied?.has(candidate)) {
    candidate = nextSafeSessionId();
  }
  return candidate;
}

export function createTerminalSessionDescriptor(params: {
  index: number;
  cwd?: string;
  hostId?: number;
  customTitle?: string;
  existingIds?: readonly number[];
}): TerminalSessionDescriptor {
  return {
    id: generateSessionId(params.existingIds),
    index: params.index,
    cwd: params.cwd,
    hostId: params.hostId,
    customTitle: params.customTitle,
    shellLabel: null,
    createdAt: Date.now(),
    status: 'active',
  };
}

export function createInitialTerminalPanelState(): TerminalPanelState {
  return {
    sessions: {},
    sessionIds: [],
    activeSessionId: 0,
  };
}

/**
 * 与 ZCode `getNextTerminalSessionIndex` 一致：
 * 从现存 session 推导最小空位编号（1, 2, 3...），避免关闭 Tab 2 后新建变成 Tab 3。
 */
export function getNextTerminalSessionIndex(state: TerminalPanelState): number {
  const usedIndices = new Set(
    state.sessionIds
      .map((id) => state.sessions[id]?.index)
      .filter((index): index is number => typeof index === 'number'),
  );
  for (let index = 1; ; index += 1) {
    if (!usedIndices.has(index)) {
      return index;
    }
  }
}

export function formatShellShortLabel(shell: string | null): string | null {
  if (!shell) return null;
  const cleaned = shell.replace(/\s*\(.*\)$/, '').trim();
  const parts = cleaned.split(/[\\/]/);
  const last = parts[parts.length - 1];
  const name = last?.replace(/\.exe$/i, '').toLowerCase();
  if (!name) return shell;
  if (name === 'powershell' || name === 'pwsh') return 'PowerShell';
  return name;
}

export function addTerminalSession(
  state: TerminalPanelState,
  params?: { cwd?: string; hostId?: number; customTitle?: string },
): { state: TerminalPanelState; session: TerminalSessionDescriptor } {
  const session = createTerminalSessionDescriptor({
    index: getNextTerminalSessionIndex(state),
    cwd: params?.cwd,
    hostId: params?.hostId,
    customTitle: params?.customTitle,
    existingIds: state.sessionIds,
  });

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

/**
 * 关闭指定终端 Tab：
 * - 若还有其余 Tab，移除目标 Tab 并就近回退激活项；
 * - 若关闭的是最后一个 Tab，置为空白状态（不自动补建新终端，留空等待用户按需打开）。
 */
export function removeTerminalSession(
  state: TerminalPanelState,
  sessionId: number,
): { state: TerminalPanelState; removedSessionId: number | null } {
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

export function readStoredTerminalPanelState(): TerminalPanelState {
  if (typeof window === 'undefined') {
    return createInitialTerminalPanelState();
  }
  try {
    const raw = getDbKv(TERMINAL_PANEL_STATE_STORAGE_KEY);
    if (!raw) {
      return createInitialTerminalPanelState();
    }
    const parsed = JSON.parse(raw) as {
      sessions?: Record<string, Partial<TerminalSessionDescriptor>>;
      sessionIds?: unknown[];
      activeSessionId?: unknown;
    };
    if (
      !parsed ||
      typeof parsed !== 'object' ||
      !Array.isArray(parsed.sessionIds) ||
      typeof parsed.sessions !== 'object' ||
      !parsed.sessions
    ) {
      return createInitialTerminalPanelState();
    }

    const sessions: Record<number, TerminalSessionDescriptor> = {};
    const validSessionIds: number[] = [];

    for (const rawId of parsed.sessionIds) {
      const item =
        typeof rawId === 'number' || typeof rawId === 'string'
          ? parsed.sessions[String(rawId)]
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
      // 应用重启后 xterm 实例与后端连接均已不在，重新挂载时会发起新连接：
      // 强制 status: 'active'，不恢复断开态字段
      sessions[numericId] = {
        id: numericId,
        index: typeof item.index === 'number' && item.index >= 1 ? item.index : 1,
        cwd: typeof item.cwd === 'string' ? item.cwd : undefined,
        hostId: typeof item.hostId === 'number' ? item.hostId : undefined,
        customTitle: typeof item.customTitle === 'string' ? item.customTitle : undefined,
        shellLabel: typeof item.shellLabel === 'string' ? item.shellLabel : null,
        createdAt: typeof item.createdAt === 'number' ? item.createdAt : Date.now(),
        status: 'active',
        disconnectReason: undefined,
        disconnectedAt: undefined,
        reconnectNonce: undefined,
      };
    }

    if (validSessionIds.length === 0) {
      return createInitialTerminalPanelState();
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
    return createInitialTerminalPanelState();
  }
}

export function persistTerminalPanelState(state: TerminalPanelState): void {
  if (typeof window === 'undefined') return;
  setDbKv(TERMINAL_PANEL_STATE_STORAGE_KEY, JSON.stringify(state));
}

/**
 * 标记会话为断开态（SSH 连接断开时调用）：
 * 幂等——会话不存在或已是 disconnected 时原样返回，不更新 disconnectedAt。
 */
export function markSessionDisconnected(
  state: TerminalPanelState,
  sessionId: number,
  reason: string,
): TerminalPanelState {
  const session = state.sessions[sessionId];
  if (!session || session.status === 'disconnected') {
    return state;
  }
  return {
    ...state,
    sessions: {
      ...state.sessions,
      [sessionId]: {
        ...session,
        status: 'disconnected',
        disconnectReason: reason,
        disconnectedAt: Date.now(),
      },
    },
  };
}

/**
 * 标记会话进入重连流程：status → 'active'、清空断开信息、reconnectNonce + 1
 * （nonce 变化强制 remount TerminalSession 走全新创建路径）、激活该会话。
 */
export function markSessionReconnecting(
  state: TerminalPanelState,
  sessionId: number,
): TerminalPanelState {
  const session = state.sessions[sessionId];
  if (!session) {
    return state;
  }
  const nextSession: TerminalSessionDescriptor = {
    ...session,
    status: 'active',
    disconnectReason: undefined,
    disconnectedAt: undefined,
    reconnectNonce: (session.reconnectNonce ?? 0) + 1,
  };
  return {
    ...state,
    sessions: {
      ...state.sessions,
      [sessionId]: nextSession,
    },
    activeSessionId: sessionId,
  };
}
