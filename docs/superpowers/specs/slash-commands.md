# Slash Commands — Implementation Spec

**Date:** 2026-06 (current session)
**Status:** Design approved — ready for implementation
**Owner:** (to be assigned)

## Background

Nebula already has one working "local command" precedent: `/remember <text>` (implemented client-side in `ChatInterface.handleSend`, lines ~1130-1162). It bypasses the LLM entirely, calls a dedicated Rust command for fact extraction, and surfaces the result as a `role: "system"` note that is persisted in the conversation.

Users have no discoverability for this feature, no general command system, and no fast access to the rich local capabilities (memory facts, full-text search via Tantivy, skills, tasks) without either:
- Using the side panels, or
- Prompting the LLM to use the corresponding tools.

This spec defines a clean slash-command system that gives power users direct, fast, local-only access to these capabilities.

## Goals

- Make existing `/remember` discoverable and consistent with a larger command surface.
- Provide immediate high-value commands for memory, search, and skills (tasks are already live in the right rail).
- Excellent keyboard-driven UX (type `/`, navigate with arrows, Esc to dismiss).
- Two discovery paths as requested: a visible "/" button + pressing `/` from the left sidebar (ConversationList) focuses the composer and opens command suggestions.
- All command output persisted as `role: "system"` messages (matching current `/remember` behavior).
- Client-only implementation (no new Tauri commands until genuinely required).
- Simple free-text argument style for v1.
- Mandatory documentation updates (in-app + written docs).

## Confirmed Design Decisions

1. **Scope** — Start with the plan outlined in the review session (memory/search heavy + meta).
2. **Architecture** — Pure client-side command registry + handlers in the frontend. Backend calls only via existing `invoke()` surface.
3. **Arguments** — Simple free text after the command name for v1 (`/remember this matters`).
4. **Persistence** — Command results are saved into the conversation as `role: "system"` messages (visible in history, export, etc.).
5. **Discovery** (user-specified):
   - Small "/" button in the composer area.
   - Pressing `/` while focused in the first column (ConversationList sidebar) focuses the chat textarea and triggers slash command suggestions.
   - In-app `/help` must be excellent.
   - Written docs update is mandatory.

## Discovery UX (Exact Behavior)

### Mechanism A — Visible "/" Button
- Location: In the composer bar (ChatInterface bottom input area), to the left of the paperclip button or between paperclip and textarea.
- Icon: Simple "/" using existing icon set or a small text label with monospace styling.
- Behavior on click:
  - Focus the textarea.
  - Insert a leading `/` (or just focus with suggestions visible).
  - Immediately show the command suggestions dropdown filtered to all commands.

### Mechanism B — "/" Key from First Column (ConversationList)
- When focus is anywhere in the ConversationList sidebar (chat list, its internal search input, etc.) and the user presses the `/` key:
  - Prevent default (do not type into sidebar search).
  - Focus the main chat textarea.
  - Seed the input value with `/` and open the suggestions dropdown showing all (or filtered) commands.
- This gives users who live in the left sidebar an extremely fast path to power features without moving their hands to the mouse or the composer.

### Standard Path (Typing in Composer)
- User types `/` as the first character in the textarea → suggestions dropdown appears below the textarea.
- Live filtering as they type more characters.
- Keyboard: ↑ ↓ to navigate, Enter to execute selected command (or submit current input if no exact match), Esc to dismiss suggestions and keep the `/` text.
- If the user types a full command + space + args and presses Enter, it executes directly (even if suggestions are open).

### Suggestions Dropdown Requirements
- Floating, positioned cleanly under the textarea (avoid clipping).
- Shows: command name (monospace), short description, usage hint.
- Filterable in real time.
- Empty state: "No matching commands. Press Enter to send as normal message."
- Max height with scrolling if many commands.
- Reusable component (potential future use for other palettes).

## v1 Command Catalog (Prioritized)

All commands are local-only, free-text args, and write a `role: "system"` result message.

| Command          | Args                  | Description                                                                 | Backend Surface Used                  | Priority |
|------------------|-----------------------|-----------------------------------------------------------------------------|---------------------------------------|----------|
| `/help`          | (none)                | Shows categorized, nicely formatted list of all available commands          | None (static registry)                | P0       |
| `/remember`      | `<text>`              | Existing fact extraction (migrate current logic)                            | `extract_facts_from_text`             | P0       |
| `/search`        | `<query>`             | Full-text search across all messages (current + past conversations)         | `search_messages` (already invoked from ConversationList) | P0 |
| `/facts`         | `[entity?]`           | List recent user facts, or facts about a specific entity                    | `list_user_facts`, `list_facts_for_entity` | P1    |
| `/skills`        | `[slug?]`             | List all skills. `/skills <slug>` shows the body (read-only view).          | `list_skills`, `get_skill`            | P1       |
| `/skill`         | `<slug>`              | **Invoke** a skill — loads its full body into the conversation as context. Typing `/skill ` (with space) shows the list of installed skills. | `get_skill` | P1 |
| `/clear`         | (none)                | Clear the current input field.                                              | None                                  | P1       |

