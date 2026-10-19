import { WebglAddon } from '@xterm/addon-webgl';
import { CanvasAddon } from '@xterm/addon-canvas';
import type { FitAddon } from '@xterm/addon-fit';
import type { SearchAddon } from '@xterm/addon-search';
import type { ITheme, Terminal as XTerm } from '@xterm/xterm';
import {
  degradeFromWebgl,
  selectRenderer,
  type TerminalRendererKind,
} from './rendererSelector';

/**
 * 浏览器 / WebView（尤其 macOS WKWebView 与 Chromium）对单页面并发活跃的 WebGL 上下文数量有硬上限（通常 8~16 个）。
 * 在多 Tab 终端下，使用 LRU 池管理活跃的 `WebglAddon`：
 * - 当前可见及最近使用的前 N 个 Tab 保持 WebGL2 硬件加速常驻（切换 Tab 零开销）；
 * - 超出上限的最久未访问后台 Tab 自动卸载 `WebglAddon`（保留 xterm 实例与 scrollback 历史），
 *   避免触发浏览器强制驱逐当前正在使用的前台 WebGL 上下文；
 * - 当被驱逐的 Tab 再次切回前台时，`ensureWebgl` 自动为其重新挂载 `WebglAddon`。
 *
 * 另外，**只有探测到 WebGL2 硬件加速时才会启用 `WebglAddon`**（见 `webglSupport`）：
 * addon 自身仅检查 `getContext('webgl2')` 是否非空，软件光栅化环境下会「成功」挂载却由 CPU 绘制，
 * 反而比 xterm 自带的 DOM renderer 更慢。
 */
const MAX_ACTIVE_WEBGL_CONTEXTS = 6;

export interface TerminalSessionRegistryEntry {
  key: number;
  term: XTerm;
  fitAddon: FitAddon;
  webglAddon: WebglAddon | null;
  /** Canvas 2D 渲染 addon（WebGL 不可用/降级后的中间层） */
  canvasAddon: CanvasAddon | null;
  /** 当前实际使用的渲染器类型（供状态栏展示与调试） */
  rendererKind: TerminalRendererKind;
  terminalId: number | null;
  cwd: string;
  hostEl: HTMLDivElement;
  profileTheme?: ITheme;
  /** 字体缩放基准字号（px，创建时按 Profile 记录；缩放以此计算目标字号） */
  baseFontSize?: number;
  /** 当前字体缩放倍率（存于注册表以跨组件重挂载/布局切换保持） */
  fontScale?: number;
  /** 文本搜索 addon（与 xterm 实例同生命周期，Ctrl+F 由组件暴露 openSearch 桥接） */
  searchAddon: SearchAddon | null;
  /** 当前挂载组件暴露的搜索栏打开动作（卸载时置空；registry.showSearch 统一调用） */
  openSearch?: () => void;
  /** 当异步 `transport.create` 完成分配 `terminalId` 时通知当前挂载的组件实例 */
  onTerminalIdReady?: (terminalId: number) => void;
  dispose: () => void;
}

const sessions = new Map<number, TerminalSessionRegistryEntry>();
/** 按最近激活顺序排列的拥有活跃 WebglAddon 的 sessionId 列表（队尾为最近使用） */
const webglLruKeys: number[] = [];

let stashDiv: HTMLDivElement | null = null;

function getStashDiv(): HTMLDivElement | null {
  if (typeof document === 'undefined') return null;
  if (!stashDiv) {
    stashDiv = document.createElement('div');
    stashDiv.style.display = 'none';
    stashDiv.setAttribute('data-veloterm-terminal-stash', '');
    document.body.appendChild(stashDiv);
  }
  return stashDiv;
}

function removeFromWebglLru(key: number): void {
  const idx = webglLruKeys.indexOf(key);
  if (idx !== -1) {
    webglLruKeys.splice(idx, 1);
  }
}

function touchWebglLru(key: number): void {
  removeFromWebglLru(key);
  webglLruKeys.push(key);
}

function disposeEntryWebgl(entry: TerminalSessionRegistryEntry): void {
  removeFromWebglLru(entry.key);
  const addon = entry.webglAddon;
  if (!addon) return;
  entry.webglAddon = null;
  try {
    addon.dispose();
  } catch {
    // 忽略 WebGL 上下文释放异常
  }
}

function disposeEntryCanvas(entry: TerminalSessionRegistryEntry): void {
  const addon = entry.canvasAddon;
  if (!addon) return;
  entry.canvasAddon = null;
  entry.rendererKind = 'dom';
  try {
    addon.dispose();
  } catch {
    // 忽略 Canvas 2D 上下文释放异常
  }
}

