# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

### Frontend (React + Vite)
```bash
npm install              # Install dependencies
npm run dev              # Run Vite dev server (frontend only)
npm run build            # Build frontend (TypeScript + Vite)
```

### Tauri Application
```bash
npm run tauri dev        # Run full Tauri app in dev mode (Rust + React)
npm run tauri build      # Build production app
```

**Linux Compatibility:**
The application automatically sets required environment variables on Linux to fix:
- IBus input/typing issues (`IBUS_ENABLE_SYNC_MODE` is set; `GTK_IM_MODULE` is commented out as it caused focus deadlocks/hangs)
- NVIDIA GPU rendering problems (`WEBKIT_DISABLE_DMABUF_RENDERER`)

These fixes are built into the binary (see `lib.rs`), so DEB/RPM/AppImage packages work out of the box.

### Rust Backend
```bash
cd src-tauri
cargo build              # Build Rust backend
cargo test               # Run Rust tests
cargo clippy             # Lint Rust code
```

## Architecture Overview

Nebula is an **Intelligent Orchestrator** built with Tauri v2 (Rust backend + React frontend) that bridges user intent, personal memory, and external tools through the Model Context Protocol (MCP).

### Core Components

**1. Rust Backend (`src-tauri/src/`)**

The backend is organized into these modules:

- **`llm/`** - Multi-provider LLM abstraction
  - `provider.rs`: Common `LlmProvider` trait and types (`Message`, `Attachment`, `ToolCall`)
  - `openai.rs`, `anthropic.rs`, `ollama.rs`: Provider-specific implementations (OpenAI also covers OpenAI-compatible endpoints like LM Studio / OpenRouter)
  - `factory.rs`: Builds the right provider from a `ProviderType`
  - `capabilities.rs`: Per-model capability flags (tools, streaming tools, multimodal, reasoning/thinking)
  - `context.rs`: `ContextManager` handles conversation state and message flow
  - `compactor.rs`: Tool-call-aware summarization/compaction of older turns
  - `tool_shaping.rs`: Normalizes tool schemas to each provider's format
  - `think_tag.rs`: Parses/streams reasoning ("thinking") content
  - `tokenizer.rs`: Token counting utilities (tiktoken-rs)

- **`mcp/`** - Model Context Protocol implementation (MCP Host)
  - `manager.rs`: `McpManager` orchestrates multiple MCP server connections
  - `client.rs`: `McpClient` implements JSON-RPC 2.0 over stdio, SSE, and StreamableHttp transports
  - `config.rs`: Settings structures (`Settings`, `McpServerConfig`, `McpTransport`, `ModelConfig`, `ProviderType`) and the `load_migrated` schema migrations
  - `builtin_prompts.rs`: Built-in MCP prompts shipped with the app

- **`memory/`** - Local memory sidecar (see Memory System below)
  - `librarian.rs`: `Librarian` coordinates SQLite + Tantivy for chat-message storage/retrieval
  - `sqlite_manager.rs`: Persistent storage for messages, conversations, and metadata
  - `tantivy_index.rs`: BM25 keyword full-text index over messages (not semantic)
  - `extraction.rs`: Fact extraction from messages
  - `audit_logger.rs`: Records every tool execution in SQLite
  - `docs/`: the **memory3** doc store — markdown docs chunked + embedded for hybrid (cosine + BM25) recall, a knowledge-graph facts layer, and deterministic per-turn auto-injection (`store.rs`, `retrieval.rs`, `chunker.rs`, `inject.rs`, `links.rs`, `tools.rs`, `watcher.rs`, `builtins.rs`, and `embedding/` with local fastembed + remote providers)

- **`skills/`** - User-editable instruction bundles surfaced via the `use_skill` / `list_skills` tools (`store.rs`, `builtins.rs`, `tools.rs`, `watcher.rs`, `api.rs`)
- **`tasks/`** - Per-conversation task checklists backing the `update_tasks` tool and the Tasks panel
- **`security/`** - OS keychain integration (`keychain.rs`) for API-key storage

**2. React Frontend (`src/`)**

- **`App.tsx`**: Main application shell, routing between Chat and Settings
- **`components/`**:
  - `ChatInterface.tsx`: Main chat UI, message rendering, streaming state; hosts the in-chat tool-approval prompt ("Allow/Deny/Always Allow") and the slash-command palette (`src/utils/chatCommands.ts`)
  - `ConversationList.tsx`: Sidebar for conversation management
  - `SettingsPage.tsx`: Unified settings page with MCP server management
  - `ToolsPanel.tsx`: Settings UI for per-server / per-tool auto-approve policy and tool enable/disable (not the runtime approval prompt — that lives in `ChatInterface.tsx`)
  - `ProvidersSettings.tsx`: LLM provider configuration
  - `PromptsSettings.tsx`: System message (prompt) management
  - `SkillsSettings.tsx`: Skill management UI
  - `ThemeSelector.tsx`: Theme picker
  - `RightRail.tsx` / `TasksPanel.tsx` / `MemoryPanel.tsx`: Collapsible right rail — Tools/Memory tabs plus the auto-appearing Tasks checklist and the live memory-injection view

