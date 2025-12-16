import { Sun, Moon, Palette } from 'lucide-react';
import { useTheme } from '../contexts/ThemeContext';

export function ThemeSelector() {
  const { theme, setTheme } = useTheme();

  const themes = [
    {
      id: 'light' as const,
      name: 'Light',
      description: 'Clean, bright interface',
      preview: '#ffffff',
      icon: Sun,
    },
    {
      id: 'dark' as const,
      name: 'Dark',
      description: 'Default dark theme',
      preview: '#0f0f0f',
      icon: Moon,
    },
    {
      id: 'solarized-light' as const,
      name: 'Solarized Light',
      description: 'Warm, low-contrast light',
      preview: '#fdf6e3',
      icon: Palette,
    },
    {
      id: 'solarized-dark' as const,
      name: 'Solarized Dark',
      description: 'Precision colors for developers',
      preview: '#002b36',
      icon: Palette,
    },
  ];

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-lg font-semibold text-white mb-1">Theme</h3>
        <p className="text-sm text-gray-400 mb-4">
          Choose your preferred color scheme
        </p>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {themes.map((themeOption) => {
          const Icon = themeOption.icon;
          const isSelected = theme === themeOption.id;

          return (
            <button
              key={themeOption.id}
              onClick={() => setTheme(themeOption.id)}
              className={`
                relative flex items-start gap-3 p-4 rounded-lg border transition-all
                ${
                  isSelected
                    ? 'bg-blue-600/10 border-blue-600 shadow-lg shadow-blue-600/20'
                    : 'bg-gray-900 border-gray-700 hover:bg-gray-800 hover:border-gray-600'
                }
              `}
            >
              <div
                className="w-10 h-10 rounded-md border-2 border-gray-600 flex items-center justify-center flex-shrink-0"
                style={{ backgroundColor: themeOption.preview }}
              >
                <Icon
                  size={20}
                  className={
                    themeOption.id.includes('light')
                      ? 'text-gray-700'
                      : 'text-gray-200'
                  }
                />
              </div>

              <div className="flex-1 text-left">
                <div className="flex items-center gap-2">
                  <h4
                    className={`font-medium ${
                      isSelected ? 'text-blue-400' : 'text-white'
                    }`}
                  >
                    {themeOption.name}
                  </h4>
                  {isSelected && (
                    <div className="w-2 h-2 rounded-full bg-blue-400" />
                  )}
                </div>
                <p className="text-sm text-gray-400 mt-1">
                  {themeOption.description}
                </p>
              </div>
            </button>
          );
        })}
      </div>

      <div className="mt-6 p-4 rounded-lg bg-gray-900 border border-gray-700">
        <div className="flex items-start gap-3">
          <Palette size={20} className="text-gray-500 mt-0.5 flex-shrink-0" />
          <div className="text-sm text-gray-400">
            <p className="mb-2">
              Theme changes apply instantly and persist across sessions.
            </p>
            <p className="text-gray-500">
              All components will adapt to your selected color scheme automatically.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
