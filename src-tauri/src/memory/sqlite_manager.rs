use anyhow::Result;
use rusqlite::{Connection, params};

pub struct SqliteManager {
    conn: Connection,
}

impl SqliteManager {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        
        // Initialize Tables
        conn.execute(
            "CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                title TEXT,
                created_at TEXT
            )",
            [],
        )?;
        
        conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                conversation_id TEXT,
                role TEXT,
                content TEXT,
                created_at TEXT,
                FOREIGN KEY(conversation_id) REFERENCES conversations(id)
            )",
            [],
        )?;

        Ok(Self { conn })
    }

    pub fn migrate_v2(&self) -> Result<()> {
        let _ = self.conn.execute("ALTER TABLE messages ADD COLUMN tool_calls TEXT", []);
        let _ = self.conn.execute("ALTER TABLE messages ADD COLUMN tool_call_id TEXT", []);
        Ok(())
    }

    pub fn list_conversations(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare("SELECT id, title, created_at FROM conversations ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        
        let mut convs = Vec::new();
        for row in rows {
            convs.push(row?);
        }
        Ok(convs)
    }

    pub fn get_conversation_messages(&self, conversation_id: &str) -> Result<Vec<(String, String, Option<String>, Option<String>, Option<String>)>> {
        // Returns (id, role, content, tool_calls_json, tool_call_id)
        let mut stmt = self.conn.prepare(
            "SELECT id, role, content, tool_calls, tool_call_id FROM messages 
             WHERE conversation_id = ?1 
             ORDER BY created_at ASC"
        )?;
        
        let rows = stmt.query_map(params![conversation_id], |row| {
            Ok((
                row.get(0)?, 
                row.get(1)?, 
                row.get(2)?, 
                row.get(3)?, 
                row.get(4)?
            ))
        })?;

        let mut msgs = Vec::new();
        for row in rows {
            msgs.push(row?);
        }
        Ok(msgs)
    }
    
    pub fn save_full_message(&self, conversation_id: &str, role: &str, content: Option<&str>, tool_calls: Option<&str>, tool_call_id: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, tool_calls, tool_call_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
            params![
                uuid::Uuid::new_v4().to_string(), 
                conversation_id, 
                role, 
                content,
                tool_calls,
                tool_call_id
            ],
        )?;
        Ok(())
    }

    pub fn save_message(&self, conversation_id: &str, role: &str, content: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))",
            params![uuid::Uuid::new_v4().to_string(), conversation_id, role, content],
        )?;
        Ok(())
    }

    pub fn save_message_with_timestamp(&self, conversation_id: &str, role: &str, content: &str, timestamp: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![uuid::Uuid::new_v4().to_string(), conversation_id, role, content, timestamp],
        )?;
        Ok(())
    }

    pub fn get_message_count(&self, conversation_id: &str) -> Result<usize> {
        let mut stmt = self.conn.prepare("SELECT COUNT(*) FROM messages WHERE conversation_id = ?1")?;
        let count: usize = stmt.query_row(params![conversation_id], |row| row.get(0))?;
        Ok(count)
    }

    pub fn delete_messages(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() { return Ok(()); }
        // Naive loop for now, or build a query
        for id in ids {
            self.conn.execute("DELETE FROM messages WHERE id = ?1", params![id])?;
        }
        Ok(())
    }

    pub fn get_oldest_messages(&self, conversation_id: &str, limit: usize) -> Result<Vec<(String, String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, content, created_at FROM messages 
             WHERE conversation_id = ?1 
             ORDER BY created_at ASC
             LIMIT ?2"
        )?;
        
        let rows = stmt.query_map(params![conversation_id, limit], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;

        let mut msgs = Vec::new();
        for row in rows {
            msgs.push(row?);
        }
        Ok(msgs)
    }

    pub fn get_history(&self, conversation_id: &str, limit: usize) -> Result<Vec<(String, String)>> {
        // Get recent messages (Reverse order first, then reverse back)
        let mut stmt = self.conn.prepare(
            "SELECT role, content FROM messages 
             WHERE conversation_id = ?1 
             ORDER BY created_at DESC
             LIMIT ?2"
        )?;
        
        let rows = stmt.query_map(params![conversation_id, limit], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;

        let mut history = Vec::new();
        for row in rows {
            history.push(row?);
        }
        history.reverse(); // Restore chronological order
        Ok(history)
    }

    pub fn init_conversation(&self, title: &str) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO conversations (id, title, created_at) VALUES (?1, ?2, datetime('now'))",
            params![id, title],
        )?;
        Ok(id)
    }

    pub fn delete_conversation(&self, id: &str) -> Result<()> {
        // Cascade delete messages first
        self.conn.execute("DELETE FROM messages WHERE conversation_id = ?1", params![id])?;
        self.conn.execute("DELETE FROM conversations WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn rename_conversation(&self, id: &str, new_title: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE conversations SET title = ?1 WHERE id = ?2",
            params![new_title, id],
        )?;
        Ok(())
    }
}
