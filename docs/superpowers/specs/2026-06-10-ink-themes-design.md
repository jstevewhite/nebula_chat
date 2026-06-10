# Spec: Port Loom's Ink themes to Nebula

**Date:** 2026-06-10
**Status:** Approved (design), pending implementation
**Source:** `../loom` — themes defined in `loom/src/lib/stores.ts` (`THEMES` array) and `loom/src/app.css`.

## Goal

Add three warm "Ink" themes from the sibling Loom project to Nebula:
`ink` (dark), `ink-light` (parchment), and `ink-medium` (dimmed tan).

## Background

Nebula's theming is a CSS-variable system:

- Each theme is a `[data-theme="<id>"]` block in `src/themes.css` defining **21** `--color-*`
  variables.
- The active theme is applied by setting `data-theme` on `<html>` (`ThemeContext.tsx`)
  and the id is persisted as a free string in settings (`theme: String` in
  `src-tauri/src/mcp/config.rs:209` — no enum/validation gate).
- The theme picker lists themes in the `themes` array in
  `src/components/ThemeSelector.tsx` (id, name, description, preview swatch, icon).

Loom's themes use a **different, smaller** 12-variable vocabulary
(`--bg-*`, `--border*`, `--text-*`). Porting = mapping those 12 onto Nebula's 21 and
deriving the rest, then registering each theme in the picker.

No mechanism changes are required. Two files change: `src/themes.css` and
`src/components/ThemeSelector.tsx`.

## Mapping rules (Loom → Nebula)

Direct maps:

| Nebula `--color-*`        | ← Loom var      |
|---------------------------|-----------------|
| `bg-primary`              | `bg-base`       |
| `bg-secondary`            | `bg-surface`    |
| `bg-tertiary`             | `bg-elevated`   |
| `bg-elevated`             | `bg-hover`      |
| `text-primary`            | `text-primary`  |
| `text-secondary`          | `text-secondary`|
| `text-tertiary`           | `text-muted`    |
| `border-primary`          | `border`        |
| `input-bg`                | `bg-elevated`   |
| `input-border`            | `border`        |
| `input-focus`             | `text-accent`   |
| `accent-primary`          | `text-accent`   |
| `error`                   | `text-error`    |
| `hover-bg`                | `bg-hover`      |
| `active-bg`               | `bg-active`     |

Derived (Loom has no equivalent; tuned to stay in the warm Ink family):

- `border-secondary` ← `text-muted` — a stronger border for hover affordance. Using the
  muted-text color keeps it *more* prominent than `border-primary` in both light and dark,
  matching Nebula's convention (secondary border = the hover/emphasis border).
- `accent-secondary` ← a lightened sibling of `text-accent`.
- `success` ← muted warm olive; `warning` ← amber harmonized with the gold accent.
- `shadow` ← `rgba(0,0,0,·)`, alpha 0.4 (dark) / ~0.12–0.15 (light/medium).
- `overlay` ← the theme's base bg at 0.9 alpha.

Note on `input-bg`: mapped to Loom's `bg-elevated` for all three (i.e. the same source
value as Nebula's `bg-tertiary`, not Nebula's `bg-elevated`) so inputs read as a defined
field. On the dark theme this is slightly raised above the page; on the light themes it is
marginally darker than the page (a recessed-field look) rather than white — an intentional,
consistent choice across the Ink family.

## Full palettes

### `ink` — warm near-black, gold accent

```css
[data-theme="ink"] {
  --color-bg-primary: #0f0e0c;
  --color-bg-secondary: #181612;
  --color-bg-tertiary: #201d18;
  --color-bg-elevated: #2c2820;

  --color-text-primary: #ece8e1;
  --color-text-secondary: #7a7060;
  --color-text-tertiary: #6a6250;

  --color-border-primary: #2c2820;
  --color-border-secondary: #6a6250;

  --color-accent-primary: #c8a96e;
  --color-accent-secondary: #dcc290;

  --color-success: #889b4a;
  --color-error: #c48576;
  --color-warning: #d89a3a;

  --color-input-bg: #201d18;
  --color-input-border: #2c2820;
  --color-input-focus: #c8a96e;
  --color-hover-bg: #2c2820;
  --color-active-bg: #332e26;

  --color-shadow: rgba(0, 0, 0, 0.4);
  --color-overlay: rgba(15, 14, 12, 0.9);
}
```

