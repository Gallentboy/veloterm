import { useCallback, useEffect, useState } from 'react';
import { getDbKv, setDbKv } from '@/services/dbStorage';

export type Theme = 'light' | 'dark' | 'system';
export type ResolvedTheme = 'light' | 'dark';
export type ThemePreset = 'default' | 'catppuccin' | 'tokyo-night' | 'nord' | 'github-dark';

const STORAGE_KEY = 'veloterm-theme';
const PRESET_STORAGE_KEY = 'veloterm-theme-preset';

function getSystemTheme(): ResolvedTheme {
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

export function resolveTheme(theme: Theme): ResolvedTheme {
  if (theme === 'system') {
    return getSystemTheme();
  }
  return theme;
}

export function applyTheme(theme: Theme, preset?: ThemePreset) {
  const resolved = resolveTheme(theme);
  document.documentElement.classList.toggle('dark', resolved === 'dark');
  document.documentElement.classList.toggle('theme-dark', resolved === 'dark');
  document.documentElement.classList.toggle('theme-light', resolved === 'light');
  if (preset) {
    document.documentElement.setAttribute('data-theme-preset', preset);
  } else if (!document.documentElement.hasAttribute('data-theme-preset')) {
    document.documentElement.setAttribute('data-theme-preset', readStoredThemePreset());
  }
}

function isTheme(value: string | null): value is Theme {
  return value === 'light' || value === 'dark' || value === 'system';
}

function isThemePreset(value: string | null): value is ThemePreset {
  return (
    value === 'default' ||
    value === 'catppuccin' ||
    value === 'tokyo-night' ||
    value === 'nord' ||
    value === 'github-dark'
  );
}

export function readStoredTheme(): Theme {
  const saved = getDbKv(STORAGE_KEY);
  return isTheme(saved) ? saved : 'dark';
}

export function readStoredThemePreset(): ThemePreset {
  const saved = getDbKv(PRESET_STORAGE_KEY);
  return isThemePreset(saved) ? saved : 'default';
}

export function useTheme() {
  const [theme, setThemeState] = useState<Theme>(readStoredTheme);
  const [themePreset, setThemePresetState] = useState<ThemePreset>(readStoredThemePreset);

  const setTheme = useCallback(
    (t: Theme) => {
      setDbKv(STORAGE_KEY, t);
      setThemeState(t);
      applyTheme(t, themePreset);
    },
    [themePreset],
  );

  const setThemePreset = useCallback(
    (p: ThemePreset) => {
      setDbKv(PRESET_STORAGE_KEY, p);
      setThemePresetState(p);
      applyTheme(theme, p);
    },
    [theme],
  );

  // 初始化 + system 模式下监听系统偏好变化
  useEffect(() => {
    applyTheme(theme, themePreset);

    if (theme !== 'system') {
      return;
    }

    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = () => applyTheme('system', themePreset);
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, [theme, themePreset]);

  return { theme, setTheme, themePreset, setThemePreset } as const;
}
