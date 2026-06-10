# Ink Themes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add three warm "Ink" themes (`ink`, `ink-light`, `ink-medium`) ported from the Loom project to Nebula's theme picker.

**Architecture:** Nebula themes are pure CSS — each is a `[data-theme="<id>"]` block of 21 `--color-*` variables in `src/themes.css`, registered as a card in the `themes` array in `src/components/ThemeSelector.tsx`. The theme id is a free string persisted to settings (no backend enum gate), so adding a theme requires no Rust changes. Loom's 12-variable palettes are mapped onto Nebula's 21-variable vocabulary, with a handful of values derived to stay in the warm Ink family.

**Tech Stack:** CSS custom properties, React + TypeScript, Vite, lucide-react icons.

**Testing note:** This repo has no JS/TS test runner (confirmed in CLAUDE.md). Verification is `npm run build` (TypeScript typecheck + Vite build) plus manual visual inspection in `npm run tauri dev`. No automated theme tests are added — that matches the approved spec.

**Spec:** `docs/superpowers/specs/2026-06-10-ink-themes-design.md`

---

### Task 1: Add the three Ink theme CSS blocks

**Files:**
- Modify: `src/themes.css` — insert after the `[data-theme="quiet-light"]` block (after line 190) and before the `/* Smooth theme transitions */` comment (line 192).

- [ ] **Step 1: Insert the three theme blocks**

Paste the following three blocks into `src/themes.css`, immediately after the closing `}` of the `[data-theme="quiet-light"]` block and before the `/* Smooth theme transitions */` comment:

```css
[data-theme="ink"] {
  /* Ink - warm near-black with gold accent (ported from Loom) */
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

[data-theme="ink-light"] {
  /* Ink Light - warm parchment, gold accent (ported from Loom) */
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

[data-theme="ink-medium"] {
  /* Ink Medium - dimmed warm tan parchment (ported from Loom) */
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

- [ ] **Step 2: Verify each block defines all 21 variables** (the same count every existing theme block defines)

This awk counts `--color-` declarations from each block's opening selector until its
closing `}` (robust against the blank lines inside the blocks):

Run: `awk '/\[data-theme="ink"\]/{f=1} f&&/--color-/{c++} f&&/^}/{print c; exit}' src/themes.css`
Expected: `21`
Run: `awk '/\[data-theme="ink-light"\]/{f=1} f&&/--color-/{c++} f&&/^}/{print c; exit}' src/themes.css`
Expected: `21`
Run: `awk '/\[data-theme="ink-medium"\]/{f=1} f&&/--color-/{c++} f&&/^}/{print c; exit}' src/themes.css`
Expected: `21`

- [ ] **Step 3: Verify the build still passes**

Run: `npm run build`
Expected: build completes with no errors (CSS is bundled; no TS impact yet).

- [ ] **Step 4: Commit**

```bash
git add src/themes.css
git commit -m "feat: add Ink, Ink Light, Ink Medium theme palettes"
```

---

### Task 2: Register the themes in the picker + fix swatch-icon contrast

**Files:**
- Modify: `src/components/ThemeSelector.tsx` — add a luminance helper above the component (before line 4, `export function ThemeSelector()`), append three cards to the `themes` array (before its closing `];` at line 50), and replace the icon `className` logic (lines 84-90).

The icons used (`Sun`, `Moon`, `Palette`) are already imported at line 1 — no new imports needed.

- [ ] **Step 1: Add the luminance helper above the component**

Insert this function immediately before `export function ThemeSelector() {` (currently line 4):

```ts
// Pick a readable icon color for a swatch based on the swatch's luminance,
// so light swatches get a dark icon and dark swatches get a light icon —
// regardless of whether the theme id contains "light".
function isLightSwatch(hex: string): boolean {
  const c = hex.replace('#', '');
  const r = parseInt(c.slice(0, 2), 16);
  const g = parseInt(c.slice(2, 4), 16);
  const b = parseInt(c.slice(4, 6), 16);
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255 > 0.6;
}
```

- [ ] **Step 2: Append the three theme cards**

In the `themes` array, insert these three entries after the `quiet-light` entry and before the array's closing `];`:

```ts
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
```

- [ ] **Step 3: Replace the icon className logic**

Find this block (currently lines 83-90):

```tsx
                <Icon
                  size={20}
                  className={
                    themeOption.id.includes('light')
                      ? 'text-gray-700'
                      : 'text-gray-200'
                  }
                />
```

Replace it with:

```tsx
                <Icon
                  size={20}
                  className={
                    isLightSwatch(themeOption.preview)
                      ? 'text-gray-700'
                      : 'text-gray-200'
                  }
                />
```

- [ ] **Step 4: Verify the build + typecheck passes**

Run: `npm run build`
Expected: TypeScript compiles and Vite build completes with no errors.

- [ ] **Step 5: Commit**

```bash
git add src/components/ThemeSelector.tsx
git commit -m "feat: register Ink themes in picker, fix swatch-icon contrast"
```

---

### Task 3: Manual visual verification

**Files:** none (verification only).

- [ ] **Step 1: Launch the app**

Run: `npm run tauri dev`
Expected: app launches without console errors.

- [ ] **Step 2: Switch through each Ink theme**

In Settings → Theme, select **Ink**, then **Ink Light**, then **Ink Medium**. For each, confirm:
- The whole UI (chat area, sidebar/ConversationList, SettingsPage, MemoryPanel) recolors — no leftover patches in the previous theme's colors.
- Text is readable against its background at all three levels (primary/secondary/tertiary).
- Inputs and buttons are visible; the focus ring on a focused input shows the gold accent.
- Hover and active states on conversation rows and buttons are visible.
- The picker card swatch icon is legible for all three (especially **Ink Medium**'s light tan swatch, which should now show a dark icon).

- [ ] **Step 3: Confirm persistence**

Select **Ink**, fully quit and relaunch the app. Expected: it reopens in **Ink** (theme persisted to settings).

- [ ] **Step 4: (No commit)**

This task changes no files. If visual issues are found, fix the relevant hex values in `src/themes.css` and amend/commit as appropriate, then re-verify.

---

## Done when

- All three themes appear in Settings → Theme and apply cleanly.
- `npm run build` passes.
- Theme selection persists across an app restart.
