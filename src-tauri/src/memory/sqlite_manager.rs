use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension, ToSql};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::memory::{Fact, NewFact, ObjectKind};

const MAX_BATCH_SIZE: usize = 1000;
const TOOL_CALL_CACHE_SIZE: usize = 1000;
const TOOL_CALL_CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Default)]
pub struct ToolCallIntegrityReport {
    pub tool_messages_without_id: Vec<(String, String)>,
    pub orphaned_tool_executions: Vec<ToolExecutionIssue>,
    pub missing_tool_executions: Vec<MissingToolExecution>,
}

#[derive(Debug)]
pub struct ToolExecutionIssue {
    pub execution_id: String,
    pub tool_name: String,
    pub tool_call_id: String,
    pub result_preview: String,
}

#[derive(Debug)]
pub struct MissingToolExecution {
    pub message_id: String,
    pub call_id: String,
    pub tool_name: String,
}

pub struct SafeInClauseBuilder {
    max_params: usize,
}

impl SafeInClauseBuilder {
    pub fn new(max_params: usize) -> Self {
        Self { max_params }
    }

    pub fn build(&self, param_count: usize) -> Result<String> {
        if param_count == 0 {
            return Ok(String::new());
        }
        if param_count > self.max_params {
            return Err(anyhow!(
                "Parameter count {} exceeds maximum {}",
                param_count,
                self.max_params
            ));
        }
        Ok(format!("({})", "?,".repeat(param_count).trim_end_matches(',')))
    }

    pub fn max_params(&self) -> usize {
        self.max_params
    }
}

pub struct SqliteManager {
    pub conn: Connection,
    tool_call_cache: Mutex<HashMap<(String, String), (bool, Instant)>>,
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

        Ok(Self {
            conn,
            tool_call_cache: Mutex::new(HashMap::new()),
        })
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