**Notes on v1 scope:**
- Keep the set small enough to ship in one or two focused PRs.
- `/search` is especially high value because the heavy lifting (`search_messages` + Tantivy) already exists.
- `/skill <slug>` loads the full skill body into the conversation (as a system note) so it is available in context for subsequent messages.
- `/skills <slug>` is the read-only version (useful for inspection without polluting the conversation).

Future candidates: direct execution of skill logic (beyond just loading body), `/model`, `/export`, context compaction helpers, etc.

## Architecture & Implementation Notes

### Registry (Recommended Shape)
```ts
// src/utils/chatCommands.ts (or inside ChatInterface for v1 simplicity)
export interface ChatCommand {
  name: string;           // "remember"
  description: string;
  usage?: string;         // "/remember <text>"
  category: "Memory" | "Search" | "Skills" | "Meta";
  handler: (args: string, ctx: CommandContext) => Promise<void>;
}

export interface CommandContext {
  conversationId: string | null;
  invoke: typeof import("@tauri-apps/api/core").invoke;
  setInput: (v: string) => void;
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>;
  // add more as needed (selectedModel, etc.)
}
```

- Parser: very simple — first token after `/` is the command name, rest is `args.trim()`.
- Case-insensitive matching on name.
- Unknown command falls through to normal message send (with a gentle system note? or silent).

### Execution Flow
1. `handleSend` (or a new `handleSlashCommand`) sees input starting with `/`.
2. Parse → lookup in registry.
3. If found: clear input, call handler, handler does `invoke` + appends `role: "system"` message.
4. If not found: either error note or let it become a normal user message (decide in impl).

### Persistence
- Exactly like current `/remember`: create a `Message` with `role: "system"`, push to local state, and it will be saved on the next `send_message` round or via explicit persistence?  
  (Current behavior just mutates local `messages` state. The system note from `/remember` is **not** written to SQLite today unless a subsequent real message is sent.)

  **Important clarification needed during impl:** Do we want pure command notes to be immediately durable even if the user never sends another message? For v1 we can keep the existing "ephemeral until next real turn" behavior, or make command results explicitly persisted via a lightweight invoke.

### Styling for System Notes
- Current `ChatMessage` treats `role === "system"` with the generic "?" avatar.
- v1 should add distinct (but still subtle) styling — e.g. purple/amber left border or small "Command" badge — without over-investing.

## Phased Delivery

**Phase 0 — Foundation (shippable by itself)**
- Command registry + parser + `/help`.
- Migrate existing `/remember` logic into a registry entry.
- Basic suggestions dropdown (even if rough).
- `/` button in composer + sidebar `/` key wiring.
- In-app `/help` is already useful.

**Phase 1 — High-Value Commands**
- `/search`, `/facts`, `/skills`, `/clear`.
- Polish suggestions UI (good keyboard support, nice layout).
- Improve `role: "system"` rendering.

**Phase 2 — Documentation & Hardening**
- Outstanding docs updates (mandatory).
- Edge cases (streaming in progress, no conversation selected, long args, etc.).
- Manual test pass + `npm run build`.

## Files Expected to Change

**Frontend (primary):**
- `src/components/ChatInterface.tsx` (largest surface)
- New: `src/utils/chatCommands.ts` (recommended)
- New or co-located: `SlashCommandSuggestions.tsx` component
- `src/components/ConversationList.tsx` (add the global `/` key listener when focused in the list)

**Docs (mandatory):**
- `docs/superpowers/specs/slash-commands.md` (this file — update status on completion)
- `docs/features.md` (add a short "Implemented" note under a new section)
- In-app help text (generated from the registry)
- `README.md` (brief mention under Features or Usage)

**Rust:** None in v1 (purely reuse existing commands).

## Risks & Open Questions for Implementer

1. **Durability of command notes** — Should a `/search` result be written to SQLite immediately (new lightweight command?) or only ride along when the user sends a real message next? Current `/remember` is the latter.
2. **Long results** — `/search` or `/facts` can return a lot of text. Truncate in the system note + offer "Show full in Memory panel" or similar?
3. **Focus management** — Sidebar key listener must not fight the existing search input in ConversationList.
4. **React 19 double-render** — Keep listener cleanup strict.

## Success Criteria

- Typing `/` in composer shows a nice, filterable, keyboard-navigable list.
- "/" button and sidebar "/" key both work as specified.
- All v1 commands execute, produce a visible persisted system note, and do something useful.
- `/help` is the single source of truth for available commands.
- `npm run build` passes.
- Docs updated in the three required places.
- Power users can discover and use the feature within 30 seconds of first seeing the "/" affordance.

---

**Next step after this spec is approved:** Implementer should open a detailed todo list and begin Phase 0 (registry + `/help` + minimal suggestions + discovery wiring).