# Code Review (Zen / Gemini 3 Pro Preview)

This review is a synthesis of:
- A local pass over the repository (frontend + Rust backend), and
- An external review pass via Zen using `google/gemini-3-pro-preview`.

Where applicable, I call out claims from the external pass that do **not** match the code in this repository.

## Top 3 issues to fix first

1) **[CRITICAL] Tool result integrity is not enforced server-side (DB-backed).**
- **Where:** `src-tauri/src/lib.rs:304-333`
- **Today:** Tool result validation relies on scanning the *client-provided* `messages` vector to see whether a preceding assistant message contains a `tool_calls[].id` that matches `tool_call_id`.
- **Why it matters:** A malicious/buggy client can spoof tool results, inject results from another conversation, or replay stale tool IDs.
- **Fix:** Validate `tool_call_id` against **persisted** conversation history (SQLite) for the *same* `conversation_id`, and reject if not present. Optionally enforce ordering (tool result must follow the assistant tool call).

2) **[HIGH] Tantivy indexing commits + reloads on every message.**
- **Where:** `src-tauri/src/memory/tantivy_index.rs:67-95`
- **Today:** `add_document()` creates a writer, adds one document, commits, and reloads the reader per call.
- **Impact:** This will become a major latency/IO bottleneck with large histories or during rebuilds.
- **Fix:** Batch writes and commit periodically (N docs or time-based), then reload once per batch.

3) **[HIGH] CSP is disabled.**
- **Where:** `src-tauri/tauri.conf.json:21-23` (`"csp": null`)
- **Impact:** Increases the blast radius of any XSS/injection vector (rendered model output, future UI changes, plugin surfaces).
- **Fix:** Set a restrictive CSP baseline and open only necessary `connect-src` origins.

## Findings by severity

### Critical
- **Tool result validation is client-trusting**
  - **Where:** `src-tauri/src/lib.rs:304-333`
  - **Recommendation:** Add DB-backed validation for `tool_call_id` (exists in prior assistant tool call within the same conversation). Reject invalid tool messages before saving.

### High
- **Tantivy commit strategy is synchronous per document**
  - **Where:** `src-tauri/src/memory/tantivy_index.rs:67-95`
  - **Recommendation:** Keep a long-lived writer/queue; commit periodically.

- **SSE transport has no explicit cancellation**
  - **Where:** `src-tauri/src/mcp/client.rs:88-175` (retry loop); plus no explicit stop signal wired into manager removal/shutdown.
  - **Recommendation:** Introduce a cancellation token / abort handle and stop the loop on `remove_server()` / shutdown.

- **Anthropic streaming drops tool calls**
  - **Where:** `src-tauri/src/llm/anthropic.rs:386-397` (`tool_calls: None // omitted`)
  - **Impact:** Tools may work in non-streaming but silently fail in streaming.
  - **Recommendation:** Implement tool-use parsing for Anthropic streaming or disable streaming when tools are present.

### Medium
- **SQLite lacks indices + deletes are O(n) loops**
  - **Where:** `src-tauri/src/memory/sqlite_manager.rs:212-221` and `src-tauri/src/memory/audit_logger.rs:51-60`
  - **Recommendation:** Add indices for common lookups and batch deletes (`WHERE id IN (...)`). Add index on `tool_executions.tool_call_id`.

- **Tool outputs stored unbounded in SQLite**
  - **Where:** `src-tauri/src/memory/audit_logger.rs:18-49`
  - **Recommendation:** Cap output sizes; store full payloads on disk and keep a preview in DB.

- **Streaming UI update assumptions**
  - **Where:** `src/components/ChatInterface.tsx:453-511`
  - **Recommendation:** Update by message ID (not “last element”), and throttle streaming updates.

- **ContextAssembler instantiates providers per call**
  - **Where:** `src-tauri/src/llm/context_assembler.rs:33-61`
  - **Recommendation:** Reuse shared HTTP client/provider instances if strategist usage is frequent.

### Low
- **Background pruning spawned even when `conversation_id` is None**
  - **Where:** `src-tauri/src/lib.rs:576-589` passing `conv_id_bg.unwrap_or_default()`
  - **Recommendation:** Only spawn pruning if `conversation_id.is_some()`.

## Corrections / unverified items from the external pass

- **Claim (external):** `McpManager` holds a write lock across `await` during server init, risking deadlock.
  - **Status:** **Not supported by code**. In `src-tauri/src/mcp/manager.rs:90-111`, the async start happens outside the `clients` write lock; the lock is taken only briefly to insert the client.

- **Claim (external):** overly-permissive `src-tauri/capabilities/default.json`.
  - **Status:** **Unverified** in this review (file not located in the earlier enumerated repository layout).

## If implementing fixes

Suggested order:
1) DB-backed tool_call_id validation + DB indices
2) CSP baseline
3) SSE cancellation wiring
4) Tantivy batching
5) Anthropic streaming tool parity
