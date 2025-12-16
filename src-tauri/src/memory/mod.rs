pub mod audit_logger;
pub mod librarian;

pub mod sqlite_manager;
pub mod tantivy_index;

use tantivy_index::SearchResult;
pub use librarian::SearchOptions;

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
    /// Create a MemoryHit from a SearchResult with configurable snippet length
    pub fn from_search_result(result: SearchResult, max_snippet_chars: usize) -> Self {
        let snippet = if result.content.len() > max_snippet_chars {
            format!("{}...", &result.content[..max_snippet_chars])
        } else {
            result.content.clone()
        };

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
