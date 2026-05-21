//! Tool definitions for the six `memory_*` LLM-facing tools. The descriptors
//! returned here are appended to the tool list passed to the model, alongside
//! MCP tools and the existing built-in `update_tasks`. Execution lives in
//! `lib.rs::execute_tool`.

use crate::llm::provider::ToolDefinition;
use serde_json::json;

pub const TOOL_REMEMBER: &str = "memory_remember";
pub const TOOL_FETCH: &str = "memory_fetch";
pub const TOOL_EDIT: &str = "memory_edit";
pub const TOOL_FORGET: &str = "memory_forget";
pub const TOOL_RECALL: &str = "memory_recall";
pub const TOOL_LINK_CONTEXT: &str = "memory_link_context";
pub const TOOL_REMEMBER_FACT: &str = "memory_remember_fact";

pub const ALL_NAMES: &[&str] = &[
    TOOL_REMEMBER,
    TOOL_FETCH,
    TOOL_EDIT,
    TOOL_FORGET,
    TOOL_RECALL,
    TOOL_LINK_CONTEXT,
    TOOL_REMEMBER_FACT,
];

pub fn is_memory_tool(name: &str) -> bool {
    ALL_NAMES.contains(&name)
}

/// Build the full set of tool definitions exposed to the LLM.
pub fn build_all() -> Vec<ToolDefinition> {
    // memory_remember_fact is intentionally listed *before* memory_remember so
    // the model considers the atomic-fact path first when the user mentions a
    // preference / identity / environment detail. The doc tools follow for
    // narrative content.
    vec![
        remember_fact_tool(),
        remember_tool(),
        edit_tool(),
        fetch_tool(),
        forget_tool(),
        recall_tool(),
        link_context_tool(),
    ]
}

fn remember_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_REMEMBER.to_string(),
        description:
            "Create a new long-form memory DOCUMENT (markdown file on disk). \
             Use this ONLY for narrative, multi-paragraph content that needs prose to make sense — \
             project briefs, architecture decision records, workflow notes, README-style overviews, \
             working agreements with multiple clauses. \
             \n\nDO NOT use this tool for atomic facts about the user or an entity \
             (name, role, preferences, OS, editor, language choice, tool choice, environment details). \
             Those belong in the knowledge graph — use `memory_remember_fact` instead. \
             A doc with title \"User Profile\" containing a bulleted list of preferences is the WRONG shape; \
             each bullet should be a separate `memory_remember_fact` call. \
             \n\nGood examples for memory_remember:\n\
             - id: \"project-nebula\", title: \"Nebula architecture notes\" — multi-section overview of the system.\n\
             - id: \"adr-2026-05-tantivy\", title: \"ADR: use Tantivy for BM25\" — decision record with context and tradeoffs.\n\
             - id: \"workflow-release\", title: \"Release workflow\" — step-by-step checklist with prose.\n\n\
             BAD examples (use memory_remember_fact instead):\n\
             - id: \"user-profile\" with a list of preferences.\n\
             - id: \"environment\" with OS / editor / shell facts.\n\
             - any single-line factual statement about the user.\n\n\
             Errors if a doc with this id already exists; use memory_edit to update."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["id", "title", "content"],
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Human-readable slug, [a-z0-9][a-z0-9-]{0,63}. Used as the filename and for [[wikilinks]]."
                },
                "title": {"type": "string", "description": "Human-readable title (any characters)."},
                "content": {"type": "string", "description": "Markdown body. Use [[other-id]] to link to other docs."},
                "tags": {"type": "array", "items": {"type": "string"}, "description": "Free-form tags."},
                "links": {"type": "array", "items": {"type": "string"}, "description": "Other doc IDs this doc links to."}
            }
        }),
    }
}

fn fetch_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_FETCH.to_string(),
        description:
            "Retrieve a memory document by id. Returns the full body, tags, and outbound links. \
             Use the returned `updated_at` as the `expected_updated_at` token for a subsequent memory_edit."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": {"type": "string"}
            }
        }),
    }
}

fn edit_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_EDIT.to_string(),
        description:
            "Edit an existing memory document. Exactly one of `replace` or `append` must be provided. \
             `replace.find` must match exactly once in the body. Pass `expected_updated_at` (from a recent memory_fetch) \
             to fail loudly if the doc changed since you read it."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": {"type": "string"},
                "expected_updated_at": {
                    "type": "string",
                    "description": "Optimistic concurrency token. The updated_at returned by the last memory_fetch."
                },
                "replace": {
                    "type": "object",
                    "required": ["find", "with"],
                    "properties": {
                        "find": {"type": "string"},
                        "with": {"type": "string"}
                    }
                },
                "append": {"type": "string"}
            }
        }),
    }
}

