# Nebula – Feature gaps & improvement ideas

This document is a quick review of the current `nebula_chat` codebase (React/Tauri UI + Rust backend). It focuses on high-impact missing features, correctness gaps, and architectural improvements.

## Recently shipped

- **Slash commands** (`feature/slash_commands`): client-side command palette accessible by typing `/` in the composer or by pressing `/` while focused in the conversation list sidebar. Ships with `/help`, `/remember`, `/search`, `/facts`, `/skills`, `/skill <slug>`, and `/clear`. Spec: [`docs/superpowers/specs/slash-commands.md`](superpowers/specs/slash-commands.md).

## Top priorities (high impact / low-medium effort)

### 1) Fix obvious correctness issues
- **Duplicate command registration**: `delete_system_prompt` appears twice in the `invoke_handler` list (`src-tauri/src/lib.rs`).
  - Why it matters: easy-to-miss wiring bug; makes the handler list look untrustworthy.
- **Anthropic streaming drops tool calls and multimodal content**: `AnthropicProvider::stream` ignores tool calls and attachments (only streams text). (`src-tauri/src/llm/anthropic.rs`)
  - Why it matters: streaming behavior diverges from non-streaming behavior; tool workflows won’t work reliably.
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
The Tantivy index is **append-only** today:
- messages are added on save (`TantivyIndex::add_document`) but never deleted on message deletion or conversation deletion.
- pruning deletes old messages in SQLite but leaves old documents in Tantivy.

Why it matters:
- search can return stale/deleted content
- memory injection can resurrect “deleted” facts

Recommended improvements:
- Store `message_id` (and ideally `created_at`) in Tantivy documents so you can delete/update specific records.
- On delete/prune, delete corresponding documents from Tantivy (or rebuild index periodically).
- Add a “Rebuild memory index” maintenance action in Settings for recovery.

## MCP & tools (feature gaps)

### 4) Permissions/guardrails actually enforced server-side
There is an `auto_approve` flag in `McpServerConfig` (`src-tauri/src/mcp/config.rs`), but tool calls are executed directly via `execute_tool` without permission checks.

Suggested feature set:
- **Backend gatekeeper** for tool execution:
  - Enforce per-server and per-tool allowlists/denylists.
  - Classify tools (read-only vs write) and require explicit consent for write operations.
  - Persist approvals in settings.
- **Tool call audit log**:
  - Store tool call + args + result metadata in SQLite (even if the full result is large, keep a hash + truncated preview).
  - Display a per-conversation “tool timeline” in the UI.

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
OpenAI streaming has a fairly detailed reassembly path for tool calls; Anthropic streaming is text-only.

Improvements:
- Stream tool calls for Anthropic (or disable tool calling when streaming is enabled for Anthropic until implemented).
- Add a provider capability matrix (“supports tools”, “supports streaming tools”, “supports multimodal”).

### 9) More generation controls
The UI exposes temperature/top_p/stream.

Common missing controls:
- max output tokens
- stop sequences
- presence/frequency penalties (OpenAI-compatible)
- seed (when supported)
- response format / JSON mode (when supported)

## UX improvements (high-value product features)

### 10) Conversation export/import
- Export a conversation as Markdown/JSON (including tool calls/results).
- Import from common formats (OpenAI chat logs, other clients).

### 11) Global search across chats
Today: conversation list search filters by title only.

Add:
- full-text search across message bodies (SQLite or Tantivy), filtered by conversation.
- show snippets and jump-to-message.

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
Settings store API keys in plaintext JSON.

Improvements:
- Store provider keys in the OS keychain (or encrypt at rest with a user-provided passphrase).
- Add a “redact secrets” view for logs/tool outputs.

### 20) Safer defaults for MCP servers
- Default-deny tool execution unless explicitly enabled.
- Add a “dangerous tools” label and require extra confirmation.
- Consider a per-server sandbox policy (allowed roots, allowed network destinations) where feasible.

---

## Notes / quick wins inventory
- `src-tauri/src/lib.rs`
  - remove duplicate handler registration (`delete_system_prompt` listed twice)
  - consider extracting provider selection into a helper to reduce repeated logic (also used by title generation and pruning)
- `src-tauri/src/memory/*`
  - index maintenance (delete/rebuild) is the biggest correctness gap
- `src-tauri/src/mcp/*`
  - add permission checks before `tools/call`
  - improve SSE robustness
