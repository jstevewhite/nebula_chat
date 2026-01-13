# Memory System Phase 2: Knowledge Graph (Facts) — Revised Plan

## Problem statement
Nebula’s current memory system is strong at full-text retrieval (Tantivy), but it cannot reliably store and retrieve durable, structured facts (user preferences, project attributes, decisions) with provenance. This plan adds a **knowledge-graph-like fact layer** backed by SQLite to complement Tantivy, while preserving existing behavior.

Goals:
- Fast structured lookups for durable facts (user profile, project tech stack, decisions).
- Better personalization (“Given your role/preferences…”).
- Relationship traversal for entity-centric queries (bounded and typed to prevent noise).
- Deduplication into canonical facts (not scattered conversational mentions).
- Provenance tracking: facts reference the source message when known.

Non-goals (v0):
- Perfect entity linking / fuzzy canonicalization (defer to later).
- Global, unbounded graph reasoning (keep traversal shallow and safe).
- Heavy UI work (optional).

## Current state (codebase alignment)
- SQLite schema is initialized in `src-tauri/src/memory/sqlite_manager.rs`.
- Migrations exist as ad-hoc helpers like `migrate_v2()`.
- `Librarian` orchestrates SQLite + Tantivy in `src-tauri/src/memory/librarian.rs`.
- The request pipeline is `send_message` in `src-tauri/src/lib.rs`.
- Memory retrieval is orchestrated by `StrategistMemoryOrchestrator::assemble_context` in `src-tauri/src/memory/strategist.rs` (today it does not accept `conversation_id`).

Key constraints to respect:
- Keep SQL in `SqliteManager` to preserve layering.
- SQLite foreign key enforcement must be explicitly enabled.
- Provenance needs message IDs; right now `Librarian::save_full_message` discards message IDs.
- `rusqlite` is synchronous; avoid long DB work on the async executor.

## Architecture (hybrid memory)
- Tantivy: best-effort semantic / keyword retrieval.
- Facts (SQLite): authoritative structured facts with optional provenance.
- Strategist: combines both into a context block for the primary model.

## Fact model (typed triples)
Use a typed triple structure:
- `subject` (entity key, normalized)
- `predicate` (relationship key, normalized)
- `object` (string value)
- `object_kind` determines whether `object` is an **ENTITY** (eligible for traversal) or a **LITERAL** (not traversed)

Examples:
- (user, role, systems_engineer, LITERAL)
- (user, prefers, vim_keybindings, ENTITY or LITERAL depending on your taxonomy)
- (nebula_chat, uses, tauri, ENTITY)
- (tauri, uses, rust, ENTITY)
- (user, decided, use_strategist_for_memory, LITERAL)

Normalization guideline:
- Store a normalized key for lookups (e.g., lowercased, whitespace collapsed).
- Optionally store a display label later; for v0, normalize consistently.

## Data model and schema

### 1) Connection safety prerequisite (mandatory)
SQLite foreign keys must be enabled for every connection.

Change in `SqliteManager::new`:
- Execute `PRAGMA foreign_keys = ON;` immediately after opening the DB connection.

Rationale:
- `ON DELETE SET NULL` for provenance cleanup will not work otherwise.

### 2) Facts schema (v0)
Add a `facts` table with typed object semantics and basic indexing.

Proposed schema:
- `id TEXT PRIMARY KEY`
- `subject TEXT NOT NULL`
- `predicate TEXT NOT NULL`
- `object TEXT NOT NULL`
- `object_kind TEXT NOT NULL`  (enum-like: 'entity' | 'literal')
- `confidence REAL NOT NULL DEFAULT 1.0`
- `source_message_id TEXT NULL`
- `created_at TEXT NOT NULL`  (RFC3339 UTC)
- `updated_at TEXT NOT NULL`  (RFC3339 UTC)

Constraints:
- Consider a uniqueness constraint to avoid duplicates:
  - `UNIQUE(subject, predicate, object, object_kind)`

