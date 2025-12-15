use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct AuditLogger {
    conn: Arc<Mutex<Connection>>,
}

impl AuditLogger {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn log_execution(
        &self,
        conversation_id: &str,
        tool_name: &str,
        server_name: &str,
        args_json: &str,
        result_preview: &str,
        result_full_json: &str,
        status: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let id = uuid::Uuid::new_v4().to_string();

        conn.execute(
            "INSERT INTO tool_executions (
                id, conversation_id, tool_name, server_name, args_json, 
                result_preview, result_full_json, status, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))",
            params![
                id,
                conversation_id,
                tool_name,
                server_name,
                args_json,
                result_preview,
                result_full_json,
                status
            ],
        )?;
        Ok(())
    }
}
