# Mitigation Plan (Zen / Gemini 3 Pro Preview)

Scope: Mitigate the top four highest-priority issues identified in `review.md`.

## 1) [CRITICAL] Server-side tool result integrity validation (DB-backed)

### Risk
`tool` messages (tool results) can be spoofed or replayed if validation depends on client-provided message history rather than persisted server-side conversation history.

### Target state
- A tool result is accepted only if:
  - `conversation_id` is present and valid
  - `tool_call_id` is present
  - `tool_call_id` exists in a *prior assistant message* for the same `conversation_id` (persisted)
  - (Optional) the tool call has not already been completed (prevent replay)

### Proposed changes
- Add a SQLite query to check whether a tool call id exists in `messages.tool_calls` JSON for the given conversation.
  - If SQLite JSON functions are available, use `json_each()` to check the tool call IDs precisely.
  - If JSON1 is unavailable (edge builds), fallback to a conservative check (e.g., substring match) but prefer JSON1.
- Enforce this validation inside `send_message` before saving tool messages.
- Add an index on `messages(conversation_id, created_at)` to make scanning efficient.

### Acceptance criteria
- If a client submits a `tool` message with an unknown `tool_call_id`, backend returns a clear error and does not save the message.
- If a client submits a tool result tied to a different conversation, backend rejects it.
- Unit/integration test (if feasible) covers:
  - Valid tool_call_id accepted
  - Invalid tool_call_id rejected
  - Replay attempt rejected (if implemented)

### Files likely touched
- `src-tauri/src/lib.rs`
- `src-tauri/src/memory/sqlite_manager.rs` (new query helper)
- (Optionally) `src-tauri/src/memory/librarian.rs` (new method)

---

## 2) [HIGH] Tantivy indexing performance (batch commits)

### Risk
Index writes currently commit and reload for every message, causing high IO and latency as data grows.

### Target state
- Indexing is amortized:
  - Writes are buffered
  - Commits happen in batches (N docs or periodic)
  - Reader reload happens after commits

### Proposed changes
Option A (simpler):
- Implement a background indexing queue:
  - `Librarian::save_full_message` enqueues an indexing job
  - A worker batches jobs and commits every N messages or after T milliseconds

Option B (in-process batching):
- Keep a long-lived writer in `TantivyIndex` behind a mutex and commit every N calls.

### Acceptance criteria
- Adding 100 messages does not perform 100 commits.
- Rebuild completes substantially faster for large histories.
- Search results still reflect recent messages after the next commit interval.

### Files likely touched
- `src-tauri/src/memory/tantivy_index.rs`
- `src-tauri/src/memory/librarian.rs`

---

## 3) [HIGH] CSP disabled in Tauri config

### Risk
CSP being null increases the impact of XSS/injection vectors, especially when rendering model output.

### Target state
- A restrictive CSP is set and verified in production builds.
- Only required external endpoints are allowed via `connect-src`.

### Proposed changes
- Update `src-tauri/tauri.conf.json` to set a baseline CSP.
  - Keep it minimal and iterate if you hit legitimate breakage.
  - Ensure `connect-src` includes:
    - the local dev server in dev mode (if necessary)
    - provider endpoints you explicitly support (OpenAI/Anthropic)
    - any approved OpenAI-compatible base URLs (if you allow arbitrary, consider a user-configurable allowlist instead)

### Acceptance criteria
- App runs in production mode with CSP enabled.
- Markdown rendering and syntax highlighting still work.
- Network calls to configured providers still work.

### Files likely touched
- `src-tauri/tauri.conf.json`

---

## 4) [HIGH] SSE lifecycle: add explicit cancellation/cleanup

### Risk
SSE loops can continue reconnecting forever, leak tasks, and keep resources alive after server removal or app shutdown.

### Target state
- SSE loop can be cleanly stopped:
  - on `remove_server`
  - on `restart_server`
  - on application shutdown

### Proposed changes
- Add a cancellation mechanism to `SseTransport`:
  - store an `AbortHandle`, `CancellationToken`, or shared atomic flag checked in the loop
  - on stop, abort the loop task and clear `sse_handle`
- When stopping, fail any pending RPC requests with a “disconnected” error so callers don’t hang.

### Acceptance criteria
- Removing a server stops reconnection attempts.
- Restarting a server does not leave the old SSE task running.
- Pending requests fail promptly when transport is stopped.

### Files likely touched
- `src-tauri/src/mcp/client.rs`
- `src-tauri/src/mcp/manager.rs`

---

## Suggested execution order
1) Tool result integrity (critical correctness/security boundary)
2) CSP baseline (hardening)
3) SSE cancellation (reliability)
4) Tantivy batching (performance)

## Open questions
- Do you want to prevent tool_call replay (one tool_call_id → one tool result), or allow retries?
- Should the CSP allow arbitrary OpenAI-compatible base URLs, or should that require an explicit allowlist?
- For Tantivy batching, is “eventual indexing” acceptable (e.g., up to 1–2 seconds delay), or must it be synchronous?
