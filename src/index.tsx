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

async function bootstrap() {
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

  const rootEl = document.getElementById('root');
  if (rootEl) {
    const root = createRoot(rootEl);
    root.render(
      <StrictMode>
        <App />
      </StrictMode>,
    );
  }
}

void bootstrap();
