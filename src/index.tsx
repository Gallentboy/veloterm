import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './styles.css';
import { applyStoredLocale } from './i18n';
import { bootstrapDbStorage } from '@/services/dbStorage';
import { applyTheme, readStoredTheme } from '@/theme/useTheme';
import { initLoggerLevel } from '@/utils/logger';
import App from './App';

async function bootstrap() {
  // 先挂默认主题 class 避免白屏闪烁，随后从 SQLCipher 数据库 (`veloterm.db`) 水合全量持久化状态
  applyTheme(readStoredTheme());
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
