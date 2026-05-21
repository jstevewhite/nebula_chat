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

pub const ALL_NAMES: &[&str] = &[
    TOOL_REMEMBER,
    TOOL_FETCH,
    TOOL_EDIT,
    TOOL_FORGET,
    TOOL_RECALL,
    TOOL_LINK_CONTEXT,
];

pub fn is_memory_tool(name: &str) -> bool {
    ALL_NAMES.contains(&name)
}

/// Build the full set of tool definitions exposed to the LLM.
pub fn build_all() -> Vec<ToolDefinition> {
    vec![
        remember_tool(),
        fetch_tool(),
        edit_tool(),
        forget_tool(),
        recall_tool(),
        link_context_tool(),
    ]
}

fn remember_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_REMEMBER.to_string(),
        description:
            "Create a new long-term memory document. The document is stored as a markdown file \
             the user can read and edit. Use this for stable, narrative knowledge \
             (user profile, project notes, working agreements, environment details). \
             Do NOT use for one-off chat content — that's already captured in conversation history. \
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