/**
 * 挂载 Canvas 2D 渲染 addon（WebGL 不可用或降级后的中间层）。
 * Canvas 2D 无上下文丢失事件，无需 LRU 管理（上下文成本远低于 WebGL）。
 */
function ensureEntryCanvas(entry: TerminalSessionRegistryEntry): boolean {
  if (entry.canvasAddon) {
    return true;
  }
  try {
    const canvasAddon = new CanvasAddon();
    entry.term.loadAddon(canvasAddon);
    entry.canvasAddon = canvasAddon;
    entry.rendererKind = 'canvas2d';
    return true;
  } catch (error) {
    console.warn('[terminalSessionRegistry] Canvas 2D addon activation failed, fallback to DOM:', error);
    entry.canvasAddon = null;
    entry.rendererKind = 'dom';
    return false;
  }
}

/**
 * 渲染器三级降级挂载：WebGL2 → Canvas 2D → DOM（xterm 内置默认）。
 * - WebGL 探测由 rendererSelector.selectRenderer() 一次性决策（缓存）
 * - WebGL 挂载失败或运行中上下文丢失 → degradeFromWebgl() → 尝试 Canvas 2D
 * - Canvas 2D 也失败 → DOM renderer（xterm 自带，无需额外 addon）
 */
function ensureEntryRenderer(
  entry: TerminalSessionRegistryEntry,
  onStatusChange?: (kind: TerminalRendererKind) => void,
): TerminalRendererKind {
  const selection = selectRenderer();

  if (selection.kind === 'webgl') {
    if (entry.webglAddon) {
      touchWebglLru(entry.key);
      onStatusChange?.('webgl');
      return 'webgl';
    }

    // 若活跃 WebGL 上下文数已达上限，淘汰最久未使用的后台 Tab 的 WebglAddon
    while (webglLruKeys.length >= MAX_ACTIVE_WEBGL_CONTEXTS) {
      const oldestKey = webglLruKeys.shift();
      if (oldestKey === undefined) break;
      const oldestEntry = sessions.get(oldestKey);
      if (oldestEntry) {
        disposeEntryWebgl(oldestEntry);
      }
    }

    try {
      const webglAddon = new WebglAddon();
      webglAddon.onContextLoss(() => {
        if (entry.webglAddon === webglAddon) {
          entry.webglAddon = null;
          removeFromWebglLru(entry.key);
        }
        try {
          webglAddon.dispose();
        } catch {
          // 忽略上下文丢失时的销毁异常
        }
        // 运行时降级：写回缓存使后续新 Tab 也走降级后的渲染器
        const degraded = degradeFromWebgl();
        if (degraded.kind === 'canvas2d') {
          ensureEntryCanvas(entry);
          syncEntryRendererModel(entry);
        } else {
          entry.rendererKind = 'dom';
        }
        onStatusChange?.(entry.rendererKind);
      });

      entry.term.loadAddon(webglAddon);
      entry.webglAddon = webglAddon;
      entry.rendererKind = 'webgl';
      touchWebglLru(entry.key);
      onStatusChange?.('webgl');
      return 'webgl';
    } catch (error) {
      console.warn('[terminalSessionRegistry] WebGL2 addon activation failed:', error);
      entry.webglAddon = null;
      const degraded = degradeFromWebgl();
      if (degraded.kind === 'canvas2d' && ensureEntryCanvas(entry)) {
        syncEntryRendererModel(entry);
        onStatusChange?.('canvas2d');
        return 'canvas2d';
      }
      entry.rendererKind = 'dom';
      onStatusChange?.('dom');
      return 'dom';
    }
  }

  if (selection.kind === 'canvas2d') {
    if (ensureEntryCanvas(entry)) {
      syncEntryRendererModel(entry);
      onStatusChange?.('canvas2d');
      return 'canvas2d';
    }
  }

  entry.rendererKind = 'dom';
  onStatusChange?.('dom');
  return 'dom';
}

function releaseEntry(key: number): void {
  const entry = sessions.get(key);
  if (!entry) return;
  disposeEntryWebgl(entry);
  disposeEntryCanvas(entry);
  sessions.delete(key);
  try {
    entry.dispose();
  } catch (error) {
    console.warn('[terminalSessionRegistry] release dispose failed', error);
  }
  entry.hostEl.remove();
}

type XTermCoreWithRenderService = {
  _renderService?: {
    clear?: () => void;
    handleResize?: (cols: number, rows: number) => void;
  };
};

function syncEntryRendererModel(entry: TerminalSessionRegistryEntry): void {
  try {
    const core = (entry.term as unknown as { _core?: XTermCoreWithRenderService })._core;
    core?._renderService?.clear?.();
    core?._renderService?.handleResize?.(entry.term.cols, entry.term.rows);
    if (entry.term.rows > 0) {
      entry.term.refresh(0, entry.term.rows - 1);
    }
  } catch {
    // 忽略渲染器同步异常
  }
}

