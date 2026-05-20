# Bug Scrub — nebula_chat

_A read-only audit of latent bugs. Findings are grouped by severity and dedup'd against `review.md`, `remediation.md`, and `settings_fix.md`. Each finding has been verified against the current code — agent-suggested issues that didn't hold up under inspection are listed at the bottom._

---

## CRITICAL — the regen "two copies" bug

**`src-tauri/src/lib.rs:585-626`** — `save_full_message_returning_id` is called **twice** in the success path for user messages. The shape is `if let Err(e) = save(...) { log } else if let Ok(msg_id) = save(...) { extract_facts }` — the `else if` branch re-invokes the save instead of using the id from the first call.

**Why it matters:** Every fresh user send (`handleSend` always creates a `userMsg` without `id`) writes the user message to SQLite twice. On regen, `handleRegenerate` keeps the in-UI user message (which was never re-fetched from the DB and so has no `id`), and the resubmit double-saves it again. The UI doesn't show this until the conversation is reloaded via `loadHistory` → `get_chat_history` returns all the duplicate rows. This is the smoking gun for "regen creates two copies of messages sometimes."

**Fix (one line):**
```rust
match lib.save_full_message_returning_id(conv_id, &last.role, last.content.as_deref(),
    tool_calls_json.as_deref(), last.tool_call_id.as_deref(), None, last.attachments.as_deref()) {
    Err(e) => tracing::error!("Failed to save user message: {}", e),
    Ok(msg_id) => { /* spawn fact extraction with msg_id */ }
}
```

**Related:** after fixing, also consider back-filling `response.id` (or making `send_message` return the saved user-message id) so subsequent regens have a stable id to short-circuit the `last.id.is_none()` guard.

---

## HIGH

1. **`src-tauri/src/memory/sqlite_manager.rs::save_full_message`** — does not invalidate `tool_call_cache`. Only `delete_messages` (line 906) calls `invalidate_tool_call_cache()`. **Why it matters:** A negative `tool_call_id_exists` lookup is cached for `TOOL_CALL_CACHE_TTL`; if a subsequent assistant message is saved with that tool call id during that window, the cached `false` poisons the lookup. The integration test `tests/tool_validation_test.rs` actually fails on this — see "broken tests" below.

2. **`src/components/ChatInterface.tsx:946-957`** (non-streaming response handler) races with **`src/components/ChatInterface.tsx:228-287`** (`maybeRecoverCompletedGeneration`). When `stream === false`, the `.then()` handler always appends (`[...prev, response]`) without checking whether `setMessages(history)` from the focus/visibility recovery already added that response. **Why it matters:** Send with streaming off → tab loses focus while the request is in flight → on refocus, recovery reads DB and pushes the assistant message; original invoke resolves and pushes it again → duplicate assistant bubble.

3. **`src-tauri/src/llm/compactor.rs:189-211`** — the "last_id not found" safety check resets `effective_last_id = None` but does **not** also flip `found_last` to `true`. The loop at 219 then compares each msg.id against `None`, never matches, and skips everything. **Why it matters:** When the compaction marker is lost (e.g. summary references a pruned message), the safety check pretends to recover but actually emits an unchanged old summary + the keep_raw tail. The test `test_last_id_not_in_range_safety` passes only because the empty-chunk fallback at line 245 returns `(Some(last_summary), ...)` — silent partial data loss. The remediation.md fix was implemented incompletely.

4. **`src-tauri/src/lib.rs:1766-1789`** — `rebuild_memory_index` holds `lib.lock()` for the entire rebuild (`list_conversations` + `get_complete_history` + `index_existing_message` for every message). **Why it matters:** Every other Tauri command needing the librarian (every send, search, delete, list) is blocked until rebuild completes. On a 10k-message DB this can be minutes; the UI looks frozen.

5. **`src-tauri/src/mcp/client.rs:606-655`** — JSON-RPC pending map / `CancelGuard` interaction. On a `request_with_timeout` timeout, the timeout branch removes the pending entry; the `CancelGuard`'s Drop also tries to send a `notifications/cancelled`. The pending-entry removal and the cancellation send are racy. **Why it matters:** Possible leaked oneshot senders on timeout, or duplicate cancellation notifications. Not data-corrupting but worth tightening.

---

## MEDIUM

6. **`src-tauri/src/llm/compactor.rs::parse_model_id`** — returns `None` for malformed ids. The compactor handles `None` (line 285-292), but other callers (e.g. `lib.rs:618` fact extraction spawn) split on `"::"` and index `parts[0]` / `parts[1]` after only checking `parts.len() == 2`. A model id with a single colon or no `::` silently aborts the spawn — fact extraction stops working with no user-visible error. Inconsistent across call sites.

7. **`src-tauri/src/lib.rs:1927-1959`** — `import_conversation` saves messages one at a time, no transaction. If one save fails mid-import, you get a partial conversation. **Why it matters:** Surfaces as silently truncated imports.

8. **`src/components/ChatInterface.tsx:1734` + `:1495`** — `ChatMessage` keys fall back to `msg-${i}` (array-index based) when a message has no id. After regen truncates and reinserts, indices shift; React can reuse stale DOM. **Why it matters:** Visual ordering glitches, especially during streaming when the placeholder has a `tempMsgId` and the final response is mid-replace.

9. **`src/components/ChatInterface.tsx:960`** — title generation triggered when `currentHistory.length === 1`. If the user sends a second message before the title finishes, or regens, two `generate_title` calls overlap. The backend writes the title via `save_settings`/`update_conversation_title_and_icon` with no concurrency control. **Why it matters:** Title flicker; potential last-writer-wins overwrite.

