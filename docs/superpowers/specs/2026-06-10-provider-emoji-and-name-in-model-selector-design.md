# Custom provider emoji + provider name in the model selector

**Date:** 2026-06-10
**Status:** Approved, ready for implementation plan

## Problem

The model selector (the typedown in the chat header) shows a per-provider emoji
icon next to each model name. The icon comes from `getProviderIcon()`, which
maps provider type/name to an emoji via heuristics. Many providers collapse to
the same icon — e.g. any `OpenAICompatible` provider that doesn't match a known
name falls through to `🔌`. So `bifrost` and `deepinfra` are visually identical,
and a model named `deepseek-v4-pro` offered by both is impossible to tell apart
in the dropdown.

Two fixes, independent of each other:

1. Let the user assign a **custom emoji per provider**, so near-identical
   providers can be distinguished at a glance.
2. **Show the provider name in the dropdown** so the source is always legible
   even without a custom emoji.

## Goals

- A provider can carry an optional custom emoji that overrides the heuristic
  icon everywhere the icon is shown.
- Each row of the model selector shows its provider name inline, dimmed:
  `🔌 deepseek-v4-pro · bifrost`. The closed/selected button shows it too.
- The provider name is filterable in the dropdown's search box.
- Backward compatible: existing `settings.json` files load unchanged.

## Non-goals (YAGNI)

- No separate "provider display name" concept — the existing provider **key**
  (`bifrost`, `deepinfra`) is the name shown.
- No per-model emoji — the emoji is per-provider only.
- No emoji-picker dependency / curated palette — a free-text field only.
- The icon is **presentation only**; it is never sent to any LLM.

## Design

### 1. Data model — one new optional field

Add an optional `icon` to `ProviderConfig` in both layers:

- **Rust** (`src-tauri/src/mcp/config.rs`):
  ```rust
  #[serde(default)]
  pub icon: Option<String>,
  ```
  `#[serde(default)]` means old settings files (which lack the field)
  deserialize fine — no migration step required.

- **TypeScript** (`src/components/ProvidersSettings.tsx`): add `icon?: string`
  to the `ProviderConfig` interface.

### 2. Emoji picker — free-text input in the provider card

In `ProviderCard` (`ProvidersSettings.tsx`), the large provider glyph at the top
(currently `getProviderIcon(config.provider_type, providerKey)`, ~line 105–107)
becomes editable:

- Clicking the glyph reveals a small inline text input (roughly `maxLength={2}`
  to allow an emoji or a short text glyph like `DS`).
- The input's `placeholder` is the current heuristic icon so the user sees what
  they're overriding.
- On change, call `onUpdate({ icon: value })`. An empty string stores
  `undefined` (i.e. clearing reverts to the heuristic).

### 3. `getProviderIcon` honors the override

Extend the signature with an optional custom-icon argument that takes priority:

```ts
getProviderIcon(type, providerId?, customIcon?)
```

If `customIcon` is a non-empty string, return it immediately; otherwise fall
through to the existing heuristics unchanged. Update both call sites to pass the
provider's `icon`:

- `ProvidersSettings.tsx` (~line 106): pass `config.icon`.
- `ChatInterface.tsx` (~line 1553): pass the model's provider `icon`. This
  requires the model option (`ModelOption`) to carry the provider's custom icon
  — sourced when `availableModels` is built (~line 761, where `providerId` /
  `providerType` are already attached from the provider config).

### 4. Model selector rows show the provider inline

- **`CustomSelect.tsx`**: add an optional `sublabel?: string` to `SelectOption`.
  When present, render it dimmed and smaller immediately after the label, in
  both the open option rows and the closed selected-button. Format:
  `{icon} {label} · {sublabel}` — the `·` separator and sublabel are dimmed
  (`text-[var(--color-text-tertiary)]`). Extend the filter predicate to also
  match `sublabel` (today it matches `label` + `value`; the provider id is
  already embedded in `value` as `providerId::modelId`, so this is belt-and-
  suspenders). The existing hover tooltip on the selected button will naturally
  include the provider once it's part of the rendered label area.

- **`ChatInterface.tsx`** (~line 1549 options map): set `sublabel: m.providerId`
  and `icon: getProviderIcon(m.providerType, m.providerId, m.icon)`.

`sublabel` is generic and optional, so other `CustomSelect` users (prompt
selector, etc.) are unaffected — they simply don't pass one.

## Affected files

- `src-tauri/src/mcp/config.rs` — add `icon` field to `ProviderConfig`.
- `src/components/ProvidersSettings.tsx` — `icon?` in interface; editable glyph
  input; pass `config.icon` to `getProviderIcon`.
- `src/utils/providerIcons.ts` — optional `customIcon` param, takes priority.
- `src/components/ui/CustomSelect.tsx` — `sublabel` on `SelectOption`; render it
  on rows + selected button; include in filter.
- `src/components/ChatInterface.tsx` — carry provider `icon` onto `ModelOption`;
  pass `sublabel` + custom icon into the selector options.

## Testing

- Manual: `npm run tauri dev` — set a custom emoji on two `OpenAICompatible`
  providers, confirm both the provider card and the model dropdown reflect it,
  confirm the provider name shows inline on rows and the closed button, confirm
  filtering by provider name works, confirm clearing the emoji reverts to the
  heuristic.
- Rust: optionally a serde round-trip test confirming `ProviderConfig` with and
  without `icon` deserializes (and that an old file lacking the field loads).
