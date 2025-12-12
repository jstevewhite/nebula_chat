use anyhow::Result;
use std::sync::Arc;
use crate::memory::sqlite_manager::SqliteManager;
use crate::memory::tantivy_index::TantivyIndex;

pub struct Librarian {
    sqlite: SqliteManager,
    tantivy: Arc<TantivyIndex>,
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
        
        Ok(Self {
            sqlite,
            tantivy: Arc::new(tantivy),
        })
    }
    
    pub fn save_interaction(&self, conversation_id: &str, role: &str, content: &str) -> Result<()> {
        self.sqlite.save_message(conversation_id, role, content)?;
        self.tantivy.add_document(conversation_id, role, content)?;
        Ok(())
    }

    pub fn save_summary(&self, conversation_id: &str, content: &str, timestamp: &str) -> Result<()> {
        self.sqlite.save_message_with_timestamp(conversation_id, "system_summary", content, timestamp)?;
        // Also index the summary
        self.tantivy.add_document(conversation_id, "system_summary", content)?;
        Ok(())
    }
    
    pub fn get_message_count(&self, conversation_id: &str) -> Result<usize> {
        self.sqlite.get_message_count(conversation_id)
    }

    pub fn get_oldest_messages(&self, conversation_id: &str, limit: usize) -> Result<Vec<(String, String, String, String)>> {
        self.sqlite.get_oldest_messages(conversation_id, limit)
    }

    pub fn delete_messages(&self, ids: &[String]) -> Result<()> {
        self.sqlite.delete_messages(ids)
    }
    
    pub fn get_context(&self, conversation_id: &str) -> Result<Vec<(String, String)>> {
        self.sqlite.get_history(conversation_id, 20)
    }

    // New Conversation Management Methods
    pub fn create_conversation(&self, title: &str) -> Result<String> {
         self.sqlite.init_conversation(title)
    }
    
    pub fn delete_conversation(&self, id: &str) -> Result<()> {
        self.sqlite.delete_conversation(id)
    }

    pub fn rename_conversation(&self, id: &str, new_title: &str) -> Result<()> {
        self.sqlite.rename_conversation(id, new_title)
    }

    pub fn list_conversations(&self) -> Result<Vec<(String, String, String)>> {
        self.sqlite.list_conversations()
    }

    pub fn get_complete_history(&self, conversation_id: &str) -> Result<Vec<(String, Option<String>, Option<String>, Option<String>)>> {
        self.sqlite.get_conversation_messages(conversation_id)
    }

    pub fn save_full_message(&self, conversation_id: &str, role: &str, content: Option<&str>, tool_calls: Option<&str>, tool_call_id: Option<&str>) -> Result<()> {
        self.sqlite.save_full_message(conversation_id, role, content, tool_calls, tool_call_id)?;
        if let Some(text) = content {
             self.tantivy.add_document(conversation_id, role, text)?;
        }
        Ok(())
    }
    
    pub fn search(&self, query: &str) -> Result<Vec<(String, String)>> {
        self.tantivy.search(query, 5)
    }
}