10. **`src/components/SettingsPage.tsx`** — `save_settings` after each toggle reads the latest `fullSettings` from a `useState`. Rapid consecutive toggles can read stale state between renders, and the second save can clobber the first. **Why it matters:** Toggle a provider off then on quickly → ends up off.

11. **`src-tauri/src/llm/compactor.rs::rollback`** — failures from `save_conversation_summary` are dropped with `let _ = ...`. **Why it matters:** Compaction rollback can silently leave the DB inconsistent.

12. **`src-tauri/src/lib.rs:1810-1856`** — `export_conversation` holds the librarian lock while serializing all messages to JSON. Less severe than #4 because it's read-only and finite, but it still blocks concurrent sends.

13. **`src-tauri/src/memory/extraction.rs::extract_and_parse_json`** — searches for `{` before `[`, so a bare JSON array of objects collapses to its first inner object and fails envelope parsing. The "bare array" fallback is effectively dead code. (Already covered by the new test `parse_json_bare_array_of_objects_is_not_recovered`.)

---

## LOW / hygiene

14. **`src-tauri/src/mcp/client.rs::McpClient::new` stdio branch (line ~416-424)** — `cmd.spawn()` returns a `Child`; the subsequent `take()` calls on stdin/stdout/stderr return `None` if the platform doesn't pipe them, which would propagate via `?`. The `Child` drops; `tokio::process::Child` does **not** kill on drop. **Why it matters:** On a malformed transport config, a zombie subprocess can survive. Realistically rare but trivial to fix with `kill_on_drop(true)`.

15. **`src-tauri/src/lib.rs:159-168`** — `regenerate_title_and_icon` is a wrapper that just forwards to `generate_title`. Either delete or give it actual distinct behaviour.

16. **`src-tauri/src/lib.rs:613-624`** — the background fact-extraction spawn reloads `Settings::load_migrated` from disk separately from the parent's load at line 649. If the user saves settings between the two reads, fact extraction can run with a different model than the rest of the request. Minor.

17. **`src/components/ChatInterface.tsx:788`** — `tempMsgId = "streaming-" + Math.random().toString(36)`. Math.random has ~52 bits of entropy and is base36-encoded with no fixed length. Collision is astronomically unlikely in practice but `crypto.randomUUID()` is already used at line 791 for `requestId` — use it here too.

18. **`src/components/ChatInterface.tsx::processDroppedPaths`** (~line 455-501) — no dedup if the same file is dropped twice; image blob URLs are never `URL.revokeObjectURL`'d.

19. **`src-tauri/src/mcp/manager.rs::get_server_for_tool`** — silently returns `None` for tool names without `__`, no log. Tools that don't match the server-prefixed naming convention vanish from the routing layer. Add a `tracing::warn!`.

20. **`:memory:/` directory tracked in git** (root of `src-tauri/`) — a literal directory created by code that passes the string `":memory:"` as a `Path`. Contains a `nebula.db` and a Tantivy index. Add to `.gitignore` and `git rm -r` it.

---

## Broken tests blocking the suite

These were already broken on `main` before the Tier-2 work — they prevent `cargo test` from going green:

- **`src-tauri/tests/tool_validation_test.rs`** — (a) `save_full_message` was called with 5 args, signature is now 6; (b) only calls `migrate_v2`, missing `migrate_v3` for the `reasoning_content` column; (c) once compiled and migrated, the test still fails because of finding #1 (cache poisoning).
- **`src-tauri/tests/sse_cancellation_test.rs`** — compile-only assertion, exercises no behaviour.
- **`src-tauri/tests/tantivy_performance_test.rs`** — named "performance" but has no time-budget assertion.
- **`src-tauri/src/lib.rs:28-32`** — `test_tool_call_validation_integration` is a placeholder with an empty body.

---

## Recommended order of attack

1. **Fix #1** (regen dup user save) — one-line change, biggest user-visible impact.
2. **Fix #2** (non-streaming recovery race) and the placeholder/key issues (#8, #17) together — small frontend cleanup, removes the remaining "duplicate message" surfaces.
3. **Fix #4** (rebuild_memory_index lock) and #12 (export lock) — move the work to a background task or stream-iterate without holding the librarian guard.
4. **Fix #3** (compactor `found_last` bug) — completes the safety check that remediation.md tried to land.
5. **Fix the tool_call_cache** — either invalidate on `save_full_message` (and other writers) or drop the cache entirely; then re-enable `tool_validation_test.rs`.
6. **Hygiene pass:** delete `:memory:/`, fix `regenerate_title_and_icon`, replace the stub tests with real ones.

---

## Findings I investigated but rejected

- _"Anthropic streaming drops tool calls" was flagged in review.md_ — still present, but already documented; not adding here.
- _Backend agent claimed `mcp/config.rs::Settings::save` loses credentials on keychain write failure._ Wrong — the `else` branch at line 404 only strips the key when `set_secret` returns `Ok`. Verified.
- _Backend agent claimed `StdioTransport::send` holds the status lock across `tx.send().await`._ Wrong — the lock is released at the end of the `if let` block on line 70, before the `await` at line 71.
- _Backend agent claimed `get_all_tools` holds the clients read lock across `client.request().await`._ Wrong — line 209 explicitly clones the map and drops the guard before the loop (the comment even calls this out).
- _Backend agent claimed Tantivy batch task panics silently break indexing._ Mostly wrong — `mpsc::UnboundedSender` is non-blocking; if the receiver dies, `add_document` returns an error. The real (much milder) issue is that `process_batch` errors are dropped with `let _` in `IndexBatchProcessor::run`.
- _Backend agent claimed `lib.rs:543-626` causes a deadlock between the librarian guard and a child `tokio::spawn`._ Wrong — the spawn captures `state.librarian.clone()` (an `Arc<Mutex<Librarian>>`) and re-locks inside the task; no deadlock. The actual bug at that site is the double-save (finding #1).
