/**
 * 统一拖放总线：在 `dragDropEnabled: true`（原生接管，见 tauri.conf.json）下，
 * 把**外部文件拖入**（原生 `tauri://drag-*` 事件，带绝对路径）与**页内拖拽**
 * （HTML5 dragstart 存载荷 + 原生 Drop 事件按坐标命中）统一成一条管线。
 *
 * # 为什么需要统一管线
 *
 * wry 开启原生拖放后会整体接管 WebView 的 DnD（Windows 侧 RevokeDragDrop
 * 替换 WebView2 的 drop target；macOS 侧 WKWebView 子类拦截 dragging 回调），
 * 页内 HTML5 dragover/drop 不再触发——SFTP 面板互拷、Tab 分栏拖拽因此不能
 * 再依赖 dataTransfer 读取，改为：dragstart 时把载荷存进本总线，Drop 事件
 * 到达时按命中元素分发。外部文件拖入同理（载荷即 paths）。这与 Tauri 上游
 * 「统一内外拖拽管线」的方向一致（tauri-apps/tauri#14373）。
 *
 * # 事件流
 *
 * - 源侧（dragstart）：`beginInternalDrag(kind, payload)`；dragend → `endInternalDrag()`；
 * - 原生 Enter/Over：按当前拖拽种类（files / 内部 kind）命中
 *   `elementFromPoint` 下最深的目标元素，回调 onHoverChange 做高亮；
 * - 原生 Drop：命中则回调 onDrop（files 带 paths；内部带 dragstart 存的载荷）；
 * - 原生 Leave：清除高亮（内部载荷保留到 drop/dragend 清理）。
 */

import { getCurrentWebview } from '@tauri-apps/api/webview';
import { platformKind } from '@/lib/platform';

/** SFTP 远端文件条目拖拽载荷（行 → 对侧面板复制） */
export interface SftpItemDragPayload {
  sessionId: string;
  path: string;
  name: string;
  isDir: boolean;
}

/** SFTP Tab 拖拽载荷（池中 Tab → 窗格分栏） */
export interface SftpTabDragPayload {
  sessionId: number;
}

/** 页内拖拽种类与其载荷的对应关系 */
export interface InternalDragPayloadMap {
  'sftp-item': SftpItemDragPayload;
  'sftp-tab': SftpTabDragPayload;
}

export type InternalDragKind = keyof InternalDragPayloadMap;

/** 目标可接受的拖拽种类：外部文件（files）或某种页内拖拽 */
export type AcceptedDragKind = 'files' | InternalDragKind;

export interface DragPosition {
  x: number;
  y: number;
}

export type DropEvent =
  | { kind: 'files'; paths: string[]; position: DragPosition }
  /** 页内拖拽：payload 为 beginInternalDrag 存入的载荷，消费方按 kind 收窄 */
  | { kind: InternalDragKind; payload: unknown; position: DragPosition };

export interface DropTargetRegistration {
  accepts: AcceptedDragKind[];
  /** 目标元素（ref 可能尚未挂载，惰性获取；返回 null 视为不可命中） */
  getElement: () => HTMLElement | null;
  onDrop: (event: DropEvent) => void;
  /** 命中/离开目标时回调（供高亮）；position 为最后一次 Over 的 CSS 坐标 */
  onHoverChange?: (hovered: boolean, position?: DragPosition) => void;
}

interface Registration {
  accepts: Set<string>;
  getElement: () => HTMLElement | null;
  onDrop: (event: DropEvent) => void;
  onHoverChange?: (hovered: boolean, position?: DragPosition) => void;
}

/** 当前页内拖拽状态（dragstart 写入，drop/dragend 清空） */
let internalDrag: { kind: InternalDragKind; payload: unknown } | null = null;

const targets = new Map<string, Registration>();
let registrationSeq = 0;
let hoveredTargetId: string | null = null;
/** 原生 Over 的最近一次 CSS 坐标（onHoverChange 附带） */
let lastOverPosition: DragPosition | null = null;
let bridgeStarted = false;
/** 会话内拖拽种类记忆：Enter 带确立，Over/Drop 复用（Over 事件不携带 paths，
 * 不能每事件独立推导——否则文件拖拽的 Over 阶段会丢失 kind） */
let activeDragKind: string | null = null;

/** 是否运行在 Tauri WebView 里（纯浏览器 dev 下原生事件不存在，桥接空转） */
function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** dragstart：登记页内拖拽载荷（native Drop 事件不带自定义 MIME，靠这里取回） */
export function beginInternalDrag<K extends InternalDragKind>(
  kind: K,
  payload: InternalDragPayloadMap[K],
): void {
  internalDrag = { kind, payload };
}

