/**
 * 终端渲染器三级降级决策：WebGL 2 → Canvas 2D → DOM。
 *
 * - 启动时一次性探测（模块级缓存），进程内共享
 * - 运行时降级：WebGL 挂载失败或上下文丢失后调 degradeFromWebgl()，
 *   结果写回缓存使后续新 Tab 也走降级后的渲染器
 */

import { hasWebgl2HardwareAcceleration } from './webglSupport';

export type TerminalRendererKind = 'webgl' | 'canvas2d' | 'dom';

export interface RendererSelection {
  kind: TerminalRendererKind;
  /** 降级原因，用于日志和状态栏展示 */
  reason: string;
}

let cachedSelection: RendererSelection | null = null;

/**
 * Canvas 2D 可用性探测：创建离屏 canvas 检查 getContext('2d') 与基本绘制 API。
 * 与 WebGL 探测不同，Canvas 2D 在所有现代浏览器/WebView 中几乎总是可用，
 * 这里仍做防御性检查以防特殊环境（如禁用 canvas 的 CSP）。
 */
export function isCanvas2DSupported(): boolean {
  if (typeof window === 'undefined' || typeof document === 'undefined') {
    return false;
  }
  try {
    const canvas = document.createElement('canvas');
    const ctx = canvas.getContext('2d');
    const supported = !!(ctx && typeof ctx.fillRect === 'function');
    canvas.width = 0;
    canvas.height = 0;
    return supported;
  } catch {
    return false;
  }
}

/**
 * 首选渲染器决策（首次挂载时调用，结果缓存）：
 * WebGL2 硬件加速 → Canvas 2D → DOM（xterm 内置默认）。
 */
export function selectRenderer(): RendererSelection {
  if (cachedSelection) {
    return cachedSelection;
  }
  if (hasWebgl2HardwareAcceleration()) {
    cachedSelection = { kind: 'webgl', reason: 'WebGL2 hardware acceleration available' };
  } else if (isCanvas2DSupported()) {
    cachedSelection = {
      kind: 'canvas2d',
      reason: 'WebGL2 unavailable or software rasterizer; Canvas 2D supported',
    };
  } else {
    cachedSelection = {
      kind: 'dom',
      reason: 'Neither WebGL2 nor Canvas 2D supported; falling back to DOM renderer',
    };
  }
  return cachedSelection;
}

/**
 * 运行时降级：WebGL 挂载失败或上下文丢失后调用。
 * 结果写回缓存，后续新 Tab 也走降级后的渲染器。
 */
export function degradeFromWebgl(): RendererSelection {
  cachedSelection = isCanvas2DSupported()
    ? {
        kind: 'canvas2d',
        reason: 'WebGL context lost or init failed; degraded to Canvas 2D',
      }
    : {
        kind: 'dom',
        reason: 'WebGL context lost and Canvas 2D unavailable; degraded to DOM renderer',
      };
  return cachedSelection;
}

/** 重置缓存（将来加「重新检测 GPU」入口时用） */
export function resetRendererCache(): void {
  cachedSelection = null;
}