Foreign key:
- `FOREIGN KEY (source_message_id) REFERENCES messages(id) ON DELETE SET NULL`

Indexes:
- `(subject)`
- `(predicate)`
- `(object)` (optional; consider if it will be queried often)
- `(subject, predicate)`
- `(source_message_id)`

Notes:
- Timestamps can remain TEXT to match existing code patterns (RFC3339 sorts lexicographically).

### 3) Migrations strategy
Current code uses ad-hoc migration methods (e.g. `migrate_v2`). For facts, keep the same approach for now to minimize churn.

Add `migrate_facts_v1()` to `SqliteManager`:
- Create the `facts` table if missing.
- Create indexes with `IF NOT EXISTS`.
- If evolving later, add `migrate_facts_v2()` and call both in sequence.

Call site:
- `Librarian::new()` should call this migration (like it calls `migrate_v2`).

## Rust API surface (layering preserved)

### Types
Add these to `src-tauri/src/memory/mod.rs` (or a new `facts.rs` module re-exported by `mod.rs`):
- `Fact`
- `ObjectKind` enum
- `RelevantFact` (for strategist consumption)

Guideline:
- Avoid exposing raw SQL concerns outside `SqliteManager`.

### SqliteManager methods (all SQL lives here)
Add to `src-tauri/src/memory/sqlite_manager.rs`:
- `save_fact(fact: NewFact) -> Result<String>`
- `upsert_fact(...) -> Result<String>` (recommended to enforce the uniqueness constraint)
- `update_fact_confidence(id, confidence) -> Result<()>`
- `delete_fact(id) -> Result<()>`
- `delete_facts_by_source_message(message_id) -> Result<()>` (optional with FK ON)
- `query_facts(subject: Option<&str>, predicate: Option<&str>, object: Option<&str>, object_kind: Option<ObjectKind>, limit: usize) -> Result<Vec<Fact>>`
- `get_facts_about_entity(entity: &str, limit: usize) -> Result<Vec<Fact>>` (where `subject = entity OR (object = entity AND object_kind='entity')` if you want inbound edges too)

Implementation guidance:
- Use parameter binding (rusqlite params) and avoid interpolating user strings into SQL.
- If you need dynamic WHERE clauses, build the SQL skeleton carefully and bind params with `params_from_iter` using owned values.

### Librarian methods (high-level orchestration)
Add to `src-tauri/src/memory/librarian.rs`:
- `get_user_profile_facts() -> Result<Vec<Fact>>` (delegates to SqliteManager query methods)
- `get_conversation_project_facts(conversation_id) -> Result<Vec<Fact>>` (optional; can be heuristic-based at first)

Note:
- `Librarian` already wraps SQLite; keep it as the orchestrator only.

## Provenance: ensure message IDs are available
This is required to store `source_message_id` and to support deletions.

### A) Add an ID-returning save method
Current:
- `SqliteManager::save_full_message(...) -> Result<(String, String)>`
- `Librarian::save_full_message(...) -> Result<()>`

Revision:
- Add a new method in `Librarian` that returns the message id:
  - `save_full_message_returning_id(...) -> Result<String>`

Then update `send_message` to use it at the points where you need provenance.

Rationale:
- This is less disruptive than changing existing callers and signatures everywhere.

## Strategist integration (facts + memories)

### 1) Add conversation scoping to strategist
Revise strategist API:
- `StrategistMemoryOrchestrator::assemble_context(..., conversation_id: Option<&str>, ...)`

Reason:
- Fact retrieval benefits from knowing which conversation (project context, relevant entities).

Update call site in `src-tauri/src/lib.rs` where `assemble_context` is invoked.

### 2) Relevant fact selection
Add a small fact retrieval function used by strategist:
- Always include user-profile facts (bounded list) for personalization.
- Extract candidate entities from the query (simple heuristic v0).
- Fetch facts about those entities.
- Optionally traverse one hop from entity-to-entity facts only (`object_kind='entity'`).

