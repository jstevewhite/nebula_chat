# Prompt Caching Plan

Status: **Shipped** — all phases implemented and merged-ready on branch
`feat/prompt-cache-phase0`. Verified live on Anthropic: the dashboard reports a
**~97% cache-read ratio**, and a representative repeat turn showed
`cache_read=18874, input=166, cache_write=195` (~98% of that prompt served from
cache). 2c was reduced in scope — see the Phase 2 note. This doc is retained as
the implementation record; per-turn behavior is in the code (`lib.rs` assembly,
`anthropic.rs` breakpoints, `ToolsPanel` lock UI).

Related change on the same branch (not part of the caching plan): the Anthropic
provider now honors `reasoning_effort` via adaptive extended thinking and
suppresses sampling params on models that reject them — see
`anthropic.rs::apply_sampling_options` and the `anthropic_*` helpers in
`capabilities.rs`.

Goal: enable LLM prompt caching (Anthropic explicit `cache_control`; OpenAI/DeepSeek
automatic) to cut token cost. Modeled on the same change shipped in the sibling
`oa` project. Magnitude scales with **tool-set size** (large MCP tool schemas →
large cached prefix) and **conversation length** (history caching) — it is not a
flat multiplier, and short chats with few tools can fall below the cacheable
threshold entirely.

## Current state (evidence, v0.9.0)

1. **No caching is requested.** `src-tauri/src/llm/anthropic.rs` builds the request
   as hand-rolled JSON (`system` / `tools` / `messages`, ~lines 53–88) and never
   sets `cache_control`. So every turn re-sends the full prefix at full price.

2. **Volatile content is front-loaded into the system block.** `src-tauri/src/lib.rs`
   assembles the prompt with repeated `final_messages.insert(0, …)`:
   - long-term memory block — `insert(0)` (~L799)
   - task checklist — `insert(0)` (~L820)
   - skills block — `insert(0)` (~L838)
   - active system prompt — `insert(0)` (~L847)
   - current date/time — `insert(1)` if a system prompt exists, else `insert(0)`
     (~L896/898), built from `chrono::Local::now()` (~L867)

   Resulting front order (top→bottom):
   `system-prompt → datetime → skills → tasks → memory → …history`.
   `convert_messages` (anthropic.rs:443) **flattens every `system`-role message into
   one `system` string** (`push_str` with `\n\n`). So a stable system prompt is
   immediately followed by a **datetime that changes every turn**, plus per-turn
   task and memory blocks — all inside what should be the cached prefix.

3. **Tool order is non-deterministic.** `McpManager::get_all_tools()`
   (`src-tauri/src/mcp/manager.rs`) iterates a `HashMap` (`for (name, client) in
   client_map`), so the tools array order can change between turns. Tools render
   **first** in the Anthropic prefix, so this silently invalidates the cache even
   when the user changes nothing.

**Net:** bolting on `cache_control` alone would barely hit — volatile content is
interleaved into the prefix and the tool order wobbles. Both must be fixed first.

## Decisions (resolved)

1. **Volatile placement:** trailing-after-history. Datetime, task checklist, and
   long-term memory move out of the `system` block to a trailing `system-reminder`
   message appended **after** the conversation history (mirrors oa Phase 2). This
   also reads more correctly — "now" is relative to the latest turn.
2. **Tool lock:** a **global** setting, persistent until the user turns it off
   (not per-conversation).
3. **Cache TTL:** 5-minute default, `1h` opt-in via config/setting.

## Work (phased)

### Phase 0 — make the prefix cacheable (provider-agnostic; benefits OpenAI/DeepSeek auto-cache immediately, no Anthropic-specific code yet)

- **0a. Deterministic tool order.** Sort the final `tools` Vec by name **after all
  tools are assembled** — i.e. after the builtin `update_tasks` / `memory_*` /
  `use_skill` tools are pushed (`lib.rs` ~L937), not at ~L904 before them. Sorting at
  L904 only orders the MCP tools and leaves the builtins appended in fixed order
  after them; that's still deterministic, but the array isn't fully canonical and
  Phase 1b's "last tool" breakpoint would land on whichever builtin happens to be
  last. Sort the whole array once, at the end. Without this, nothing in the tools
  block caches. Smallest, highest-leverage change.
- **0b. Separate stable vs volatile system content.** Restructure the `lib.rs`
  assembly so the **stable** system content (active system prompt + skills block)
  forms one contiguous front block, and the **volatile** content (datetime + task
  checklist + long-term memory) is collected separately and appended **after the
  history** as a trailing `<system-reminder>` message **with `role: "user"`** (not
  `role: "system"`). This is load-bearing: `convert_messages` (anthropic.rs:442)
  collects **every** `system`-role message into the flat system string regardless of
  its position in the vec — so a trailing reminder left as `role: "system"` gets
  pulled straight back into the cached prefix, defeating the whole change. Emit it as
  a user-role message, **or** change `convert_messages` to only fold *leading*
  system messages into the system block. (Anthropic's `mid-conversation-system`
  beta would allow a true trailing system message, but it's model-gated; the
  user-role reminder is the model-agnostic choice.) After this, the
  `[tools + stable-system + history]` prefix is byte-stable across turns.