    pub fn delete_attachments(&self, message_ids: &[String]) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }

        let in_clause = SafeInClauseBuilder::new(MAX_BATCH_SIZE).build(message_ids.len())?;
        let stmt = format!("DELETE FROM attachments WHERE message_id IN {}", in_clause);

        let params: Vec<&dyn ToSql> = message_ids.iter().map(|s| s as &dyn ToSql).collect();
        self.conn.execute(&stmt, params.as_slice())?;

        Ok(())
    }

    pub fn delete_attachments_batched(&self, message_ids: &[String]) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }

        if message_ids.is_empty() {
            return Ok(());
        }

        for chunk in message_ids.chunks(MAX_BATCH_SIZE) {
            self.delete_attachments(chunk)?;
        }

        Ok(())
    }

    pub fn get_messages_by_ids(
        &self,
        ids: &[String],
    ) -> Result<Vec<(String, String, Option<String>, Option<String>)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let in_clause = SafeInClauseBuilder::new(MAX_BATCH_SIZE).build(ids.len())?;
        let sql = format!(
            "SELECT id, role, tool_calls, tool_call_id FROM messages WHERE id IN {}",
            in_clause
        );

        let params: Vec<&dyn ToSql> = ids.iter().map(|s| s as &dyn ToSql).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params.as_slice(), |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn delete_tool_executions_by_tool_call_ids(&self, tool_call_ids: &[String]) -> Result<()> {
        if tool_call_ids.is_empty() {
            return Ok(());
        }

        let in_clause = SafeInClauseBuilder::new(MAX_BATCH_SIZE).build(tool_call_ids.len())?;
        let sql = format!("DELETE FROM tool_executions WHERE tool_call_id IN {}", in_clause);

        let params: Vec<&dyn ToSql> = tool_call_ids.iter().map(|s| s as &dyn ToSql).collect();
        self.conn.execute(&sql, params.as_slice())?;

        Ok(())
    }

    pub fn delete_tool_executions_batched(&self, tool_call_ids: &[String]) -> Result<()> {
        if tool_call_ids.is_empty() {
            return Ok(());
        }

        for chunk in tool_call_ids.chunks(MAX_BATCH_SIZE) {
            self.delete_tool_executions_by_tool_call_ids(chunk)?;
        }

        Ok(())
    }

    pub fn migrate_v3(&self) -> Result<()> {
        let _ = self
            .conn
            .execute("ALTER TABLE messages ADD COLUMN reasoning_content TEXT", []);
        Ok(())
    }

    pub fn migrate_v4(&self) -> Result<()> {
        let _ = self
            .conn
            .execute("ALTER TABLE conversations ADD COLUMN icon TEXT", []);
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

    /// Migration for conversation summaries table.
    /// Used for rolling context compaction.
    pub fn migrate_summaries_v1(&self) -> Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS conversation_summaries (
                conversation_id TEXT PRIMARY KEY,
                last_message_id TEXT NOT NULL,
                summary TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(conversation_id) REFERENCES conversations(id)
            )",
            [],
        )?;
        Ok(())
    }

    /// Migration for compaction checkpoints table.
    /// Enables resumable compaction for large conversations.
    pub fn migrate_compaction_checkpoints_v1(&self) -> Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS compaction_checkpoints (
                conversation_id TEXT NOT NULL,
                phase TEXT NOT NULL,
                last_processed_index INTEGER NOT NULL DEFAULT 0,
                last_message_id TEXT,
                status TEXT NOT NULL DEFAULT 'in_progress',
                error_message TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (conversation_id, phase),
                FOREIGN KEY(conversation_id) REFERENCES conversations(id)
            )",
            [],
        )?;
        Ok(())
    }

    pub fn save_compaction_checkpoint(
        &self,
        conversation_id: &str,
        phase: &str,
        last_processed_index: usize,
        last_message_id: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR REPLACE INTO compaction_checkpoints
             (conversation_id, phase, last_processed_index, last_message_id, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'in_progress', ?5, ?6)",
            params![
                conversation_id,
                phase,
                last_processed_index as i64,
                last_message_id,
                now,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn get_compaction_checkpoint(
        &self,
        conversation_id: &str,
        phase: &str,
    ) -> Result<Option<(usize, Option<String>, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT last_processed_index, last_message_id, status
             FROM compaction_checkpoints WHERE conversation_id = ?1 AND phase = ?2",
        )?;
        stmt.query_row(params![conversation_id, phase], |row| {
            Ok((
                row.get::<_, i64>(0)? as usize,
                row.get(1)?,
                row.get(2)?,
            ))
        })
        .optional()
        .map_err(|e| e.into())
    }

    pub fn complete_compaction_checkpoint(&self, conversation_id: &str, phase: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE compaction_checkpoints SET status = 'completed', updated_at = ?1
             WHERE conversation_id = ?2 AND phase = ?3",
            params![now, conversation_id, phase],
        )?;
        Ok(())
    }

    pub fn fail_compaction_checkpoint(
        &self,
        conversation_id: &str,
        phase: &str,
        error_message: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE compaction_checkpoints SET status = 'failed', error_message = ?1, updated_at = ?2
             WHERE conversation_id = ?3 AND phase = ?4",
            params![error_message, now, conversation_id, phase],
        )?;
        Ok(())
    }

    pub fn delete_compaction_checkpoint(&self, conversation_id: &str, phase: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM compaction_checkpoints WHERE conversation_id = ?1 AND phase = ?2",
            params![conversation_id, phase],
        )?;
        Ok(())
    }

    pub fn list_conversations(&self) -> Result<Vec<(String, String, Option<String>, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, icon, created_at FROM conversations ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;

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

    /// Update all editable fields of a fact while preserving created_at and provenance.
    pub fn update_fact(
        &self,
        id: &str,
        subject: &str,
        predicate: &str,
        object: &str,
        object_kind: ObjectKind,
        confidence: f32,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE facts
             SET subject = ?1,
                 predicate = ?2,
                 object = ?3,
                 object_kind = ?4,
                 confidence = ?5,
                 updated_at = ?6
             WHERE id = ?7",
            params![
                subject,
                predicate,
                object,
                object_kind.as_str(),
                confidence,
                now,
                id,
            ],
        )?;
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
            let object_kind_str: String = row.get(4)?; // object_kind column index
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

    /// List distinct entity keys seen in the facts table.
    ///
    /// For now we treat subjects as the canonical entity keys. Objects
    /// (even when tagged as ENTITY) can still be surfaced via
    /// `get_facts_about_entity`, but they are not listed as top-level
    /// entities here.
    pub fn list_fact_entities(&self, limit: usize) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT subject AS key
             FROM facts
             WHERE subject != ''
             ORDER BY key
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit], |row| row.get(0))?;
        let mut entities = Vec::new();
        for row in rows {
            let key: String = row?;
            if !key.is_empty() {
                entities.push(key);
            }
        }
        Ok(entities)
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
            Option<String>, // reasoning_content
            String,         // created_at
            String,
        )>,
    > {
        // Returns (id, role, content, tool_calls_json, tool_call_id, reasoning_content, created_at, attachments_json)
        let mut stmt = self.conn.prepare(
            "SELECT
                m.id,
                m.role,
                m.content,
                m.tool_calls,
                m.tool_call_id,
                m.reasoning_content,
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
                row.get(7)?,
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
        reasoning_content: Option<&str>,
    ) -> Result<(String, String)> {
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, tool_calls, tool_call_id, reasoning_content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                conversation_id,
                role,
                content,
                tool_calls,
                tool_call_id,
                reasoning_content,
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
        let cache_key = (conversation_id.to_string(), tool_call_id.to_string());

        // Check cache first
        {
            let cache = self.tool_call_cache.lock().unwrap();
            if let Some((cached_result, timestamp)) = cache.get(&cache_key) {
                if timestamp.elapsed() < TOOL_CALL_CACHE_TTL {
                    return Ok(*cached_result);
                }
            }
        }

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

        let result = match self
            .conn
            .query_row(query, params![conversation_id, tool_call_id], |row| {
                row.get::<_, bool>(0)
            }) {
            Ok(exists) => exists,
            Err(_) => self.tool_call_id_exists_fallback(conversation_id, tool_call_id)?,
        };

        // Update cache
        {
            let mut cache = self.tool_call_cache.lock().unwrap();
            if cache.len() >= TOOL_CALL_CACHE_SIZE {
                // Simple eviction: clear half the cache when full
                let to_remove = TOOL_CALL_CACHE_SIZE / 2;
                let keys: Vec<_> = cache.keys().take(to_remove).cloned().collect();
                for key in keys {
                    cache.remove(&key);
                }
            }
            cache.insert(cache_key, (result, Instant::now()));
        }

        Ok(result)
    }

    /// Fallback implementation when JSON1 extension is not available
    /// Parses JSON manually to avoid false positives from LIKE matching
    fn tool_call_id_exists_fallback(&self, conversation_id: &str, tool_call_id: &str) -> Result<bool> {
        let mut stmt = self.conn.prepare(
            "SELECT tool_calls FROM messages
             WHERE conversation_id = ?1 AND role = 'assistant' AND tool_calls IS NOT NULL"
        )?;

        let rows = stmt.query_map([conversation_id], |row| {
            row.get::<_, String>(0)
        })?;

        for row in rows {
            if let Ok(json_str) = row {
                if let Ok(calls) = serde_json::from_str::<Vec<serde_json::Value>>(&json_str) {
                    for call in calls {
                        if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                            if id == tool_call_id {
                                return Ok(true);
                            }
                        }
                    }
                }
            }
        }
        Ok(false)
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
        for id in ids {
            self.conn
                .execute("DELETE FROM messages WHERE id = ?1", params![id])?;
        }
        self.invalidate_tool_call_cache();
        Ok(())
    }

    pub fn invalidate_tool_call_cache(&self) {
        let mut cache = self.tool_call_cache.lock().unwrap();
        cache.clear();
    }

    /// Check tool call integrity for a conversation
    /// Returns a report of any issues found
    pub fn check_tool_call_integrity(&self, conversation_id: &str) -> Result<ToolCallIntegrityReport> {
        let mut report = ToolCallIntegrityReport::default();

        // Check for tool messages without valid tool_call_id
        let mut stmt = self.conn.prepare(
            "SELECT id, content FROM messages
             WHERE conversation_id = ?1 AND role = 'tool'
             AND (tool_call_id IS NULL OR tool_call_id = '')"
        )?;
        let rows = stmt.query_map([conversation_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, content) = row?;
            let preview = content.chars().take(50).collect::<String>();
            report.tool_messages_without_id.push((id, preview));
        }

        // Check for tool executions without matching tool calls
        let mut stmt = self.conn.prepare(
            "SELECT te.id, te.tool_name, te.tool_call_id, te.result_preview
             FROM tool_executions te
             WHERE te.conversation_id = ?1
             AND NOT EXISTS (
                 SELECT 1 FROM messages m
                 WHERE m.conversation_id = ?1
                 AND m.role = 'assistant'
                 AND m.tool_calls IS NOT NULL
                 AND EXISTS (
                     SELECT 1 FROM json_each(m.tool_calls)
                     WHERE json_extract(json_each.value, '$.id') = te.tool_call_id
                 )
             )"
        )?;
        let rows = stmt.query_map([conversation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        for row in rows {
            let (id, name, call_id, preview) = row?;
            report.orphaned_tool_executions.push(ToolExecutionIssue {
                execution_id: id,
                tool_name: name,
                tool_call_id: call_id,
                result_preview: preview.unwrap_or_default(),
            });
        }

        // Check for assistant messages with tool_calls that don't match any tool executions
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.tool_calls FROM messages m
             WHERE m.conversation_id = ?1 AND m.role = 'assistant'
             AND m.tool_calls IS NOT NULL"
        )?;
        let rows = stmt.query_map([conversation_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (msg_id, tool_calls_json) = row?;
            if let Ok(calls) = serde_json::from_str::<Vec<serde_json::Value>>(&tool_calls_json) {
                for call in calls {
                    if let Some(call_id) = call.get("id").and_then(|v| v.as_str()) {
                        let exists = self.tool_call_id_exists(conversation_id, call_id)?;
                        if !exists {
                            report.missing_tool_executions.push(MissingToolExecution {
                                message_id: msg_id.clone(),
                                call_id: call_id.to_string(),
                                tool_name: call.get("name").and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
                            });
                        }
                    }
                }
            }
        }

        Ok(report)
    }

    /// Get count of tool call integrity issues
    pub fn count_tool_call_integrity_issues(&self, conversation_id: &str) -> Result<usize> {
        let report = self.check_tool_call_integrity(conversation_id)?;
        Ok(report.tool_messages_without_id.len()
            + report.orphaned_tool_executions.len()
            + report.missing_tool_executions.len())
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
        // Cascade delete in correct order: attachments -> messages -> conversation

        // 1. Delete attachments for all messages in this conversation
        self.conn.execute(
            "DELETE FROM attachments WHERE message_id IN (SELECT id FROM messages WHERE conversation_id = ?1)",
            params![id],
        )?;

        // 2. Delete messages for this conversation
        self.conn.execute(
            "DELETE FROM messages WHERE conversation_id = ?1",
            params![id],
        )?;

        // 3. Delete the conversation itself
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

    pub fn update_conversation_icon(&self, id: &str, icon: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE conversations SET icon = ?1 WHERE id = ?2",
            params![icon, id],
        )?;
        Ok(())
    }

    pub fn update_conversation_title_and_icon(
        &self,
        id: &str,
        title: &str,
        icon: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE conversations SET title = ?1, icon = ?2 WHERE id = ?3",
            params![title, icon, id],
        )?;
        Ok(())
    }

    pub fn get_conversation_summary(
        &self,
        conversation_id: &str,
    ) -> Result<Option<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT last_message_id, summary FROM conversation_summaries WHERE conversation_id = ?1",
        )?;

        let result = stmt
            .query_row(params![conversation_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;

        Ok(result)
    }

    pub fn save_conversation_summary(
        &self,
        conversation_id: &str,
        last_message_id: &str,
        summary: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO conversation_summaries (conversation_id, last_message_id, summary, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(conversation_id) DO UPDATE SET
                last_message_id = excluded.last_message_id,
                summary = excluded.summary,
                updated_at = excluded.updated_at",
            params![conversation_id, last_message_id, summary, now],
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
            .query_facts(
                Some("user"),
                Some("role"),
                Some("systems_engineer"),
                None,
                10,
            )
            .expect("query facts");

        assert_eq!(results.len(), 1);
        let stored = &results[0];
        assert_eq!(stored.id, id1);
        assert!((stored.confidence - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_delete_attachments_empty_input() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let result = mgr.delete_attachments(&[]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_delete_attachments_normal_batch() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        let msg_id = mgr.save_message(&conv_id, "user", "test").unwrap().0;
        let att_id = uuid::Uuid::new_v4().to_string();
        mgr.conn.execute(
            "INSERT INTO attachments (id, message_id, filename, created_at) VALUES (?1, ?2, ?3, datetime('now'))",
            params![att_id, msg_id, "test.txt"],
        ).unwrap();

        let result = mgr.delete_attachments(&[msg_id.clone()]);
        assert!(result.is_ok());

        let count: i64 = mgr.conn
            .query_row("SELECT COUNT(*) FROM attachments WHERE id = ?", params![att_id], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_delete_attachments_batch_size_limit() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        mgr.init_conversation("test_conv").unwrap();

        let large_batch: Vec<String> = (0..=MAX_BATCH_SIZE).map(|i| format!("msg_{}", i)).collect();
        let result = mgr.delete_attachments(&large_batch);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("exceeds maximum"));
    }

    #[test]
    fn test_delete_attachments_batched_handles_large_inputs() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        mgr.init_conversation("test_conv").unwrap();

        let large_batch: Vec<String> = (0..=MAX_BATCH_SIZE).map(|i| format!("msg_{}", i)).collect();
        let result = mgr.delete_attachments_batched(&large_batch);
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_messages_by_ids_empty_input() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let result = mgr.get_messages_by_ids(&[]);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_get_messages_by_ids_normal_batch() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_v2();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        let msg_id1 = mgr.save_message(&conv_id, "user", "test1").unwrap().0;
        let msg_id2 = mgr.save_message(&conv_id, "assistant", "test2").unwrap().0;

        let result = mgr.get_messages_by_ids(&[msg_id1.clone(), msg_id2.clone()]);
        assert!(result.is_ok(), "Failed: {:?}", result.err());
        let messages = result.unwrap();
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn test_get_messages_by_ids_batch_size_limit() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let large_batch: Vec<String> = (0..=MAX_BATCH_SIZE).map(|i| format!("msg_{}", i)).collect();
        let result = mgr.get_messages_by_ids(&large_batch);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("exceeds maximum"));
    }

    #[test]
    fn test_delete_tool_executions_by_tool_call_ids_empty_input() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let result = mgr.delete_tool_executions_by_tool_call_ids(&[]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_delete_tool_executions_by_tool_call_ids_normal_batch() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        mgr.init_conversation("test_conv").unwrap();

        let call_id = uuid::Uuid::new_v4().to_string();
        mgr.conn.execute(
            "INSERT INTO tool_executions (id, conversation_id, tool_call_id, tool_name, created_at) VALUES (?1, ?2, ?3, ?4, datetime('now'))",
            params![uuid::Uuid::new_v4().to_string(), "test_conv", call_id.clone(), "test_tool"],
        ).unwrap();

        let result = mgr.delete_tool_executions_by_tool_call_ids(&[call_id.clone()]);
        assert!(result.is_ok());

        let count: i64 = mgr.conn
            .query_row("SELECT COUNT(*) FROM tool_executions WHERE tool_call_id = ?", params![call_id], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_delete_tool_executions_by_tool_call_ids_batch_size_limit() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let large_batch: Vec<String> = (0..=MAX_BATCH_SIZE).map(|i| format!("call_{}", i)).collect();
        let result = mgr.delete_tool_executions_by_tool_call_ids(&large_batch);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("exceeds maximum"));
    }

    #[test]
    fn test_delete_tool_executions_batched_handles_large_inputs() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        mgr.init_conversation("test_conv").unwrap();

        let large_batch: Vec<String> = (0..=MAX_BATCH_SIZE).map(|i| format!("call_{}", i)).collect();
        let result = mgr.delete_tool_executions_batched(&large_batch);
        assert!(result.is_ok());
    }

    #[test]
    fn test_sql_injection_attempt_is_prevented() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        mgr.init_conversation("test_conv").unwrap();

        let malicious_id = "msg_1'; DROP TABLE attachments; --".to_string();
        let result = mgr.delete_attachments(&[malicious_id.clone()]);
        assert!(result.is_ok());

        let count: i64 = mgr.conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='attachments'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_max_batch_size_constant() {
        assert_eq!(MAX_BATCH_SIZE, 1000);
    }

    #[test]
    fn test_tool_call_id_exists_exact_match() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_v2();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        let tool_calls_json = r#"[{"id": "call_abc123", "name": "test_tool"}]"#;
        mgr.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, tool_calls, created_at) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
            params![uuid::Uuid::new_v4().to_string(), conv_id, "assistant", "test", tool_calls_json],
        ).unwrap();

        let result = mgr.tool_call_id_exists(&conv_id, "call_abc123");
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_tool_call_id_exists_no_false_positive_partial_match() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_v2();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        let tool_calls_json = r#"[{"id": "call_abc123", "name": "test_tool"}]"#;
        mgr.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, tool_calls, created_at) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
            params![uuid::Uuid::new_v4().to_string(), conv_id, "assistant", "test", tool_calls_json],
        ).unwrap();

        let result = mgr.tool_call_id_exists(&conv_id, "call_abc");
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_tool_call_id_exists_no_match() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_v2();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        let tool_calls_json = r#"[{"id": "call_abc123", "name": "test_tool"}]"#;
        mgr.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, tool_calls, created_at) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
            params![uuid::Uuid::new_v4().to_string(), conv_id, "assistant", "test", tool_calls_json],
        ).unwrap();

        let result = mgr.tool_call_id_exists(&conv_id, "call_xyz789");
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_tool_call_id_exists_multiple_calls() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_v2();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        let tool_calls_json = r#"[{"id": "call_1", "name": "tool_a"}, {"id": "call_2", "name": "tool_b"}, {"id": "call_3", "name": "tool_c"}]"#;
        mgr.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, tool_calls, created_at) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
            params![uuid::Uuid::new_v4().to_string(), conv_id, "assistant", "test", tool_calls_json],
        ).unwrap();

        assert!(mgr.tool_call_id_exists(&conv_id, "call_1").unwrap());
        assert!(mgr.tool_call_id_exists(&conv_id, "call_2").unwrap());
        assert!(mgr.tool_call_id_exists(&conv_id, "call_3").unwrap());
        assert!(!mgr.tool_call_id_exists(&conv_id, "call_4").unwrap());
    }

    #[test]
    fn test_tool_call_id_exists_no_tool_calls_column() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_v2();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        mgr.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, datetime('now'))",
            params![uuid::Uuid::new_v4().to_string(), conv_id, "assistant", "test"],
        ).unwrap();

        let result = mgr.tool_call_id_exists(&conv_id, "any_call_id");
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_tool_call_id_exists_empty_conversation() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_v2();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        let result = mgr.tool_call_id_exists(&conv_id, "call_123");
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_safe_in_clause_builder_basic() {
        let builder = SafeInClauseBuilder::new(1000);
        assert_eq!(builder.build(3).unwrap(), "(?,?,?)");
        assert_eq!(builder.build(1).unwrap(), "(?)");
        assert_eq!(builder.build(0).unwrap(), "");
    }

    #[test]
    fn test_safe_in_clause_builder_exceeds_limit() {
        let builder = SafeInClauseBuilder::new(5);
        let result = builder.build(10);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("exceeds maximum"));
    }

    #[test]
    fn test_safe_in_clause_builder_max_params() {
        let builder = SafeInClauseBuilder::new(100);
        assert_eq!(builder.max_params(), 100);
        assert!(builder.build(100).is_ok());
        assert!(builder.build(101).is_err());
    }

    #[test]
    fn test_tool_call_id_exists_special_characters_in_id() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_v2();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        let special_id = "call_abc-123_xyz";
        let tool_calls_json = format!(r#"[{{"id": "{}", "name": "test_tool"}}]"#, special_id);
        mgr.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, tool_calls, created_at) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
            params![uuid::Uuid::new_v4().to_string(), conv_id, "assistant", "test", tool_calls_json],
        ).unwrap();

        let result = mgr.tool_call_id_exists(&conv_id, &special_id);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_compaction_checkpoint_save_and_retrieve() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_compaction_checkpoints_v1();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        mgr.save_compaction_checkpoint(&conv_id, "compact_messages", 10, Some("msg_123"))
            .unwrap();

        let checkpoint = mgr.get_compaction_checkpoint(&conv_id, "compact_messages").unwrap();
        assert!(checkpoint.is_some());
        let (index, msg_id, status) = checkpoint.unwrap();
        assert_eq!(index, 10);
        assert_eq!(msg_id, Some("msg_123".to_string()));
        assert_eq!(status, "in_progress");
    }

    #[test]
    fn test_compaction_checkpoint_not_found() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_compaction_checkpoints_v1();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        let checkpoint = mgr.get_compaction_checkpoint(&conv_id, "nonexistent_phase").unwrap();
        assert!(checkpoint.is_none());
    }

    #[test]
    fn test_compaction_checkpoint_complete_and_delete() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_compaction_checkpoints_v1();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        mgr.save_compaction_checkpoint(&conv_id, "compact_messages", 5, None)
            .unwrap();
        mgr.complete_compaction_checkpoint(&conv_id, "compact_messages")
            .unwrap();

        let checkpoint = mgr.get_compaction_checkpoint(&conv_id, "compact_messages").unwrap();
        let (_, _, status) = checkpoint.unwrap();
        assert_eq!(status, "completed");

        mgr.delete_compaction_checkpoint(&conv_id, "compact_messages")
            .unwrap();
        let checkpoint = mgr.get_compaction_checkpoint(&conv_id, "compact_messages").unwrap();
        assert!(checkpoint.is_none());
    }

    #[test]
    fn test_compaction_checkpoint_fail() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_compaction_checkpoints_v1();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        mgr.save_compaction_checkpoint(&conv_id, "compact_messages", 5, None)
            .unwrap();
        mgr.fail_compaction_checkpoint(&conv_id, "compact_messages", "Out of memory")
            .unwrap();

        let checkpoint = mgr.get_compaction_checkpoint(&conv_id, "compact_messages").unwrap();
        let (_, _, status) = checkpoint.unwrap();
        assert_eq!(status, "failed");
    }

    #[test]
    fn test_tool_call_integrity_clean() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_v2();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        let msg_id = uuid::Uuid::new_v4().to_string();
        let tool_call_id = "call_123";
        let tool_calls_json = format!(r#"[{{"id": "{}", "name": "test_tool"}}]"#, tool_call_id);
        mgr.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, tool_calls, tool_call_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
            params![msg_id, conv_id, "assistant", "test", tool_calls_json, tool_call_id],
        ).unwrap();

        mgr.conn.execute(
            "INSERT INTO tool_executions (id, conversation_id, tool_call_id, tool_name, args_json, status, created_at) VALUES (?1, ?2, ?3, ?4, '{}', 'completed', datetime('now'))",
            params![uuid::Uuid::new_v4().to_string(), conv_id, tool_call_id, "test_tool"],
        ).unwrap();

        let report = mgr.check_tool_call_integrity(&conv_id).unwrap();
        assert!(report.tool_messages_without_id.is_empty());
        assert!(report.orphaned_tool_executions.is_empty());
        assert!(report.missing_tool_executions.is_empty());
    }

    #[test]
    fn test_tool_call_integrity_tool_message_without_id() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_v2();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        mgr.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, datetime('now'))",
            params![uuid::Uuid::new_v4().to_string(), conv_id, "tool", "tool result"],
        ).unwrap();

        let report = mgr.check_tool_call_integrity(&conv_id).unwrap();
        assert_eq!(report.tool_messages_without_id.len(), 1);
        assert!(report.orphaned_tool_executions.is_empty());
        assert!(report.missing_tool_executions.is_empty());
    }

    #[test]
    fn test_tool_call_integrity_orphaned_tool_execution() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_v2();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        mgr.conn.execute(
            "INSERT INTO tool_executions (id, conversation_id, tool_call_id, tool_name, args_json, status, created_at) VALUES (?1, ?2, ?3, ?4, '{}', 'completed', datetime('now'))",
            params![uuid::Uuid::new_v4().to_string(), conv_id, "orphan_call", "orphan_tool"],
        ).unwrap();

        let report = mgr.check_tool_call_integrity(&conv_id).unwrap();
        assert!(report.tool_messages_without_id.is_empty());
        assert_eq!(report.orphaned_tool_executions.len(), 1);
        assert_eq!(report.orphaned_tool_executions[0].tool_name, "orphan_tool");
        assert!(report.missing_tool_executions.is_empty());
    }

    #[test]
    fn test_tool_call_integrity_count_issues() {
        let mgr = SqliteManager::new(":memory:").expect("in-memory sqlite");
        let _ = mgr.migrate_v2();
        let conv_id = mgr.init_conversation("test_conv").unwrap();

        mgr.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, datetime('now'))",
            params![uuid::Uuid::new_v4().to_string(), conv_id, "tool", "tool result"],
        ).unwrap();

        let count = mgr.count_tool_call_integrity_issues(&conv_id).unwrap();
        assert_eq!(count, 1);
    }
}
