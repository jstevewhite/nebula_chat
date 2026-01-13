# Memory system improvements (Strategist-driven retrieval loop, internal-only)

Goal: improve memory quality and controllability by letting the **strategist** (a secondary “context_model”) manage retrieval + context assembly **internally** (no memory exposed as a tool to the main chat model). This keeps the UX safe while enabling smarter context.

This doc is grounded in the current implementation:
- Retrieval/injection happens inside `src-tauri/src/lib.rs` in `send_message` (memory search + optional strategist summarization).
- Strategist implementation is `src-tauri/src/llm/context_assembler.rs`.
- Memory search is `Librarian::search` (`src-tauri/src/memory/librarian.rs`) backed by Tantivy (`src-tauri/src/memory/tantivy_index.rs`).
- Messages live in SQLite via `src-tauri/src/memory/sqlite_manager.rs`.

## Current state (what happens today)

### Retrieval and injection
- `send_message` captures `query` from the latest **user** message.
- If `settings.memory_enabled && !query.is_empty()`:
  - `lib.search(&query)` returns `Vec<SearchResult>`.
  - Results are immediately converted to `Vec<String>` via `res.content.clone()`.
  - If `settings.context_model` is set, the strategist is invoked via `ContextAssembler::assemble(...)`.
  - Strategist output is injected as a system message (“Refined Context: …”).

### Why this caps quality
1. **Strategist is downstream of retrieval**: it only filters/summarizes a single pass of search; it cannot steer retrieval.
2. **You discard metadata**: `SearchResult` includes `message_id`, `role`, `created_at`, `score`, but strategist only sees raw strings.
3. **No explicit retrieval controls**: no scope (conversation-only vs global), role filters, recency windows, or query refinements.
4. **Potential tool-output pollution**: tool outputs and other noisy content can be indexed and retrieved unless filtered.

## Target design

### Core concept: 2-stage strategist loop (bounded)
Implement an internal **retrieval planner + synthesizer** loop:

1. **Initial retrieval (fast, deterministic)**
   - Run Tantivy search using the user query with safe defaults.
2. **Planning step (strategist decides how to retrieve)**
   - Strategist returns a small JSON plan describing 0..N additional searches and filters.
3. **Follow-up retrieval (bounded)**
   - Execute strategist-requested searches with hard limits.
4. **Synthesis step (strategist composes final context)**
   - Provide merged hits (with metadata) and ask strategist to produce a concise context block.

This is internal-only. The main chat model still receives only:
- the final injected context
- not the ability to call memory search

### Hard safety limits (enforced in Rust)
- Max retrieval rounds: **2** (initial + follow-up)
- Max additional queries: **3**
- Max total hits considered: **20**
- Max snippet per hit: **400–800 chars**
- Default roles included: `assistant`, `system_summary` (optionally `user`), **exclude `tool` by default**
- Default recency window: optional, but recommended (e.g. last 365 days) with override allowed
- Timeout for strategist calls and retrieval loop so the UI does not hang

## Implementation plan

### Phase 1 — Preserve metadata end-to-end

#### 1.1 Introduce a memory hit DTO
Add a lightweight struct for strategist consumption (typed, stable):
- `message_id: String`
- `conversation_id: String`
- `role: String`
- `created_at: String`
- `score: f32`
- `snippet: String` (truncated content)

This can be either:
- a new struct in `src-tauri/src/memory/mod.rs`, or
- reuse `SearchResult` and add a snippet method (prefer dedicated DTO so you control shape).

#### 1.2 Stop converting hits to `Vec<String>`
In `src-tauri/src/lib.rs` where you build `memory_list_preview`, instead keep the structured hits.

- Update `ContextAssembler::assemble(...)` signature to accept `&[MemoryHit]` rather than `&[String]`.
- Provide the strategist enough context to reason: role + created_at + score + snippet.

#### 1.3 Improve the memory UI event payload
You currently emit `memory-context` with an array of strings.

Upgrade (without breaking UI):
- Keep existing `memory-context` for backward compatibility.
- Add a new event: `memory-hits` containing structured hits (IDs + snippets).

### Phase 2 — Add search options (scope/roles/recency) to Librarian/Tantivy

#### 2.1 Add `Librarian::search_with_options`
Add a new API in `src-tauri/src/memory/librarian.rs`:
- `search_with_options(query: &str, opts: SearchOptions) -> Result<Vec<SearchResult>>`

`SearchOptions` should include:
- `limit: usize`
- `conversation_id: Option<String>` (scope)
- `include_roles: Option<Vec<String>>`
- `exclude_roles: Option<Vec<String>>`
- `max_age_days: Option<u64>`

Initially, it’s acceptable to implement role/scope filtering **in Rust** after retrieving TopDocs (fast to ship).
Later, implement proper Tantivy filtering.

#### 2.2 Tantivy query improvements (optional but recommended)
`TantivyIndex::search` currently parses the query over `content` only.

Medium-term improvements:
- Add query-time filtering by `conversation_id` and `role`.
- Add recency filtering:
  - simplest: store `created_at` as RFC3339 and post-filter in Rust
  - best: store a sortable numeric timestamp field for range queries

