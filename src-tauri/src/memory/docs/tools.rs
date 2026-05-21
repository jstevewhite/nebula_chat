//! Tool definitions for the LLM-facing memory subsystem.
//!
//! There are two clearly-separated subsystems, named consistently:
//!
//! - `memory_doc_*`  — markdown documents on disk (user-editable, narrative).
//! - `memory_fact_*` — atomic (subject, predicate, object) triples in the KG.
//!
//! The model should never need to choose between "remember as doc" vs
//! "remember as fact" by name alone — pick the prefix that matches the
//! information's shape, then the verb.

use crate::llm::provider::ToolDefinition;
use serde_json::json;

// Doc subsystem.
pub const TOOL_DOC_REMEMBER: &str = "memory_doc_remember";
pub const TOOL_DOC_FETCH: &str = "memory_doc_fetch";
pub const TOOL_DOC_EDIT: &str = "memory_doc_edit";
pub const TOOL_DOC_FORGET: &str = "memory_doc_forget";
pub const TOOL_DOC_RECALL: &str = "memory_doc_recall";
pub const TOOL_DOC_LINK_CONTEXT: &str = "memory_doc_link_context";

// Fact subsystem.
pub const TOOL_FACT_REMEMBER: &str = "memory_fact_remember";
pub const TOOL_FACT_RECALL: &str = "memory_fact_recall";
pub const TOOL_FACT_FORGET: &str = "memory_fact_forget";

pub const ALL_NAMES: &[&str] = &[
    TOOL_FACT_REMEMBER,
    TOOL_FACT_RECALL,
    TOOL_FACT_FORGET,
    TOOL_DOC_REMEMBER,
    TOOL_DOC_FETCH,
    TOOL_DOC_EDIT,
    TOOL_DOC_FORGET,
    TOOL_DOC_RECALL,
    TOOL_DOC_LINK_CONTEXT,
];

pub fn is_memory_tool(name: &str) -> bool {
    ALL_NAMES.contains(&name)
}

/// Build the full set of tool definitions exposed to the LLM. Facts are listed
/// first so the model considers the atomic path before the narrative path when
/// a statement could plausibly go either way.
pub fn build_all() -> Vec<ToolDefinition> {
    vec![
        fact_remember_tool(),
        fact_recall_tool(),
        fact_forget_tool(),
        doc_remember_tool(),
        doc_fetch_tool(),
        doc_edit_tool(),
        doc_forget_tool(),
        doc_recall_tool(),
        doc_link_context_tool(),
    ]
}

// ---------- FACT tools (knowledge graph) ----------

fn fact_remember_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_FACT_REMEMBER.to_string(),
        description:
            "FACT WRITE — store a single atomic (subject, predicate, object) triple in the \
             knowledge graph. PREFER this over memory_doc_remember for any short, durable \
             factual statement about the user or an entity.\n\n\
             Use for: identity (name, role, employer), preferences, environment details, \
             tech-stack and tool choices, decisions, project metadata.\n\n\
             Call ONCE PER FACT — three preferences = three calls. Re-calling with the same \
             (subject, predicate, object, object_kind) upserts in place, so this doubles as \
             the update path.\n\n\
             Examples:\n\
             - \"I prefer dark mode\" → subject=\"user\" predicate=\"prefers\" object=\"dark mode\"\n\
             - \"I run Arch Linux\" → subject=\"user\" predicate=\"runs_os\" object=\"arch linux\"\n\
             - \"nebula_chat uses Tauri\" → subject=\"nebula_chat\" predicate=\"built_with\" object=\"tauri\" object_kind=\"entity\""
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["subject", "predicate", "object"],
            "properties": {
                "subject":     {"type": "string", "description": "lowercase snake_case key; use \"user\" for facts about the human."},
                "predicate":   {"type": "string", "description": "lowercase snake_case key (e.g. \"prefers\", \"runs_os\")."},
                "object":      {"type": "string", "description": "Free-form literal value, or snake_case entity key when object_kind=\"entity\"."},
                "object_kind": {"type": "string", "enum": ["literal", "entity"], "description": "Defaults to \"literal\"."},
                "confidence":  {"type": "number", "minimum": 0, "maximum": 1, "description": "Default 0.9."}
            }
        }),
    }
}

