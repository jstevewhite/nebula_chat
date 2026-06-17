# Nebula – Feature gaps & improvement ideas

This document is a quick review of the current `nebula_chat` codebase (React/Tauri UI + Rust backend). It focuses on high-impact missing features, correctness gaps, and architectural improvements.

> **Status (2026-06-13, v0.9.0):** This was a pre-v0.9.0 review and several gaps below have since shipped — they're annotated inline with **✅ Shipped**. Notably: the **memory3** doc store + knowledge-graph facts replaced the old Tantivy-only / "Strategist" design; **Anthropic streaming now handles tool calls**; the Tantivy index supports **deletes + a rebuild action**; a **tool-execution audit log** exists; API keys can live in the **OS keychain**; and **conversation export/import + global full-text search** shipped. Items without a ✅ are still open.

## Recently shipped

- **Slash commands** (`feature/slash_commands`): client-side command palette accessible by typing `/` in the composer or by pressing `/` while focused in the conversation list sidebar. Ships with `/help`, `/new`, `/model`, `/clear`, `/remember`, `/recall`, `/facts`, `/search`, `/skills`, and `/skill <slug>` (10 total; see `src/utils/chatCommands.ts`). Spec: [`docs/superpowers/specs/slash-commands.md`](superpowers/specs/slash-commands.md).

## Top priorities (high impact / low-medium effort)

### 1) Fix obvious correctness issues
- **✅ Shipped — Duplicate command registration**: `delete_system_prompt` is now registered exactly once in the `invoke_handler` list (`src-tauri/src/lib.rs:3586`).
- **✅ Shipped — Anthropic streaming drops tool calls**: `AnthropicProvider::stream` now accumulates tool calls via `content_block_start/_delta/_stop` (`src-tauri/src/llm/anthropic.rs`), and capabilities mark `supports_streaming_tools: true` (`src-tauri/src/llm/capabilities.rs`).
- **Tool-result ID integrity**: the UI sets `tool_call_id` based on the tool call’s id (or a generated fallback), but the backend does not validate a matching tool call exists in the preceding assistant message.
  - Why it matters: for providers like Anthropic, `tool_call_id`/`tool_use_id` must match exactly; mismatches lead to confusing “tool result not found” failures.

### 2) Make tool calling a true “tool loop”
Current UX is a single-step pattern: model responds with one tool call → UI pauses → user clicks Allow → tool result is injected → UI calls `send_message` again.

Missing pieces that would make tools feel first-class:
- **Support multiple tool calls per assistant message** (and repeated tool turns): handle N tool calls sequentially, with per-call approval.
- **Tool call provenance in the UI**: show which server, tool name, and a summarized risk level (“read-only”, “writes files”, etc.).
- **Approval modes** beyond one-off Allow/Deny:
  - Allow once
  - Always allow this tool
  - Always allow this server
  - Allow read-only tools automatically (deny by default for write tools)
- **Tool output handling**:
  - Truncate/compact very large tool outputs before storing/injecting into context (DoS prevention + token budget control).
  - Store full output locally but inject a summarized version to the model.

### 3) Memory correctness: keep the search index consistent with deletes
**✅ Shipped.** The Tantivy index is no longer append-only: documents store `message_id` and the index supports `DeleteByMessageId` / `delete_term` / `delete_all_documents` (`src-tauri/src/memory/tantivy_index.rs`), deletes propagate on message/conversation deletion, and a **Rebuild memory index** maintenance action exists in Settings. (Worth a re-check that every prune path calls the delete, but the "append-only" framing no longer holds.)

## MCP & tools (feature gaps)

### 4) Permissions/guardrails actually enforced server-side
**Partially shipped.** There is now a real approval flow, not direct execution: disabled-tool filtering plus a context-inspection approval gate (oneshot + timeout) before `execute_tool`, with `auto_approve` / `auto_approve_tools` and per-server allowlist/denylist plumbed through (`src-tauri/src/lib.rs`, `src-tauri/src/mcp/config.rs`). **✅ Tool call audit log** is implemented (`AuditLogger::log_execution` / `get_execution_by_tool_call_id`, `src-tauri/src/memory/audit_logger.rs`).

Still open:
- **Read-only vs write tool classification** with auto-allow-read / deny-write defaults.
- A per-conversation **“tool timeline”** UI over the existing audit log.

### 5) Better MCP transport reliability
- **SSE parsing/reconnect**: `SseTransport` reads raw byte chunks and splits by lines; it does not robustly parse SSE event boundaries, and has minimal retry/backoff.
- **Stdio lifecycle**: `StdioTransport::is_connected()` is “channel open”, not “child still alive”. If the subprocess dies, the system may misreport connectivity.

Suggested improvements:
- Implement SSE buffering by event delimiter (`\n\n`) and support reconnect with exponential backoff.
- Track subprocess exit status for stdio servers and expose status + last error in UI.

### 6) Tool discovery caching & performance
`McpManager::get_all_tools()` calls `tools/list` for every server and every refresh.

Improvements:
- Cache tool lists per server, with invalidation when servers restart or send `listChanged`.
- Add “last refreshed at” and error status per server.

