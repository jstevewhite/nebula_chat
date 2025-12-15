# Update plan – phased implementation of `features.md`

This plan is organized to be safe and incremental: build a secure/observable foundation first, fix correctness issues next, then address data integrity, and finally layer on user-facing features. Testing is treated as a per-phase deliverable (not a final “catch-up” activity).

## Guiding principles (safety)

- Prefer **additive schema changes** and **backwards-compatible settings migrations**.
- Use **feature flags** (stored in settings) for behavior-changing features, defaulting to “off” until stable.
- Keep a **rollback path** for each phase (ability to disable the new behavior without data loss).
- Treat **tests as part of each phase’s acceptance** (unit/integration/manual). Do not defer testing to a final phase.
- Use a **real DB migration strategy** for SQLite schema evolution (versioned migrations; no “best effort” ALTER-only approach).
- Establish **performance baselines** for message send, memory indexing, and MCP tool execution so regressions are visible.
- Ensure **timeouts + cancellation** exist for long-running network/tool operations to avoid stuck UI/backend states.

## Phase 0 — Foundations: hygiene, observability, secrets, migrations

### Goals
- Establish confidence in current behavior.
- Make the system **observable** for upcoming refactors.
- Eliminate the highest-severity security issue (plaintext secrets) before expanding surface area.
- Put a durable migration framework in place before adding tables/columns.

### Work
- Add a lightweight “dev smoke test checklist” in documentation (manual steps): create chat, send message, stream response, use a tool, view memory panel, edit/delete conversation.
- Make Phase 0 refactors concrete (reduce ambiguity):
  - consolidate settings load/save operations into a single module (one public API)
  - consolidate provider selection into a helper function used by title generation, pruning, context assembly, and send_message
- Structured logging (move earlier):
  - replace ad-hoc `println!/eprintln!` with `tracing` + log levels
  - add a minimal “export logs”/“copy diagnostics” path (UI can come later)
- Secrets at rest (move earlier):
  - stop storing provider API keys in plaintext settings; store in OS keychain (or encrypt at rest)
  - define an explicit migration behavior for existing plaintext keys (one-time import to keychain then redact)
- SQLite migration strategy:
  - introduce versioned migrations (schema version table + ordered migrations)
  - retrofit existing schema init/migrate paths into the migration framework
- Feature flag management:
  - define where flags live in settings (e.g., `settings.feature_flags`)
  - define how they are toggled (doc-only is acceptable initially; UI toggle later)
- Cancellation/timeouts baseline:
  - define backend-side request timeouts for MCP calls and provider calls
  - define how UI cancels tool executions / generations and how backend surfaces cancellations
- Performance baselines:
  - add simple timing logs/metrics for: saving message, indexing, searching, tool call, provider call

### Acceptance
- No product-level behavior changes besides secrets storage.
- Provider keys are no longer stored plaintext in settings.
- Structured logging exists and is usable for debugging.
- SQLite migrations run deterministically on startup.
- Smoke test checklist exists and is runnable.

Tests
- Unit tests for settings load/migration behavior.
- Smoke test checklist updated to include “upgrade with existing settings” scenario.

Rollback
- Provide a documented switch to continue reading legacy plaintext keys temporarily (dev-only) if keychain integration fails, guarded behind a flag and clearly marked as insecure.

---

## Phase 1 — Systemic correctness + capability model

### 1.1 Remove duplicate Tauri command registration
- Fix the duplicate `delete_system_prompt` entry in `tauri::generate_handler![...]`.

### 1.2 Provider capability matrix (moved earlier)
- Add a capability matrix per provider type:
  - supports tools
  - supports streaming
  - supports streaming tools
  - supports multimodal
- Use this matrix everywhere the UI/backend decides “is this combination allowed?”

### 1.3 Tool-call / tool-result integrity checks
- Add backend validation when a `tool` role message is submitted:
  - Verify `tool_call_id` is present and matches a tool call ID from the preceding assistant message (or most recent assistant tool-call set).
  - If not matchable, return a clear error to the UI.
- Update frontend to stop generating synthetic `tool_call_id` values; only use the actual tool call id.

### 1.4 Streaming parity safety for Anthropic (via capability model)
- Use the capability matrix to prevent unsupported combinations:
  - If provider does not support streaming tools, then for `stream=true` either:
    - pass no tools for that request, or
    - force non-streaming for that request with a clear UI indicator.
- Full streaming tool-call parity is a later phase item (not required here), but no silent failures.

### Acceptance
- The app builds and runs.
- Tool results cannot be submitted with mismatched IDs.
- Unsupported provider/stream/tool combinations are blocked by a single, centralized capability model.

Tests
- Unit tests for capability decisions (matrix) and tool_call_id validation.

Rollback
- Strict tool ID validation can be disabled behind `settings.feature_flags.strict_tool_ids` if needed.

---

## Phase 2 — Memory correctness (Tantivy + deletes) + maintenance tooling

