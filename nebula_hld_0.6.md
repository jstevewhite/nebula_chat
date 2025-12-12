# High-Level Design Document: “Nebula” Native AI Client
**Version:** 0.6 (MCP First-Class Integration)
**Date:** December 11, 2025
**Architecture:** Rust Core (MCP Host) + Tauri v2 + Memory Sidecar

---

# 1. Executive Summary

“Nebula” is a native, high-performance personal AI client acting as a bridge between user intent, personal memory, and external tools.

**Core Pillars:**
1.  **Native MCP Host:** Out-of-the-box support for the Model Context Protocol. Nebula can connect to local or remote MCP servers to give the AI agency (file system access, browser automation, git integration) from Day 1.
2.  **Best-in-class UX:** Fast streaming, multi-model support, and a dedicated UI for managing tool permissions.
3.  **Local, Privacy-Preserving Memory:** Facts and episodic memories stored entirely offline (SQLite + Tantivy).
4.  **Provider-Agnostic:** Unified interface for OpenAI, Anthropic, and local GGUF models.

**Architecture Shift:** Nebula is now defined as an **Intelligent Orchestrator**. It manages the conversation context by weaving together:
* User Input
* Retrieved Memory (Internal Sidecar)
* Active Tools (External MCP Servers)

---

# 2. Goals and Non-Goals

## 2.1 Goals

1.  **Full MCP Host Implementation**
    * Support `stdio` and `SSE` transport for MCP servers.
    * Seamlessly translate MCP Tools into LLM function schemas (OpenAI/Anthropic formats).
    * UI for "Human-in-the-loop" tool approval (Allow/Deny/Always Allow).

2.  **High-Quality Chat UX**
    * Streaming completions with UI indicators for "Thinking" vs "Tool Execution."

3.  **Reliable Personal Memory**
    * Durable storage of facts and summaries, isolated from the main thread.

4.  **Security Sandbox**
    * Users must explicitly authorize MCP servers.
    * Granular permissions per server.

## 2.2 Non-Goals

* **Nebula as an MCP Server (MVP)**: In v1, Nebula is the *Host* (consumer). Exposing Nebula's memory *as* an MCP server to other apps is a v2 goal.
* **Arbitrary Code Execution**: Nebula executes *defined tools* via MCP, not arbitrary Python/Bash scripts unless an MCP server specifically provides that capability (and the user permits it).

---

# 3. System Overview

## 3.1 High-Level Architecture

The architecture now includes the **MCP Manager** as a peer to the Context Planner.

```text
+-----------------------------------------------------------+
|                      Frontend (Tauri UI)                  |
|  - Chat Interface                                         |
|  - Tool Approval Modals                                   |
|  - MCP Server Config                                      |
+-----------+-----------------------+-----------------------+
            |                       |
            v                       v
+-----------------------------------------------------------+
|                     Nebula Core (Rust)                    |
|                                                           |
| +-------------------+      +----------------------------+ |
| | Conversation Eng. |<---->|       MCP Manager          | |
| +-------------------+      |  - Server Registry         | |
|           ^                |  - Client Implementation   | |
|           |                |  - Tool Aggregator         | |
| +-------------------+      +-------------+--------------+ |
| |  ContextPlanner   |                    | (stdio/SSE)    |
| +---------+---------+                    v                |
|           |                 +--------------------------+  |
|           |                 | External MCP Servers     |  |
|           v                 | (Filesystem, Git, etc.)  |  |
| +-------------------+       +--------------------------+  |
| |  ProviderManager  |                                     |
| +-------------------+                                     |
+-----------------------------------------------------------+
            ^
            | (async IPC)
+-----------+-----------+
|    Memory Sidecar     |
| (Librarian/Strategist)|
+-----------------------+
```

## 3.2 Execution Flow (The "Tool Loop")

1.  **Input:** User sends message.
2.  **Planning:** `ContextPlanner` gathers:
    * Relevant Memories (from Sidecar).
    * Available Tools (from `McpManager`).
3.  **Inference:** LLM receives system prompt + user msg + tool definitions.
4.  **Decision:** LLM decides to call a tool (e.g., `fs.read_file`).
5.  **Interception:** `McpManager` intercepts the tool call.
    * *Check:* Does user need to approve?
    * *Action:* Execute tool via specific MCP Server.
6.  **Loop:** Tool result injected back into context. LLM continues or finalizes.

---

# 4. Core Modules

