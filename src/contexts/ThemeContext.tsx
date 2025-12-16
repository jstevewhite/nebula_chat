import { createContext, useContext, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

type Theme = 'light' | 'dark' | 'solarized-light' | 'solarized-dark';

interface ThemeContextType {
  theme: Theme;
  setTheme: (theme: Theme) => void;
  isLoading: boolean;
}

const ThemeContext = createContext<ThemeContextType | undefined>(undefined);

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [theme, setThemeState] = useState<Theme>('dark');
  const [isLoading, setIsLoading] = useState(true);

  // Load theme from settings on mount
  useEffect(() => {
    const loadTheme = async () => {
      try {
        const savedTheme = await invoke<string>('get_theme');
        setThemeState(savedTheme as Theme);
      } catch (error) {
        console.error('Failed to load theme:', error);
        // Keep default 'dark' theme on error
      } finally {
        setIsLoading(false);
      }
    };

    loadTheme();
  }, []);

  // Apply theme to document when it changes
  useEffect(() => {
    const root = document.documentElement;

    // Remove no-transition class after initial load to enable smooth transitions
    if (!isLoading && root.classList.contains('no-transition')) {
      // Small delay to ensure styles are applied before enabling transitions
      setTimeout(() => {
        root.classList.remove('no-transition');
      }, 100);
    }

    root.setAttribute('data-theme', theme);
  }, [theme, isLoading]);

  const setTheme = async (newTheme: Theme) => {
    try {
      await invoke('set_theme', { theme: newTheme });
      setThemeState(newTheme);
    } catch (error) {
      console.error('Failed to save theme:', error);
      // Still update UI even if save fails
      setThemeState(newTheme);
    }
  };

  return (
    <ThemeContext.Provider value={{ theme, setTheme, isLoading }}>
      {children}
    </ThemeContext.Provider>
  );
}

export const useTheme = () => {
  const context = useContext(ThemeContext);
  if (!context) {
    throw new Error('useTheme must be used within ThemeProvider');
  }
  return context;
};
