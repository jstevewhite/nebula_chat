# TODO

Items deferred from completed work, with enough context to pick them back up
without re-reading the whole branch history.

## memory3 Phase 5 — deferred features

Phases 0–4 of the memory redesign shipped on `claude/redesign-memory-system-haxAt`
(see `docs/memory3-redesign.md` §16 for the full plan). These were intentionally
deferred at the time as "if needed" and remain unimplemented. Each entry below
has goal, files to touch, sketch of approach, and the gotchas worth knowing.

---

### 1. `memory_suggest_links` LLM tool

**Goal:** Given a doc id, propose new `[[wikilinks]]` to other existing docs
based on semantic similarity. The model picks which to accept; the tool just
surfaces candidates with a short rationale. Useful once a workspace has enough
docs that the link graph starts paying off for
`memory_doc_link_context`.

**Files:**
- `src-tauri/src/memory/docs/tools.rs` — add `TOOL_DOC_SUGGEST_LINKS` constant,
  `doc_suggest_links_tool()` descriptor, append to `ALL_NAMES` and `build_all()`.
- `src-tauri/src/memory/docs/mod.rs` — add `DocStore::suggest_links(id, k)`
  method.
- `src-tauri/src/lib.rs::dispatch_memory_tool` — add the new branch.
- `src/components/ChatInterface.tsx` — add the new name to `MEMORY_TOOL_NAMES`
  so the auto-approve toggle covers it.

**Approach:**
1. Fetch the source doc.
2. Embed the doc body (or its top-N chunks) and run cosine against the
   `VectorCache` for *other* doc ids, take top K with score > floor.
3. Optionally pass the candidates through an LLM "rationale" pass using
   `crate::llm::factory::create_provider(settings.context_model)` to write a
   one-line "why these connect" string per candidate. Skip if no
   `context_model` is configured.
4. Return `{ suggestions: [{ doc_id, title, score, rationale? }] }`.

**Gotchas:**
- Filter out docs already in the source's outbound `doc_links`.
- Limit to small K (≤ 5) — the LLM rationale pass costs a round trip per call.
- Auto-approve gate behaves identically; no new setting needed.

---

### 2. Multi-project / workspaces

**Goal:** Partition memory (docs + facts + Tantivy) by named project so
personal/work/project-X contexts don't bleed into each other. Maps to botmem's
`use_project` / `get_active_project`.

**Files:**
- `src-tauri/src/mcp/config.rs` — add `memory_active_project: String` (default
  `"default"`).
- `src-tauri/src/memory/sqlite_manager.rs` — add `project_id` columns to `docs`,
  `doc_chunks`, `doc_links`, `doc_chunk_vecs`, `facts`. Migration `v7` ALTERs
  with default `"default"`. Update every query to filter on project.
- `src-tauri/src/memory/docs/mod.rs` — DocStore takes an `active_project: String`;
  `docs_dir` becomes `memory/projects/<id>/docs/`; Tantivy index path becomes
  `memory/projects/<id>/docs_index/`. Watcher + reconciliation operate per
  project.
- `src-tauri/src/memory/docs/tools.rs` — add `memory_use_project`,
  `memory_get_active_project`.
- `src/components/SettingsPage.tsx` — UI to create/switch/delete projects.
- `src/App.tsx` — project switcher in the activity bar, similar to the chat
  conversation list.

**Approach:**
1. **Schema migration is the load-bearing piece.** Add `project_id TEXT NOT
   NULL DEFAULT 'default'` to all five tables. Existing rows land in the
   `"default"` project automatically.
2. Per-project on-disk layout: `~/.config/nebula/memory/projects/<id>/docs/*.md`
   plus `docs_index/`. The startup reconciliation pass runs per active project,
   not globally.
3. `memory_use_project(id)` — switch active project. Rebuild VectorCache,
   re-arm watcher, re-run reconciliation.
4. Auto-inject + recall stay scoped to the active project.
5. KG facts get `project_id` too; the auto-inject "About the user" block is
   per-project, which is the point.

**Gotchas:**
- Conversations are *not* scoped to a project in this design — a single chat
  can still pull from whatever project is active when it runs. If you want
  conversation ↔ project pinning, that's an extra column on `conversations`.
- The watcher needs to be torn down + recreated on project switch.
- Facts about "user" (identity, OS, etc.) probably belong in a shared
  "personal" project; consider a `personal_project_id` setting that auto-pulls
  user-profile facts alongside the active project's facts during auto-inject.