### `ink-light` — warm parchment

```css
[data-theme="ink-light"] {
  --color-bg-primary: #f6f1e6;
  --color-bg-secondary: #efe9da;
  --color-bg-tertiary: #e8e1cd;
  --color-bg-elevated: #ddd4bc;

  --color-text-primary: #1f1b14;
  --color-text-secondary: #6a6250;
  --color-text-tertiary: #8a8170;

  --color-border-primary: #d2c8ad;
  --color-border-secondary: #8a8170;

  --color-accent-primary: #9a6e1e;
  --color-accent-secondary: #b8842c;

  --color-success: #5f7a2e;
  --color-error: #a94a38;
  --color-warning: #b07d1c;

  --color-input-bg: #e8e1cd;
  --color-input-border: #d2c8ad;
  --color-input-focus: #9a6e1e;
  --color-hover-bg: #ddd4bc;
  --color-active-bg: #d2c8ad;

  --color-shadow: rgba(0, 0, 0, 0.12);
  --color-overlay: rgba(246, 241, 230, 0.9);
}
```

### `ink-medium` — dimmed warm tan

```css
[data-theme="ink-medium"] {
  --color-bg-primary: #cdc5af;
  --color-bg-secondary: #c4bca6;
  --color-bg-tertiary: #bbb39d;
  --color-bg-elevated: #b1a892;

  --color-text-primary: #1f1b14;
  --color-text-secondary: #443b2c;
  --color-text-tertiary: #5d5443;

  --color-border-primary: #ada593;
  --color-border-secondary: #5d5443;

  --color-accent-primary: #8a5e1e;
  --color-accent-secondary: #a8762c;

  --color-success: #57702a;
  --color-error: #9a4030;
  --color-warning: #9c6c1a;

  --color-input-bg: #bbb39d;
  --color-input-border: #ada593;
  --color-input-focus: #8a5e1e;
  --color-hover-bg: #b1a892;
  --color-active-bg: #a89e87;

  --color-shadow: rgba(0, 0, 0, 0.15);
  --color-overlay: rgba(205, 197, 175, 0.9);
}
```

## ThemeSelector entries

Append three cards to the `themes` array in `src/components/ThemeSelector.tsx`:

| id           | name        | description                       | preview   | icon    |
|--------------|-------------|-----------------------------------|-----------|---------|
| `ink`        | Ink         | Warm near-black with gold accent  | `#0f0e0c` | Moon    |
| `ink-light`  | Ink Light   | Warm parchment, gold accent       | `#f6f1e6` | Sun     |
| `ink-medium` | Ink Medium  | Dimmed warm tan parchment         | `#cdc5af` | Palette |

## Swatch-icon contrast fix (targeted)

`ThemeSelector` currently chooses the swatch icon color with
`themeOption.id.includes('light') ? 'text-gray-700' : 'text-gray-200'`. `ink-medium` has a
*light* tan swatch but no `"light"` in its id, so it would get a low-contrast light-gray
icon on a light background.

Replace the string check with a luminance test on the preview hex:

```ts
function isLightSwatch(hex: string): boolean {
  const c = hex.replace('#', '');
  const r = parseInt(c.slice(0, 2), 16);
  const g = parseInt(c.slice(2, 4), 16);
  const b = parseInt(c.slice(4, 6), 16);
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255 > 0.6;
}
```

Use `isLightSwatch(themeOption.preview) ? 'text-gray-700' : 'text-gray-200'`. This is
backward-compatible: it produces the same icon color as today for all six existing themes
(light/solarized-light/quiet-light → dark icon; dark/solarized-dark/kimbie-dark → light
icon) and fixes `ink-medium`.

## Out of scope

- No changes to `ThemeContext`, settings backend, or the `data-theme` mechanism.
- Loom's `parchment-medium` / `dark` and any other Loom themes are not ported.
- Loom's editor-specific vars (`--editor-*`, `--find-match-*`, `--mono-font-*`) have no
  Nebula analog and are not ported.

## Testing

- `npm run build` — TypeScript + Vite build must pass.
- `npm run tauri dev` — switch through `ink`, `ink-light`, `ink-medium` in
  Settings → Theme; verify chat, sidebar, settings, inputs, hover/active states, and the
  picker swatch/icon all read correctly. No automated theme tests exist in the repo.