Safety bounds:
- Max entities extracted: ~10
- Max traversal depth: 1–2
- Max facts returned into prompts: 20

### 3) Prompt formatting
When formatting facts for planner/synthesizer:
- Separate into blocks:
  - USER PROFILE (stable)
  - KNOWN FACTS (entity/context)
- Include confidence and provenance presence (do not dump full message ids, but can indicate “(sourced)” vs “(inferred)”).

## Fact extraction pipeline (background)

### 1) Extraction trigger points
Hook extraction after saving new messages in `send_message`:
- For the user message (role="user")
- For the assistant response message (role="assistant")

Only do this if:
- memory is enabled
- a “context model” (or a dedicated extraction model setting) exists

### 2) Implementation details
- Extract facts from message content using an LLM prompt that returns JSON.
- For each extracted fact:
  - Normalize subject/predicate/object
  - Determine object_kind:
    - If object is a known entity term or matches a compact entity regex, mark ENTITY; else LITERAL
  - Upsert into SQLite with provenance `source_message_id = saved_message_id`

Async safety:
- Since `rusqlite` is blocking, keep DB writes small and bounded.
- If extraction becomes slow, consider:
  - `tokio::task::spawn_blocking` for DB writes, or
  - moving extraction into a dedicated worker queue later.

Provider construction:
- If you need to create an LLM provider from background tasks, expose a reusable factory:
  - either make `StrategistMemoryOrchestrator::create_provider` public, or
  - add a small `llm_factory.rs` utility used by both strategist and extraction.

## Deletion and maintenance
- With FK enabled, deleting a message will NULL out `source_message_id` in facts, preserving the fact while removing provenance linkage.
- Optionally add cleanup endpoints later:
  - delete facts with NULL provenance and low confidence
  - decay confidence over time

## Testing strategy

Unit tests (Rust):
- Facts CRUD + upsert dedupe behavior.
- FK behavior (if practical with a temp DB): delete message and verify facts’ `source_message_id` is NULL.
- Traversal safety: verify traversal only follows ENTITY objects and respects hop/limit caps.

Integration tests (manual for v0):
- Create a conversation, send messages that contain clear preferences, verify facts appear and are used in context.
- Validate that strategist prompts include USER PROFILE and KNOWN FACTS blocks.

## Rollout plan (phases)

### Phase 2a: Storage foundations
- Enable SQLite FK enforcement in `SqliteManager::new`.
- Add facts schema + migration (`migrate_facts_v1`).
- Add fact types (`Fact`, `ObjectKind`) and SqliteManager CRUD/upsert.

### Phase 2b: Librarian API
- Add `Librarian` convenience methods that delegate to SqliteManager.
- Add ID-returning message save method in `Librarian` to support provenance.

### Phase 2c: Strategist integration
- Add `conversation_id` plumbing to `assemble_context` and call site.
- Add relevant fact retrieval + safe traversal.
- Update planner/synthesizer prompts to include facts.

### Phase 2d: Background fact extraction
- Add extraction prompt + parsing.
- Trigger extraction after saving user/assistant messages in `send_message`.
- Upsert extracted facts with `source_message_id`.

### Phase 2e (optional): UI surfacing
- Add Tauri commands to query profile facts / facts-about-entity.
- Add a basic “Facts” tab in the memory panel.

## Success metrics
- Fact extraction accuracy (manual sample review): target ≥ 80% “reasonable facts”.
- Fact retrieval latency: keep facts query + formatting under 50ms typical.
- Prompt quality: fewer redundant re-explanations, more consistent personalization.

## Open questions
- Do we want a separate “extraction_model” setting distinct from strategist’s context_model?
- How aggressive should normalization be (lowercase vs preserve display)?
- Should we store inbound edges (object as entity) explicitly or derive them at query time?
