# Memory System Phase 3: Redesign — KG + botmem-style docs

Status: design draft, not yet implemented. Branch: `claude/redesign-memory-system-haxAt`.
Supersedes: `docs/memory-fix.md`, `docs/memory2-revised.md` (their KG layer survives; the
strategist loop and per-message extraction do not).

## 1. Problem statement

The current memory pipeline is **verbose** (it dumps low-signal triples and many
snippets into every system prompt) and **greedy** (it runs LLM extraction after
every single user and assistant turn, and runs a multi-query planner→synthesizer
LLM loop on every retrieval). The diagnosis:

1. **Per-turn LLM extraction is too aggressive.** `FactExtractor::extract` is invoked
   after every user message (`src-tauri/src/lib.rs:602`) *and* every assistant
   message (`src-tauri/src/lib.rs:1194`). Each call is a low-temperature LLM round
   trip with strict instructions to ignore one-offs, but most turns yield zero
   useful facts. The cost-to-signal ratio is poor.
2. **Injection format is hostile to the consuming LLM.** The strategist renders
   facts as `subject predicate object (conf=0.85, sourced)` triples
   (`src-tauri/src/memory/strategist.rs:523-662`) alongside up to 20 BM25 snippets
   capped at 400 chars each. Prose paragraphs would carry the same information at
   a fraction of the tokens, in a form models read more reliably.
3. **Retrieval is unbounded enough to matter.** The strategist planner can issue
   up to 3 follow-up search queries × 20 results each, then call a synthesizer
   LLM to format the union. Two LLM calls per user turn just to assemble context.
4. **Static, user-owned knowledge has no home.** Stable things — user profile,
   project notes, environment details, working agreements — currently live as
   scattered triples in the `facts` table with no way for the user to read or
   edit them as a coherent document.

The redesign splits memory by *shape*:
- **Knowledge graph (facts)**: atomic, queryable, machine-curated.
- **Documents (botmem-style)**: narrative, user-readable markdown on disk, linked.

## 2. Goals / non-goals

**Goals**
- Markdown-on-disk document store the user can read and hand-edit.
- Tool surface the LLM can call to remember / recall / fetch / edit / forget /
  traverse links, modeled on botmem.
