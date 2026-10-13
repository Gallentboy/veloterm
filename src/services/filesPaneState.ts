import { getDbKv, setDbKv } from '@/services/dbStorage';
import { JS_MAX_SAFE_INTEGER } from '@/utils/safeUsize';

/**
 * 文件页双栏（Dual-Pane）布局状态管理（对齐 sftpPanelState 的持久化风格）：
 *
 * - 左右两个窗格各自可绑定一个 SFTP 会话（聚焦窗格 + Tab 池模型）：
 *   顶部 Tab 栏管理全部会话，点击/激活的会话被分配到「当前聚焦的窗格」展示，
 *   其余会话保留在隐藏 Keep-alive 池中，目录状态不丢失；
 * - `focusedPane` 记录当前聚焦侧（键盘 ⌘W、⌘⇧[ ] 与新会话落点均以它为目标）；
 * - 左栏宽度独立持久化，拖拽中缝或键盘微调后写回；
 * - 状态持久化到 SQLCipher 数据库 (`veloterm.db` 的 app_kv)，重启后恢复双栏布局；
 * - 读取时对未知/损坏数据做防御性校验，非法形状一律回退到默认值。
 */

const FILES_PANE_STATE_STORAGE_KEY = 'veloterm:files:pane-state:v1';
const FILES_PANE_WIDTH_STORAGE_KEY = 'veloterm:files:pane-width:v1';

/** 窗格侧别 */
export type FilesPaneSide = 'left' | 'right';

/** 双栏窗格分配状态（会话 ID 均指向 SftpPanelState 中的会话） */
export interface FilesPaneState {
  leftSessionId: number | null;
  rightSessionId: number | null;
  focusedPane: FilesPaneSide;
}

/** 左右窗格的最小宽度（拖拽/键盘调整时双侧都不允许低于该值）。 */
export const FILES_PANE_MIN_WIDTH_PX = 280;

export function createInitialFilesPaneState(): FilesPaneState {
  return { leftSessionId: null, rightSessionId: null, focusedPane: 'left' };
}

/** 校验持久化数据中的窗格会话 ID：仅接受安全范围内的非负整数，否则视为空槽。 */
function parsePaneSessionId(value: unknown): number | null {
  if (
    typeof value === 'number' &&
    Number.isSafeInteger(value) &&
    value >= 0 &&
    value <= JS_MAX_SAFE_INTEGER
  ) {
    return value;
  }
  return null;
}

export function readStoredFilesPaneState(): FilesPaneState {
  if (typeof window === 'undefined') {
    return createInitialFilesPaneState();
  }

  const raw = getDbKv(FILES_PANE_STATE_STORAGE_KEY);
  if (!raw) {
    return createInitialFilesPaneState();
  }

  try {
    const parsed = JSON.parse(raw) as Partial<FilesPaneState>;
    if (!parsed || typeof parsed !== 'object') {
      return createInitialFilesPaneState();
    }

    return {
      leftSessionId: parsePaneSessionId(parsed.leftSessionId),
      rightSessionId: parsePaneSessionId(parsed.rightSessionId),
      // 未知聚焦侧回退到左侧
      focusedPane: parsed.focusedPane === 'right' ? 'right' : 'left',
    };
  } catch {
    return createInitialFilesPaneState();
  }
}

export function persistFilesPaneState(state: FilesPaneState): void {
  if (typeof window === 'undefined') return;
  setDbKv(FILES_PANE_STATE_STORAGE_KEY, JSON.stringify(state));
}

/**
 * 读取持久化的左栏宽度：
 * 仅接受有限数值并夹取到 ≥ FILES_PANE_MIN_WIDTH_PX，非法/缺失时返回 null
 * （null 表示用户未自定义，界面回退到左右各半的弹性布局）。
 */
export function readStoredLeftPaneWidth(): number | null {
  if (typeof window === 'undefined') return null;

  const raw = getDbKv(FILES_PANE_WIDTH_STORAGE_KEY);
  if (!raw) return null;

  const parsed = Number(raw);
  if (!Number.isFinite(parsed)) return null;

  return Math.max(FILES_PANE_MIN_WIDTH_PX, Math.round(parsed));
}

export function persistLeftPaneWidth(px: number): void {
  if (typeof window === 'undefined') return;
  setDbKv(FILES_PANE_WIDTH_STORAGE_KEY, String(Math.round(px)));
}
