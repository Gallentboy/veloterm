/**
 * 侧栏菜单配置：kv 存「顺序 + 隐藏」两个维度（JSON 对象
 * `{ order: string[], hidden: string[] }`；兼容旧版纯数组=仅隐藏清单）。
 * 默认（无记录）= 内置顺序全展示。设置页宫格拖拽排序/勾选 → 写 kv + 广播事件；
 * WorkspaceSidebar 监听事件实时刷新（顺序与可见性都即时生效）。
 */

import { getDbKv, setDbKv } from './dbStorage';

export const SIDEBAR_VISIBILITY_KEY = 'veloterm:sidebar:hidden-pages';
export const SIDEBAR_VISIBILITY_EVENT = 'veloterm:sidebar-visibility-changed';

export interface SidebarMenuConfig {
  /** 菜单 id 的展示顺序（缺项/新增菜单按内置顺序补齐到尾部） */
  order: string[];
  /** 隐藏的菜单 id 集合（序列化用数组） */
  hidden: string[];
}

function parse(raw: string | null): SidebarMenuConfig {
  if (!raw) return { order: [], hidden: [] };
  try {
    const value = JSON.parse(raw);
    if (Array.isArray(value)) {
      // 旧版格式：纯数组 = 隐藏清单
      return {
        order: [],
        hidden: value.filter((v): v is string => typeof v === 'string'),
      };
    }
    if (typeof value === 'object' && value !== null) {
      const { order, hidden } = value as { order?: unknown; hidden?: unknown };
      return {
        order: Array.isArray(order) ? order.filter((v): v is string => typeof v === 'string') : [],
        hidden: Array.isArray(hidden) ? hidden.filter((v): v is string => typeof v === 'string') : [],
      };
    }
  } catch {
    // 损坏数据按「默认全展示」处理
  }
  return { order: [], hidden: [] };
}

export function getSidebarMenuConfig(): SidebarMenuConfig {
  return parse(getDbKv(SIDEBAR_VISIBILITY_KEY));
}

/** 按内置清单补齐顺序（未收录的菜单保持相对序并追加新菜单到尾部）。 */
export function normalizeSidebarOrder(config: SidebarMenuConfig, defaultOrder: string[]): string[] {
  const known = new Set(defaultOrder);
  const ordered = config.order.filter((id) => known.has(id));
  const seen = new Set(ordered);
  for (const id of defaultOrder) {
    if (!seen.has(id)) ordered.push(id);
  }
  return ordered;
}

export function setSidebarMenuConfig(config: SidebarMenuConfig): void {
  setDbKv(SIDEBAR_VISIBILITY_KEY, JSON.stringify(config));
  window.dispatchEvent(new CustomEvent(SIDEBAR_VISIBILITY_EVENT));
}
