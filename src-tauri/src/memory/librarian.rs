use crate::memory::sqlite_manager::SqliteManager;
use crate::memory::tantivy_index::{SearchResult, TantivyIndex};
use anyhow::Result;
use std::sync::Arc;

use crate::memory::audit_logger::AuditLogger;

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

    pub fn save_full_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: Option<&str>,
        tool_calls: Option<&str>,
        tool_call_id: Option<&str>,
        attachments: Option<&[crate::llm::provider::Attachment]>,
    ) -> Result<()> {
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

    pub fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        self.tantivy.search(query, 10)
    }
}
