use crate::memory::sqlite_manager::SqliteManager;
use crate::memory::tantivy_index::{SearchResult, TantivyIndex};
use anyhow::Result;
use std::sync::Arc;

use crate::memory::audit_logger::AuditLogger;

/// Options for customizing memory search behavior
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// Maximum number of results to return
    pub limit: usize,
    /// Filter to specific conversation (None = search all conversations)
    pub conversation_id: Option<String>,
    /// Only include messages with these roles (None = include all)
    pub include_roles: Option<Vec<String>>,
    /// Exclude messages with these roles (None = exclude none)
    pub exclude_roles: Option<Vec<String>>,
    /// Only include messages from last N days (None = no time limit)
    pub max_age_days: Option<u64>,
}

impl SearchOptions {
    /// Create default search options with the given limit
    pub fn with_limit(limit: usize) -> Self {
        Self {
            limit,
            ..Default::default()
        }
    }
}

pub struct Librarian {
    sqlite: SqliteManager,
    tantivy: Arc<TantivyIndex>,
    pub audit: Arc<AuditLogger>,
}

impl Librarian {
    pub fn new(data_dir: &std::path::Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;

        let db_path = data_dir.join("nebula.db");
        let idx_path = data_dir.join("fulltext_index");

        let sqlite = SqliteManager::new(db_path.to_str().unwrap())?;
        // Attempt migration (ignore error if cols exist)
        let _ = sqlite.migrate_v2();
        // Initialize facts schema; uses IF NOT EXISTS so this is safe to run repeatedly.
        sqlite.migrate_facts_v1()?;

        let tantivy = TantivyIndex::new(idx_path.to_str().unwrap())?;
        let audit = AuditLogger::new(&db_path)?;

        Ok(Self {
            sqlite,
            tantivy: Arc::new(tantivy),
            audit: Arc::new(audit),
        })
    }

    pub fn save_interaction(&self, conversation_id: &str, role: &str, content: &str) -> Result<()> {
        let (id, created_at) = self.sqlite.save_message(conversation_id, role, content)?;
        self.tantivy
            .add_document(conversation_id, role, content, &id, &created_at)?;
        Ok(())
    }

    pub fn save_summary(
        &self,
        conversation_id: &str,
        content: &str,
        timestamp: &str,
    ) -> Result<()> {
        let (id, created_at) = self.sqlite.save_message_with_timestamp(
            conversation_id,
            "system_summary",
            content,
            timestamp,
        )?;
        // Also index the summary
        self.tantivy
            .add_document(conversation_id, "system_summary", content, &id, &created_at)?;
        Ok(())
    }

    pub fn get_message_count(&self, conversation_id: &str) -> Result<usize> {
        self.sqlite.get_message_count(conversation_id)
    }

    pub fn get_oldest_messages(
        &self,
        conversation_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, String, String, String)>> {
        self.sqlite.get_oldest_messages(conversation_id, limit)
    }

    pub fn delete_messages(&self, ids: &[String]) -> Result<()> {
        self.sqlite.delete_messages(ids)?;
        for id in ids {
            self.tantivy.delete_by_message_id(id)?;
        }
        Ok(())
    }

    pub fn get_context(&self, conversation_id: &str) -> Result<Vec<(String, String)>> {
        self.sqlite.get_history(conversation_id, 20)
    }

    // New Conversation Management Methods
    pub fn create_conversation(&self, title: &str) -> Result<String> {
        self.sqlite.init_conversation(title)
    }

    pub fn delete_conversation(&self, id: &str) -> Result<()> {
        self.sqlite.delete_conversation(id)?;
        self.tantivy.delete_by_conversation_id(id)?;
        Ok(())
    }

    pub fn rename_conversation(&self, id: &str, new_title: &str) -> Result<()> {
        self.sqlite.rename_conversation(id, new_title)
    }

    pub fn list_conversations(&self) -> Result<Vec<(String, String, String)>> {
        self.sqlite.list_conversations()
    }

    pub fn get_complete_history(
        &self,
        conversation_id: &str,
    ) -> Result<
        Vec<(
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String, // created_at added
            String, // attachments
        )>,
    > {
        self.sqlite.get_conversation_messages(conversation_id)
    }

