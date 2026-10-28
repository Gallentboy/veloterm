import { createContext, useContext, type ReactNode } from 'react';
import { useTheme, type Theme, type ThemePreset } from './useTheme';

export type { Theme, ThemePreset };

interface ThemeContextValue {
  theme: Theme;
  setTheme: (theme: Theme) => void;
  themePreset: ThemePreset;
  setThemePreset: (preset: ThemePreset) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const { theme, setTheme, themePreset, setThemePreset } = useTheme();
  return (
    <ThemeContext value={{ theme, setTheme, themePreset, setThemePreset }}>
      {children}
    </ThemeContext>
  );
}

export function useThemeContext(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) {
    throw new Error('useThemeContext 必须在 ThemeProvider 内使用');
  }
  return ctx;
}
