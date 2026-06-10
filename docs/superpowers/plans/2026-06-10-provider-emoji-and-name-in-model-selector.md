# Custom Provider Emoji + Provider Name in Model Selector — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let each LLM provider carry an optional custom emoji, and show the provider's name inline in the model-selector dropdown so near-identical models from different providers are distinguishable.

**Architecture:** Add one optional `icon: Option<String>` field to `ProviderConfig` (Rust + TS). `getProviderIcon()` gains an optional override argument that takes priority over its heuristics. The provider card gets an editable emoji input. The generic `CustomSelect` component gains an optional `sublabel` that renders dimmed after the label (and on the closed button) and is included in filtering. `ChatInterface` feeds the provider id as `sublabel` and the provider's custom icon into the selector. The icon is presentation-only — never sent to any LLM.

**Tech Stack:** Rust (serde) backend, React 19 + TypeScript + Vite + Tailwind frontend, Tauri v2.

**Testing reality:** The backend has a `cargo test` suite (`src-tauri/src/mcp/config.rs` has a `#[cfg(test)] mod tests`), so the data-model change is done TDD-first there. The frontend has **no test suite** (per CLAUDE.md), so the React tasks are verified by `npm run build` (TypeScript typecheck) plus explicit manual checks in `npm run tauri dev`. Each task is self-contained and ends in a commit.

---

## File Structure

- `src-tauri/src/mcp/config.rs` — add `icon: Option<String>` to `ProviderConfig`; add serde tests. (Data model + backend test.)
- `src/utils/providerIcons.ts` — add optional `customIcon` param to `getProviderIcon`, taking priority. (Pure presentation helper.)
- `src/components/ui/CustomSelect.tsx` — add optional `sublabel` to `SelectOption`; render on rows + selected button; include in filter. (Generic, reusable select.)
- `src/components/ProvidersSettings.tsx` — add `icon?: string` to the `ProviderConfig` interface; make the provider glyph an editable emoji input; pass `config.icon` to `getProviderIcon`. (Settings UI.)
- `src/components/ChatInterface.tsx` — add `icon?` to `ModelOption`; populate it when building `availableModels`; pass `sublabel` + custom icon into the selector options. (Chat header wiring.)

Order is bottom-up: data model → presentation helper → generic component → settings UI → chat wiring. Each task compiles/builds and commits on its own.

---

## Task 1: Add `icon` field to Rust `ProviderConfig` (TDD)

**Files:**
- Modify: `src-tauri/src/mcp/config.rs:89-97` (the `ProviderConfig` struct)
- Test: `src-tauri/src/mcp/config.rs` (existing `#[cfg(test)] mod tests` at line ~670)

- [ ] **Step 1: Write the failing tests**

Add these two tests inside the existing `mod tests { ... }` block (after the existing `image_allowlist_explicit_empty_is_preserved` test, around line 708):

```rust
    #[test]
    fn provider_config_missing_icon_defaults_none() {
        // An existing provider entry written before the `icon` field existed
        // must still deserialize.
        let back: ProviderConfig = serde_json::from_str(
            r#"{"enabled":true,"provider_type":"OpenAICompatible","base_url":"http://x/v1","api_key":"k","models":[]}"#,
        )
        .unwrap();
        assert_eq!(back.icon, None);
    }

    #[test]
    fn provider_config_icon_round_trips_via_serde() {
        let cfg = ProviderConfig {
            enabled: true,
            provider_type: ProviderType::OpenAICompatible,
            base_url: Some("http://x/v1".to_string()),
            api_key: Some("k".to_string()),
            models: vec![],
            icon: Some("🦄".to_string()),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ProviderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.icon.as_deref(), Some("🦄"));
    }
```

Note: `ProviderType::OpenAICompatible` is the existing enum variant (see `ProviderType` near the top of `config.rs`). If the variant name differs, match the actual variant — do not invent one.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test provider_config_ 2>&1 | tail -20`
Expected: **compile error** — `ProviderConfig` has no field named `icon` (both the struct literal in the round-trip test and the `back.icon` access fail to compile). This is the expected "red" state.

- [ ] **Step 3: Add the field to the struct**

Modify `ProviderConfig` (lines 89-97) to add the field after `models`:

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProviderConfig {
    pub enabled: bool,
    pub provider_type: ProviderType,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
    /// Optional user-chosen emoji/glyph that overrides the heuristic provider
    /// icon in the UI. Presentation-only; never sent to any LLM.
    #[serde(default)]
    pub icon: Option<String>,
}
```

