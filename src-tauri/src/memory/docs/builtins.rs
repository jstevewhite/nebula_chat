//! Built-in memory docs shipped with the app. Materialized to
//! `<docs_dir>/<id>.md` at startup *before* `startup_reconcile` so they
//! flow through the normal ingest path (SQLite row + Tantivy + embeddings).
//!
//! Policy: always overwrite when the bundled body differs from disk
//! (parallel to `skills::builtins`). The materialiser uses a body-equality
//! check to avoid re-embedding on every launch.
//!
//! Marked with the `built-in` tag in frontmatter. `DocStore::edit` and
//! `DocStore::forget` reject any doc carrying this tag so the LLM (or a
//! direct UI action) cannot mutate them. To customise, the user / model
//! should `memory_doc_remember` a fresh doc with a different ID.

/// (id, title, body, links). IDs must be stable kebab-case slugs and
/// are referenced by `memory_doc_fetch` and the link graph.
pub const ALL: &[(&str, &str, &str, &[&str])] = &[
    (
        "nebula-architecture",
        "Nebula architecture",
        NEBULA_ARCHITECTURE,
        &["nebula-mcp", "nebula-memory", "nebula-skills"],
    ),
    (
        "nebula-mcp",
        "Nebula MCP integration",
        NEBULA_MCP,
        &["nebula-architecture", "nebula-skills"],
    ),
    (
        "nebula-memory",
        "Nebula memory system",
        NEBULA_MEMORY,
        &["nebula-architecture", "nebula-skills"],
    ),
    (
        "nebula-skills",
        "Nebula skills",
        NEBULA_SKILLS,
        &["nebula-architecture", "nebula-memory"],
    ),
];

/// The tag every built-in doc carries. Used by `DocStore::edit` /
/// `DocStore::forget` to refuse mutations and by the UI to render a badge.
pub const BUILT_IN_TAG: &str = "built-in";

/// All tags applied to built-in docs. `built-in` is the load-bearing one;
/// `nebula` is a discoverability convenience.
pub const TAGS: &[&str] = &[BUILT_IN_TAG, "nebula"];

const NEBULA_ARCHITECTURE: &str = r#"Nebula is a desktop AI assistant built with Tauri v2 — Rust backend, React 19
frontend. It positions itself as an "Intelligent Orchestrator": a chat UI
that bridges the user's intent, a persistent memory system, and external
tools surfaced via MCP (Model Context Protocol).

## Stack

- **Backend:** Rust (`src-tauri/src/`), Tokio async runtime, Tauri v2 IPC.
- **Frontend:** React 19 + Vite + Tailwind, single-window desktop app.
- **Storage:** SQLite for structured data, Tantivy for full-text search,
  on-disk markdown for documents, OS keychain for secrets.

## Three primary subsystems

- **LLM** (`src-tauri/src/llm/`) — provider abstraction. Implementations
  for OpenAI, Anthropic, Ollama, OpenAI-compatible (LMStudio etc.). All
  providers implement the `LlmProvider` trait. `ContextManager` assembles
  the message list; `ContextAssembler` is an optional "strategist" model
  that filters memories before injection.
- **MCP** (`src-tauri/src/mcp/`) — the MCP host. `McpManager` orchestrates
  connected servers; `McpClient` speaks JSON-RPC 2.0 over Stdio, SSE, or
  Streamable HTTP. See the `nebula-mcp` doc for details.
- **Memory** (`src-tauri/src/memory/`) — Librarian (SQLite + Tantivy for
  chat history), DocStore (markdown + embeddings + vector cache for
  long-term docs), knowledge graph for facts. See the `nebula-memory` doc.

## Request lifecycle

1. User message lands in `ChatInterface.tsx` → Tauri IPC.
2. `ContextManager` retrieves conversation history from `Librarian`.
3. Memory injection: if `memory_auto_inject_docs` is on, the most
   relevant doc + top KG facts are prefixed to the system context.
4. `McpManager` merges tools from every connected MCP server into the
   request.
5. Selected provider streams the response. Tool calls intercept →
   `McpManager` → user approval (configurable) → execute.
6. Final message saved via `Librarian` (SQLite write + Tantivy index).

## Settings

`settings.json` lives in the platform config dir (`~/.config/nebula/` on
Linux). It's the single source of truth for providers, MCP servers,
system prompts, memory tuning, and UI preferences. The UI mutates
settings in real time; the backend re-loads on demand. Schema:
`src-tauri/src/mcp/config.rs`.

## Built-ins lifecycle

Skills, system prompts, and these memory docs all follow the same
"shipped-in-binary" pattern: bundled content is materialized on each
startup, edits to built-ins are overwritten. Users customise by cloning
(skills/prompts) or by creating new entries with different IDs (docs).
The `built_in: true` flag (skills/prompts) or `built-in` tag (docs)
marks app-shipped content in the UI.
"#;