### Phase 1 — Anthropic `cache_control` breakpoints

- **1a.** In `anthropic.rs`, emit `system` as an **array of text blocks** instead of
  a flat string, with `cache_control: {type:"ephemeral"}` on the (single) stable
  system block. Requires the provider call to receive stable-system separately from
  the volatile trailing content.
- **1b.** Add `cache_control` to the **last tool** in the tools array (caches the
  whole tools block) and to the **last history message block** (caches the
  conversation prefix). The volatile trailing reminder stays uncached.
- **1c.** TTL knob: 5m default, `1h` opt-in (provider/setting). 1h doubles the write
  cost but survives idle gaps > 5 min — useful for bursty desktop sessions.

### Phase 2 — global tool lock (the cache-protecting toggle)

- **2a. Done.** Global, persisted `tools_locked` setting (`Settings`,
  default false). The 1h-TTL toggle (`anthropic_cache_ttl_1h`, Phase 1c) gets its
  UI here too.
- **2b. Done.** When locked, the tool enable/disable controls in `ToolsPanel`
  (the RightRail Tools tab — the only place tools are toggled) are disabled and
  show "Tools locked — unlock to change". Defense-in-depth: `toggle_tool` and
  `toggle_tool_list` also **reject** with an error when `tools_locked` is set, so
  even a direct IPC call can't alter `disabled_tools` mid-conversation. A hint
  ("preserves prompt cache") sits on the toggle.
- **2c. Reduced in scope — not implemented as a snapshot.** The original concern
  was the *resolved* tool set changing between turns via runtime/async gating
  (`memory_*` on `doc_store_ready`, `use_skill` on skills existence). On
  reflection this is a **startup-only, self-healing transient**: `doc_store_ready`
  flips false→true once during init and stays true, so a conversation started
  after init is stable, and the only cost in the rare flip case is one extra cache
  write (never a correctness issue or persistent miss). The live `cache_read`
  result confirms steady state is solid. Per-conversation snapshot machinery was
  judged disproportionate; the lock (2a/2b) covers the real, user-driven
  cache-bust. Skills added/removed mid-conversation (a deliberate filesystem
  action) remain an accepted, un-locked source of change. Revisit only if a
  snapshot proves necessary in practice.

## Caveats / magnitude

- **Min cacheable prefix:** 2048 tokens (Sonnet 4.6) / 4096 (Opus 4.x). Short chats
  with few tools and a small system prompt won't cache until the prefix is large
  enough — below the floor there's no error, just `cache_creation_input_tokens: 0`.
  The win concentrates where tools are loaded and conversations run long.
- **Max 4 `cache_control` breakpoints per request.** The plan uses 3 (stable system
  + last tool + last history), leaving headroom.
- **Compaction resets the messages cache.** When `llm/compactor.rs` fires, it
  rewrites older turns, changing the history prefix and invalidating the Phase 1b
  conversation-history breakpoint. This is expected — the tools+system cache
  survives — but a cache-miss immediately after compaction is **not** a regression.
- **Ollama** is local — caching there is a latency nicety, not a billing win.
- **5-minute TTL:** idle gaps longer than the TTL cold-write the prefix again; that's
  when the 1h opt-in pays off.

## Conventions (from CLAUDE.md / AGENTS.md / WARP.md)

- Rust: `cargo fmt -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test` (tests in `#[cfg(test)]` blocks).
- Frontend: `npm run build` must pass; React functional components, 2-space indent,
  double quotes.
- Commits: imperative subjects with `feat:` / `fix` / `chore` prefixes; small,
  focused PRs.

## Tests

- **Rust unit tests:**
  - tool ordering is deterministic across repeated `get_all_tools` calls, **and the
    sort happens after the builtins are appended** (full array is canonical);
  - the trailing reminder is emitted with `role: "user"` (or `convert_messages` only
    folds leading system messages), so datetime/tasks/memory land **after** history
    and not in the flattened system string;
  - the system split puts stable content in the cached block and volatile content
    (datetime/tasks/memory) in the trailing reminder, not the prefix;
  - the resolved tool set is identical whether or not `doc_store_ready` flipped
    between turns once the conversation is locked (Phase 2c);
  - the Anthropic request body carries `cache_control` on the stable system block,
    the last tool, and the last history message — and **not** on the volatile tail.
- **Manual:** two-turn check asserting `cache_creation_input_tokens > 0` on turn 1
  and `cache_read_input_tokens > 0` on turn 2 (zero reads across identical-prefix
  turns means a silent invalidator — diff the rendered prefix bytes). Verify the
  lock toggle freezes tools, and that a fresh-startup first turn followed by a
  second turn (after DocStore init) still reads cache.

## Out of scope (for now)

- OpenAI/DeepSeek get the Phase 0 benefit automatically (stable prefix + sorted
  tools); no explicit breakpoint API needed.
- Worker/sub-agent style flows (if any) are not addressed here.
