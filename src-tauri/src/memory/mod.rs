pub mod audit_logger;
pub mod docs;
pub mod extraction;
pub mod librarian;
pub mod strategist;

pub mod sqlite_manager;
pub mod tantivy_index;

pub use librarian::SearchOptions;
pub use strategist::{
    SearchPlan, SearchQuery, StrategistContextResult, StrategistMemoryOrchestrator,
};
use tantivy_index::SearchResult;

/// Kind of fact object: another entity eligible for traversal or a literal value.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum ObjectKind {
    Entity,
    Literal,
}

impl ObjectKind {
    /// Stable string representation used in the SQLite schema.
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectKind::Entity => "entity",
            ObjectKind::Literal => "literal",
        }
    }

    /// Parse from the stored TEXT representation; returns None for unknown kinds.
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "entity" => Some(ObjectKind::Entity),
            "literal" => Some(ObjectKind::Literal),
            _ => None,
        }
    }
}

/// Canonical fact row backing the knowledge-graph-like memory layer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Fact {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub object_kind: ObjectKind,
    pub confidence: f32,
    pub source_message_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// DTO used when creating or upserting a fact before it has a canonical id.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NewFact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub object_kind: ObjectKind,
    pub confidence: f32,
    pub source_message_id: Option<String>,
}

impl NewFact {
    pub fn new(
        subject: impl Into<String>,
        predicate: impl Into<String>,
        object: impl Into<String>,
        object_kind: ObjectKind,
        confidence: f32,
        source_message_id: Option<String>,
    ) -> Self {
        Self {
            subject: subject.into(),
            predicate: predicate.into(),
            object: object.into(),
            object_kind,
            confidence,
            source_message_id,
        }
    }
}

/// Strategist-oriented view of a fact with optional extra metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RelevantFact {
    pub fact: Fact,
    /// True if this fact still has a provenance message id attached.
    pub has_provenance: bool,
}

impl RelevantFact {
    pub fn from_fact(fact: Fact) -> Self {
        let has_provenance = fact.source_message_id.is_some();
        Self {
            fact,
            has_provenance,
        }
    }
}

/// Lightweight DTO for memory hits optimized for strategist consumption.
/// Contains metadata + truncated snippet instead of full content.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryHit {
    pub message_id: String,
    pub conversation_id: String,
    pub role: String,
    pub created_at: String,
    pub score: f32,
    pub snippet: String,
}

impl MemoryHit {
    /// Create a MemoryHit from a SearchResult with configurable snippet length.
    ///
    /// `max_snippet_chars` is interpreted in terms of Unicode scalar values,
    /// not raw bytes, to avoid slicing in the middle of a multi-byte
    /// character (which would panic on invalid UTF-8 boundaries).
    pub fn from_search_result(result: SearchResult, max_snippet_chars: usize) -> Self {
        let mut snippet = String::new();
        let mut truncated = false;

        for (i, ch) in result.content.chars().enumerate() {
            if i >= max_snippet_chars {
                truncated = true;
                break;
            }
            snippet.push(ch);
        }

        if !truncated {
            // Either the string was shorter than the limit or exactly equal;
            // in either case we want the full content.
            // If snippet is empty here, just clone the original.
            if snippet.is_empty() && !result.content.is_empty() {
                snippet = result.content.clone();
            }
        } else {
            snippet.push_str("...");
        }

        Self {
            message_id: result.message_id,
            conversation_id: result.conversation_id,
            role: result.role,
            created_at: result.created_at,
            score: result.score,
            snippet,
        }
    }

    /// Create a MemoryHit from a SearchResult with default snippet length (400 chars)
    pub fn from_search_result_default(result: SearchResult) -> Self {
        Self::from_search_result(result, 400)
    }
}
