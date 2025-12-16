use anyhow::Result;
use rusqlite::{params, Connection};

pub struct SqliteManager {
    pub conn: Connection, // Made public for testing purposes
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

        // Create index for better performance on conversation lookups
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_conversation_created ON messages(conversation_id, created_at)",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS tool_executions (
                id TEXT PRIMARY KEY,
                conversation_id TEXT,
                tool_call_id TEXT,
                tool_name TEXT,
                server_name TEXT,
                args_json TEXT,
                result_preview TEXT,
                result_full_json TEXT,
                status TEXT,
                created_at TEXT
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS attachments (
                id TEXT PRIMARY KEY,
                message_id TEXT NOT NULL,
                filename TEXT,
                media_type TEXT,
                data TEXT,
                is_binary BOOLEAN,
                created_at TEXT,
                FOREIGN KEY(message_id) REFERENCES messages(id)
            )",
            [],
        )?;

        Ok(Self { conn })
    }

    pub fn migrate_v2(&self) -> Result<()> {
        let _ = self
            .conn
            .execute("ALTER TABLE messages ADD COLUMN tool_calls TEXT", []);
        let _ = self
            .conn
            .execute("ALTER TABLE messages ADD COLUMN tool_call_id TEXT", []);
        Ok(())
    }

    pub fn list_conversations(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, title, created_at FROM conversations ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;

        let mut convs = Vec::new();
        for row in rows {
            convs.push(row?);
        }
        Ok(convs)
    }

    pub fn get_conversation_messages(
        &self,
        conversation_id: &str,
    ) -> Result<
        Vec<(
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String, // created_at
            String,
        )>,
    > {
        // Returns (id, role, content, tool_calls_json, tool_call_id, created_at, attachments_json)
        let mut stmt = self.conn.prepare(
            "SELECT 
                m.id, 
                m.role, 
                m.content, 
                m.tool_calls, 
                m.tool_call_id, 
                m.created_at,
                (SELECT json_group_array(
                    json_object(
                        'id', a.id, 
                        'name', a.filename, 
                        'media_type', a.media_type, 
                        'data', a.data,
                        'is_binary', a.is_binary
                    )
                ) FROM attachments a WHERE a.message_id = m.id) as attachments
             FROM messages m
             WHERE m.conversation_id = ?1 
             ORDER BY m.created_at ASC",
        )?;

        let rows = stmt.query_map(params![conversation_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })?;

        let mut msgs = Vec::new();
        for row in rows {
            msgs.push(row?);
        }
        Ok(msgs)
    }

    pub fn save_full_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: Option<&str>,
        tool_calls: Option<&str>,
        tool_call_id: Option<&str>,
    ) -> Result<(String, String)> {
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, tool_calls, tool_call_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                conversation_id,
                role,
                content,
                tool_calls,
                tool_call_id,
                created_at
            ],
        )?;
        Ok((id, created_at))
    }

    pub fn save_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
    ) -> Result<(String, String)> {
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, conversation_id, role, content, created_at],
        )?;
        Ok((id, created_at))
    }

    pub fn save_message_with_timestamp(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        timestamp: &str,
    ) -> Result<(String, String)> {
        let id = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, conversation_id, role, content, timestamp],
        )?;
        Ok((id, timestamp.to_string()))
    }

    /// Check if a tool_call_id exists in assistant messages for a given conversation
    /// Returns true if the tool_call_id is found in any assistant message's tool_calls JSON
    pub fn tool_call_id_exists(&self, conversation_id: &str, tool_call_id: &str) -> Result<bool> {
        // Try using SQLite JSON functions first (JSON1 extension)
        let query = "
            SELECT EXISTS(
                SELECT 1 FROM messages m
                WHERE m.conversation_id = ?1
                AND m.role = 'assistant'
                AND m.tool_calls IS NOT NULL
                AND EXISTS (
                    SELECT 1 FROM json_each(m.tool_calls)
                    WHERE json_extract(json_each.value, '$.id') = ?2
                )
            )
        ";
        
        match self.conn.query_row(query, params![conversation_id, tool_call_id], |row| {
            row.get::<_, bool>(0)
        }) {
            Ok(exists) => Ok(exists),
            Err(_) => {
                // Fallback to conservative check if JSON1 is not available
                // This is less precise but prevents false positives
                let fallback_query = "
                    SELECT EXISTS(
                        SELECT 1 FROM messages m
                        WHERE m.conversation_id = ?1
                        AND m.role = 'assistant'
                        AND m.tool_calls IS NOT NULL
                        AND m.tool_calls LIKE ?2
                    )
                ";
                Ok(self.conn.query_row(fallback_query, params![conversation_id, format!("%{}", tool_call_id)], |row| row.get::<_, bool>(0))?)
            }
        }
    }

    pub fn get_message_count(&self, conversation_id: &str) -> Result<usize> {
        let mut stmt = self
            .conn
            .prepare("SELECT COUNT(*) FROM messages WHERE conversation_id = ?1")?;
        let count: usize = stmt.query_row(params![conversation_id], |row| row.get(0))?;
        Ok(count)
    }

    pub fn delete_messages(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        // Naive loop for now, or build a query
        for id in ids {
            self.conn
                .execute("DELETE FROM messages WHERE id = ?1", params![id])?;
        }
        Ok(())
    }

    pub fn get_oldest_messages(
        &self,
        conversation_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, content, created_at FROM messages 
             WHERE conversation_id = ?1 
             ORDER BY created_at ASC
             LIMIT ?2",
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

    pub fn get_history(
        &self,
        conversation_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        // Get recent messages (Reverse order first, then reverse back)
        let mut stmt = self.conn.prepare(
            "SELECT role, content FROM messages 
             WHERE conversation_id = ?1 
             ORDER BY created_at DESC
             LIMIT ?2",
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
        self.conn.execute(
            "DELETE FROM messages WHERE conversation_id = ?1",
            params![id],
        )?;
        self.conn
            .execute("DELETE FROM conversations WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn save_attachment(
        &self,
        message_id: &str,
        filename: &str,
        media_type: &str,
        data: &str,
        is_binary: bool,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO attachments (id, message_id, filename, media_type, data, is_binary, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, message_id, filename, media_type, data, is_binary, created_at],
        )?;
        Ok(())
    }

    pub fn get_attachments_for_message(
        &self,
        message_id: &str,
    ) -> Result<Vec<(String, String, String, bool)>> {
        let mut stmt = self.conn.prepare(
            "SELECT filename, media_type, data, is_binary FROM attachments WHERE message_id = ?1",
        )?;
        let rows = stmt.query_map(params![message_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;

        let mut attachments = Vec::new();
        for row in rows {
            attachments.push(row?);
        }
        Ok(attachments)
    }

    pub fn rename_conversation(&self, id: &str, new_title: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE conversations SET title = ?1 WHERE id = ?2",
            params![new_title, id],
        )?;
        Ok(())
    }
}