### Data Flow

1. **User Message** → `ChatInterface.tsx` → Tauri IPC → `lib.rs` commands
2. **Context Assembly**:
   - `ContextManager` retrieves conversation history from `Librarian`
   - The memory3 `DocStore` deterministically auto-injects the top-recall doc body plus top knowledge-graph facts (token-budgeted, score-floored; no secondary LLM call)
   - `McpManager` aggregates available tools from connected MCP servers
3. **LLM Inference**: Provider-specific streaming via `LlmProvider` trait
4. **Tool Execution**: If model requests tool call → `McpManager` → user approval (if needed) → execute via `McpClient`
5. **Memory Storage**: Chat messages are persisted via `Librarian` (SQLite + Tantivy); long-term memory docs and KG facts are persisted separately by the memory3 `DocStore`

### MCP Integration

Nebula acts as an **MCP Host**:
- Supports three transports: `Stdio` (subprocess), `Sse` (server-sent events), and `StreamableHttp` (streamable HTTP). See [`McpTransport`](src-tauri/src/mcp/config.rs) for the exact shapes.
- MCP servers configured in `settings.json` (`mcp_servers` section)
- Tools from all connected servers are merged and presented to the LLM
- Human-in-the-loop approval for tool execution; `auto_approve` and `auto_approve_tools` on each server config customize the policy

**Enable/disable mechanism.** There is no per-server `enabled` flag on `McpServerConfig`. The settings UI disables a "server" by adding all of its tool names to the top-level `Settings.disabled_tools` list (see `toggle_tool_list` in `lib.rs`). The server process stays connected; its tools are filtered out before being exposed to the LLM. To remove a server entirely, delete its entry from `mcp_servers`.

Example MCP server configs:
```json
"mcp_servers": {
  "filesystem": {
    "type": "Stdio",
    "command": "npx",
    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"],
    "env": {}
  },
  "jina": {
    "type": "StreamableHttp",
    "url": "https://mcp.jina.ai/v1",
    "headers": { "Authorization": "Bearer ${JINA_API_KEY}" }
  }
}
```

### Settings Management

- Location: Platform-specific config dir derived from the Tauri bundle identifier (`~/.config/com.stwhite.nebula/settings.json` on Linux)
- Loaded via `Settings::load_migrated()` with automatic schema migrations
- UI updates settings in real-time; Rust backend reloads on demand
- **Critical**: The settings page has had bugs with MCP server initialization hanging. When modifying settings logic, ensure proper error handling and timeouts for MCP server connections.

### Memory System

The **Memory Sidecar** runs in-process (not via MCP for performance) and has two layers:
- **Chat history (`Librarian`)**: SQLite for structured data + a Tantivy **BM25 keyword** full-text index over messages. Retrieval latency target: <100ms.
- **memory3 doc store (`memory/docs/`)**: markdown docs chunked and embedded — local `fastembed` (ONNX `bge-small-en-v1.5`, 384-dim) or a remote OpenAI-compatible endpoint — for **semantic** recall, fused with BM25. A knowledge-graph facts layer stores `(subject, predicate, object)` triples. Each turn a deterministic recall + KG-fact-prose block is auto-injected (token-budgeted, score-floored); this **replaced the older "Strategist" planner/synthesizer**.
- Older turns are compacted/summarized via `llm/compactor.rs` while keeping the most recent N messages raw.

## Key Design Patterns

1. **Provider Abstraction**: All LLM providers implement `LlmProvider` trait with normalized `Message` format. Tool calling is translated to provider-specific schemas (OpenAI `tools`, Anthropic `system` blocks).

2. **Async Everywhere**: Rust backend uses Tokio runtime. Tauri commands are `async fn`. Frontend uses React 19 with async state management.

3. **Tool Security**: Default-deny for tool execution. Users must approve (or pre-whitelist) tools. MCP servers run as isolated child processes.

4. **Context Window Management**: Token counting via `tokenizer.rs` ensures messages fit within model limits. Old conversations can be pruned/summarized.

## Common Gotchas

- **Streaming State**: Chat streaming involves multiple state transitions (thinking → tool_use → responding). Frontend must handle partial chunks correctly.
- **MCP Server Lifecycle**: Servers are spawned on app startup. If a server fails to initialize, it shouldn't block the entire app. Recent bugfixes improved robustness around settings page hangs.
- **React 19 Strictness**: Development mode double-renders. Ensure Tauri event listeners are properly cleaned up to avoid duplicate subscriptions.
- **File Attachments**: Use `Attachment` type in messages. Multi-modal models (GPT-4V, Claude 3) can process images.

## Testing Notes

- **Frontend**: No test suite currently. Manual testing via `npm run tauri dev`.
- **Backend**: Run `cargo test` in `src-tauri/`. Tests cover core logic in `mcp/`, `memory/`, and `llm/` modules.
- **Integration**: Test MCP integration with reference servers like `@modelcontextprotocol/server-filesystem`.
