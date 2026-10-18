/**
 * 侧栏菜单可见性：kv 存「隐藏的菜单 id 集合」（JSON 数组），默认无记录 = 全部展示。
 * 设置页勾选切换 → 写 kv + 广播事件；WorkspaceSidebar 监听事件实时刷新。
 */

import { getDbKv, setDbKv } from './dbStorage';

export const SIDEBAR_VISIBILITY_KEY = 'veloterm:sidebar:hidden-pages';
export const SIDEBAR_VISIBILITY_EVENT = 'veloterm:sidebar-visibility-changed';

function parse(raw: string | null): Set<string> {
  if (!raw) return new Set();
  try {
    const list = JSON.parse(raw);
    if (Array.isArray(list)) return new Set(list.filter((v): v is string => typeof v === 'string'));
  } catch {
    // 损坏数据按「全部展示」处理
  }
  return new Set();
}

export function getHiddenSidebarPages(): Set<string> {
  return parse(getDbKv(SIDEBAR_VISIBILITY_KEY));
}

export function setHiddenSidebarPages(pages: Set<string>): void {
  setDbKv(SIDEBAR_VISIBILITY_KEY, JSON.stringify([...pages]));
  window.dispatchEvent(new CustomEvent(SIDEBAR_VISIBILITY_EVENT));
}