const NEBULA_MCP: &str = r#"Nebula acts as an MCP host: it spawns / connects to MCP servers, merges
their tool lists, and presents them to the LLM. Tool calls from the model
route back through nebula's approval system before execution.

## Three transports

`McpTransport` (see `src-tauri/src/mcp/config.rs`):

- **Stdio** — Local subprocess. Required: `command`, `args`. Optional
  `env` map. Server speaks JSON-RPC 2.0 over stdin/stdout.
- **Sse** — Remote server-sent events. Required: `url`. Optional
  `headers` map (e.g. `Authorization: Bearer ...`).
- **StreamableHttp** — Remote streamable HTTP. Same shape as SSE. Newer
  MCP remote transport spec; Jina's hosted MCP uses this.

## settings.json shape

Servers live under `mcp_servers` keyed by name:

```json
"mcp_servers": {
  "jina": {
    "type": "StreamableHttp",
    "url": "https://mcp.jina.ai/v1",
    "headers": { "Authorization": "Bearer <KEY>" }
  }
}
```

Each entry also takes optional `auto_approve: bool`,
`auto_approve_tools: [String]`, and
`permissions: { allowlist: [], denylist: [] }`. There is no per-server
`enabled` flag in `McpServerConfig` — see "Enable/disable mechanism".

## Tools panel and approval

Approval and tool-visibility controls live in the right-side Tools panel,
grouped by server. Four toggles per server:

- **Server-level auto-approve** (shield icon by the server name) — every
  tool from that server skips the per-call approval prompt. Sets
  `auto_approve: true`.
- **Per-tool auto-approve** (shield icon on a tool, hover-revealed) —
  scoped to one tool. Adds the tool to `auto_approve_tools`. Locked when
  server-level is on.
- **Server-level enable/disable** (checkbox icon, "Enable All" /
  "Disable All") — toggles every tool from the server in one click.
- **Per-tool enable/disable** (per-tool checkbox) — toggles one tool.

## Enable/disable mechanism

There is no `enabled` field on `McpServerConfig`. The UI "disable server"
toggle adds all of that server's tool names to the top-level
`Settings.disabled_tools` list. The MCP server process stays connected;
its tools are filtered out before being exposed to the LLM. To remove a
server entirely, delete its entry from `mcp_servers`.

## Recommended starter stack

The built-in `nebula-setup` skill walks through Jina (web fetch/search),
Context7 (live library docs), Serper (Google search), and either Serena
or DesktopCommander (code/filesystem). Jina is the bootstrap
recommendation because it's a remote MCP with a free tier — no install
required, just sign up and paste the URL + key into the Add Server
dialog.

## When things break

The built-in `mcp-diagnostics` skill diagnoses by layer (settings →
transport → tool advertisement → invocation → approval). Invoke it for
any "tools aren't appearing", "server won't connect", or "approval keeps
firing" issue.
"#;

const NEBULA_MEMORY: &str = r#"Nebula's memory has three layers, all local and inspectable.

## Layer 1: Librarian — chat history

`src-tauri/src/memory/librarian.rs`. Combines SQLite (`sqlite_manager.rs`)
for structured chat storage and Tantivy (`tantivy_index.rs`) for full-text
search across past conversations. Every user/assistant message is
persisted. Retrieval target: under 100 ms.

When `ContextManager` assembles a message list, it can pull
historically-relevant turns from the Librarian via similarity search.
Recent uncompressed messages are controlled by
`context_uncompressed_msg_count` (default 20); older turns can be
summarised to stay within the context window.

## Layer 2: DocStore — long-term memory docs

`src-tauri/src/memory/docs/`. Markdown files on disk under
`~/.config/nebula/memory/docs/`, indexed by both Tantivy (BM25) and a
local embedding model (default: `bge-small-en-v1.5` via fastembed → 384-
dim vectors). Hybrid retrieval fuses cosine + BM25 scores.

Each doc has YAML frontmatter (`id`, `title`, `tags`, `links`, timestamps)
and a markdown body. Body is chunked, embedded, and indexed. Cross-
document links use `[[other-doc-id]]` syntax and feed a link graph for
context expansion via `memory_doc_link_context`.

**Six tools the model can call directly** (see `memory/docs/tools.rs`):

- `memory_doc_remember` — create a new doc.
- `memory_doc_fetch` — get a doc by ID.
- `memory_doc_edit` — replace or append a section (optimistic-concurrency
  via `expected_updated_at`).
- `memory_doc_forget` — delete a doc.
- `memory_doc_recall` — hybrid search across the corpus.
- `memory_doc_link_context` — walk the link graph from a starting doc.