export const terminalSessionRegistry = {
  has(key: number): boolean {
    return sessions.has(key);
  },

  get(key: number): TerminalSessionRegistryEntry | undefined {
    return sessions.get(key);
  },

  register(key: number, entry: TerminalSessionRegistryEntry): void {
    sessions.set(key, entry);
  },

  /**
   * 确保指定会话已挂载最优渲染器（WebGL2 → Canvas 2D → DOM 三级降级）。
   * 返回当前实际使用的渲染器类型。
   */
  ensureRenderer(
    key: number,
    onStatusChange?: (kind: TerminalRendererKind) => void,
  ): TerminalRendererKind | null {
    const entry = sessions.get(key);
    if (!entry) return null;
    return ensureEntryRenderer(entry, onStatusChange);
  },

  /**
   * 当 Tab 从隐藏切回可见时，重置当前终端 WebglRenderer 的 RenderModel 与 GlyphRenderer 顶点 UV 缓存并重绘，
   * 但不清空全局共享的 `CharAtlasCache` 纹理图集（避免破坏其他 Tab 已缓存的字形纹理坐标）。
   */
  refreshWebglViewport(key: number): void {
    const entry = sessions.get(key);
    if (!entry) return;
    syncEntryRendererModel(entry);
  },

  /**
   * 主题切换或 DPI 变化后清理 WebGL 纹理图集并触发重绘。
   * 注意：`@xterm/addon-webgl` 内部通过模块级 `CharAtlasCache` 在所有相同字号/主题的终端间共享同一个 `TextureAtlas`，
   * 一旦调用 `clearTextureAtlas()` 清空共享图集，必须同步重置所有会话的 `RenderModel` 与顶点 UV 缓存，
   * 否则后台 Tab 切回前台时会用旧的 UV 坐标采样新图集而产生字符错位乱码。
   */
  clearWebglTextureAtlas(key: number): void {
    const entry = sessions.get(key);
    if (!entry?.webglAddon) return;
    try {
      entry.webglAddon.clearTextureAtlas();
    } catch {
      // 忽略图集清理异常
    }
    for (const peer of sessions.values()) {
      syncEntryRendererModel(peer);
    }
  },

  isWebglActive(key: number): boolean {
    return Boolean(sessions.get(key)?.webglAddon);
  },

  /** 获取指定会话当前使用的渲染器类型（webgl / canvas2d / dom） */
  getRendererKind(key: number): TerminalRendererKind {
    return sessions.get(key)?.rendererKind ?? 'dom';
  },

  /** 将 `entry.hostEl` 物理挂载到当前可见的容器元素 */
  attachDom(key: number, host: HTMLElement): void {
    const entry = sessions.get(key);
    if (!entry) return;
    if (entry.hostEl.parentElement !== host) {
      host.appendChild(entry.hostEl);
    }
    try {
      const core = (entry.term as unknown as { _core?: { _charSizeService?: { measure?: () => void } } })._core;
      core?._charSizeService?.measure?.();
    } catch {
      // 忽略测量异常
    }
  },

  /** 组件卸载（如切换侧边栏导航页面或 StrictMode 探测卸载）时将 `entry.hostEl` 移入隐藏暂存区，保持 xterm 与会话存活 */
  detachDom(key: number): void {
    const entry = sessions.get(key);
    if (!entry) return;
    const stash = getStashDiv();
    if (!stash) return;
    if (entry.hostEl.parentElement === stash) return;
    stash.appendChild(entry.hostEl);
  },

  /** 显式关闭某个终端 Tab 或进程退出时调用，彻底释放资源 */
  release(key: number): void {
    releaseEntry(key);
  },

  /** 获取指定前端会话对应的后端 terminalId */
  getTerminalId(key: number): number | null {
    return sessions.get(key)?.terminalId ?? null;
  },

  /** 按后端 terminalId 反查会话（ZMODEM 等后端事件只带 terminalId 时用） */
  findByTerminalId(terminalId: number): TerminalSessionRegistryEntry | undefined {
    for (const entry of sessions.values()) {
      if (entry.terminalId === terminalId) return entry;
    }
    return undefined;
  },

  /** 打开指定会话的终端搜索栏（Ctrl+F 统一入口；组件未挂载时静默忽略） */
  showSearch(key: number): boolean {
    const entry = sessions.get(key);
    if (!entry?.openSearch) return false;
    entry.openSearch();
    return true;
  },

  /** 清空当前终端屏幕内容 */
  clear(key: number): void {
    const entry = sessions.get(key);
    if (!entry) return;
    entry.term.clear();
  },
};