## LLM provider & generation features

### 7) Token budgeting & context pruning is currently provider-blind
`ContextManager::prune_messages(..., 64000)` is hard-coded and counts only message `content` using a single tokenizer (`cl100k_base`).

Improvements:
- Per-provider / per-model context window configuration.
- Count tokens across:
  - tool arguments
  - tool results
  - system prompt(s)
  - attachments (text)
- Smarter pruning policy:
  - keep system prompt(s)
  - keep the last N user/assistant turns
  - aggressively summarize older tool outputs

### 8) Streaming parity across providers
**✅ Shipped.** Anthropic streaming now reassembles tool calls like OpenAI does, and a provider **capability matrix** exists — `Capabilities` / `get_capabilities()` cover `supports_tools`, `supports_streaming_tools`, and `supports_multimodal` (`src-tauri/src/llm/capabilities.rs`).

### 9) More generation controls
**Partially shipped.** `provider.rs` now carries `max_tokens`, `presence_penalty`, `frequency_penalty`, and `reasoning_effort` (`src-tauri/src/llm/provider.rs`), in addition to temperature/top_p/stream. `reasoning_effort` is honored across providers — including Anthropic, where it drives adaptive extended thinking on 4.6+ models — and an unset ("Auto") `max_tokens` resolves to the model's real output ceiling rather than a fixed 4096.

Still missing:
- stop sequences
- seed (when supported)
- response format / JSON mode (when supported)

## UX improvements (high-value product features)

### 10) Conversation export/import
**✅ Shipped (core).** Conversations export as JSON (lossless) and Markdown, and JSON import restores/migrates conversations. Still open: importing from other clients' formats (e.g. OpenAI chat logs).

### 11) Global search across chats
**✅ Shipped.** Full-text search across all message bodies is available from the sidebar and via the `/search` command (Tantivy BM25), not just title filtering. Possible follow-ups: richer snippet display and jump-to-message.

### 12) Branching / rewind / “fork chat”
You support delete/regenerate but not true branching.

Add:
- “Fork from here” to create a new conversation starting at an earlier message.
- “Rewind to message” (soft delete tail; keep a recoverable history).

### 13) Better tool approval experience
- Show a diff-style view for file edits (when tools return patches).
- Add “always allow for this session” (no settings write) as a safer option than “always allow forever”.

### 14) Attachments ergonomics
Current attachments are embedded into the user message content (text) or passed as data URLs (images).

Improvements:
- File size limits + warnings.
- Image downscaling/compression before sending.
- Persist attachments in the conversation history (right now only the last user message gets `attachments` assigned at send time, and the DB schema doesn’t store attachments).

## Backend architecture & performance

### 15) Avoid blocking the async runtime with SQLite/Tantivy writes
The memory layer uses synchronous `rusqlite` + Tantivy commits on each message.

Improvements:
- Move DB/index writes onto a dedicated worker via `spawn_blocking` or a background queue.
- Batch Tantivy commits (commit per message is expensive) and reload on a timer.

### 16) Make memory pruning safer and more controllable
- Pruning thresholds are hard-coded (`limit=50`, `prune_count=20`).
- Summaries are stored with `role = system_summary` but the frontend doesn’t render these specially.

Improvements:
- Expose pruning settings in UI.
- Tag summaries clearly in the UI and allow expanding the summarized chunk.

## Observability, debugging, and quality

### 17) Structured logging + user-visible diagnostics
Backend uses `println!/eprintln!` with some `[DEBUG]` markers.

Improvements:
- Use `tracing` + log levels.
- Add an in-app “Diagnostics” panel:
  - current provider, model
  - last error
  - MCP server statuses
  - last tool call results (truncated)

### 18) Test coverage + CI sanity
- Add Rust unit tests for:
  - MCP name parsing (`server__tool`)
  - settings migration
  - tool-call serialization
  - pruning logic edge cases
- Add basic integration tests for SQLite schema migrations.

## Security & privacy

### 19) Secrets handling & at-rest protection
**Partially shipped.** Provider keys can live in the **OS keychain** (`src-tauri/src/security/keychain.rs`; `enable_keychain` defaults to true) instead of plaintext `settings.json`, and keys can also be supplied via env vars. Still open: a "redact secrets" view for logs/tool outputs.

### 20) Safer defaults for MCP servers
- Default-deny tool execution unless explicitly enabled.
- Add a “dangerous tools” label and require extra confirmation.
- Consider a per-server sandbox policy (allowed roots, allowed network destinations) where feasible.

---

## Notes / quick wins inventory
- `src-tauri/src/lib.rs`
  - ~~remove duplicate handler registration (`delete_system_prompt` listed twice)~~ ✅ done
  - consider extracting provider selection into a helper to reduce repeated logic (also used by title generation and pruning)
- `src-tauri/src/memory/*`
  - ~~index maintenance (delete/rebuild) is the biggest correctness gap~~ ✅ done (delete-by-message-id + rebuild action)
- `src-tauri/src/mcp/*`
  - ~~add permission checks before `tools/call`~~ ✅ done (approval gate); read/write tool classification still open
  - improve SSE robustness
