import { isTauri } from '@tauri-apps/api/core';

/**
 * Release 环境下屏蔽 WebView 的默认右键菜单（Reload / Back / Forward / Stop / Inspect Element）。
 *
 * 背景：Tauri 2 未暴露右键菜单的配置项，macOS WKWebView / Windows WebView2 的默认菜单都是
 * DOM `contextmenu` 事件的默认行为；对其 `preventDefault()` 即可整体抑制。
 *
 * 边界 —— **只动默认菜单，不碰应用内自定义菜单**：
 * - 应用内自定义菜单（终端 / SFTP / Monaco）走 React `onContextMenu`，自行 `preventDefault()`
 *   并渲染自己的菜单；capture 阶段的 `preventDefault()` 不阻断事件传播，故它们的 handler
 *   照常执行、菜单照常弹出，行为完全不受影响。
 * - 仅在 release 安装：dev 保留完整默认菜单，便于 Inspect Element 调试。
 *
 * 说明：`Inspect Element` 在 release 下本就不生效（`src-tauri/Cargo.toml` 未启用 tauri 的
 * `devtools` feature，release 构建无 inspector），此处再移除其入口，双保险。
 */

/** 抑制 WebView 默认右键菜单；不改动应用内自定义菜单。 */
export function installReleaseContextMenuGuard(): () => void {
  if (typeof window === 'undefined' || !import.meta.env.PROD || !isTauri()) {
    return () => {};
  }

  const onContextMenu = (event: MouseEvent) => {
    event.preventDefault();
  };

  window.addEventListener('contextmenu', onContextMenu, { capture: true });
  return () => window.removeEventListener('contextmenu', onContextMenu, { capture: true });
}
