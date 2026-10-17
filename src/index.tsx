import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './styles.css';
import { applyStoredLocale } from './i18n';
import { installReleaseContextMenuGuard } from '@/lib/contextMenuGuard';
import { applyPlatformAttribute } from '@/lib/platform';
import { bootstrapDbStorage } from '@/services/dbStorage';
import { applyTheme, readStoredTheme } from '@/theme/useTheme';
import { initLoggerLevel } from '@/utils/logger';
import App from './App';

/**
 * 「关于」子窗口分支（Rust 侧 about.rs 以 `index.html?window=about` 创建）：
 * 不启 SQLCipher 前端水合（全局池在 Rust 侧，主题/语言直接 invoke db_get_kv），
 * 渲染自绘卡片（透明无边框窗口，圈角由卡片 CSS 自绘，同 ZCode 架构）。
 */
async function bootstrapAboutWindow(rootEl: HTMLElement) {
  document.body.style.background = 'transparent';
  applyTheme(readStoredTheme());
  applyPlatformAttribute();
  // 主题/语言以数据库为准（前端缓存本窗口未水合）；失败回退默认深色/中文
  const { invoke } = await import('@tauri-apps/api/core');
  const theme = await invoke<string | null>('db_get_kv', { key: 'veloterm-theme' }).catch(() => null);
  if (theme === 'light' || theme === 'dark' || theme === 'system') {
    applyTheme(theme);
  }
  const locale = await invoke<string | null>('db_get_kv', { key: 'veloterm-locale' }).catch(() => null);
  if (locale === 'zh-CN' || locale === 'en-US') {
    const { default: i18n } = await import('./i18n');
    void i18n.changeLanguage(locale);
  }
  const { AboutWindow } = await import('@/components/AboutWindow');
  createRoot(rootEl).render(
    <StrictMode>
      <AboutWindow />
    </StrictMode>,
  );
}

async function bootstrap() {
  const rootEl = document.getElementById('root');
  if (!rootEl) {
    return;
  }
  if (new URLSearchParams(window.location.search).get('window') === 'about') {
    await bootstrapAboutWindow(rootEl);
    return;
  }
  // 先挂默认主题 class 避免白屏闪烁，随后从 SQLCipher 数据库 (`veloterm.db`) 水合全量持久化状态
  applyTheme(readStoredTheme());
  // 标记运行平台，首屏渲染前生效：非 macOS / 非 iOS 走 HarmonyOS Sans SC 首选字体
  applyPlatformAttribute();
  // release 下屏蔽 WebView 默认右键菜单（Reload / Inspect Element）；不改动应用内自定义菜单
  installReleaseContextMenuGuard();
  await bootstrapDbStorage();
  applyTheme(readStoredTheme());
  applyStoredLocale();
  void initLoggerLevel();

  createRoot(rootEl).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

void bootstrap();