---

### 3. Cross-encoder reranker

**Goal:** Replace the current BM25 + cosine fusion in `retrieval::fuse` with a
small cross-encoder that scores `(query, chunk)` pairs jointly. Higher recall
quality when the workspace has many similar docs; the trade is ~100 MB of
bundled model and another ONNX runtime call per recall.

**Files:**
- `src-tauri/Cargo.toml` — fastembed already pulls ONNX; the cross-encoder
  model just needs to be loaded alongside `BGESmallENV15`. Check
  fastembed-rs's `Reranker` API (added in 4.x).
- `src-tauri/src/memory/docs/embedding/` — add `reranker.rs` or extend
  `fastembed_provider.rs` to expose `rerank(query, candidates) -> Vec<f32>`.
- `src-tauri/src/memory/docs/retrieval.rs::fuse` — gated behind a setting; if
  the reranker is available, after fusion take the top 2*k and rerank, then
  truncate to k.
- `src-tauri/src/mcp/config.rs` — `memory_use_cross_encoder: bool` (default
  false), `memory_cross_encoder_model: String`.

**Approach:**
1. fastembed-rs `TextRerank` initialised similarly to `TextEmbedding`. Default
   model: `Xenova/ms-marco-MiniLM-L-12-v2` (or whichever ships in fastembed).
2. In `recall()`, after the BM25+cosine fusion, if cross-encoder is enabled,
   take top `2 * k` (capped at MAX_K_CANDIDATES), call `rerank()`, blend or
   replace the score, truncate to `k`.
3. Off by default. UI toggle in SettingsPage.

**Gotchas:**
- Adds another ONNX model download on first use.
- Latency goes from ~5–10ms (rayon cosine) to ~50–100ms (single rerank call).
  Auto-inject runs on every turn; budget accordingly.
- Skip entirely when `memory_embedding_provider == "remote"` unless a remote
  reranker is wired (it isn't in this design).

---

### 4. Obsidian-style display-text wikilinks

**Goal:** Support `[[doc-id|display text]]` in markdown bodies so docs read
naturally while links still resolve to the slug. Currently only `[[doc-id]]`
parses.

**Files:**
- `src-tauri/src/memory/docs/links.rs` — extend the regex + parser. Add a
  capture group for the optional `|display`.
- `src-tauri/src/memory/docs/links.rs::tests` — round-trip tests for
  `[[id|text]]` and `[[id]]`.

**Approach:**
1. Update the regex: `\[\[([a-z0-9][a-z0-9\-]{0,63})(\|([^\]]+))?\]\]`.
2. `extract_wikilinks` still returns the slug only (link graph cares about ids,
   not display text).
3. No renderer changes needed today; we don't render markdown server-side. If
   the frontend ever renders memory doc bodies, it'll need a small ReactMarkdown
   transformer to swap `[[id|text]]` for a link element. File a separate
   frontend ticket if/when that happens.

**Gotchas:**
- Tiny change; the only reason to defer it was lack of demand. Pick it up as
  a warm-up for whoever takes Phase 5.

---

### 5. `memory_list` LLM tool

**Status:** Probably not worth doing. `memory_doc_recall` with a broad query
covers it for the model, and the UI's Docs tab already lists everything via
the `list_memory_docs` Tauri command. Leaving here only so we don't forget we
considered it.

**Revisit when:** a real-world model trace shows the LLM repeatedly calling
`memory_doc_recall` with empty/dummy queries trying to enumerate.

---

## Other deferred items (not memory3)

### Combine Tools / Memory / Tasks panels

**Goal:** The left activity bar currently has separate panels for tool
approvals, memory context, and tasks. Combine them into a single panel with
tabs or sections so the sidebar isn't cluttered with three near-identical
chrome treatments.

**Files:**
- `src/components/ToolsPanel.tsx`, `src/components/MemoryPanel.tsx`, and the
  tasks panel — merge into one container component (e.g. `SidePanel.tsx`) with
  tabbed sub-views.
- `src/App.tsx` — activity bar wiring collapses three buttons into one.

**Gotchas:**
- Each panel currently owns its own Tauri event subscriptions; the merged
  component needs to manage all three subscriptions cleanly (React 19 double-
  render + cleanup, per CLAUDE.md).
- Don't lose the per-panel state (which tool is pending approval, which memory
  doc is selected) when switching tabs — keep state lifted or per-tab.
