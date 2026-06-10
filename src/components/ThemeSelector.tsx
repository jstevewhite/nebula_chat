import { Sun, Moon, Palette } from 'lucide-react';
import { useTheme } from '../contexts/ThemeContext';

// Pick a readable icon color for a swatch based on the swatch's luma
// (Rec.601 perceived brightness), so light swatches get a dark icon and
// dark swatches get a light icon —
// regardless of whether the theme id contains "light".
function isLightSwatch(hex: string): boolean {
  const c = hex.replace('#', '');
  const r = parseInt(c.slice(0, 2), 16);
  const g = parseInt(c.slice(2, 4), 16);
  const b = parseInt(c.slice(4, 6), 16);
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255 > 0.6;
}

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
    {
      id: 'kimbie-dark' as const,
      name: 'Kimbie Dark',
      description: 'Warm, earthy dark theme',
      preview: '#221a0f',
      icon: Moon,
    },
    {
      id: 'quiet-light' as const,
      name: 'Quiet Light',
      description: 'Subtle, low-contrast light',
      preview: '#f5f5f5',
      icon: Sun,
    },
    {
      id: 'ink' as const,
      name: 'Ink',
      description: 'Warm near-black with gold accent',
      preview: '#0f0e0c',
      icon: Moon,
    },
    {
      id: 'ink-light' as const,
      name: 'Ink Light',
      description: 'Warm parchment, gold accent',
      preview: '#f6f1e6',
      icon: Sun,
    },
    {
      id: 'ink-medium' as const,
      name: 'Ink Medium',
      description: 'Dimmed warm tan parchment',
      preview: '#cdc5af',
      icon: Palette,
    },
  ];

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-lg font-semibold text-[var(--color-text-primary)] mb-1">Theme</h3>
        <p className="text-sm text-[var(--color-text-tertiary)] mb-4">
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
                    ? 'bg-[var(--color-bg-tertiary)] border-[var(--color-accent-primary)] shadow-lg'
                    : 'bg-[var(--color-bg-secondary)] border-[var(--color-border-primary)] hover:bg-[var(--color-bg-tertiary)] hover:border-[var(--color-border-secondary)]'
                }
              `}
            >
              <div
                className="w-10 h-10 rounded-md border-2 border-[var(--color-border-primary)] flex items-center justify-center flex-shrink-0"
                style={{ backgroundColor: themeOption.preview }}
              >
                <Icon
                  size={20}
                  className={
                    isLightSwatch(themeOption.preview)
                      ? 'text-gray-700'
                      : 'text-gray-200'
                  }
                />
              </div>

              <div className="flex-1 text-left">
                <div className="flex items-center gap-2">
                  <h4
                    className={`font-medium ${
                      isSelected ? 'text-[var(--color-accent-primary)]' : 'text-[var(--color-text-primary)]'
                    }`}
                  >
                    {themeOption.name}
                  </h4>
                  {isSelected && (
                    <div className="w-2 h-2 rounded-full bg-[var(--color-accent-primary)]" />
                  )}
                </div>
                <p className="text-sm text-[var(--color-text-tertiary)] mt-1">
                  {themeOption.description}
                </p>
              </div>
            </button>
          );
        })}
      </div>

      <div className="mt-6 p-4 rounded-lg bg-[var(--color-bg-secondary)] border border-[var(--color-border-primary)]">
        <div className="flex items-start gap-3">
          <Palette size={20} className="text-[var(--color-text-tertiary)] mt-0.5 flex-shrink-0" />
          <div className="text-sm text-[var(--color-text-secondary)]">
            <p className="mb-2">
              Theme changes apply instantly and persist across sessions.
            </p>
            <p className="text-[var(--color-text-tertiary)]">
              All components will adapt to your selected color scheme automatically.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
