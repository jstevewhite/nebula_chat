import { createContext, useContext, useEffect, useState, useCallback, ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface FontSettings {
  interface_font: string;
  interface_font_size: number;
  interface_font_weight: string;
  chat_font: string;
  chat_font_size: number;
  chat_font_weight: string;
}

const defaultFontSettings: FontSettings = {
  // IMPORTANT: include fallbacks. If the primary font isn't installed (e.g., Inter on macOS),
  // a bare "Inter" can fall back to the browser default (often serif).
  interface_font: 'Inter, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif',
  interface_font_size: 14,
  interface_font_weight: '400',
  chat_font: 'Inter, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif',
  chat_font_size: 14,
  chat_font_weight: '400',
};

interface ThemeContextType {
  theme: string;
  setTheme: (theme: string) => void;
  fontSettings: FontSettings;
  setFontSettings: (settings: FontSettings) => void;
  isLoading: boolean; // Keep isLoading in context type
}

const ThemeContext = createContext<ThemeContextType | undefined>(undefined);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState('light'); // Default to light execution until loaded
  const [fontSettings, setFontSettingsState] = useState<FontSettings>(defaultFontSettings);
  const [isLoading, setIsLoading] = useState(true);

  // Function to apply theme to document
  const updateTheme = useCallback((currentTheme: string) => {
    const root = document.documentElement;
    root.setAttribute('data-theme', currentTheme);
  }, []);

  const SANS_FALLBACK = 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif';
  const MONO_FALLBACK = 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace';

  const normalizeFontFamily = (fontFamily: string, fallback: string) => {
    const v = (fontFamily || '').trim();
    if (!v) return fallback;

    // If it already contains a stack, assume it is intentional.
    if (v.includes(',')) return v;

    // Otherwise, treat it as a single family name and append a safe fallback stack.
    return `${v}, ${fallback}`;
  };

  // Function to apply font settings to document
  const updateFonts = useCallback((fonts: FontSettings) => {
    const root = document.documentElement;

    // Use a mono fallback if the selected family clearly intends monospace.
    const interfaceFont = normalizeFontFamily(fonts.interface_font, SANS_FALLBACK);
    const chatFallback = (fonts.chat_font || '').toLowerCase().includes('mono')
      ? MONO_FALLBACK
      : SANS_FALLBACK;
    const chatFont = normalizeFontFamily(fonts.chat_font, chatFallback);

    root.style.setProperty('--font-interface', interfaceFont);
    root.style.setProperty('--size-interface', `${fonts.interface_font_size}px`);
    root.style.setProperty('--weight-interface', fonts.interface_font_weight);

    root.style.setProperty('--font-chat', chatFont);
    root.style.setProperty('--size-chat', `${fonts.chat_font_size}px`);
    root.style.setProperty('--weight-chat', fonts.chat_font_weight);
  }, []);

  // Load theme and font settings from backend on mount
  useEffect(() => {
    const loadSettings = async () => {
      try {
        const settings = await invoke<any>('get_settings');

        if (settings.theme) {
          setThemeState(settings.theme);
          updateTheme(settings.theme);
        }

        const loadedFonts = {
          interface_font: settings.interface_font || defaultFontSettings.interface_font,
          interface_font_size: settings.interface_font_size || defaultFontSettings.interface_font_size,
          interface_font_weight: settings.interface_font_weight || defaultFontSettings.interface_font_weight,
          chat_font: settings.chat_font || defaultFontSettings.chat_font,
          chat_font_size: settings.chat_font_size || defaultFontSettings.chat_font_size,
          chat_font_weight: settings.chat_font_weight || defaultFontSettings.chat_font_weight,
        };
        setFontSettingsState(loadedFonts);
        updateFonts(loadedFonts);
      } catch (error) {
        console.error('Failed to load settings:', error);
        // Apply defaults if loading fails
        updateTheme(theme); // Apply initial default theme
        updateFonts(defaultFontSettings); // Apply initial default fonts
      } finally {
        setIsLoading(false);
      }
    };

    loadSettings();
  }, [theme, updateTheme, updateFonts]); // Dependencies for initial load

  // Apply theme to document when it changes (after initial load)
  useEffect(() => {
    const root = document.documentElement;

    // Remove no-transition class after initial load to enable smooth transitions
    if (!isLoading && root.classList.contains('no-transition')) {
      // Small delay to ensure styles are applied before enabling transitions
      setTimeout(() => {
        root.classList.remove('no-transition');
      }, 100);
    }

    updateTheme(theme); // Ensure theme is applied if it changes after initial load
  }, [theme, isLoading, updateTheme]);

  const setTheme = async (newTheme: string) => {
    // Update UI immediately
    setThemeState(newTheme);
    updateTheme(newTheme);

    try {
      // Merge into the full settings object so we don't wipe providers/MCP servers.
      const currentSettings = await invoke<any>('get_settings');
      await invoke('save_settings', {
        settings: {
          ...currentSettings,
          theme: newTheme
        }
      });
    } catch (error) {
      console.error('Failed to save theme:', error);
    }
  };

  const setFontSettings = async (newSettings: FontSettings) => {
    setFontSettingsState(newSettings);
    updateFonts(newSettings);
    try {
      // We need to save the full settings object, so get it first
      const currentSettings = await invoke<any>('get_settings');
      await invoke('save_settings', {
        settings: {
          ...currentSettings,
          ...newSettings
        }
      });
    } catch (error) {
      console.error('Failed to save font settings:', error);
    }
  };

  return (
    <ThemeContext.Provider value={{ theme, setTheme, fontSettings, setFontSettings, isLoading }}>
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
