# Nebula

**Nebula** is a native, high-performance **Intelligent Orchestrator** designed to bridge user intent, personal memory, and external tools. Built with [Tauri](https://tauri.app/) and Rust, it serves as a privacy-first AI client that doesn't just chat—it *remembers* and *acts*.

## Key Features

### 🧠 Deep Context & Memory
Unlike standard chat clients, Nebula features a **Memory Sidecar** (powered by SQLite + Tantivy) that runs locally.
- **Episodic Memory**: Automatically indexes and stores conversation history.
- **Semantic Search**: Fast, full-text retrieval of past context ($<100$ms).
- **Smart Pruning**: Automatically summarizes and compresses old conversations to maintain context without blowing up token budgets.

### 🔌 Native MCP Host
Nebula implements the **[Model Context Protocol (MCP)](https://modelcontextprotocol.io/)**, treating external tools as first-class citizens.
- **Connect Tools**: Give the AI access to your file system, Git repositories, or browser automation via MCP servers.
- **Security First**: Granular "Human-in-the-loop" permissions. You verify every tool execution (Allow/Deny/Always Allow).
- **Tool Management**: Visual panel to view, search, and granularly enable/disable individual tools or entire servers.
- **System Message Management**: Create, edit, and switch between reusable system prompts to guide the AI's behavior.
- **Transport Support**: Supports `stdio` and `sse` (Streamable HTTP) transports for flexible integration.

### ⚡ Performance & Privacy
- **Local-First**: Your memory stays on your machine.
- **Provider Agnostic**: Unified interface for **OpenAI**, **Anthropic**, and **Ollama** (for fully local privacy).
- **Model Management**: Easily toggle visibility for models, bulk enable/disable providers, and **set a default model** for new chats.
- **Smart Chat Management**: Auto-titles conversations, allows renaming/deleting, and intelligently handles chat deletion without unnecessary empty chats.
- **Rust Core**: Heavy lifting is done in optimized Rust for maximum speed.

### 💬 Rich Chat Interface
- **Markdown Rendering**: Full GitHub Flavored Markdown support with tables, lists, and headers.
- **Code Highlighting**: Syntax highlighting for code blocks with "Copy Code" functionality, styled to match the `vscDarkPlus` theme.
- **Interactive Messages**: Edit, copy, delete, and regenerate messages on the fly.
- **Aesthetic UI**: Polished, editor-like typography with custom scrollbars and clean spacing.

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
   git clone https://github.com/your-username/nebula.git
   cd nebula
   ```

2. **Install frontend dependencies:**
   ```bash
   npm install
   ```

3. **Run the development application:**
   ```bash
   npm run tauri dev
   ```

## Configuration

Nebula uses a `settings.json` file located in your system's application data directory. You can configure providers and MCP servers directly via the UI or by editing this file.

### Valid `settings.json` Example:

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
  "mcp_servers": {
    "filesystem": {
      "type": "Stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user/workspace"],
      "enabled": true
    },
    "remote-server": {
      "type": "Sse",
      "url": "http://localhost:3000/mcp",
      "enabled": true
    }
  }
}
```

## Tech Stack

- **Frontend**: React 19, Vite, TailwindCSS 4, Lucide Icons
- **Backend**: Rust (Tauri v2)
- **Data**: SQLite, Tantivy (Search), embedded in-process
- **Communication**: Tauri IPC, JSON-RPC (for MCP)

## License

[MIT](LICENSE)