### Phase 3 — Strategist “intentional search” loop (internal)

#### 3.1 Create `StrategistMemoryOrchestrator`
Add `src-tauri/src/memory/strategist.rs` with a single entrypoint, e.g.:
- `assemble_context(query, recent_history, librarian, settings) -> Result<StrategistContextResult>`

Responsibilities:
1. Call `librarian.search_with_options` for initial hits.
2. If no `settings.context_model`: return baseline formatted context.
3. Else run:
   - planner call → get plan JSON
   - follow-up searches → merge/dedupe
   - synthesis call → final context string + selected IDs

Return shape:
- `context_text: String`
- `selected_message_ids: Vec<String>`
- `search_plan: Option<SearchPlan>` (for diagnostics)

#### 3.2 Define a strict “SearchPlan” JSON schema
The strategist should output a JSON object you validate and clamp.

Suggested schema:
- `queries: [{ q, limit?, scope?, roles?, max_age_days? }]`
- `notes: string` (optional)

Validation rules:
- clamp counts (max queries, max limit)
- default role behavior if absent
- reject malformed JSON and fall back to initial retrieval

#### 3.3 Planner prompt
Inputs:
- user query
- recent conversation snippet (last N user/assistant turns, already supported by `context_turns`)
- initial hits list with metadata

Output:
- SearchPlan JSON only (no prose)

#### 3.4 Synthesis prompt
Inputs:
- user query
- recent conversation
- merged hits with metadata

Output:
- concise context block
- optionally include a compact “Sources:” list of message IDs (not shown to user unless you want)

### Phase 4 — Reduce noise and improve memory quality

#### 4.1 Default retrieval filters
Set defaults for initial retrieval:
- exclude `tool` role unless explicitly requested by planner
- exclude extremely short/empty content
- optionally deprioritize “system” prompts that are not summaries

#### 4.2 Snippet strategy
Instead of passing full content to strategist:
- store full content in DB as you do today
- pass snippets:
  - first N chars
  - plus optional match-context later

This protects token budgets and keeps strategist behavior stable.

#### 4.3 Conversation-local vs global retrieval
Use planner to decide:
- first search conversation-local for continuity
- then broaden to global if needed

Implement as:
- initial: `conversation_id = current` (small limit)
- follow-up: `conversation_id = None` (global), if planner asks

### Phase 5 — Observability + UX for debugging memory

#### 5.1 Add events for transparency
Emit structured events:
- `memory-search-plan`
- `memory-search-hits-initial`
- `memory-search-hits-final`

Keep payloads bounded.

#### 5.2 Add Settings toggles (optional)
Add settings for:
- `memory.strategist_enabled` (or use presence of `context_model`)
- `memory.max_queries`, `memory.max_hits`, `memory.snippet_chars`
- `memory.default_scope` (conversation vs global)

Keep defaults safe.

## Integration points (what to change)

### Backend
- `src-tauri/src/lib.rs`
  - Replace direct `ContextAssembler::assemble(...)` call with orchestrator call.
  - Stop converting search results to `Vec<String>`.
- `src-tauri/src/llm/context_assembler.rs`
  - Evolve into (or be used by) the orchestrator:
    - keep the “synthesis” behavior, but add the “planner” behavior.
  - Change input types from `&[String]` to structured hits.
- `src-tauri/src/memory/librarian.rs`
  - Add `search_with_options`.
- `src-tauri/src/memory/tantivy_index.rs`
  - Optional: add filtering support; otherwise rely on post-filtering.

### Frontend
- `src/components/ChatInterface.tsx`
  - Optionally listen to new events (`memory-search-plan`, `memory-hits`) to display what the strategist did.
- `src/components/MemoryPanel.tsx`
  - Extend panel to show:
    - queries executed
    - hit snippets
    - selected sources

## Acceptance criteria

Functional
- With strategist enabled, memory context improves on queries that require disambiguation or multi-step retrieval.
- The strategist can request follow-up searches (bounded) and results are reflected in the final injected context.

Safety
- The system never runs more than the configured max queries/hits/rounds.
- If strategist JSON is invalid or times out, fallback to baseline retrieval + simple injection.

Performance
- Typical retrieval loop remains fast (initial retrieval is Tantivy-only; strategist calls are bounded).

Traceability
- For any injected context block, you can show which memory IDs were selected (at least internally).

## Test plan

Unit tests
- SearchPlan JSON parsing + clamping.
- Role/scope filtering in `search_with_options`.

Integration tests
- “planner invalid JSON” → fallback path.
- “planner requests extra queries” → merged hits include results from both stages.

Manual test scripts
- Create a chat with multiple topics.
- Ask an ambiguous question (requires older context).
- Confirm:
  - strategist issues at least one targeted search
  - final context is smaller and more relevant than raw hits

## Recommended next step

If you want, I can implement this incrementally starting with Phase 1 (metadata preservation + new DTO + updated assembler signature) while keeping behavior identical (no planner loop yet), then add the planner loop behind a settings flag for controlled rollout.
