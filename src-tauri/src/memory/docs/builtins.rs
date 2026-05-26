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
    (
        "nebula-conversations",
        "Nebula: using conversations",
        NEBULA_CONVERSATIONS,
        &["nebula-architecture", "nebula-memory-usage"],
    ),
    (
        "nebula-providers",
        "Nebula: LLM providers",
        NEBULA_PROVIDERS,
        &["nebula-architecture"],
    ),
    (
        "nebula-memory-usage",
        "Nebula: using memory",
        NEBULA_MEMORY_USAGE,
        &["nebula-memory", "nebula-conversations"],
    ),
    (
        "nebula-troubleshooting",
        "Nebula: troubleshooting",
        NEBULA_TROUBLESHOOTING,
        &["nebula-mcp", "nebula-memory-usage", "nebula-skills"],
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

const NEBULA_CONVERSATIONS: &str = r#"Conversations live in the left sidebar (ConversationList). Each
conversation is independent — its own message history, system prompt,
model, and memory context. Switching does not reset nebula; it just
changes which thread you're viewing.

## Sidebar actions

- **New conversation:** plus button at the top of the sidebar. Creates
  a fresh thread using the current default system prompt + model (see
  "Setting defaults" below).
- **Switch:** click a conversation in the list.
- **Rename:** double-click the title. No length rules.
- **Set icon:** click the small icon next to the title and pick from
  the emoji picker — purely cosmetic, helps visual scanning.
- **Delete:** hover icon → trash. Confirmation prompt warns once;
  deletion is permanent (messages and Librarian index entries removed).

## Search

The search field at the top of the sidebar does two things at once:

- **Title search:** filters the sidebar list by conversation title
  (case-insensitive substring).
- **"Match in Chat" content search:** surfaces message-content matches
  across *all* conversations. Click a match to jump to that
  conversation.

## Import / export

- **Export:** Download icon in the chat top bar. Saves the active
  conversation as JSON — self-contained, includes messages, role / tool-
  call structure, attachment references.
- **Import:** import button in the sidebar header. Reads a JSON file
  produced by Export and creates a new conversation entry.

## Setting defaults for new conversations

The Pin icon in the chat input area sets the *current* conversation's
system prompt + model as the default for new conversations created
afterwards. Doesn't retroactively change existing conversations.

## Side panels

- **Tools panel** (right): tool list + per-server / per-tool approval
  toggles. See `nebula-mcp`.
- **Memory panel** (right): three tabs — Context, Facts, Docs. See
  `nebula-memory-usage`.
- **Context inspection** (eye icon, left tool strip): when enabled, the
  full context payload is shown before each send with a Cancel / OK
  dialog so you can sanity-check what the model will receive. Setting
  key: `context_inspection_enabled`.

## Attachments

Image-only at present. Supported types: PNG, JPG / JPEG, WebP, GIF.
Multiple attachments per message. Drop files into the chat or use the
paperclip icon. Only multi-modal models (GPT-4V, Claude 3+) actually
process the images; other models receive only the text portion.
"#;

const NEBULA_PROVIDERS: &str = r#"LLM providers are managed in Settings → Providers. Each provider has a
type, an enabled flag, an optional base URL, an API key, and a list of
models you've imported or configured.

## Supported provider types

- **OpenAI** — OpenAI API (GPT-4, GPT-5, etc.). Needs an API key.
- **Anthropic** — Anthropic API (Claude models). Needs an API key.
- **Ollama** — local Ollama instance. No key; requires base URL
  pointing at the Ollama server (typically `http://localhost:11434`).
- **OpenAI-compatible** — anything that speaks the OpenAI API shape:
  LMStudio, vLLM, OpenRouter, Together, Groq, custom proxies. Set base
  URL + API key (key may be ignored by some local servers).

## Adding a provider

1. Open Settings → Providers and pick the provider type.
2. Toggle Enabled.
3. Paste API key (remote providers) or set base URL (local).
4. Add models via the Import flow (fetches the provider's model list
   and lets you pick) or by hand (id + display name + context window).
5. Save.

## API key storage

When `enable_keychain` is on (default), API keys are stored in the OS
keychain, not in `settings.json`. The settings file holds the provider
configuration shell; keys are read from the keychain on load. Disable
keychain only for headless or restricted environments — the key falls
back to `settings.json` in plain text.

## Environment variable overrides

Two env vars override the corresponding API keys on every load, useful
for CI or shared installs:

- `NEBULA_OPENAI_KEY` — overrides the OpenAI provider's `api_key`.
- `NEBULA_ANTHROPIC_KEY` — overrides Anthropic's.

Other providers (Ollama, OpenAI-compatible) use settings only.

## Selecting the active model

The model dropdown lives in the chat input area. It lists models from
every enabled provider, deduplicated. Switch any time — the next
message uses the new model. The current model + system prompt can be
pinned as the default for new conversations via the Pin icon (see
`nebula-conversations`).

## Per-model knobs

Each model entry in a provider config carries optional fields:

- `context_window`, `max_tokens` — budget caps used by token accounting.
- `prompt_cost`, `completion_cost` — for cost estimation.
- `supports_reasoning_effort` (OpenAI o-series),
  `supports_thinking_mode` (DeepSeek-style),
  `supports_extended_thinking` (Anthropic Claude 4) — capability flags
  that surface model-specific reasoning knobs in the UI.

The Import flow auto-detects most of these for OpenRouter-style
catalogues. For hand-added models, set them yourself.
"#;

const NEBULA_MEMORY_USAGE: &str = r#"Nebula remembers across conversations through three mechanisms.
Different mechanisms have different opt-in profiles.

## What's automatic vs. opt-in

- **Always automatic:** every chat message is saved to Librarian
  (SQLite + Tantivy). You can search past conversations from the
  sidebar (see `nebula-conversations`). This is your raw chat history;
  nothing is extracted from it automatically.
- **Opt-in: knowledge graph (KG) facts.** Durable subject–predicate–
  object triples about you or entities you care about. Three ways to
  add facts:
  1. **Per-message brain icon.** Click the brain icon on any assistant
     message. Nebula extracts facts from *that message* via the
     `FactExtractor`. Returns "Saved N facts from message".
  2. **`/remember <text>` chat command.** Type
     `/remember <assertion>` in the chat input. Nebula runs fact
     extraction over the supplied text only — no LLM call against the
     full conversation. Returns "Remembered N facts from /remember:
     <text>". May return 0 if the text contains no durable triples
     (e.g. `/remember the test passed` — too transient).
  3. **Natural-language requests.** Saying "remember that I prefer
     Python for ML" lets the model itself decide to call the
     `memory_fact_remember` tool. Less predictable, more flexible.
- **Opt-in: long-term docs.** Markdown documents under
  `memory/docs/<id>.md`. The model creates and edits them via the
  `memory_doc_*` tools when you ask it to. Doc bodies get chunked,
  embedded, and made searchable. See `nebula-memory` for the
  architecture.

## Memory panel

The Memory panel on the right has three tabs:

- **Context** — every memory fragment that got injected into the
  system prompt for *this turn*. Inspect what the model actually sees
  from memory before answering.
- **Facts** — KG triples. User-profile facts at the top; below, look
  up facts about a specific entity by name.
- **Docs** — long-term memory documents. Click to read the body. Built-
  in nebula docs are tagged `built-in`.

## Settings worth knowing (Settings → Memory)

- **Memory enabled** — master switch. Off disables KG fact injection
  and doc auto-injection. Chat history persistence is not affected.
- **Auto-inject memory docs** — whether the most-relevant doc + top KG
  facts are prefixed to the system prompt automatically each turn. On
  by default. Off means the model has to call `memory_doc_*` tools
  explicitly to retrieve anything.
- **Token budget for auto-injection** — hard cap on the injected
  block. Default 4000. Lower it if context is tight; raise it for big
  contexts with more memory in scope.
- **Recall score floor** — minimum fusion score (cosine + BM25) for a
  doc to qualify for auto-injection. Default 0.20. Raise if injection
  pulls in irrelevant docs; lower if relevant ones aren't surfacing.
- **Fact extraction policy** — `explicit` (default; only the three
  methods above), `session_end` (also extracts when you switch
  conversations), or `off` (no session-end pass; explicit still works).
- **Auto-approve memory tools** — when on, the model's `memory_*` tool
  calls skip the per-call approval prompt. On by default since these
  only touch local audit-visible markdown + the local KG.

## Why `/remember <X>` sometimes returns 0 facts

The FactExtractor looks for durable, atemporal subject–predicate–object
assertions. "The test passed" lacks a stable subject + persistent
predicate — it's an event, not a fact about the world. Try
`/remember the user prefers Python over Rust for ML prototyping` —
that mines a real triple.
"#;

const NEBULA_TROUBLESHOOTING: &str = r#"When something doesn't work in nebula, the fastest path is usually
invoking the right diagnostic skill rather than poking around in
settings. This doc is a symptom → skill routing table.

## Symptom → skill

| Symptom                                                    | Skill to invoke                |
|------------------------------------------------------------|--------------------------------|
| MCP server won't connect, red dot in settings              | `mcp-diagnostics` (Layer 1–2)  |
| Tools don't appear in the model's available list           | `mcp-diagnostics` (Layer 3)    |
| Model never calls a tool you expected it to                | `mcp-diagnostics` (Layer 4)    |
| Tool call fails, times out, returns garbage                | `mcp-diagnostics` (Layer 4)    |
| Approval prompts loop despite "always allow"               | `mcp-diagnostics` (Layer 5)    |
| User reports a bug in their own code                       | `debug-help`                   |
| User wants a code walkthrough                              | `explain-code`                 |
| User wants a diff reviewed                                 | `code-review`                  |
| User wants a README written or audited                     | `readme-pro`                   |
| User is setting up MCP servers for the first time          | `nebula-setup`                 |
| User wants to design a new skill                           | `skill-architect`              |
| User asks "what have we covered?"                          | `summarize-conversation`       |

If you are the assistant reading this: call the matching skill via
`use_skill(slug)` *before* attempting the work yourself. The skill body
overrides your default approach for that task.

## Things to check before invoking a skill

A few issues have very common causes worth eyeballing first:

- **Tools not appearing.** Check Settings → MCP servers — is the server
  connected (green)? Are its tools listed *and* enabled in the right-
  side Tools panel? Disabled tools are filtered out before the model
  sees them (see `nebula-mcp` for the disable mechanism).
- **Approval prompt loops.** Check the Tools panel for that server.
  Server-level shield icon = auto-approve every tool from this server.
  Per-tool shield = auto-approve one specific tool. Server-level locks
  out per-tool when on.
- **Memory not surfacing what you expect.** Open the Memory panel →
  Context tab. That's exactly what got injected this turn. If the
  relevant doc isn't there, raise the auto-inject budget or lower the
  recall score floor (see `nebula-memory-usage`).
- **Slow first response.** Local models (Ollama, LMStudio) often need
  warmup time after a model swap. Hosted models can also see latency
  spikes during provider incidents.
- **/remember returned 0 facts.** Not a bug — the text didn't contain a
  durable triple. See `nebula-memory-usage`.

## When no skill fits

For everything else, follow the `debug-help` pattern even if the
problem isn't strictly a bug: get a precise statement of expected vs.
observed behaviour, check the layer below (Tauri logs, dev console,
MCP server output), isolate before guessing.
"#;
