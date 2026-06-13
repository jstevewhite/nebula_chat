# WARP.md

This file provides guidance to WARP (warp.dev) when working with code in this repository.
``

## Commands

- Install dependencies (uses package-lock):
  - npm ci
- Run the desktop app in dev (spawns Vite and Tauri):
  - npm run tauri dev
- Web-only dev server (Vite):
  - npm run dev
- Preview the production build (served locally):
  - npm run preview
- Build web assets only (type-checks via tsc first):
  - npm run build
- Type-check only (without building bundles):
  - npx tsc --noEmit
- Package the desktop app (all platforms configured by Tauri):
  - npm run tauri build
- Rust backend only (within Tauri sub-crate):
  - cd src-tauri && cargo build
- Rust linting:
  - cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings
- Rust formatting (fail CI locally if not formatted):
  - cd src-tauri && cargo fmt -- --check
  - To auto-fix: cd src-tauri && cargo fmt
- Rust tests (no Rust tests exist today, but supported by Cargo):
  - cd src-tauri && cargo test
  - Single test example:
    - cd src-tauri && cargo test name_substring
- Frontend tests: none are configured (no Jest/Vitest in package.json).

## High-level architecture

This is a Tauri desktop application with a React + TypeScript/Vite frontend and a Rust backend. The backend exposes Tauri commands that orchestrate LLM providers, long‑term memory, and MCP tool execution. The frontend uses @tauri-apps/api to invoke those commands.

- Frontend (React + Vite)
  - Location: src/
  - Entry: src/main.tsx → src/App.tsx
  - Major UI components (only those that establish the frontend↔backend contract):
    - components/ConversationList.tsx: lists, selects, renames, and deletes conversations via Tauri commands (list_conversations, rename_conversation, delete_conversation).
    - components/ChatInterface.tsx: chat UI; loads conversation history (get_chat_history), sends messages (send_message), displays and optionally executes tool calls (execute_tool), triggers auto‑titling (generate_title), and allows pinning a default model (set_default_model).
    - components/SettingsPage.tsx and components/ProvidersSettings.tsx: reads and writes provider/server configuration (get_settings, save_settings); adds/edits MCP servers (add_mcp_server, edit_mcp_server); optionally fetches provider models (fetch_models).
  - Dev server: Vite on port 1420 (see vite.config.ts). When running Tauri dev, Vite is started automatically per src-tauri/tauri.conf.json.

- Tauri/Rust backend
  - Entrypoint: src-tauri/src/main.rs → tauri_appnebula_lib::run() in src-tauri/src/lib.rs
  - App state (lib.rs):
    - McpManager: manages external MCP servers and aggregates their tools.
    - Librarian: long‑term memory, backed by SQLite and Tantivy for full‑text search.
  - Exposed Tauri commands (lib.rs):
    - Conversations: list_conversations, create_conversation, delete_conversation, rename_conversation, get_chat_history, delete_message, generate_title.
    - Chat: send_message (saves user/tool messages, retrieves/searches memory, injects context, prunes with ContextManager, calls selected LLM provider, saves assistant message, and schedules background pruning/summarization).
    - Tools/MCP: get_mcp_servers, execute_tool (server__toolname), add_mcp_server, edit_mcp_server.
    - Settings and models: get_settings, save_settings, fetch_models, set_default_model (persists selected default in Settings.default_model).
  - LLM providers (src-tauri/src/llm):
    - Common interface: llm/provider.rs defines Message, ToolDefinition, and the LlmProvider trait (chat).
    - Implementations: openai.rs, anthropic.rs, ollama.rs; supports OpenAI‑compatible endpoints.
    - Context management: llm/context.rs prunes message history by token budget using llm/tokenizer.rs.
  - Memory subsystem (src-tauri/src/memory):
    - sqlite_manager.rs: conversations/messages tables; stores full messages including tool_calls and tool_call_id; supports listing, renaming, deletion, and history retrieval.
    - tantivy_index.rs: full‑text index of message content; used to retrieve relevant snippets as context.
    - librarian.rs: orchestrates SQLite + Tantivy; provides save/search/list primitives used by Tauri commands and background pruning/summarization.
    - docs/ (memory3): on-disk markdown doc store chunked + embedded for hybrid cosine/BM25 recall, plus a knowledge-graph facts layer and deterministic per-turn auto-injection (this replaced the older "Strategist" planner/synthesizer). Embeddings via local fastembed (ONNX bge-small-en-v1.5) or a remote OpenAI-compatible endpoint.
  - MCP integration (src-tauri/src/mcp):
    - client.rs: JSON‑RPC over stdio, SSE, and StreamableHttp transports to external MCP servers; async request/response handling.
    - manager.rs: starts servers from Settings, performs initialize handshake, lists available tools, routes tool calls, and supports runtime restarts on config edits.
    - config.rs: Settings schema (providers, models, mcp_servers), ProviderType enum, migration/overrides.

## Configuration and data

- Settings file: JSON stored in the app config directory (path resolved via Tauri’s app.path().app_config_dir()). The schema is in src-tauri/src/mcp/config.rs (Settings, ProviderConfig, ModelConfig, McpServerConfig). Environment overrides are supported for provider keys (e.g., NEBULA_OPENAI_KEY, NEBULA_ANTHROPIC_KEY) during load_migrated.
- Default model pinning: Settings.default_model is used by the UI; set via the set_default_model Tauri command.
- Memory storage: SQLite database (nebula.db) plus Tantivy index under a memory/ directory inside the app config directory. Librarian initializes and manages both.
- Tool naming: MCP tools are namespaced as server__toolname to guarantee uniqueness across servers.

## Notes for future changes

- Adding a new LLM provider: implement LlmProvider in a new file under src-tauri/src/llm/, then extend the provider selection in lib.rs where ProviderType is matched for chat and pruning flows.
- Extending memory features: prefer changes in librarian.rs; keep SQLite (persistence) and Tantivy (search) concerns separated.
- Frontend↔backend contract: keep Tauri command names/signatures stable; the UI relies on invoke("…") with the exact names listed above.
