# Generation-info Hover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the inline `tok/s` text on each assistant message with an info (ℹ) icon whose hover popover shows model, provider, speed, tokens, and duration.

**Architecture:** The backend already emits a `stream-stats` event after each streamed response; add `model` + `provider` (both in scope in `send_message`) to that event. The frontend already stores those stats per message id; swap the inline tok/s span for a small self-contained `GenerationInfo` component (icon + hover card). Live-session only — no persistence.

**Tech Stack:** Rust (Tauri v2, serde), React 19 + TypeScript (Vite), Tailwind utility classes with CSS custom properties, lucide-react icons.

## Global Constraints

- **Live-session only.** No DB column, no persistence, no survival across reload. The icon/popover appears under the exact same condition tok/s does today (`m.role === "assistant" && genStats`); historical/reloaded messages show nothing.
- **No provider-reported usage.** Tokens is the existing local tokenizer estimate of the output, shown plainly as "Tokens" (no "(est.)" qualifier).
- **No inline model text.** Only the icon shows in the action bar; model/provider/etc. live in the hover card.
- **Popover fields, in order:** Model, Provider, Speed (tok/s), Tokens, Duration.
- **Theme via existing CSS vars** (`--color-bg-secondary`, `--color-border-primary`, `--color-text-tertiary`, `--color-text-secondary`). No new colors.
- **No new dependencies.** `Info` comes from the already-installed `lucide-react`.
- Commit trailer on every commit:
  ```
  🤖 Generated with [Claude Code](https://claude.com/claude-code)

  Co-Authored-By: Claude <noreply@anthropic.com>
  ```
- Spec: `docs/superpowers/specs/2026-07-07-generation-info-hover-design.md`.

---

### Task 1: Backend — add `model` + `provider` to the stream-stats event

**Files:**
- Modify: `src-tauri/src/lib.rs` (struct `StreamStatsEvent` ~134-141; emission ~1216-1225)

**Interfaces:**
- Consumes: in-scope owned `String`s `model` (bound at `lib.rs:572`) and `provider_id` (bound at `lib.rs:571`); both are still alive at the emission site (the `json!` at `lib.rs:1011-1017` borrows its values — `request_id` from that same `json!` is reused at the emission).
- Produces: the `stream-stats` Tauri event now carries `model: string` and `provider: string` (serde field names → JSON keys `"model"`, `"provider"`), consumed by Task 2.

This change forwards two in-scope strings into an existing event; there is no isolable unit to unit-test, so it is compile-and-regression verified here and end-to-end verified in Task 2's manual smoke test.

- [ ] **Step 1: Add the two fields to `StreamStatsEvent`**

In `src-tauri/src/lib.rs`, replace the struct (currently lines ~134-141):

```rust
#[derive(Clone, Serialize)]
struct StreamStatsEvent {
    request_id: Option<String>,
    conversation_id: Option<String>,
    tokens_per_second: f64,
    total_tokens: usize,
    duration_ms: u64,
    model: String,
    provider: String,
}
```

- [ ] **Step 2: Populate them at the emission site**

In `src-tauri/src/lib.rs`, in the `app_handle.emit("stream-stats", …)` call (currently lines ~1216-1225), add the two fields to the struct literal:

```rust
                use tauri::Emitter;
                let _ = app_handle.emit(
                    "stream-stats",
                    StreamStatsEvent {
                        request_id: request_id.clone(),
                        conversation_id: conversation_id.clone(),
                        tokens_per_second,
                        total_tokens: token_count,
                        duration_ms,
                        model: model.clone(),
                        provider: provider_id.clone(),
                    },
                );
```

- [ ] **Step 3: Verify it compiles**

Run: `cd src-tauri && cargo build`
Expected: compiles cleanly (no errors). In particular, no "borrow of moved value" on `model` or `provider_id`.

- [ ] **Step 4: Verify no test regressions**