    fn save_full_message_internal(
        &self,
        conversation_id: &str,
        role: &str,
        content: Option<&str>,
        tool_calls: Option<&str>,
        tool_call_id: Option<&str>,
        attachments: Option<&[crate::llm::provider::Attachment]>,
    ) -> Result<(String, String)> {
        let (id, created_at) = self.sqlite.save_full_message(
            conversation_id,
            role,
            content,
            tool_calls,
            tool_call_id,
        )?;

        if let Some(atts) = attachments {
            for att in atts {
                self.sqlite.save_attachment(
                    &id,
                    &att.name,
                    &att.media_type,
                    &att.data,
                    att.is_binary,
                )?;
            }
        }

        if let Some(text) = content {
            self.tantivy
                .add_document(conversation_id, role, text, &id, &created_at)?;
        }

        Ok((id, created_at))
    }

    /// Save a message and return its generated message id.
    ///
    /// This is a thin wrapper around the internal save helper that preserves
    /// existing behavior (SQLite + attachments + Tantivy indexing) while
    /// exposing the id for provenance-aware features like fact extraction.
    pub fn save_full_message_returning_id(
        &self,
        conversation_id: &str,
        role: &str,
        content: Option<&str>,
        tool_calls: Option<&str>,
        tool_call_id: Option<&str>,
        attachments: Option<&[crate::llm::provider::Attachment]>,
    ) -> Result<String> {
        let (id, _created_at) = self.save_full_message_internal(
            conversation_id,
            role,
            content,
            tool_calls,
            tool_call_id,
            attachments,
        )?;
        Ok(id)
    }

    pub fn save_full_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: Option<&str>,
        tool_calls: Option<&str>,
        tool_call_id: Option<&str>,
        attachments: Option<&[crate::llm::provider::Attachment]>,
    ) -> Result<()> {
        let _ = self.save_full_message_internal(
            conversation_id,
            role,
            content,
            tool_calls,
            tool_call_id,
            attachments,
        )?;
        Ok(())
    }

    pub fn index_existing_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        message_id: &str,
        created_at: &str,
    ) -> Result<()> {
        self.tantivy
            .add_document(conversation_id, role, content, message_id, created_at)
    }

    pub fn clear_search_index(&self) -> Result<()> {
        self.tantivy.clear_index()
    }

    /// Check if a tool_call_id exists in assistant messages for a given conversation
    pub fn tool_call_id_exists(&self, conversation_id: &str, tool_call_id: &str) -> Result<bool> {
        self.sqlite.tool_call_id_exists(conversation_id, tool_call_id)
    }

    /// Retrieve a bounded set of user-profile facts for personalization.
    ///
    /// For v0 we assume the canonical subject key for the current user is "user";
    /// higher layers are responsible for normalizing subjects consistently.
    pub fn get_user_profile_facts(&self, limit: usize) -> Result<Vec<crate::memory::Fact>> {
        self.sqlite
            .query_facts(Some("user"), None, None, None, limit)
    }

    /// Retrieve facts that mention the given entity either as subject or as an
    /// inbound ENTITY object.
    pub fn get_facts_about_entity(
        &self,
        entity: &str,
        limit: usize,
    ) -> Result<Vec<crate::memory::Fact>> {
        self.sqlite.get_facts_about_entity(entity, limit)
    }

    /// Upsert a fact into the knowledge-graph layer and return its canonical id.
    pub fn upsert_fact(&self, fact: crate::memory::NewFact) -> Result<String> {
        self.sqlite.upsert_fact(fact)
    }

    pub fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        self.search_with_options(query, SearchOptions::with_limit(10))
    }

    /// Search with custom options for filtering and scoping results
    pub fn search_with_options(&self, query: &str, options: SearchOptions) -> Result<Vec<SearchResult>> {
        let limit = if options.limit == 0 { 10 } else { options.limit };

        self.tantivy.search_with_options(
            query,
            options.conversation_id.as_deref(),
            options.include_roles.as_deref(),
            options.exclude_roles.as_deref(),
            options.max_age_days,
            limit,
        )
    }
}
