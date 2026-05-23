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

The backend is organized into three main modules:

- **`llm/`** - Multi-provider LLM abstraction
  - `provider.rs`: Common `LlmProvider` trait and types (`Message`, `Attachment`, `ToolCall`)
  - `openai.rs`, `anthropic.rs`, `ollama.rs`: Provider-specific implementations
  - `context.rs`: `ContextManager` handles conversation state and message flow
  - `context_assembler.rs`: Secondary "Strategist" model that filters/summarizes memories before injection
  - `tokenizer.rs`: Token counting utilities (tiktoken-rs)

- **`mcp/`** - Model Context Protocol implementation (MCP Host)
  - `manager.rs`: `McpManager` orchestrates multiple MCP server connections
  - `client.rs`: `McpClient` implements JSON-RPC 2.0 over stdio/SSE transports
  - `config.rs`: Settings structures (`Settings`, `McpServerConfig`, `ProviderType`)

- **`memory/`** - Local memory sidecar (SQLite + Tantivy)
  - `librarian.rs`: `Librarian` coordinates SQLite + Tantivy for conversation storage/retrieval
  - `sqlite_manager.rs`: Persistent storage for messages, conversations, and metadata
  - `tantivy_index.rs`: Full-text search index for semantic memory retrieval

**2. React Frontend (`src/`)**

- **`App.tsx`**: Main application shell, routing between Chat and Settings
- **`components/`**:
  - `ChatInterface.tsx`: Main chat UI, message rendering, streaming state
  - `ConversationList.tsx`: Sidebar for conversation management
  - `SettingsPage.tsx`: Unified settings page with MCP server management
  - `ToolsPanel.tsx`: Tool execution approval UI ("Allow/Deny/Always Allow")
  - `PromptsSettings.tsx`: System message (prompt) management
  - `ProvidersSettings.tsx`: LLM provider configuration
  - `MemoryPanel.tsx`: Real-time display of memory context injected into conversation

### Data Flow

1. **User Message** → `ChatInterface.tsx` → Tauri IPC → `lib.rs` commands
2. **Context Assembly**:
   - `ContextManager` retrieves conversation history from `Librarian`
   - Optional: `ContextAssembler` uses a secondary model to filter/summarize memories
   - `McpManager` aggregates available tools from connected MCP servers
3. **LLM Inference**: Provider-specific streaming via `LlmProvider` trait
4. **Tool Execution**: If model requests tool call → `McpManager` → user approval (if needed) → execute via `McpClient`
5. **Memory Storage**: All interactions saved via `Librarian` (SQLite + Tantivy indexing)

### MCP Integration

Nebula acts as an **MCP Host**:
- Supports `stdio` (subprocess) and `sse` (HTTP) transports
- MCP servers configured in `settings.json` (`mcp_servers` section)
- Tools from all connected servers are merged and presented to the LLM
- Human-in-the-loop approval for tool execution (configurable per server)

Example MCP server config:
```json
"mcp_servers": {
  "filesystem": {
    "type": "Stdio",
    "command": "npx",
    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"],
    "enabled": true
  }
}
```

### Settings Management

- Location: Platform-specific config dir (`~/.config/nebula/settings.json` on Linux)
- Loaded via `Settings::load_migrated()` with automatic schema migrations
- UI updates settings in real-time; Rust backend reloads on demand
- **Critical**: The settings page has had bugs with MCP server initialization hanging. When modifying settings logic, ensure proper error handling and timeouts for MCP server connections.

### Memory System

The **Memory Sidecar** runs in-process (not via MCP for performance):
- **Librarian**: Dual storage (SQLite for structured data, Tantivy for full-text search)
- Retrieval latency target: <100ms
- Supports semantic search across conversation history
- Automatic conversation summarization for context window management

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
