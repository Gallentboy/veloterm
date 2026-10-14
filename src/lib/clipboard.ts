import { isTauri } from '@tauri-apps/api/core';
import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager';

/**
 * 剪贴板读写统一封装：
 * - Tauri 环境走原生插件（Rust 侧直接读写系统剪贴板），
 *   绕开 WKWebView `navigator.clipboard.readText()` 触发的系统粘贴授权弹层
 *   （macOS 原生「粘贴」按钮需手动点击，终端 Cmd+V 无法一键粘贴）；
 * - 浏览器预览（Mock 通道）回退 `navigator.clipboard`。
 */
export async function readClipboardText(): Promise<string> {
  if (isTauri()) {
    return (await readText()) ?? '';
  }
  return navigator.clipboard.readText();
}

export async function writeClipboardText(text: string): Promise<void> {
  if (isTauri()) {
    await writeText(text);
    return;
  }
  await navigator.clipboard.writeText(text);
}