This phase addresses the biggest correctness gap: stale results from an append-only search index.

### 2.1 Extend the full-text index schema to support deletions (explicit migration)
- Add fields to Tantivy documents:
  - `message_id` (STRING | STORED)
  - `created_at` (optional, STORED)
  - keep `conversation_id`, `role`, `content`
- Update indexing calls so that when saving a message, you index with `message_id`.
  - This requires generating the message UUID in Rust and using it for both SQLite insert and Tantivy insert, or returning the inserted id from SQLite.
- Migration note:
  - existing index documents will not have `message_id` until a rebuild is run.
  - search/retrieval should tolerate mixed documents during the transition.

### 2.2 Implement deletion hooks
- On message delete: delete Tantivy documents by `message_id`.
- On conversation delete: delete Tantivy documents by `conversation_id` and/or by enumerating message_ids.
- On pruning: delete Tantivy docs for pruned message IDs.

### 2.3 Add “Rebuild index” maintenance
- Backend command `rebuild_memory_index`:
  - wipe Tantivy index directory
  - read all messages from SQLite
  - re-index in batches
- Frontend Settings page: “Rebuild memory index” button with confirmation and progress indicator.

### 2.4 Make the index consistent with “deleted” data expectations
- Ensure that once a message is deleted in SQLite, it does not appear in search results.

### Acceptance
- After deleting a message/conversation, it no longer appears in memory search.
- Rebuild index restores consistency and populates `message_id` for all documents.

Tests
- Integration test: delete message -> search no longer returns it.
- Integration test: rebuild -> search returns expected docs.

Rollback
- Deletion hooks can be disabled behind `settings.feature_flags.enable_index_deletes` to revert to append-only behavior while investigating bugs.

---

## Phase 3 — Safe multi-tool execution (tool loop + permissions + audit)

This phase intentionally merges the previous “tool loop” and “permissions/audit” work because they are tightly coupled.

### 3.1 Multi-tool request and approval UX
- Update UI state model to handle multiple tool calls per assistant message and repeated tool turns.
- Replace “one modal per call” with a single approval modal listing all requested tool calls:
  - allow/deny per call
  - approve all / deny all
  - display server/tool name and short description

### 3.2 Permission policy model in settings
Extend settings schema for MCP tool execution:
- per-server policy:
  - `auto_approve` (already exists)
  - allowlist/denylist of tools
  - optional “read-only auto approve” mode
- per-tool overrides (optional)
- session-only approvals (non-persistent)

### 3.3 Enforce permissions server-side
- Before executing `tools/call`, evaluate policy:
  - if not approved, return a structured `approval_required` response
  - do not execute tools without passing the backend gatekeeper
- Update frontend to treat `approval_required` as a modal request (and to persist choices when requested).

### 3.4 Tool execution audit log + result storage
- Add SQLite table `tool_executions` (via migration framework from Phase 0):
  - id
  - conversation_id
  - tool_name
  - args_json
  - result_preview
  - result_full_json (or file-backed storage reference)
  - result_hash
  - created_at
  - status (ok/denied/error)
- UI: add a per-conversation “tool timeline” panel.

### 3.5 Tool output shaping (token-safety)
- Add backend shaping for tool results:
  - hard cap injected context size
  - store full result in `tool_executions`, inject truncated/summarized preview
  - clearly mark truncation boundaries
- UI:
  - collapsed tool output with “show preview” and “show full” (when stored)
  - show how much was truncated

### Acceptance
- Multiple tool calls work end-to-end.
- Tools cannot execute without server-side approval logic.
- Tool executions are auditable, and large outputs do not explode prompt size.

Tests
- Unit tests: permission evaluation logic.
- Integration tests: approval-required path; approved execution path; audit log persisted.

Rollback
- Keep the old single-tool flow behind `settings.feature_flags.multi_tool_loop`.
- Keep policy enforcement behind `settings.feature_flags.enforce_tool_permissions` until stable.

---

## Phase 4 — MCP transport reliability + tool discovery performance

### 4.1 SSE robustness
- Implement SSE buffering by event delimiter (`\n\n`) instead of naive `.lines()` parsing.
- Add reconnect with exponential backoff.
- Preserve support for POST responses that include a JSON-RPC response body.

### 4.2 Stdio lifecycle tracking
- Track child process exit:
  - retain a handle to the spawned process
  - reflect “disconnected” when the child dies
  - store last stderr/error state for UI display

### 4.3 Timeouts and cancellation for tool calls
- Ensure tool calls have:
  - backend timeouts
  - cancellation propagation when user stops/cancels
  - clear error surfaces in UI for timeouts/cancellation

### 4.4 Tool discovery caching
- Cache tool lists per server (TTL + invalidation on restart/initialize).
- Invalidate cache on “listChanged” notifications when supported.

### Acceptance
- SSE servers survive transient disconnects.
- Stdio servers show accurate status.
- Tools panel refresh is faster and does fewer round trips.
- Tool calls do not hang indefinitely.