- [ ] **Step 4: Check for other `ProviderConfig` struct literals that now miss the field**

Run: `cd src-tauri && cargo build 2>&1 | tail -30`
If the build reports `missing field \`icon\`` at any other construction site, add `icon: None,` there. (As of writing, `ProviderConfig` is normally built from deserialization, not struct literals, so there may be none — but the compiler is the source of truth. Fix every site it names.)

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test provider_config_ 2>&1 | tail -20`
Expected: both `provider_config_missing_icon_defaults_none` and `provider_config_icon_round_trips_via_serde` **pass**.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/mcp/config.rs
git commit -m "feat: add optional icon field to ProviderConfig

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: `getProviderIcon` honors a custom-icon override

**Files:**
- Modify: `src/utils/providerIcons.ts:4` (function signature + first lines)

This helper has no test harness on the frontend; verification is by typecheck and by the manual checks in later tasks. Keep the change minimal and additive (new optional 3rd param), so existing 1- and 2-arg call sites stay valid.

- [ ] **Step 1: Add the optional `customIcon` parameter that takes priority**

Modify the start of `getProviderIcon` (currently line 4) from:

```ts
export function getProviderIcon(type: ProviderType | string | undefined, providerId?: string): string {
    const normalizedId = providerId?.toLowerCase() || "";
```

to:

```ts
export function getProviderIcon(
    type: ProviderType | string | undefined,
    providerId?: string,
    customIcon?: string,
): string {
    // A user-chosen emoji/glyph always wins over the heuristics below.
    const trimmedCustom = customIcon?.trim();
    if (trimmedCustom) return trimmedCustom;

    const normalizedId = providerId?.toLowerCase() || "";
```

Leave the rest of the function (all the heuristic `if` branches and the final `return "❓"`) unchanged.

- [ ] **Step 2: Typecheck the build**

Run: `npm run build 2>&1 | tail -20`
Expected: build succeeds (TypeScript compiles). Existing 2-arg callers remain valid because the new param is optional.

- [ ] **Step 3: Commit**

```bash
git add src/utils/providerIcons.ts
git commit -m "feat: let getProviderIcon accept a custom-icon override

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: Add `sublabel` to `CustomSelect` (render + filter)

**Files:**
- Modify: `src/components/ui/CustomSelect.tsx` — `SelectOption` interface (line 4-9), filter predicate (line 62-68), selected-button render (line 93-98), option-row render (line 148-154)

The `sublabel` is generic and optional. Other callers of `CustomSelect` (e.g. the prompt selector) don't pass it and are unaffected.

- [ ] **Step 1: Add `sublabel` to the `SelectOption` interface**

Modify the interface (lines 4-9) to add the field:

```ts
export interface SelectOption {
    id: string;
    label: string;
    value: string;
    icon?: React.ReactNode;
    sublabel?: string;
}
```

- [ ] **Step 2: Include `sublabel` in the filter predicate**

Modify the filter (lines 62-68) so typing a provider name narrows the list:

```ts
    const normalizedFilter = filter.toLowerCase();
    const matchingOptions = !filterable || !normalizedFilter
        ? options
        : options.filter((option) => {
              const label = option.label.toLowerCase();
              const valueStr = option.value.toLowerCase();
              const sublabel = option.sublabel?.toLowerCase() ?? "";
              return (
                  label.includes(normalizedFilter) ||
                  valueStr.includes(normalizedFilter) ||
                  sublabel.includes(normalizedFilter)
              );
          });
```

- [ ] **Step 3: Render `sublabel` on the closed/selected button**

Modify the selected-button label area (lines 93-98) from:

```tsx
                <div className="flex items-center gap-2 min-w-0">
                    {selectedOption?.icon && <span className="opacity-70">{selectedOption.icon}</span>}
                    <span className={`${!selectedOption ? "text-[var(--color-text-tertiary)]" : ""} truncate`}>
                        {selectedOption ? selectedOption.label : placeholder}
                    </span>
                </div>
```

to:

```tsx
                <div className="flex items-center gap-2 min-w-0">
                    {selectedOption?.icon && <span className="opacity-70">{selectedOption.icon}</span>}
                    <span className={`${!selectedOption ? "text-[var(--color-text-tertiary)]" : ""} truncate`}>
                        {selectedOption ? selectedOption.label : placeholder}
                    </span>
                    {selectedOption?.sublabel && (
                        <span className="text-[var(--color-text-tertiary)] text-xs truncate flex-shrink-0">
                            · {selectedOption.sublabel}
                        </span>
                    )}
                </div>
```

- [ ] **Step 4: Render `sublabel` on each option row**

Modify the option-row label area (lines 148-154) from:

```tsx
                                {option.icon && <span className="opacity-70 w-4 h-4 flex items-center justify-center">{option.icon}</span>}
                                <span
                                    className="flex-1 whitespace-normal break-words"
                                    title={option.label}
                                >
                                    {option.label}
                                </span>
```

to:

```tsx
                                {option.icon && <span className="opacity-70 w-4 h-4 flex items-center justify-center">{option.icon}</span>}
                                <span
                                    className="flex-1 whitespace-normal break-words"
                                    title={option.sublabel ? `${option.label} — ${option.sublabel}` : option.label}
                                >
                                    {option.label}
                                    {option.sublabel && (
                                        <span className="text-[var(--color-text-tertiary)] text-xs"> · {option.sublabel}</span>
                                    )}
                                </span>
```

- [ ] **Step 5: Typecheck the build**

Run: `npm run build 2>&1 | tail -20`
Expected: build succeeds.

- [ ] **Step 6: Commit**

```bash
git add src/components/ui/CustomSelect.tsx
git commit -m "feat: support optional dimmed sublabel in CustomSelect

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: Editable emoji input on the provider card

**Files:**
- Modify: `src/components/ProvidersSettings.tsx:24-30` (`ProviderConfig` interface), `:104-107` (the provider glyph in `ProviderCard`)

`ProviderCard` already receives `config` and `onUpdate(updates: Partial<ProviderConfig>)`. We add local edit state and turn the static glyph into a click-to-edit input.

- [ ] **Step 1: Add `icon?` to the `ProviderConfig` interface**

Modify the interface (lines 24-30) to add the field:

```ts
export interface ProviderConfig {
    enabled: boolean;
    provider_type: ProviderType;
    base_url?: string;
    api_key?: string;
    models: ModelConfig[];
    icon?: string;
}
```

- [ ] **Step 2: Add edit state to `ProviderCard`**

In `ProviderCard`, alongside the existing `useState` hooks (after line 42, `const [searchQuery, setSearchQuery] = useState("");`), add:

```ts
    const [editingIcon, setEditingIcon] = useState(false);
```

(`useState` is already imported at the top of the file.)

- [ ] **Step 3: Replace the static glyph with a click-to-edit input**

Modify the glyph block (lines 104-107) from:

```tsx
                <div className="flex items-center gap-3">
                    <div className={`text-2xl`} title={config.enabled ? "Enabled" : "Disabled"}>
                        {getProviderIcon(config.provider_type, providerKey)}
                    </div>
```

to:

```tsx
                <div className="flex items-center gap-3">
                    {editingIcon ? (
                        <input
                            autoFocus
                            maxLength={2}
                            defaultValue={config.icon ?? ""}
                            placeholder={getProviderIcon(config.provider_type, providerKey)}
                            onBlur={(e) => {
                                const v = e.target.value.trim();
                                onUpdate({ icon: v || undefined });
                                setEditingIcon(false);
                            }}
                            onKeyDown={(e) => {
                                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                                if (e.key === "Escape") setEditingIcon(false);
                            }}
                            className="w-10 text-2xl text-center bg-[var(--color-bg-primary)] border border-[var(--color-accent-primary)] rounded outline-none"
                        />
                    ) : (
                        <button
                            type="button"
                            onClick={() => setEditingIcon(true)}
                            className="text-2xl leading-none hover:bg-[var(--color-bg-tertiary)] rounded px-1 transition-colors"
                            title="Click to set a custom emoji (clear to reset)"
                        >
                            {getProviderIcon(config.provider_type, providerKey, config.icon)}
                        </button>
                    )}
```

Note the `getProviderIcon(config.provider_type, providerKey, config.icon)` call — the 3rd arg makes the custom emoji win. The `placeholder` on the input uses the 2-arg heuristic so the user sees what they're overriding.

- [ ] **Step 4: Typecheck the build**

Run: `npm run build 2>&1 | tail -20`
Expected: build succeeds.

- [ ] **Step 5: Manual verification**

Run: `npm run tauri dev` (or use a running dev instance). In Settings → Providers:
- Click a provider's emoji → an input appears prefilled with the current custom emoji (empty if none), placeholder shows the heuristic icon.
- Type/paste an emoji, press Enter → the glyph updates to the chosen emoji.
- Click again, clear the field, blur → the glyph reverts to the heuristic icon.
- Confirm the change persists after closing/reopening Settings (it's saved through the normal settings save path via `onUpdate`).

Expected: all four behaviors hold.

- [ ] **Step 6: Commit**

```bash
git add src/components/ProvidersSettings.tsx
git commit -m "feat: click-to-edit custom emoji on provider cards

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 5: Wire provider name + custom icon into the model selector

**Files:**
- Modify: `src/components/ChatInterface.tsx:174-183` (`ModelOption` interface), `:756-767` (model build loop), `:1549-1554` (selector options map)

- [ ] **Step 1: Add `icon?` to the `ModelOption` interface**

Modify the interface (lines 174-183) to add the field:

```ts
interface ModelOption {
    id: string;
    name: string;
    providerId: string;
    providerType: string;
    icon?: string;
    context_window?: number;
    supports_reasoning_effort?: boolean;
    supports_thinking_mode?: boolean;
    supports_extended_thinking?: boolean;
}
```

- [ ] **Step 2: Populate `icon` from the provider config when building models**

In the model build loop, modify the `models.push({ ... })` block (lines 758-767) to carry the provider's custom icon. `config` is the provider config object already in scope:

```ts
                                models.push({
                                    id: m.id,
                                    name: m.name || m.id,
                                    providerId: providerKey,
                                    providerType: config.provider_type, // Extract provider type
                                    icon: config.icon, // user-chosen emoji override, if any
                                    context_window: m.context_window,
                                    supports_reasoning_effort: m.supports_reasoning_effort,
                                    supports_thinking_mode: m.supports_thinking_mode,
                                    supports_extended_thinking: m.supports_extended_thinking,
                                });
```

- [ ] **Step 3: Pass `sublabel` + custom icon into the selector options**

Modify the model `CustomSelect` options map (lines 1549-1554) from:

```tsx
                            options={availableModels.map(m => ({
                                id: `${m.providerId}::${m.id}`,
                                label: m.name,
                                value: `${m.providerId}::${m.id}`,
                                icon: getProviderIcon(m.providerType, m.providerId)
                            }))}
```

to:

```tsx
                            options={availableModels.map(m => ({
                                id: `${m.providerId}::${m.id}`,
                                label: m.name,
                                value: `${m.providerId}::${m.id}`,
                                sublabel: m.providerId,
                                icon: getProviderIcon(m.providerType, m.providerId, m.icon)
                            }))}
```

- [ ] **Step 4: Typecheck the build**

Run: `npm run build 2>&1 | tail -20`
Expected: build succeeds.

- [ ] **Step 5: Manual verification**

Run: `npm run tauri dev`. With two enabled providers (e.g. two `OpenAICompatible` ones) that offer a same-named model:
- Open the model dropdown → each row reads `{icon} {model name} · {providerId}`, provider name dimmed/smaller.
- Set a distinct custom emoji on each provider (Task 4) → the dropdown rows for each provider now show their distinct emoji.
- Select a model → the closed button shows `{icon} {model name} · {providerId}`.
- Type a provider name (e.g. `bifrost`) in the dropdown's search box → the list narrows to that provider's models.
- Hover the closed button → tooltip shows the full label (now includes the provider).

Expected: all behaviors hold; the two same-named models are now distinguishable by both emoji and provider name.

- [ ] **Step 6: Commit**

```bash
git add src/components/ChatInterface.tsx
git commit -m "feat: show provider name and custom emoji in model selector

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Final verification

- [ ] **Backend tests pass:** `cd src-tauri && cargo test 2>&1 | tail -20` → all pass.
- [ ] **Frontend builds clean:** `npm run build 2>&1 | tail -20` → succeeds.
- [ ] **End-to-end manual check** (from Tasks 4 & 5 combined): set custom emoji on two providers, confirm they're distinct in both the provider cards and the model dropdown, provider names show inline on rows and the closed button, provider-name filtering works, and clearing an emoji reverts to the heuristic.
