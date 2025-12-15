// src/migration/mod.rs
// Simple SQLite migration framework for Phase 0.
// It creates a `migrations` table to track applied versions and runs pending migrations.

use crate::settings::Settings;
use rusqlite::{params, Connection, Result as SqlResult};
use tracing::info;

/// Represents a single migration step.
struct Migration {
    version: i32,
    description: &'static str,
    sql: &'static str,
}

/// List of migrations in order.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "Add `metadata` column to messages table",
        sql: "ALTER TABLE messages ADD COLUMN metadata TEXT;",
    },
    Migration {
        version: 2,
        description: "Create `tool_executions` table",
        sql: "CREATE TABLE IF NOT EXISTS tool_executions (
            id TEXT PRIMARY KEY,
            conversation_id TEXT,
            tool_name TEXT,
            server_name TEXT,
            args_json TEXT,
            result_preview TEXT,
            result_full_json TEXT,
            status TEXT,
            created_at TEXT
        );",
    },
];

/// Ensure the `migrations` table exists.
fn ensure_migration_table(conn: &Connection) -> SqlResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS migrations (version INTEGER PRIMARY KEY, applied_at TEXT)",
        [],
    )?;
    Ok(())
}

/// Get the highest applied migration version.
fn current_version(conn: &Connection) -> SqlResult<i32> {
    let mut stmt = conn.prepare("SELECT MAX(version) FROM migrations")?;
    let version: Option<i32> = stmt.query_row([], |row| row.get(0)).optional()?;
    Ok(version.unwrap_or(0))
}

/// Apply a single migration.
fn apply_migration(conn: &Connection, mig: &Migration) -> SqlResult<()> {
    info!(
        "Applying migration",
        version = mig.version,
        description = mig.description
    );
    conn.execute_batch(mig.sql)?;
    conn.execute(
        "INSERT INTO migrations (version, applied_at) VALUES (?1, datetime('now'))",
        params![mig.version],
    )?;
    Ok(())
}

/// Run all pending migrations against the SQLite database used by the app.
/// The database path is derived from the settings' config directory.
pub fn run_pending(settings: &mut Settings) {
    // Determine the DB path (same logic as Librarian::new).
    // For simplicity, we assume the data directory is `app_config_dir`.
    // In production code this would be more robust.
    if let Ok(config_dir) = tauri::api::path::app_config_dir(&tauri::Config::default()) {
        let db_path = config_dir.join("nebula.db");
        if let Ok(conn) = Connection::open(db_path) {
            if let Err(e) = (|| -> SqlResult<()> {
                ensure_migration_table(&conn)?;
                let cur = current_version(&conn)?;
                for mig in MIGRATIONS.iter().filter(|m| m.version > cur) {
                    apply_migration(&conn, mig)?;
                }
                Ok(())
            })() {
                info!("Migration error", error = %e);
            }
        }
    }
}