Tests
- Unit tests: SSE event framing parser.
- Integration tests: reconnect behavior (where feasible) and request timeout path.

---

## Phase 5 — Context management v2 (token budgeting + pruning policy)

### 5.1 Token accounting improvements
- Count tokens across:
  - tool arguments/results
  - system prompts
  - attachments
- Make context window configurable per model/provider.

### 5.2 Smarter pruning strategy
- Replace “reverse until budget exceeded” with a policy:
  - always keep system prompt(s)
  - keep last N turns
  - summarize older tool outputs and older assistant text first

### Acceptance
- Context pruning is predictable and configurable.
- Token budgeting accounts for tool data and attachments.

Tests
- Unit tests: pruning edge cases (empty content, huge tool outputs, multi-system prompts).

Rollback
- Keep old pruning logic behind `settings.feature_flags.pruning_v2`.

---

## Phase 6 — UX features: export/import, global search, branching

### 6.1 Conversation export/import
- Export formats:
  - Markdown
  - JSON (including tool calls/results and metadata)
- Import:
  - the same JSON format
  - optionally OpenAI-style logs (best-effort)

### 6.2 Global search across chats
- Add backend command `search_messages(query, conversation_id?)`:
  - implement via SQLite FTS or Tantivy
  - results must include message_id and a snippet
- UI:
  - search box with results list
  - click to jump to message

### 6.3 Branching / fork / rewind (requires mini-design)
- Before implementation, write a short design decision note covering:
  - how to represent “rewound/archived” messages in SQLite
  - how Tantivy and memory injection exclude non-canonical history
  - how UI indicates fork/rewind state
- Backend support:
  - “fork conversation from message_id” (copy messages up to id into new conversation)
  - “rewind conversation to message_id” (soft delete tail or archive)
- UI:
  - per-message actions: Fork from here, Rewind to here

### Acceptance
- Export/import round-trips.
- Search returns accurate results and navigates.
- Fork creates a new conversation with correct history.
- Rewind semantics are explicit and do not corrupt search/memory.

Tests
- Integration tests: fork creates correct message set; search results align with canonical history.

---

## Phase 7 — Attachments persistence + ergonomics

### 7.1 Persist attachments in storage
- Add a new SQLite table `attachments` (preferred) or add columns to `messages`:
  - message_id, filename, media_type, is_binary, storage_path or blob reference
- Store binaries on disk and keep references in DB.
- Ensure attachments can be reconstructed into provider-specific request formats.

### 7.2 Client-side constraints
- Add file size caps + warnings.
- Downscale/compress images before sending.

### Acceptance
- Attachments survive app restart.
- Large files are handled gracefully.

Tests
- Integration tests: attachment persists across restart; message export includes attachment metadata.

Rollback
- Make persistence optional behind `settings.feature_flags.persist_attachments`.

---

## Phase 8 — Diagnostics UI + ongoing hardening

This phase is for polishing developer and user support tooling (the foundation was laid in Phase 0).

### 8.1 Diagnostics panel
- Add an in-app diagnostics view:
  - current provider/model
  - MCP server statuses + last errors
  - recent tool execution summaries
- Add redaction for sensitive data in diagnostics and logs.

### 8.2 CI sanity
- Add a minimal CI workflow that runs:
  - Rust checks/tests
  - TypeScript build/typecheck

### Acceptance
- Diagnostics are useful and do not leak secrets.
- CI catches obvious regressions.

---

## Suggested implementation order (summary)

1. Phase 0 (foundations) — observability, secrets, migrations, and concrete refactor targets.
2. Phase 1 (systemic correctness) — capability model + tool-id correctness + wiring fixes.
3. Phase 2 (memory/index correctness) — prevent data integrity surprises.
4. Phase 3 (safe multi-tool execution) — tool loop + permissions + audit in one cohesive slice.
5. Phase 4–5 (reliability + context mgmt) — reduce operational issues and token blowups.
6. Phase 6–7 (UX + attachments) — product features.
7. Phase 8 (diagnostics + CI) — polish and long-term maintainability.

## Open questions (to resolve before implementation)
- Keychain/encryption approach: key naming, multi-profile support, and behavior on keychain access failure.
- Migration policy for existing plaintext provider keys: one-time import + redaction, and how to recover if import fails.
- “Large payload” storage choices: whether tool results and attachments should be DB blobs vs file-backed storage with hashes.
- Message identity strategy: ensure SQLite and Tantivy share the same stable `message_id` (and how to handle historical rows).
- Search implementation for global search: SQLite FTS vs Tantivy (and expected snippet/jump-to-message behavior).
- Fork/rewind semantics: canonical history rules, how summaries/indexing handle archived branches, and UI affordances.

## Deliverables tracking

For each phase, deliverables should include:
- code changes + migrations
- a short section added/updated in README (how to use the new feature)
- tests (or documented manual verification if tests are not feasible)
- a feature flag entry (when behavior changes) and a clear rollback instruction
