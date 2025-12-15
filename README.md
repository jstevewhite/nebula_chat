# Nebula

**Nebula** is a native, high-performance **Intelligent Orchestrator** designed to bridge user intent, personal memory, and external tools. Built with [Tauri](https://tauri.app/) and Rust, it serves as a privacy-first AI client that doesn't just chat—it *remembers* and *acts*.

## Key Features

### 🧠 Deep Context & Memory
Unlike standard chat clients, Nebula features a **Memory Sidecar** (powered by SQLite + Tantivy) that runs locally.
- **Episodic Memory**: Automatically indexes and stores conversation history.
- **Semantic Search**: Fast, full-text retrieval of past context ($<100$ms).
- **Smart Pruning**: Automatically summarizes and compresses old conversations to maintain context without blowing up token budgets.
- **Index Maintenance**: Built-in tools to rebuild and optimize the search index if data gets out of sync.
- **Deletion Support**: Full support for deleting messages and conversations from both the database and search index.

### 🔌 Native MCP Host
Nebula implements the **[Model Context Protocol (MCP)](https://modelcontextprotocol.io/)**, treating external tools as first-class citizens.
- **Connect Tools**: Give the AI access to your file system, Git repositories, or browser automation via MCP servers.
- **Security First**: Granular "Human-in-the-loop" permissions. You verify every tool execution (Allow/Deny/Always Allow).
- **Permission Policy**: Configure allowlists and denylists per server to restrict tool access automatically. server-side enforcement ensures tools never run without approval.
- **Audit Logging**: Every tool execution is logged to a local SQLite database for transparency, including full inputs and outputs.
- **Token Safety**: Large tool outputs are automatically truncated for the LLM to save tokens, but you can always view the full output in the UI.
- **Tool Management**: Visual panel to view, search, and granularly enable/disable individual tools or entire servers.
- **System Message Management**: Create, edit, and switch between reusable system prompts to guide the AI's behavior.
- **Transport Support**: Supports `stdio` and `sse` (Streamable HTTP) transports for flexible integration.

### ⚡ Performance & Privacy
- **Local-First**: Your memory stays on your machine.
- **Provider Agnostic**: Unified interface for **OpenAI**, **Anthropic**, and **Ollama** (for fully local privacy).
- **Model Management**: Easily toggle visibility for models, bulk enable/disable providers, **filter large model lists (typedown search)**, and **set a default model** for new chats.
- **Smart Chat Management**: Auto-titles conversations, allows renaming/deleting, and intelligently handles chat deletion without unnecessary empty chats.
- **Rust Core**: Heavy lifting is done in optimized Rust for maximum speed.

### 💬 Rich Chat Interface
- **Markdown Rendering**: Full GitHub Flavored Markdown support with tables, lists, and headers.
- **Code Highlighting**: Syntax highlighting for code blocks with "Copy Code" functionality, styled to match the `vscDarkPlus` theme.
- **Interactive Messages**: Edit, copy, delete, and regenerate messages on the fly.
- **Aesthetic UI**: Polished, editor-like typography with custom scrollbars and clean spacing.
- **File Attachments**: Support for generic file attachments (Text, Code, Images) with multi-modal LLM support.
- **Generation Settings**: Real-time control over **Temperature**, **Top P**, and **Streaming** directly from the chat interface.
- **Stop Generation**: Instantly abort long-running LLM responses with a dedicated stop button.
- **Memory Panel**: Real-time transparency into what memory context is being injected into your conversation.
- **Intelligent Context Assembly**: Configure a secondary "Strategist" model to filter and summarize memories before they reach the main chat.

## Architecture

Nebula acts as an orchestrator between the user, the memory sidecar, and external MCP servers.

```mermaid
graph TD
    User[User] --> UI[Frontend (Tauri/React)]
    UI --> Core[Nebula Core (Rust)]
    
    subgraph Core "Nebula Core"
        Planner[Context Planner]
        Manager[MCP Manager]
        Provider[LLM Provider]
    end

    Core -->|Search & Store| Memory[(Memory Sidecar\nSQLite + Tantivy)]
    Core -->|Execute| Tools[External MCP Servers\n(File System, Git, etc.)]
    Core -->|Inference| API[LLM APIs\n(OpenAI / Anthropic / Local)]

    Tools -- MCP Protocol --> Manager
    Memory -- Context --> Planner
```

## Getting Started

### Prerequisites
- **Rust**: Latest stable version.
- **Node.js**: v18+.
- **System Dependencies**: Standard Tauri prerequisites (e.g., `libwebkit2gtk-4.0-dev`, `build-essential`, etc. on Linux).

### Installation
1. **Clone the repository:**
   ```bash
   git clone https://github.com/jstevewhite/nebula_chat.git
   cd nebula_chat
   ```
2. **Install dependencies:**
   ```bash
   npm ci
   ```
   If you don’t have a lockfile yet, you can use `npm install` instead.
3. **Run the development application (Vite + Tauri):**
   ```bash
   npm run tauri dev
   ```
   **NB** On Linux + NVIDIA, you may need:
   `WEBKIT_DISABLE_DMABUF_RENDERER=1 npm run tauri dev`

## Configuration
Nebula uses a `settings.json` file stored in your system’s app config directory. You can configure providers, models, memory, and MCP servers in the UI (recommended) or by editing the file.

Provider credentials can be set in `settings.json` *or* via environment variables (e.g. `NEBULA_OPENAI_KEY`, `NEBULA_ANTHROPIC_KEY`).

### Valid `settings.json` Example
```json
{
  "providers": {
    "openai": {
      "enabled": true,
      "api_key": "sk-...",
      "provider_type": "OpenAI"
    },
    "ollama": {
      "enabled": true,
      "base_url": "http://localhost:11434",
      "provider_type": "Ollama"
    }
  },
  "memory_enabled": true,
  "context_turns": 0,
  "mcp_servers": {
    "filesystem": {
      "type": "Stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user/workspace"],
      "enabled": true,
      "env": {
        "SOME_FLAG": "1"
      }
    },
    "remote-server": {
      "type": "Sse",
      "url": "http://localhost:3000/mcp",
      "enabled": true
    }
  }
}
```

Notes:
- `memory_enabled`: master toggle for long‑term memory retrieval/injection.
- `context_turns`: number of recent conversation turns (user/assistant) that are explicitly included during context assembly (0 disables the feature).
- `mcp_servers.*.env`: optional environment variables for `Stdio` MCP servers (same format as `process.env`).

## Tech Stack

- **Frontend**: React 19, Vite, TailwindCSS 4, Lucide Icons
- **Backend**: Rust (Tauri v2)
- **Data**: SQLite, Tantivy (Search), embedded in-process
- **Communication**: Tauri IPC, JSON-RPC (for MCP)

## License

[MIT](LICENSE)