fn forget_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_FORGET.to_string(),
        description:
            "Delete a memory document and all its chunks. This is a destructive action; \
             prefer memory_edit for narrowing or correcting a doc."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": {"type": "string"}
            }
        }),
    }
}

fn recall_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_RECALL.to_string(),
        description:
            "Semantically search the memory documents. Returns up to k chunks (default 3), \
             each tagged with its parent doc_id. Use memory_fetch to read the full doc for any chunk."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {"type": "string"},
                "k": {"type": "integer", "minimum": 1, "maximum": 10, "default": 3},
                "tags": {"type": "array", "items": {"type": "string"}}
            }
        }),
    }
}

fn remember_fact_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_REMEMBER_FACT.to_string(),
        description:
            "PREFERRED tool for atomic facts about the user or any entity. Writes a single \
             (subject, predicate, object) triple into the structured knowledge graph. \
             \n\nALWAYS use this — NOT `memory_remember` — for any short, durable factual statement, \
             including but not limited to:\n\
             - The user's name, role, location, pronouns, employer.\n\
             - Stable preferences (\"user prefers dark mode\", \"user uses vim\").\n\
             - Environment details (\"user runs Linux\", \"user has an RTX 4090\", \"user uses zsh\").\n\
             - Tech stack and tool choices (\"user writes Rust\", \"project uses Postgres\").\n\
             - Concrete decisions (\"we picked Tantivy over Meilisearch\").\n\
             - Project metadata (\"nebula_chat is built with Tauri\").\n\n\
             Call this tool ONCE PER FACT. If the user tells you three preferences, make three calls. \
             Do not batch facts into a markdown doc. Do not create a \"user-profile\" doc — \
             user-profile-shaped knowledge lives in the KG and is rendered automatically into \
             every system prompt.\n\n\
             Examples:\n\
             - User: \"I prefer dark mode\" → memory_remember_fact(subject=\"user\", predicate=\"prefers\", object=\"dark mode\")\n\
             - User: \"I'm on Arch Linux\" → memory_remember_fact(subject=\"user\", predicate=\"runs_os\", object=\"arch linux\")\n\
             - User: \"My main editor is helix\" → memory_remember_fact(subject=\"user\", predicate=\"main_editor\", object=\"helix\")\n\
             - User: \"nebula_chat uses Tauri\" → memory_remember_fact(subject=\"nebula_chat\", predicate=\"built_with\", object=\"tauri\", object_kind=\"entity\")\n\n\
             Use `memory_remember` only for genuinely narrative content (project briefs, ADRs, \
             multi-section notes) that needs prose to make sense."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["subject", "predicate", "object"],
            "properties": {
                "subject": {
                    "type": "string",
                    "description": "Normalised subject key, lowercase snake_case. Use \"user\" for facts about the human."
                },
                "predicate": {
                    "type": "string",
                    "description": "Normalised predicate key, lowercase snake_case (e.g. \"prefers\", \"uses\", \"main_editor\")."
                },
                "object": {
                    "type": "string",
                    "description": "Free-form literal value, or a snake_case entity key when object_kind=\"entity\"."
                },
                "object_kind": {
                    "type": "string",
                    "enum": ["literal", "entity"],
                    "description": "\"entity\" if the object is itself a graph node you'd traverse to; \"literal\" otherwise. Defaults to \"literal\"."
                },
                "confidence": {
                    "type": "number",
                    "minimum": 0,
                    "maximum": 1,
                    "description": "Subjective confidence in the fact (default 0.9)."
                }
            }
        }),
    }
}

fn link_context_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_LINK_CONTEXT.to_string(),
        description:
            "Breadth-first traversal of the doc link graph starting at the given id. \
             Returns a list of reachable doc ids with their depth. Full bodies are not included; \
             use memory_fetch to retrieve any doc."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": {"type": "string"},
                "depth": {"type": "integer", "minimum": 1, "maximum": 3, "default": 2},
                "max_docs": {"type": "integer", "minimum": 1, "maximum": 20, "default": 10}
            }
        }),
    }
}
