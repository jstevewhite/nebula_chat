use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::memory::{Fact, NewFact, ObjectKind};

pub struct SqliteManager {
    pub conn: Connection, // Made public for testing purposes
}

impl SqliteManager {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;

        // Ensure foreign key constraints are enforced on this connection.
        // This is required for ON DELETE SET NULL on fact provenance to work.
        conn.execute("PRAGMA foreign_keys = ON;", [])?;

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

    /// Initial migration for the structured facts table backing the knowledge graph.
    /// Uses IF NOT EXISTS to remain idempotent across app startups.
    pub fn migrate_facts_v1(&self) -> Result<()> {
        // Core facts table with typed object semantics and provenance.
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS facts (
                id TEXT PRIMARY KEY,
                subject TEXT NOT NULL,
                predicate TEXT NOT NULL,
                object TEXT NOT NULL,
                object_kind TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 1.0,
                source_message_id TEXT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(source_message_id) REFERENCES messages(id) ON DELETE SET NULL
            )",
            [],
        )?;

        // Uniqueness constraint to avoid duplicate canonical facts.
        self.conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_facts_unique \
             ON facts(subject, predicate, object, object_kind)",
            [],
        )?;

        // Lookup and traversal helpers.
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_facts_subject ON facts(subject)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_facts_predicate ON facts(predicate)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_facts_object ON facts(object)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_facts_subject_predicate \
             ON facts(subject, predicate)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_facts_source_message_id \
             ON facts(source_message_id)",
            [],
        )?;

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

    /// Insert a new fact row without attempting to deduplicate.
    /// Most callers should prefer `upsert_fact` to enforce the UNIQUE constraint.
    pub fn save_fact(&self, fact: NewFact) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        self.conn.execute(
            "INSERT INTO facts (
                id, subject, predicate, object, object_kind,
                confidence, source_message_id, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                fact.subject,
                fact.predicate,
                fact.object,
                fact.object_kind.as_str(),
                fact.confidence,
                fact.source_message_id,
                now,
                now,
            ],
        )?;

        Ok(id)
    }

    /// Upsert a fact based on its (subject, predicate, object, object_kind) identity.
    /// Returns the canonical fact id after insert/update.
    pub fn upsert_fact(&self, fact: NewFact) -> Result<String> {
        // First, see if a fact with this identity already exists.
        let mut stmt = self.conn.prepare(
            "SELECT id FROM facts
             WHERE subject = ?1 AND predicate = ?2 AND object = ?3 AND object_kind = ?4
             LIMIT 1",
        )?;

        let existing_id: Option<String> = stmt
            .query_row(
                params![
                    &fact.subject,
                    &fact.predicate,
                    &fact.object,
                    fact.object_kind.as_str(),
                ],
                |row| row.get(0),
            )
            .optional()?;

        let now = chrono::Utc::now().to_rfc3339();

        if let Some(id) = existing_id {
            // Update existing row: refresh confidence, provenance, and updated_at.
            self.conn.execute(
                "UPDATE facts
                 SET confidence = ?1,
                     source_message_id = ?2,
                     updated_at = ?3
                 WHERE id = ?4",
                params![fact.confidence, fact.source_message_id, now, id],
            )?;
            Ok(id)
        } else {
            // Insert new canonical fact.
            self.save_fact(fact)
        }
    }

    pub fn update_fact_confidence(&self, id: &str, confidence: f32) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE facts SET confidence = ?1, updated_at = ?2 WHERE id = ?3",
            params![confidence, now, id],
        )?;
        Ok(())
    }

    pub fn delete_fact(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM facts WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Delete all facts that reference a given source message id.
    /// With foreign keys enabled and ON DELETE SET NULL this is optional,
    /// but can be useful for explicit cleanup flows.
    pub fn delete_facts_by_source_message(&self, message_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM facts WHERE source_message_id = ?1",
            params![message_id],
        )?;
        Ok(())
    }

    /// Generic fact query helper with optional filters and a hard limit.
    pub fn query_facts(
        &self,
        subject: Option<&str>,
        predicate: Option<&str>,
        object: Option<&str>,
        object_kind: Option<ObjectKind>,
        limit: usize,
    ) -> Result<Vec<Fact>> {
        let mut sql = String::from(
            "SELECT id, subject, predicate, object, object_kind,
                    confidence, source_message_id, created_at, updated_at
             FROM facts",
        );
        let mut conditions: Vec<String> = Vec::new();
        let mut params: Vec<String> = Vec::new();

        if let Some(s) = subject {
            conditions.push("subject = ?".to_string());
            params.push(s.to_string());
        }
        if let Some(p) = predicate {
            conditions.push("predicate = ?".to_string());
            params.push(p.to_string());
        }
        if let Some(o) = object {
            conditions.push("object = ?".to_string());
            params.push(o.to_string());
        }
        if let Some(kind) = object_kind {
            conditions.push("object_kind = ?".to_string());
            params.push(kind.as_str().to_string());
        }

        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        sql.push_str(" ORDER BY updated_at DESC LIMIT ?");
        params.push(limit.to_string());

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
            let object_kind_str: String = row.get(4)?;
            let object_kind = ObjectKind::from_str(&object_kind_str).unwrap_or(ObjectKind::Literal);
            let confidence_f64: f64 = row.get(5)?;

            Ok(Fact {
                id: row.get(0)?,
                subject: row.get(1)?,
                predicate: row.get(2)?,
                object: row.get(3)?,
                object_kind,
                confidence: confidence_f64 as f32,
                source_message_id: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?;

        let mut facts = Vec::new();
        for row in rows {
            facts.push(row?);
        }
        Ok(facts)
    }

    /// Get facts about a given entity, including inbound edges where the
    /// object is that entity and is itself typed as an ENTITY.
    pub fn get_facts_about_entity(&self, entity: &str, limit: usize) -> Result<Vec<Fact>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, subject, predicate, object, object_kind,
                    confidence, source_message_id, created_at, updated_at
             FROM facts
             WHERE subject = ?1
                OR (object = ?1 AND object_kind = 'entity')
             ORDER BY updated_at DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![entity, limit], |row| {
            let object_kind_str: String = row.get(3 + 1)?; // object_kind column index
            let object_kind = ObjectKind::from_str(&object_kind_str).unwrap_or(ObjectKind::Literal);
            let confidence_f64: f64 = row.get(5)?;

            Ok(Fact {
                id: row.get(0)?,
                subject: row.get(1)?,
                predicate: row.get(2)?,
                object: row.get(3)?,
                object_kind,
                confidence: confidence_f64 as f32,
                source_message_id: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?;

        let mut facts = Vec::new();
        for row in rows {
            facts.push(row?);
        }
        Ok(facts)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_migration_and_upsert_deduplicates() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        mgr.migrate_facts_v1().expect("migrate facts v1");

        let fact1 = NewFact::new(
            "user",
            "role",
            "systems_engineer",
            ObjectKind::Literal,
            0.9,
            None,
        );

        let id1 = mgr.upsert_fact(fact1).expect("first upsert");

        // Second upsert with same identity but different confidence should
        // reuse the same id and update confidence.
        let fact2 = NewFact::new(
            "user",
            "role",
            "systems_engineer",
            ObjectKind::Literal,
            0.5,
            None,
        );

        let id2 = mgr.upsert_fact(fact2).expect("second upsert");
        assert_eq!(id1, id2);

        let results = mgr
            .query_facts(Some("user"), Some("role"), Some("systems_engineer"), None, 10)
            .expect("query facts");

        assert_eq!(results.len(), 1);
        let stored = &results[0];
        assert_eq!(stored.id, id1);
        assert!((stored.confidence - 0.5).abs() < f32::EPSILON);
    }
}