/** dragend / Drop 后：清空页内拖拽载荷（幂等） */
export function endInternalDrag(): void {
  internalDrag = null;
}

/** 注册拖放目标，返回反注册函数（组件卸载时调用） */
export function registerDropTarget(registration: DropTargetRegistration): () => void {
  const id = `drop-target-${++registrationSeq}`;
  targets.set(id, { ...registration, accepts: new Set(registration.accepts) });
  ensureDragDropBridge();
  return () => {
    targets.delete(id);
    if (hoveredTargetId === id) {
      hoveredTargetId = null;
    }
  };
}

/** wry 的 position 单位随平台不同：Windows 为物理像素（ScreenToClient），macOS 为逻辑点 */
function toCssPosition(x: number, y: number): DragPosition {
  if (platformKind === 'windows') {
    const dpr = window.devicePixelRatio || 1;
    return { x: x / dpr, y: y / dpr };
  }
  return { x, y };
}

/** 当前活跃拖拽种类：外部文件（paths 非空）或页内拖拽（dragstart 登记） */
function currentDragKind(paths: string[] | null): string | null {
  if (paths && paths.length > 0) return 'files';
  return internalDrag?.kind ?? null;
}

/**
 * 命中测试：取 `elementFromPoint` 下包含命中点的注册目标里**最内层**的一个
 * （面积最小），且其 accepts 须包含当前拖拽种类。
 */
function resolveTarget(
  kind: string,
  position: DragPosition,
): { id: string; registration: Registration } | null {
  const hit = document.elementFromPoint(position.x, position.y);
  if (!hit) return null;
  let best: { id: string; registration: Registration; area: number } | null = null;
  for (const [id, registration] of targets) {
    const el = registration.getElement();
    if (!el || el.isConnected === false) continue;
    if (!registration.accepts.has(kind)) continue;
    if (el !== hit && !el.contains(hit)) continue;
    const rect = el.getBoundingClientRect();
    const area = rect.width * rect.height;
    if (!best || area < best.area) best = { id, registration, area };
  }
  return best ? { id: best.id, registration: best.registration } : null;
}

function clearHover(): void {
  if (hoveredTargetId === null) return;
  const registration = targets.get(hoveredTargetId);
  hoveredTargetId = null;
  registration?.onHoverChange?.(false, lastOverPosition ?? undefined);
}

function updateHover(kind: string, position: DragPosition): void {
  const next = resolveTarget(kind, position);
  if (next?.id === hoveredTargetId) return;
  clearHover();
  if (next) {
    hoveredTargetId = next.id;
    next.registration.onHoverChange?.(true, position);
  }
}

/** 订阅原生拖放事件（首个目标注册时惰性启动，进程内只订阅一次） */
function ensureDragDropBridge(): void {
  if (bridgeStarted || !isTauriRuntime()) return;
  bridgeStarted = true;

  void getCurrentWebview().onDragDropEvent((event) => {
    const payload = event.payload;
    switch (payload.type) {
      case 'enter': {
        activeDragKind = currentDragKind(payload.paths);
        if (!activeDragKind) return;
        lastOverPosition = toCssPosition(payload.position.x, payload.position.y);
        updateHover(activeDragKind, lastOverPosition);
        return;
      }
      case 'over': {
        if (!activeDragKind) return;
        lastOverPosition = toCssPosition(payload.position.x, payload.position.y);
        updateHover(activeDragKind, lastOverPosition);
        return;
      }
      case 'drop': {
        const kind = currentDragKind(payload.paths) ?? activeDragKind;
        const position = toCssPosition(payload.position.x, payload.position.y);
        lastOverPosition = position;
        activeDragKind = null;
        if (!kind) {
          clearHover();
          return;
        }
        const target = resolveTarget(kind, position);
        clearHover();
        if (kind === 'files') {
          target?.registration.onDrop({
            kind: 'files',
            paths: payload.paths,
            position,
          });
        } else if (internalDrag) {
          target?.registration.onDrop({
            kind: internalDrag.kind,
            payload: internalDrag.payload,
            position,
          });
        }
        // Drop 即拖拽会话结束：页内载荷立即清空（dragend 仍会再次触发 endInternalDrag，幂等）
        internalDrag = null;
        lastOverPosition = null;
        return;
      }
      case 'leave': {
        // 仅清除高亮与会话记忆（拖出窗口后可能再拖回，Enter 会重新确立；
        // 页内载荷保留，dragend 才是源侧会话终点）
        clearHover();
        activeDragKind = null;
        lastOverPosition = null;
        return;
      }
    }
  });
}
