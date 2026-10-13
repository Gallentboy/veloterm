import { useCallback, useEffect, useState } from 'react';
import { getDbKv, setDbKv } from '@/services/dbStorage';

export type Theme = 'light' | 'dark' | 'system';
export type ResolvedTheme = 'light' | 'dark';

const STORAGE_KEY = 'veloterm-theme';

function getSystemTheme(): ResolvedTheme {
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

export function resolveTheme(theme: Theme): ResolvedTheme {
  if (theme === 'system') {
    return getSystemTheme();
  }
  return theme;
}

export function applyTheme(theme: Theme) {
  const resolved = resolveTheme(theme);
  document.documentElement.classList.toggle('dark', resolved === 'dark');
  document.documentElement.classList.toggle('theme-dark', resolved === 'dark');
  document.documentElement.classList.toggle('theme-light', resolved === 'light');
}

function isTheme(value: string | null): value is Theme {
  return value === 'light' || value === 'dark' || value === 'system';
}

export function readStoredTheme(): Theme {
  const saved = getDbKv(STORAGE_KEY);
  // 与 ZCode 一致：默认主题收敛到深色。
  return isTheme(saved) ? saved : 'dark';
}

export function useTheme() {
  const [theme, setThemeState] = useState<Theme>(readStoredTheme);

  const setTheme = useCallback((t: Theme) => {
    setDbKv(STORAGE_KEY, t);
    setThemeState(t);
    applyTheme(t);
  }, []);

  // 初始化 + system 模式下监听系统偏好变化
  useEffect(() => {
    applyTheme(theme);

    if (theme !== 'system') {
      return;
    }

    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = () => applyTheme('system');
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, [theme]);

  return { theme, setTheme } as const;
}