- Chunk-level retrieval with embeddings + BM25 fusion rerank, returning top-3
  chunks with doc IDs (matches botmem's contract).
- Optional auto-injection of one top doc + a small fact budget into system
  context per turn, **toggleable independently** of the tool surface.
- Replace per-message fact extraction with explicit (`/remember`) + lazy
  (session-end) triggers.
- Keep the existing KG (`facts` table + provenance + extraction *prompt*); only
  the *trigger policy* changes.

**Non-goals (v1)**
- Cross-encoder reranker. BM25+cosine score fusion is enough to start.
- Multi-project / workspace concept. Single global workspace; `use_project`
  and `get_active_project` are deferred.
- Automated `suggest_links` via LLM. Defer until the rest is stable.
- Embedding-based search over chat messages. Chunks come from docs only;
  messages stay in Tantivy BM25 for now.
- UI for editing docs in-app. External editor + file watcher covers v1.

## 3. Current state (grounded)

Active code being modified or removed:

- **Per-turn extraction call sites** — `src-tauri/src/lib.rs:602` (user),
  `src-tauri/src/lib.rs:1194` (assistant). Both invoke `FactExtractor::extract`
  with `context_model` (`lib.rs:634, 1179`).
- **Strategist orchestrator** — `src-tauri/src/memory/strategist.rs` (789 lines).
  `assemble_context` runs initial Tantivy + facts retrieval, optional planner
  LLM (up to 3 follow-up queries × 20 results), and a synthesizer LLM that
  renders the verbose triple-and-snippet output (`strategist.rs:523-662`).
- **Fact extractor** — `src-tauri/src/memory/extraction.rs` (322 lines). The
  prompt and JSON parser stay; only the trigger changes.
- **Storage** — `src-tauri/src/memory/sqlite_manager.rs` (1887 lines) holds
  `messages`, `conversation_summaries`, `facts`. `src-tauri/src/memory/tantivy_index.rs`
  (459 lines) indexes message bodies for BM25.
- **Settings** — `src-tauri/src/mcp/config.rs:100-175` defines `Settings`.
  Relevant fields today: `context_model`, `context_turns`, `memory_enabled`,
  `context_uncompressed_msg_count`.

The CLAUDE.md still references `src-tauri/src/llm/context_assembler.rs`; that file
no longer exists. The active orchestrator is `memory/strategist.rs`. CLAUDE.md
will be updated as part of this work.

## 4. Target architecture

```
                ┌──────────────────────────────┐
   user turn ──▶│  send_message (lib.rs)       │
                └──────────┬───────────────────┘
                           │
                  ┌────────┴────────┐
                  │ MemoryContext   │  (gated by settings.memory.auto_inject_docs)
                  │   Builder       │
                  └────────┬────────┘
                           │
        ┌──────────────────┼──────────────────────┐
        ▼                  ▼                      ▼
   ┌─────────┐      ┌─────────────┐        ┌──────────────┐
   │ docs    │      │  facts (KG) │        │ recent turns │
   │ recall  │      │  lookup     │        │              │
   │ top-1   │      │  top-N      │        │              │
   └────┬────┘      └──────┬──────┘        └──────┬───────┘
        │                  │                      │
        └──────────────────┴──────────────────────┘
                           │ prose-rendered, capped
                           ▼
                ┌──────────────────────┐
                │ system context block │
                └──────────┬───────────┘
                           ▼
                    LLM with tools:
                  memory_recall,
                  memory_fetch,
                  memory_remember,
                  memory_edit,
                  memory_forget,
                  memory_link_context
                    + MCP tools
```

Two access modes live side by side:
- **Auto-inject** runs unconditionally on each turn when enabled; pulls a small,
  fixed budget into the system prompt.
- **Tool-call** is always available; the LLM can call any of the six tools at
  any point to fetch more context or write new docs.

## 5. Storage layout

### Disk

```
~/.config/nebula/memory/
├── docs/
│   ├── user-profile.md
│   ├── project-nebula.md
│   ├── workflow-notes.md
│   └── ...
└── memory.sqlite     (existing, schema extended)
```

Path resolved via Tauri's app-config dir; same root that already holds
`settings.json`.

### Markdown file format

```markdown
---
id: project-nebula
title: Nebula project notes
tags: [project, rust, tauri]
links: [user-profile, workflow-notes]
created_at: 2026-05-21T10:00:00Z
updated_at: 2026-05-21T10:00:00Z
---

Nebula is an intelligent orchestrator built with Tauri v2...

See also [[workflow-notes]] for the dev loop.
```

Rules:
- `id` is a human-readable slug, lowercased, `[a-z0-9-]+`, max 64 chars.
  Filename is always `<id>.md`. Renaming a doc means changing both the file
  name and the `id` field; the watcher handles re-indexing and rewriting
  inbound `[[wikilinks]]` is **out of scope** for v1 (broken links are
  acceptable and surfaced via `link_context`).
- `links:` in frontmatter is the canonical link set, but `[[id]]` wikilinks in
  the body are also parsed and merged when the indexer runs. Frontmatter wins
  on conflict.
- `tags` are free-form strings; used as `recall` filters in a future iteration
  (out of scope for v1 retrieval).
- All timestamps are RFC3339 UTC.

### SQLite schema additions

New tables (live alongside `messages`, `conversation_summaries`, `facts`):

```sql
CREATE TABLE docs (
  id            TEXT PRIMARY KEY,          -- slug, mirrors filename
  title         TEXT NOT NULL,
  tags_json     TEXT NOT NULL DEFAULT '[]',
  path          TEXT NOT NULL,             -- absolute path on disk
  mtime_ns      INTEGER NOT NULL,          -- last seen mtime, for watcher reconciliation
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);

CREATE TABLE doc_chunks (
  chunk_id      INTEGER PRIMARY KEY AUTOINCREMENT,
  doc_id        TEXT NOT NULL REFERENCES docs(id) ON DELETE CASCADE,
  ord           INTEGER NOT NULL,          -- chunk index within the doc
  text          TEXT NOT NULL,
  char_start    INTEGER NOT NULL,
  char_end      INTEGER NOT NULL
);
CREATE INDEX idx_doc_chunks_doc ON doc_chunks(doc_id);

CREATE TABLE doc_links (
  src_doc_id    TEXT NOT NULL REFERENCES docs(id) ON DELETE CASCADE,
  dst_doc_id    TEXT NOT NULL,             -- not FK: dangling links allowed
  PRIMARY KEY (src_doc_id, dst_doc_id)
);
CREATE INDEX idx_doc_links_dst ON doc_links(dst_doc_id);

-- Embedding storage as raw packed f32 bytes. dim is recorded in memory_meta;
-- on embedding-model change, this table is truncated and re-populated.
-- v1 deliberately avoids sqlite-vec to dodge cross-platform extension builds;
-- queries happen against an in-memory cache (see §8).
CREATE TABLE doc_chunk_vecs (
  chunk_id  INTEGER PRIMARY KEY REFERENCES doc_chunks(chunk_id) ON DELETE CASCADE,
  embedding BLOB NOT NULL          -- packed little-endian f32, length = dim * 4
);
```

The embedding `dim` is captured in a `memory_meta` key/value row:

```sql
CREATE TABLE memory_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- Seeded with: embedding_model, embedding_dim, schema_version.
```

On embedding-model change, startup detects mismatch and triggers a full reindex
(truncate `doc_chunk_vecs`, re-embed all chunks, repopulate the in-memory cache).

### Tantivy

Add a `doc_type` field to the existing schema (or a separate index — TBD during
implementation, see §15). Each doc chunk is indexed as a Tantivy document with
fields `{doc_type: "doc_chunk", doc_id, chunk_id, text}`. Message indexing is
unchanged.

## 6. Tool surface

Six tools, namespaced `memory_*`, registered into the same tool registry MCP
tools use so the LLM sees them inline. JSON schemas below are the contract;
internal Rust types live in `memory/docs/api.rs`.

### `memory_remember`

Create a new doc. Errors if the slug exists (use `memory_edit` to update).

```json
{
  "name": "memory_remember",
  "input_schema": {
    "type": "object",
    "required": ["id", "title", "content"],
    "properties": {
      "id": {"type": "string", "description": "Slug, [a-z0-9-]+, max 64 chars"},
      "title": {"type": "string"},
      "content": {"type": "string", "description": "Markdown body"},
      "tags": {"type": "array", "items": {"type": "string"}},
      "links": {"type": "array", "items": {"type": "string"}, "description": "Doc IDs to link to"}
    }
  }
}
```

Returns `{ "id": "...", "path": "..." }`.

### `memory_fetch`

Retrieve a doc by ID. Returns full body, frontmatter, and outbound links.

```json
{
  "name": "memory_fetch",
  "input_schema": {
    "type": "object",
    "required": ["id"],
    "properties": {"id": {"type": "string"}}
  }
}
```

Returns `{ "id", "title", "tags", "links", "content", "created_at", "updated_at" }`.

### `memory_edit`

Mutate an existing doc. Two modes; exactly one of `replace` / `append` must be
provided.

```json
{
  "name": "memory_edit",
  "input_schema": {
    "type": "object",
    "required": ["id"],
    "properties": {
      "id": {"type": "string"},
      "expected_updated_at": {
        "type": "string",
        "description": "Optimistic concurrency token. The updated_at value returned by the most recent memory_fetch. If the document has changed since then, the edit fails with a CONFLICT error and the LLM must re-fetch."
      },
      "replace": {
        "type": "object",
        "required": ["find", "with"],
        "properties": {
          "find":  {"type": "string"},
          "with":  {"type": "string"}
        }
      },
      "append": {"type": "string"}
    }
  }
}
```

`replace.find` must match exactly once in the body (botmem's convention; avoids
ambiguous edits). Returns the new body length and `updated_at`.

`expected_updated_at` is optional but recommended. When supplied, the store
compares it to the current `updated_at` under the per-doc lock and rejects the
edit if it has changed (user edited the file, watcher reindexed, another tool
call landed first). The error returns the current `updated_at` so the LLM can
re-fetch and retry. Omitting it is "force write" — accepted but discouraged.

### `memory_forget`

Delete a doc. Removes the file, cascades chunks/links/vectors via FK.

```json
{
  "name": "memory_forget",
  "input_schema": {
    "type": "object",
    "required": ["id"],
    "properties": {"id": {"type": "string"}}
  }
}
```

### `memory_recall`

Semantic + lexical search. Returns top-3 **chunks** by default, each with its
parent `doc_id`, score components, and a preview.

```json
{
  "name": "memory_recall",
  "input_schema": {
    "type": "object",
    "required": ["query"],
    "properties": {
      "query": {"type": "string"},
      "k":     {"type": "integer", "minimum": 1, "maximum": 10, "default": 3},
      "tags":  {"type": "array", "items": {"type": "string"}}
    }
  }
}
```

Returns:
```json
{
  "hits": [
    {
      "doc_id": "project-nebula",
      "chunk_id": 42,
      "ord": 3,
      "text": "...chunk body...",
      "score": 0.81,
      "score_components": {"cosine": 0.74, "bm25": 12.3}
    }
  ]
}
```

### `memory_link_context`

BFS from a starting doc over `doc_links`, bounded by depth and total node count.

```json
{
  "name": "memory_link_context",
  "input_schema": {
    "type": "object",
    "required": ["id"],
    "properties": {
      "id":       {"type": "string"},
      "depth":    {"type": "integer", "minimum": 1, "maximum": 3, "default": 2},
      "max_docs": {"type": "integer", "minimum": 1, "maximum": 20, "default": 10}
    }
  }
}
```

Returns the start doc and a flat list of reachable docs (id, title, depth) plus
an edge list. Full bodies are **not** included; the LLM follows up with
`memory_fetch` for any it wants in full.

### Tools intentionally omitted

- `memory_list` — covered by `memory_recall` with an empty/wide query and via
  filesystem `ls` for the user. Can add later if the LLM asks for it often.
- `memory_reindex` — handled automatically by the file watcher and by
  embedding-model change detection at startup. An admin/debug command may exist
  internally but is not exposed to the LLM.
- `memory_use_project` / `memory_get_active_project` — deferred until
  multi-project is a real need.
- `memory_suggest_links` — deferred. Add as a separate tool when wanted.

## 7. Embedding provider abstraction

```rust
// memory/docs/embedding/mod.rs
#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn model_id(&self) -> &str;
    fn dim(&self) -> usize;
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}
```

Two impls:
- `FastembedProvider` (local, default). Wraps `fastembed-rs`. Default model:
  `BAAI/bge-small-en-v1.5` (384-dim, ~130MB ONNX, downloaded to a cache dir on
  first use). MiniLM-L6-v2 (384-dim, ~90MB) is the lighter alternative.
- `RemoteEmbeddingProvider`. Calls the configured provider's embeddings endpoint
  (OpenAI: `text-embedding-3-small` → 1536, `-3-large` → 3072; Ollama:
  `/api/embeddings` with a user-selected model). Uses the same `providers`
  HashMap from `Settings`.

Selection lives in settings (§11). Switching providers triggers a full reindex
because vector dimension is fixed per model (and is recorded in
`memory_meta.embedding_dim`); the startup check compares the configured model
to `memory_meta.embedding_model` and rebuilds when they differ.

Batch size and timeouts are provider-specific; both impls accept up to ~64
chunks per call.

## 8. Retrieval pipeline

`memory_recall(query, k)`:

1. **Embed the query** via the configured `EmbeddingProvider`.
2. **Vector recall**: brute-force cosine against the in-memory `VectorCache`
   (a `Vec<(chunk_id, [f32; DIM])>` loaded from `doc_chunk_vecs` at startup,
   parallelized with `rayon`). Take top `k_vec = min(k * 5, 30)`. At expected
   scales (≤ 50k chunks × 384 dims ≈ 75 MB RAM, sub-10ms scans) this beats
   the cross-platform pain of bundling a SQLite extension.
3. **BM25 recall** over the docs Tantivy index (separate from the message
   index; see §15.1). Same `k_vec` cap.
4. **Fusion rerank**: union the two candidate sets, normalize each score to
   [0, 1] within its own list, blend with `score = 0.6 * cosine + 0.4 * bm25`.
   Drop chunks below a configurable floor (default 0.15). Tie-break by
   `updated_at DESC`.
5. **Diversification**: prefer at most 2 chunks from the same doc in the
   returned top-K (cheap MMR-lite).
6. Return top-K (default 3) with `doc_id`, `chunk_id`, `ord`, `text`, blended
   `score`, and raw `score_components`.

The `VectorCache` is the single source of truth for vector search at runtime;
SQLite is the durability layer. All chunk writes update both, under a write
lock on the cache. Cache rebuild cost on app start is bounded by disk I/O
(reading `BLOB`s) and is fast enough to do eagerly before the watcher arms.

Chunking strategy (write-side):
- Split on paragraph boundaries; pack into ~500-char windows with ~80-char
  overlap. Code fences and frontmatter are not split.
- Each chunk row records `char_start`/`char_end` for future highlighting.

The pipeline is **deterministic and LLM-free**. No planner, no synthesizer.

## 9. Auto-injection layer

Gated by `settings.memory.auto_inject_docs` (default `true`). When enabled, on
every user turn:

1. Run `memory_recall(latest_user_message, k = 1)`. If a hit clears a floor
   score (default 0.20), `memory_fetch` its doc and include the **full body**
   (or a clipped 4k-char prefix if huge) under a `## Relevant document:
   <title>` heading.
2. Look up up to 3 KG facts via the same logic that's there today
   (user-profile facts + facts about entities mentioned in the query). Render
   them as **prose**, one short paragraph: `"About the user: prefers X,
   working on Y, uses Z."` — not as `subject predicate object`.
3. Include the last N conversation turns (existing `context_turns` setting).

A single, hard token budget (default 4000 tokens for the whole memory block)
caps the result; if exceeded, drop the doc first, then trim facts, then trim
recent turns. Budget enforced via the existing `tokenizer.rs`.

When disabled, the system prompt contains no memory block at all; the LLM must
use `memory_recall` / `memory_fetch` explicitly. Useful for users who find
auto-inject too leaky or want maximum determinism.

## 10. Link graph + BFS

Links are populated at index time from two sources, deduplicated:
- Frontmatter `links: [...]` array.
- `[[id]]` wikilink syntax in the body (simple regex `\[\[([a-z0-9-]+)\]\]`).

Stored in `doc_links(src_doc_id, dst_doc_id)`. `memory_link_context` runs an
iterative BFS in SQL using a recursive CTE, capped by depth and node count:

```sql
WITH RECURSIVE
  reachable(id, depth) AS (
    SELECT :start, 0
    UNION
    SELECT dl.dst_doc_id, r.depth + 1
      FROM doc_links dl JOIN reachable r ON dl.src_doc_id = r.id
     WHERE r.depth < :max_depth
  )
SELECT DISTINCT id, MIN(depth) AS depth FROM reachable
 GROUP BY id ORDER BY depth, id LIMIT :max_docs;
```

Dangling links (dst not in `docs`) are returned with a `missing: true` flag so
the LLM can decide to `memory_remember` to fill them in.

## 11. Settings additions

Add a nested `memory` section to `Settings` (`src-tauri/src/mcp/config.rs`). All
fields default-on so existing users get sane behavior post-migration.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySettings {
    #[serde(default = "default_true_bool")]
    pub docs_enabled: bool,             // master switch for docs subsystem

    #[serde(default = "default_true_bool")]
    pub auto_inject_docs: bool,         // gate for §9; tools stay live either way

    #[serde(default = "default_embedding_provider")]
    pub embedding_provider: String,     // "fastembed" | "remote"

    #[serde(default = "default_fastembed_model")]
    pub fastembed_model: String,        // e.g. "bge-small-en-v1.5"

    #[serde(default)]
    pub remote_embedding_provider_id: Option<String>,  // key into Settings.providers

    #[serde(default = "default_remote_embedding_model")]
    pub remote_embedding_model: String, // e.g. "text-embedding-3-small"

    #[serde(default = "default_auto_inject_budget")]
    pub auto_inject_token_budget: usize, // default 4000

    #[serde(default = "default_recall_floor")]
    pub recall_score_floor: f32,        // default 0.15

    #[serde(default = "default_extraction_policy")]
    pub fact_extraction_policy: String, // "explicit" | "session_end" | "off"
}
```

Existing top-level fields kept for back-compat: `memory_enabled` (now
specifically gates Tantivy message retrieval; docs are gated separately by
`memory.docs_enabled`), `context_model`, `context_turns`,
`context_uncompressed_msg_count`.

A settings migration in `Settings::load_migrated` populates the new section
with defaults on first read.

## 12. Startup reconciliation + file watcher

The watcher only catches changes that happen while the app is running.
Anything that happens while Nebula is closed — `git pull`, manual `vim` edits,
external scripts, full directory restores — must be reconciled at startup
before the watcher arms. Otherwise the SQLite index drifts from disk.

### Startup reconciliation (runs once, before the watcher starts)

1. Scan `~/.config/nebula/memory/docs/` recursively for `*.md`. Build a map
   `disk_files: {path → mtime_ns}`.
2. Load all rows from `docs` into `db_docs: {id → (path, mtime_ns)}`.
3. **Deleted on disk**: rows in `db_docs` whose `path` is not in `disk_files`
   → cascade-delete from `docs` (FKs remove `doc_chunks`, `doc_links`,
   `doc_chunk_vecs`).
4. **New on disk**: paths in `disk_files` with no row in `db_docs` → parse
   frontmatter, chunk, embed, insert.
5. **Modified on disk**: paths whose `disk_files[path].mtime_ns >
   db_docs[id].mtime_ns` → re-parse, re-chunk, re-embed the chunks whose
   `text` hash changed, update `doc_links`, refresh the docs Tantivy entries.
6. Rebuild the in-memory `VectorCache` from `doc_chunk_vecs` after step 5.
7. Emit a single `memory:reconciled` Tauri event with counts for the UI.
8. Arm the `notify` watcher.

Reconciliation is bounded by disk I/O plus re-embedding cost for changed
files. For users who don't edit docs externally, it's a cheap mtime scan.
For a `git pull` that replaced many docs, it does the work that the watcher
would have done if it had been running.

### Watcher (runtime)

`memory/docs/watcher.rs` uses the `notify` crate to watch `docs/`. On any FS
event for `*.md`:

1. Debounce 250 ms.
2. Compare current `mtime_ns` against `docs.mtime_ns`. Skip if unchanged.
3. Re-parse frontmatter, re-chunk, re-embed (only the chunks whose text hash
   changed), upsert vectors in both the DB and the `VectorCache`, rebuild
   that doc's `doc_links` rows, refresh its Tantivy entries.
4. Emit a `memory:doc_changed` Tauri event so `MemoryPanel.tsx` can refresh.

Tool writes (`memory_remember`, `memory_edit`, `memory_forget`) call the same
ingestion path directly under the per-doc lock and bump `mtime_ns`, so the
watcher's debounced re-read for the same file is a no-op.

## 13. Module layout

```
src-tauri/src/memory/
├── mod.rs               (existing; export new submodule)
├── audit_logger.rs      (existing, unchanged)
├── extraction.rs        (existing prompt + parser; trigger policy reworked)
├── librarian.rs         (existing; gains docs-facing accessors)
├── sqlite_manager.rs    (existing; new tables + migrations)
├── tantivy_index.rs     (existing; doc_type field added)
├── strategist.rs        (gutted; see §14)
└── docs/                (NEW)
    ├── mod.rs           (DocStore facade + public API)
    ├── api.rs           (request/response types matching tool schemas)
    ├── store.rs         (markdown read/write, atomic FS ops, frontmatter)
    ├── chunker.rs       (chunking strategy)
    ├── links.rs         (link parsing + doc_links upsert + BFS)
    ├── watcher.rs       (notify-based file watcher)
    ├── retrieval.rs     (recall pipeline: embed → vec + bm25 → fuse → rerank)
    ├── inject.rs        (auto-injection: doc + facts → prose block)
    └── embedding/
        ├── mod.rs       (EmbeddingProvider trait)
        ├── fastembed.rs (local ONNX impl)
        └── remote.rs    (provider-API impl)
```

Tools are registered in `lib.rs` alongside MCP tools so the LLM sees them in
the same `tools` array sent to the model. The tool handlers call into
`DocStore` directly (in-process; no MCP round trip).

## 14. Migration / kill list

**Removed**
- Per-turn `FactExtractor::extract` calls at `src-tauri/src/lib.rs:602` and
  `:1194`. The function and prompt remain; only the trigger moves.
- The strategist planner LLM loop and synthesizer in
  `src-tauri/src/memory/strategist.rs:523-662`. `assemble_context` is replaced
  by `DocStore::auto_inject` (§9) wired into `send_message`.
- Verbose `subject predicate object (conf=X.XX, sourced|inferred)` rendering
  for facts in injected context.

**Kept**
- `facts` table and schema (already KG-shaped).
- `messages`, `conversation_summaries`, Tantivy message indexing.
- `extraction.rs` prompt and JSON parser.
- `Compactor` (`src-tauri/src/llm/compactor.rs`) — orthogonal to this work.
- `MemoryPanel.tsx` (frontend); gets new tabs for Docs and updated Facts view.

**New trigger policy for fact extraction**
- `fact_extraction_policy = "explicit"` (default): facts are only extracted on
  user request. UX: a `/remember` command in the chat input, or a "save as
  fact" button on a message.
- `fact_extraction_policy = "session_end"`: when a conversation is closed or
  goes idle for >15 minutes, run extraction once over its summary.
- `fact_extraction_policy = "off"`: extraction disabled entirely.

This collapses extraction LLM calls from O(messages) to O(conversations) in the
worst case.

**Schema migrations**
- `migrate_v4_docs.rs` creates `docs`, `doc_chunks`, `doc_links`,
  `doc_chunk_vecs` (regular table, BLOB embeddings), and `memory_meta`.
- No data migration required; the docs subsystem starts empty.

## 15. Open questions / risks

1. **Tantivy schema change — resolved.** Tantivy schemas are immutable, so
   adding a `doc_type` field to the message index would force a destructive
   rebuild of every user's chat history index. **Decision: second Tantivy
   index** at `~/.config/nebula/memory/docs_index/`, dedicated to doc
   chunks. The message index is untouched; the docs index can be blown away
   and rebuilt at any time (e.g. on chunking-strategy changes) without
   touching chat history.

2. **`sqlite-vec` cross-platform builds — resolved.** Bundling a loadable C
   extension for macOS (Intel + ARM), Windows, and Linux through Tauri's
   build pipeline is a real cross-compilation headache, and at the scale of
   a single user's memory the payoff is small. **Decision: skip `sqlite-vec`
   for v1.** Store embeddings as packed `f32` BLOBs in the regular
   `doc_chunk_vecs` table; load them all into an in-memory `VectorCache` at
   startup; do brute-force cosine parallelized with `rayon` (§8). At ≤ 50k
   chunks × 384 dims (≈ 75 MB) this is sub-10ms on commodity hardware. We
   revisit only if real users hit scale that hurts.

3. **fastembed model download UX.** First-run download is ~90–130MB and
   blocking. We should:
   - Surface a progress indicator in the settings page.
   - Cache to the same config dir.
   - Document that disabling `docs_enabled` skips the download.

4. **Re-embedding on model switch.** Switching embedding provider/model means
   re-reading every doc, re-chunking, and re-embedding. This is acceptable
   (rare action, runs in the background) but must show progress. We may
   throttle to avoid hammering remote APIs.

5. **`memory_edit` `replace.find` uniqueness.** If the string appears multiple
   times the tool errors. That's botmem's behavior and the right default. We
   may add an `occurrence: int` field later if the LLM struggles with it.

6. **`[[wikilink]]` syntax overlap with Obsidian etc.** Intentional — users
   familiar with Obsidian get the same link semantics. We do **not** support
   Obsidian's `[[id|display text]]` form in v1.

7. **Provenance for docs.** Unlike facts, docs don't track a `source_message_id`.
   They're user-curated artifacts, not extracted assertions. If we want a
   "where did this come from" trail later, we can add a `created_by:
   message_id` frontmatter field, but it's not in v1.

8. **Concurrency on `memory_edit` — two layers.**
   - *Physical*: per-doc `Mutex` keyed by doc ID inside `DocStore` prevents
     interleaved disk writes / partial files.
   - *Logical*: optimistic concurrency via `expected_updated_at` on
     `memory_edit` (§6). A mutex alone doesn't stop a stale-read overwrite —
     the LLM could `fetch`, the user could `vim`-edit (or another tool call
     could land), then the LLM's `edit` would silently clobber both. With
     `expected_updated_at` present, the store compares under the lock and
     returns a `CONFLICT` error with the current `updated_at`; the LLM
     re-fetches and retries. Omitting the field is "force write".

9. **Security: malicious docs.** A doc body is injected into system context.
   In a single-user desktop app this is low-risk, but worth flagging:
   `memory_remember` from the LLM is a write primitive that affects future
   prompts. The user can audit docs on disk at any time. The audit log
   (`memory/audit_logger.rs`) should be extended to log all doc writes.

10. **Slug vs. title split — intentional.** `id` is strict kebab-case
    (`[a-z0-9-]+`) for filesystem safety and unambiguous `[[wikilinks]]`;
    `title` lives in YAML frontmatter with no character restrictions. Users
    get readable titles in the UI and search results, while filenames and
    cross-doc references stay machine-stable. Renaming a slug remains a
    user-driven, manual action in v1 (no auto-rewrite of inbound links).

## 16. Phased rollout

**Phase 0 — design freeze (this doc).**

**Phase 1 — storage + tools, no auto-inject.**
- New schema (incl. `doc_chunk_vecs` BLOB table, `memory_meta`), `DocStore`,
  `EmbeddingProvider` trait, `FastembedProvider`.
- In-memory `VectorCache` with `rayon`-parallel cosine.
- Second Tantivy index at `~/.config/nebula/memory/docs_index/`.
- Startup reconciliation pass (§12) runs before the watcher arms.
- `notify`-based file watcher functional.
- Six tools registered, available to the LLM, including
  `expected_updated_at` optimistic concurrency on `memory_edit`.
- Per-doc `Mutex` map inside `DocStore`.
- No auto-injection yet. Old strategist still in place; nothing existing breaks.
- `MemoryPanel.tsx` gets a Docs tab (read-only list + fetch).

**Phase 2 — auto-inject + strategist removal.**
- Implement `DocStore::auto_inject`; wire into `send_message`.
- Delete strategist planner/synthesizer path.
- Switch fact rendering to prose.
- Settings UI for `memory.auto_inject_docs`, budget, score floor.

**Phase 3 — extraction policy change.**
- Remove per-turn `FactExtractor::extract` calls.
- Implement `/remember` UI + `session_end` extraction worker.
- Settings UI for `fact_extraction_policy`.

**Phase 4 — remote embeddings + polish.**
- `RemoteEmbeddingProvider`.
- Re-embed migration job with progress.
- Audit log entries for all doc writes.

**Phase 5 — deferred features (if needed).**
- `memory_list`, `memory_suggest_links`, multi-project, cross-encoder rerank,
  Obsidian display-text wikilinks.

Each phase is independently shippable and reversible.