**Auto-injection.** When `memory_auto_inject_docs` is on (default), each
user turn prefixes the system context with the most relevant doc plus a
few KG facts. Budget: `memory_auto_inject_token_budget` (default 4000).
Score floor: `memory_recall_score_floor` (default 0.20).

**Built-in docs.** Nebula ships four built-in docs covering its own
architecture, MCP integration, memory system, and skills. They're
materialized into the docs dir on each startup (overwritten when bundled
content changes), tagged `built-in`, and protected from `edit`/`forget`
tool calls.

## Layer 3: Knowledge graph — facts

Subject–predicate–object triples (entity or literal). Stored alongside
Librarian's SQLite. Extracted via the `FactExtractor` either on demand
(`/remember` chat command, "Save as fact" message action,
`memory_fact_remember` tool) or in a session-end pass if
`fact_extraction_policy = "session_end"`. Default policy is `"explicit"`
— no implicit extraction.

Top KG facts about the user (and any entities mentioned in the current
conversation) are injected alongside the auto-injected doc when memory
injection is on.

## Persistence summary

| What                          | Where                                          |
|-------------------------------|------------------------------------------------|
| Chat history                  | SQLite + Tantivy index under `memory/`         |
| Long-term docs (markdown)     | `memory/docs/<id>.md`                          |
| Doc Tantivy index             | `memory/docs_index/`                           |
| Doc chunk embeddings          | SQLite blob column + in-memory vector cache    |
| KG facts                      | SQLite tables in the same DB as chat history   |
| User-configurable settings    | `settings.json`                                |

## Reconciliation

`DocStore::startup_reconcile` runs on every launch: scans the docs dir,
compares to SQLite, ingests new/changed files, deletes orphans. This
makes external edits (vim, git pull) survive cleanly — the FS watcher
also catches live edits.

If the embedding model changes (e.g. user switches from fastembed to a
remote provider), the vector cache is truncated and every doc is re-
embedded on next startup. Progress is reported to the frontend via the
`memory:reembed-progress` IPC event.
"#;

const NEBULA_SKILLS: &str = r##"Skills are reusable, named bundles of imperative instructions the LLM can
pull into context via the `use_skill` tool. Conceptually parallel to
memory docs but with different semantics: docs hold *factual content*;
skills hold *how to approach a task*.

## How skills surface to the model

At system-prompt assembly time, nebula injects a compact "Available
skills" block listing each skill's slug + description. The model decides
when to invoke `use_skill(slug)`; the tool returns the skill body, which
the model treats as authoritative guidance for the current request.

The default Nebula system prompt biases the model toward invoking skills
— the cost of an unnecessary call is small, the cost of a missed one is
a worse answer.

## File layout

Skills live as markdown files at `~/.config/nebula/skills/`. Built-ins
are at `~/.config/nebula/skills/built-ins/<slug>.md`. Each file has YAML
frontmatter:

```
---
name: Code Review
description: Use when the user asks for a code review...
built_in: true
---
[body — system-prompt-shaped guidance]
```

Slugs must match `[a-z0-9][a-z0-9-]{0,63}`.

## Built-in vs. user skills

**Built-ins** ship in the nebula binary and are re-materialized to disk
on every launch. Any edits to files under `built-ins/` are overwritten.
Currently shipped:

- `code-review` — diff review with per-tier hunt lists
  (correctness → security → maintainability → style)
- `summarize-conversation` — structured chat recap
- `debug-help` — systematic bug isolation
- `skill-architect` — design new skills via structured interview
- `mcp-diagnostics` — diagnose MCP server / tool issues by layer
- `explain-code` — walk through unfamiliar code
- `readme-pro` — generate or audit READMEs
- `nebula-setup` — bootstrap a recommended MCP stack

**User skills** live at the top level (`~/.config/nebula/skills/<slug>.md`)
and persist across launches.

A user skill with the same slug as a built-in **shadows the built-in** —
`SkillStore::get(slug)` returns the user version first. This is the
override mechanism for advanced customisation.

## Customising a built-in

The Settings → Skills UI exposes a "Clone to edit" button when a built-
in is selected. Clone creates a new user skill at slug `<original>-copy`
(or `-copy-2`, `-copy-3` if taken) with the same content, ready to edit.
Users who want override semantics instead of dual-listing can rename
the clone to match the original slug.

## Watcher

A debounced FS watcher catches external edits to skill files (vim, git
pull, etc.) and emits `skills-updated` so the Settings page refreshes
without a manual reload.

## Adding a built-in (for nebula developers)

Built-in skill content lives inline in `src-tauri/src/skills/builtins.rs`
as `r#"..."#` raw strings. Register a new slug in the `ALL` array,
recompile, ship. On next launch, every user gets the new skill
automatically — no migration needed.
"##;