Run: `cd src-tauri && cargo test`
Expected: the full suite passes (this change adds two serialized fields to a private struct; nothing constructs it in tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(chat): include model + provider in stream-stats event

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Frontend — info icon + hover card replacing inline tok/s

**Files:**
- Modify: `src/components/ChatInterface.tsx` (lucide import line 4; `StreamStatsEvent` interface ~161-167; new `GenerationInfo` component just before `ChatMessage` ~2203; action-bar render ~2466-2473)

**Interfaces:**
- Consumes: the `stream-stats` event fields `model: string` and `provider: string` from Task 1; the existing `genStats` prop of type `StreamStatsEvent` passed to `ChatMessage`.
- Produces: UI only.

No automated test (no frontend test suite per CLAUDE.md). Verified by `npm run build` + a manual smoke test.

- [ ] **Step 1: Add `model` + `provider` to the `StreamStatsEvent` interface**

In `src/components/ChatInterface.tsx`, replace the interface (currently ~161-167):

```tsx
interface StreamStatsEvent {
    request_id?: string | null;
    conversation_id?: string | null;
    tokens_per_second: number;
    total_tokens: number;
    duration_ms: number;
    model: string;
    provider: string;
}
```

- [ ] **Step 2: Import the `Info` icon**

In `src/components/ChatInterface.tsx` line 4, add `Info` to the existing `lucide-react` import (keep the other names; just add `Info`):

```tsx
import { Send, Terminal, AlertTriangle, Copy, Edit2, Trash2, RefreshCw, Check, Pin, FileText, Book, Paperclip, X, Brain, Square, Sliders, Download, Eye, ChevronRight, ChevronDown, Info } from "lucide-react";
```

- [ ] **Step 3: Add the `GenerationInfo` component**

In `src/components/ChatInterface.tsx`, immediately before the `const ChatMessage = memo(…)` definition (~line 2203), add this self-contained component. `useState` is already imported at the top of the file.

```tsx
// Info icon shown on assistant messages; hover/focus reveals a small card with
// the generation metadata (model, provider, speed, tokens, duration). Live-only:
// rendered only when stream stats exist for the message.
function GenerationInfo({ stats }: { stats: StreamStatsEvent }) {
    const [open, setOpen] = useState(false);
    const rows: [string, string][] = [
        ["Model", stats.model || "—"],
        ["Provider", stats.provider || "—"],
        ["Speed", `${stats.tokens_per_second.toFixed(1)} tok/s`],
        ["Tokens", `${stats.total_tokens}`],
        ["Duration", `${(stats.duration_ms / 1000).toFixed(1)}s`],
    ];
    return (
        <span
            className="relative inline-flex items-center mr-1 outline-none"
            tabIndex={0}
            aria-label="Generation info"
            onMouseEnter={() => setOpen(true)}
            onMouseLeave={() => setOpen(false)}
            onFocus={() => setOpen(true)}
            onBlur={() => setOpen(false)}
        >
            <Info
                size={13}
                className="text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] cursor-default"
            />
            {open && (
                <span
                    role="tooltip"
                    className="pointer-events-none absolute bottom-full left-0 mb-1 z-50 whitespace-nowrap rounded-lg border border-[var(--color-border-primary)] bg-[var(--color-bg-secondary)] px-3 py-2 text-[11px] font-mono shadow-lg"
                >
                    <span className="flex flex-col gap-0.5">
                        {rows.map(([label, value]) => (
                            <span key={label} className="flex justify-between gap-4">
                                <span className="text-[var(--color-text-tertiary)]">{label}</span>
                                <span className="text-[var(--color-text-secondary)]">{value}</span>
                            </span>
                        ))}
                    </span>
                </span>
            )}
        </span>
    );
}
```

Layout note: each row is a single keyed `<span>` using flex `justify-between`, so no `Fragment` and no `React`-namespace value import is required — this file imports only named hooks from `react` (`useState` is among them).

- [ ] **Step 4: Replace the inline tok/s span with the icon**

In `src/components/ChatInterface.tsx`, in the action bar (currently ~2466-2473), replace the entire assistant-stats span block:

```tsx
                        {m.role === "assistant" && genStats && (
                            <span
                                className="text-[var(--color-text-tertiary)] text-[11px] font-mono mr-1"
                                title={`${genStats.total_tokens} tokens in ${(genStats.duration_ms / 1000).toFixed(1)}s`}
                            >
                                {genStats.tokens_per_second.toFixed(1)} tok/s
                            </span>
                        )}
```

with:

```tsx
                        {m.role === "assistant" && genStats && (
                            <GenerationInfo stats={genStats} />
                        )}
```

- [ ] **Step 5: Build the frontend**

Run: `npm run build`
Expected: TypeScript + Vite build succeeds with no errors.

- [ ] **Step 6: Manual smoke test**

Run: `npm run tauri dev`

Verify:
1. Send a message; when the response finishes, the action bar shows a small **ℹ icon** where `42.1 tok/s` used to be (no inline tok/s text).
2. Hovering the icon shows a card with rows **Model**, **Provider**, **Speed**, **Tokens**, **Duration**, and the values are correct (model + provider match the model you used).
3. Moving the mouse away hides the card; tabbing to the icon (keyboard focus) also shows it.
4. Reload the conversation (switch away and back): historical assistant messages show **no icon** (live-only), matching the previous tok/s behavior.

- [ ] **Step 7: Commit**

```bash
git add src/components/ChatInterface.tsx
git commit -m "$(cat <<'EOF'
feat(chat): show generation info (model/provider/speed) in a hover card

Replace the inline tok/s text with an info icon; hover/focus reveals a
card with model, provider, speed, tokens, and duration.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**1. Spec coverage:**

| Spec requirement | Task |
|---|---|
| Add `model` + `provider` to `StreamStatsEvent` (backend), populate from in-scope vars | Task 1 |
| Extend frontend `StreamStatsEvent` interface with `model`/`provider` | Task 2, Step 1 |
| Replace inline tok/s with an `Info` icon under the same render condition | Task 2, Steps 2 & 4 |
| Hover card with Model / Provider / Speed / Tokens / Duration | Task 2, Step 3 |
| Themed with existing CSS vars, no new colors/deps | Task 2, Step 3 (uses `--color-*`, lucide `Info`) |
| Live-session only; historical messages show nothing | Preserved by keeping the `m.role === "assistant" && genStats` condition (Task 2, Step 4); verified in Step 6.4 |
| Tokens shown plainly (no "(est.)") | Task 2, Step 3 (`["Tokens", …]`) |
| No inline model text | Task 2, Step 4 (only `<GenerationInfo>` renders) |

No gaps.

**2. Placeholder scan:** No TBD/TODO. Every code step shows complete code; verification steps show exact commands and expected results. The one conditional (React.Fragment vs Fragment) is fully specified with the exact fallback, not left vague.

**3. Type consistency:**
- `StreamStatsEvent` gains `model: string` / `provider: string` (frontend, Task 2 Step 1) mirroring `model: String` / `provider: String` (backend, Task 1 Step 1); serde field names produce JSON keys `model`/`provider` that match the TS field names.
- `GenerationInfo({ stats }: { stats: StreamStatsEvent })` (defined Task 2 Step 3) is called as `<GenerationInfo stats={genStats} />` (Task 2 Step 4); `genStats` is already typed `StreamStatsEvent`.
- Field access (`stats.model`, `stats.provider`, `stats.tokens_per_second`, `stats.total_tokens`, `stats.duration_ms`) all exist on the interface.

Consistent.
