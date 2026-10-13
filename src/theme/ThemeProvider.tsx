import { createContext, useContext, type ReactNode } from 'react';
import { useTheme, type Theme } from './useTheme';

export type { Theme };

interface ThemeContextValue {
  theme: Theme;
  setTheme: (theme: Theme) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const { theme, setTheme } = useTheme();
  return <ThemeContext value={{ theme, setTheme }}>{children}</ThemeContext>;
}

export function useThemeContext(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) {
    throw new Error('useThemeContext 必须在 ThemeProvider 内使用');
  }
  return ctx;
}
