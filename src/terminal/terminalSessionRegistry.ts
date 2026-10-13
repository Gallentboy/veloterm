import { WebglAddon } from '@xterm/addon-webgl';
import type { FitAddon } from '@xterm/addon-fit';
import type { ITheme, Terminal as XTerm } from '@xterm/xterm';

/**
 * 浏览器 / WebView（尤其 macOS WKWebView 与 Chromium）对单页面并发活跃的 WebGL 上下文数量有硬上限（通常 8~16 个）。
 * 在多 Tab 终端下，使用 LRU 池管理活跃的 `WebglAddon`：
 * - 当前可见及最近使用的前 N 个 Tab 保持 WebGL2 硬件加速常驻（切换 Tab 零开销）；
 * - 超出上限的最久未访问后台 Tab 自动卸载 `WebglAddon`（保留 xterm 实例与 scrollback 历史），
 *   避免触发浏览器强制驱逐当前正在使用的前台 WebGL 上下文；
 * - 当被驱逐的 Tab 再次切回前台时，`ensureWebgl` 自动为其重新挂载 `WebglAddon`。
 */
const MAX_ACTIVE_WEBGL_CONTEXTS = 6;

export interface TerminalSessionRegistryEntry {
  key: number;
  term: XTerm;
  fitAddon: FitAddon;
  webglAddon: WebglAddon | null;
  terminalId: number | null;
  cwd: string;
  hostEl: HTMLDivElement;
  profileTheme?: ITheme;
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

function ensureEntryWebgl(
  entry: TerminalSessionRegistryEntry,
  onStatusChange?: (enabled: boolean) => void,
): boolean {
  if (entry.webglAddon) {
    touchWebglLru(entry.key);
    onStatusChange?.(true);
    return true;
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
      onStatusChange?.(false);
    });

    entry.term.loadAddon(webglAddon);
    entry.webglAddon = webglAddon;
    touchWebglLru(entry.key);
    onStatusChange?.(true);
    return true;
  } catch (error) {
    console.warn('[terminalSessionRegistry] WebGL2 addon activation failed, fallback to DOM:', error);
    entry.webglAddon = null;
    onStatusChange?.(false);
    return false;
  }
}

function releaseEntry(key: number): void {
  const entry = sessions.get(key);
  if (!entry) return;
  disposeEntryWebgl(entry);
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
   * 确保指定会话已挂载 WebGL2 硬件加速渲染器（含 LRU 上下文池管理与上下文丢失恢复）。
   */
  ensureWebgl(key: number, onStatusChange?: (enabled: boolean) => void): boolean {
    const entry = sessions.get(key);
    if (!entry) return false;
    return ensureEntryWebgl(entry, onStatusChange);
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

  /** 清空当前终端屏幕内容 */
  clear(key: number): void {
    const entry = sessions.get(key);
    if (!entry) return;
    entry.term.clear();
  },
};
