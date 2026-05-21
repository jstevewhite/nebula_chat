//! Public API types for the docs subsystem, matching the JSON schemas of the
//! six `memory_*` tools exposed to the LLM.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocSummary {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocRecord {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub links: Vec<String>,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RememberInput {
    pub id: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RememberOutput {
    pub id: String,
    pub path: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditReplace {
    pub find: String,
    pub with: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditInput {
    pub id: String,
    #[serde(default)]
    pub expected_updated_at: Option<String>,
    #[serde(default)]
    pub replace: Option<EditReplace>,
    #[serde(default)]
    pub append: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditOutput {
    pub id: String,
    pub updated_at: String,
    pub new_length: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallInput {
    pub query: String,
    #[serde(default = "default_k")]
    pub k: usize,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_k() -> usize {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreComponents {
    pub cosine: f32,
    pub bm25: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallHit {
    pub doc_id: String,
    pub chunk_id: i64,
    pub ord: i64,
    pub text: String,
    pub score: f32,
    pub score_components: ScoreComponents,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallOutput {
    pub hits: Vec<RecallHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkContextInput {
    pub id: String,
    #[serde(default = "default_depth")]
    pub depth: i64,
    #[serde(default = "default_max_docs")]
    pub max_docs: i64,
}

fn default_depth() -> i64 {
    2
}

fn default_max_docs() -> i64 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkNode {
    pub id: String,
    pub title: Option<String>,
    pub depth: i64,
    pub missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkEdge {
    pub src: String,
    pub dst: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkContextOutput {
    pub start: String,
    pub nodes: Vec<LinkNode>,
    pub edges: Vec<LinkEdge>,
}

/// Standard error envelope returned to the LLM as the tool result. The
/// `code` field is stable; `current_updated_at` only populated on CONFLICT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocsError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_updated_at: Option<String>,
}

impl DocsError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            current_updated_at: None,
        }
    }

    pub fn conflict(current: String) -> Self {
        Self {
            code: "CONFLICT".into(),
            message:
                "Document has changed since fetch. Re-fetch and retry.".into(),
            current_updated_at: Some(current),
        }
    }
}