fn fact_recall_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_FACT_RECALL.to_string(),
        description:
            "FACT READ — search the knowledge graph by subject / predicate / object substring. \
             Any combination of filters; an omitted filter matches anything. Returns up to \
             `limit` facts (default 20) including their stable `id`s for follow-up calls to \
             memory_fact_forget.\n\n\
             Use this — NOT memory_doc_recall — when you want to look up structured facts \
             you previously stored. Examples:\n\
             - All facts about the user → subject=\"user\".\n\
             - Everything stored about a project → subject=\"nebula_chat\".\n\
             - What the user prefers → subject=\"user\" predicate=\"prefer\" (LIKE match)."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "subject":   {"type": "string", "description": "Substring match against subject."},
                "predicate": {"type": "string", "description": "Substring match against predicate."},
                "object":    {"type": "string", "description": "Substring match against object."},
                "limit":     {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}
            }
        }),
    }
}

fn fact_forget_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_FACT_FORGET.to_string(),
        description:
            "FACT DELETE — remove a single fact from the knowledge graph by id. Pair with \
             memory_fact_recall to find the id first. Destructive; prefer memory_fact_remember \
             to overwrite (it upserts on the (subject, predicate, object, object_kind) key)."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": {"type": "string", "description": "Fact id from memory_fact_recall."}
            }
        }),
    }
}

// ---------- DOC tools (markdown documents) ----------

fn doc_remember_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DOC_REMEMBER.to_string(),
        description:
            "DOCUMENT WRITE — create a new markdown memory document on disk. Use ONLY for \
             narrative, multi-paragraph content that needs prose to make sense — project briefs, \
             architecture decision records, workflow notes, README-style overviews.\n\n\
             DO NOT use for atomic facts (name, preferences, OS, editor, tech-stack choice). \
             Those belong in the knowledge graph via memory_fact_remember. A \"user-profile.md\" \
             with a bulleted list of preferences is the WRONG shape; each bullet should be a \
             memory_fact_remember call.\n\n\
             Errors if a doc with this id already exists; use memory_doc_edit to update."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["id", "title", "content"],
            "properties": {
                "id":      {"type": "string", "description": "Slug, [a-z0-9][a-z0-9-]{0,63}. Used as the filename and for [[wikilinks]]."},
                "title":   {"type": "string", "description": "Human-readable title (any characters)."},
                "content": {"type": "string", "description": "Markdown body. Use [[other-id]] to link to other docs."},
                "tags":    {"type": "array", "items": {"type": "string"}, "description": "Free-form tags."},
                "links":   {"type": "array", "items": {"type": "string"}, "description": "Other doc IDs this doc links to."}
            }
        }),
    }
}

fn doc_fetch_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DOC_FETCH.to_string(),
        description:
            "DOCUMENT READ — retrieve a memory document by id. Returns the full body, tags, and \
             outbound links. Use the returned `updated_at` as `expected_updated_at` for a \
             subsequent memory_doc_edit to fail loudly on stale-read overwrites."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["id"],
            "properties": {"id": {"type": "string"}}
        }),
    }
}

fn doc_edit_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DOC_EDIT.to_string(),
        description:
            "DOCUMENT UPDATE — modify an existing memory document. Exactly one of `replace` or \
             `append` must be provided. `replace.find` must match exactly once in the body. \
             Pass `expected_updated_at` (from a recent memory_doc_fetch) to fail loudly if the \
             doc changed since you read it."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": {"type": "string"},
                "expected_updated_at": {"type": "string"},
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

fn doc_forget_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DOC_FORGET.to_string(),
        description:
            "DOCUMENT DELETE — remove a memory document and all its chunks. Destructive; prefer \
             memory_doc_edit to narrow or correct rather than delete."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["id"],
            "properties": {"id": {"type": "string"}}
        }),
    }
}

fn doc_recall_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DOC_RECALL.to_string(),
        description:
            "DOCUMENT SEARCH — semantic + lexical search across memory document chunks. Returns \
             up to k chunks (default 3), each tagged with its parent doc_id. Use memory_doc_fetch \
             to read the full doc for any chunk. For finding atomic facts, use \
             memory_fact_recall instead."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {"type": "string"},
                "k":     {"type": "integer", "minimum": 1, "maximum": 10, "default": 3},
                "tags":  {"type": "array", "items": {"type": "string"}}
            }
        }),
    }
}

fn doc_link_context_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DOC_LINK_CONTEXT.to_string(),
        description:
            "DOCUMENT GRAPH — breadth-first traversal of the doc link graph starting at the given \
             id. Returns reachable doc ids with their depth. Full bodies are not included; use \
             memory_doc_fetch for any doc you want to read in full."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id":       {"type": "string"},
                "depth":    {"type": "integer", "minimum": 1, "maximum": 3, "default": 2},
                "max_docs": {"type": "integer", "minimum": 1, "maximum": 20, "default": 10}
            }
        }),
    }
}