## 4.1 MCP Manager (NEW)

This is the central hub for agency.

### 4.1.1 Components
* **`ServerRegistry`**: Manages configuration for connected servers (cmd args, env vars).
* **`McpClient`**: A Rust implementation of the MCP protocol (client-side). Handles JSON-RPC 2.0.
* **`ToolCache`**: Caches available tools on startup to avoid round-trips on every keystroke.

### 4.1.2 Permissions Logic
Every connected server has a permission policy stored in `settings.json`:
* `auto_approve`: Boolean (dangerous, strictly controlled).
* `whitelisted_tools`: List of tools that don't require confirmation (e.g., `list_directory` might be safe, `delete_file` is not).

---

## 4.2 Provider Abstraction (Updated)

The `LlmProvider` trait is updated to handle tool calling natively.

```rust
struct ToolDefinition {
    name: String,
    description: String,
    schema: serde_json::Value, // JSON Schema
}

struct ToolCall {
    id: String,
    function_name: String,
    arguments: serde_json::Value,
}

trait LlmProvider {
    // ... existing methods ...

    // NEW: Capability flags
    fn supports_tool_calling(&self) -> bool;

    // Updated stream signature to handle ToolCall chunks
    fn stream_completion(
        &self,
        req: CompletionRequest,
        tools: Vec<ToolDefinition> // Inject tools here
    ) -> (Pin<Box<dyn Stream<Item = StreamChunk>>>, CancellationHandle);
}
```

**Normalization:** The `ProviderManager` is responsible for converting generic MCP tool definitions into the specific format required by OpenAI (`tools` array), Anthropic (`system` prompt XML or API), or Ollama.

---

# 5. Memory Sidecar

*Unchanged from v0.5, but with a specific note:*

**Interaction with MCP:**
The Memory Sidecar remains a dedicated internal module for now to ensure sub-100ms latency for retrieval. It is **not** accessed via MCP in the MVP.
*Reasoning:* MCP JSON-RPC over stdio introduces overhead. The internal memory needs to be "hot" and extremely fast.

---

# 6. Storage & Data Model

## 6.1 Settings (Updated for MCP)

New configuration structures required:

```json
{
  "mcp_servers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/Users/me/Desktop"],
      "env": {},
      "auto_approve": false
    },
    "git": {
      "command": "docker",
      "args": ["run", "-i", "mcp/git"],
      "auto_approve": false
    }
  }
}
```

---

# 7. Frontend & UX (Crucial for MCP)

## 7.1 Server Management
A settings page to:
* Add/Remove MCP Servers.
* View status (Connected/Error).
* View list of tools provided by each server.

## 7.2 Tool Invocation UI
When the model wants to use a tool:
1.  **Pending State:** UI shows "Model wants to execute `read_file` on `./src/main.rs`".
2.  **Action Buttons:** [Allow] [Deny] [Always Allow for this Server].
3.  **Result State:** Once executed, the tool output is collapsed by default (expandable for debugging).

---

# 8. Security & Guardrails

## 8.1 The "Human-in-the-Loop" Firewall
Since MCP allows the AI to interact with the OS, strict security is paramount.

1.  **Default Deny:** All tool executions default to requiring user confirmation unless explicitly whitelisted.
2.  **Environment Isolation:** MCP servers are child processes. Nebula does not inject its own env vars into them unless specified.
3.  **Output Sanitization:** Large tool outputs (e.g., `cat huge_log.txt`) must be truncated by the `McpManager` before being fed back to the LLM to prevent context window overflows (DoS).

---

# 9. Implementation Plan (Revised)

## Phase 1: Core Chat + MCP Foundation (The MVP)
* **Objective:** Functional Chat with agency.
* **Tasks:**
    * Implement `LlmProvider` and basic Chat UI.
    * Implement `McpManager` (Rust).
    * Create "Add Server" UI.
    * Implement "Tool Approval" flow.
    * Verify integration with standard `filesystem` and `memory` MCP servers.

## Phase 2: Memory Sidecar
* **Objective:** Long-term memory.
* **Tasks:**
    * Implement Librarian/Strategist threads.
    * Integrate SQLite/Tantivy.
    * Connect ContextPlanner to Memory.

## Phase 3: Advanced Features
* **Objective:** Optimization.
* **Tasks:**
    * Tokenizer abstraction.
    * Advanced RRF Retrieval.
    * Memory Pruning.
