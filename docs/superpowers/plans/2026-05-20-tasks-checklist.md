# Per-Conversation Task Checklist Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give nebula_chat a per-conversation, LLM-managed task checklist with a built-in `update_tasks` tool, SQLite persistence, and a live-updating side panel.

**Architecture:** New `tasks` module holds type definitions and the tool descriptor. Storage extends the existing `SqliteManager` / `Librarian` (single DB, single mutex — same pattern as facts/summaries). `lib.rs::send_message` injects the built-in tool into the LLM tool list and injects the current task list into context; `lib.rs::execute_tool` short-circuits `update_tasks` to the librarian and emits `tasks-updated` to the frontend. New `TasksPanel.tsx` mirrors `MemoryPanel.tsx`.

**Tech Stack:** Rust (Tauri 2, rusqlite, serde, anyhow, tokio), TypeScript / React 19, Vite.

**Spec:** `docs/superpowers/specs/2026-05-20-tasks-checklist-design.md`

---

## File Map

**Created:**
- `src-tauri/src/tasks/mod.rs` — Task types, `format_task_list_for_context`, `build_update_tasks_tool`.
- `src-tauri/tests/tasks_integration_test.rs` — Round-trip test across SqliteManager + Librarian.
- `src/components/TasksPanel.tsx` — New side panel component.

**Modified:**
- `src-tauri/src/memory/sqlite_manager.rs` — Add `migrate_v5()` + `set_tasks_for_conversation()` + `get_tasks_for_conversation()`.
- `src-tauri/src/memory/librarian.rs` — Wire `migrate_v5()` and add pass-through methods.
- `src-tauri/src/mcp/config.rs` — Add `disable_builtin_task_tool` field on `Settings`.
- `src-tauri/src/lib.rs` — Declare `mod tasks`; inject built-in tool + context in `send_message`; short-circuit `update_tasks` in `execute_tool`; add `get_conversation_tasks` command; register both in `invoke_handler`.
- `src/components/ChatInterface.tsx` — Mount `TasksPanel`, add toggle + state, listen for `tasks-updated`.
- `src/components/SettingsPage.tsx` — Add `disable_builtin_task_tool` checkbox.

---

## Task 1: Add `migrate_v5()` for the tasks table

**Files:**
- Modify: `src-tauri/src/memory/sqlite_manager.rs` — add migration method (after `migrate_v4` around line 244).
- Modify: `src-tauri/src/memory/librarian.rs:51` — call `migrate_v5()` during startup.

- [ ] **Step 1: Write the failing test**

Append to the end of `src-tauri/src/memory/sqlite_manager.rs`'s `#[cfg(test)] mod tests` block:

```rust
#[test]
fn test_migrate_v5_creates_tasks_table() {
    let temp = std::env::temp_dir().join(format!("nebula_v5_{}.db", uuid::Uuid::new_v4()));
    let mgr = SqliteManager::new(temp.to_str().unwrap()).unwrap();
    mgr.migrate_v5().unwrap();

    // Insert a row and read it back to prove the schema is correct.
    mgr.conn.execute(
        "INSERT INTO tasks (id, conversation_id, position, content, active_form, status, updated_at) \
         VALUES ('t1','c1',0,'do thing','doing thing','pending','2026-05-20T00:00:00Z')",
        [],
    ).unwrap();

    let count: i64 = mgr.conn
        .query_row("SELECT COUNT(*) FROM tasks WHERE conversation_id='c1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tauri-appnebula --lib memory::sqlite_manager::tests::test_migrate_v5_creates_tasks_table`
Expected: FAIL (`migrate_v5` does not exist).

- [ ] **Step 3: Add `migrate_v5()` to `SqliteManager`**

Insert immediately after `migrate_v4()` (around line 244 of `src-tauri/src/memory/sqlite_manager.rs`):

```rust
pub fn migrate_v5(&self) -> Result<()> {
    self.conn.execute(
        "CREATE TABLE IF NOT EXISTS tasks (
            id              TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            position        INTEGER NOT NULL,
            content         TEXT NOT NULL,
            active_form     TEXT NOT NULL,
            status          TEXT NOT NULL CHECK(status IN ('pending','in_progress','completed')),
            updated_at      TEXT NOT NULL,
            FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
        )",
        [],
    )?;
    self.conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tasks_conv ON tasks(conversation_id, position)",
        [],
    )?;
    Ok(())
}
```

- [ ] **Step 4: Wire migrate_v5 into Librarian startup**

In `src-tauri/src/memory/librarian.rs` after line 51 (`let _ = sqlite.migrate_v4();`), add:

```rust
        let _ = sqlite.migrate_v5();
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p tauri-appnebula --lib memory::sqlite_manager::tests::test_migrate_v5_creates_tasks_table`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/memory/sqlite_manager.rs src-tauri/src/memory/librarian.rs
git commit -m "Add migrate_v5 creating per-conversation tasks table"
```

---

## Task 2: SqliteManager task CRUD with replace-all semantics

**Files:**
- Modify: `src-tauri/src/memory/sqlite_manager.rs` — add `TaskRow` struct + `set_tasks_for_conversation` + `get_tasks_for_conversation`.

- [ ] **Step 1: Write the failing tests**

Append to the same `#[cfg(test)] mod tests` block:

```rust
fn fresh_mgr() -> SqliteManager {
    let temp = std::env::temp_dir().join(format!("nebula_tasks_{}.db", uuid::Uuid::new_v4()));
    let mgr = SqliteManager::new(temp.to_str().unwrap()).unwrap();
    mgr.migrate_v5().unwrap();
    mgr.conn.execute(
        "INSERT INTO conversations (id, title, created_at) VALUES ('c1','Conv 1','2026-05-20T00:00:00Z'),\
         ('c2','Conv 2','2026-05-20T00:00:00Z')",
        [],
    ).unwrap();
    mgr
}

#[test]
fn test_set_and_get_tasks_round_trip() {
    let mgr = fresh_mgr();
    let input = vec![
        crate::memory::sqlite_manager::TaskRow {
            content: "Look up data model".into(),
            active_form: "Looking up data model".into(),
            status: "completed".into(),
        },
        crate::memory::sqlite_manager::TaskRow {
            content: "Build the parser".into(),
            active_form: "Building the parser".into(),
            status: "in_progress".into(),
        },
    ];
    mgr.set_tasks_for_conversation("c1", &input).unwrap();

    let got = mgr.get_tasks_for_conversation("c1").unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].content, "Look up data model");
    assert_eq!(got[0].position, 0);
    assert_eq!(got[1].status, "in_progress");
    assert_eq!(got[1].position, 1);
}

#[test]
fn test_set_tasks_replaces_existing() {
    let mgr = fresh_mgr();
    let initial = vec![crate::memory::sqlite_manager::TaskRow {
        content: "Old A".into(), active_form: "Doing old A".into(), status: "pending".into(),
    }, crate::memory::sqlite_manager::TaskRow {
        content: "Old B".into(), active_form: "Doing old B".into(), status: "pending".into(),
    }];
    mgr.set_tasks_for_conversation("c1", &initial).unwrap();

    let replacement = vec![crate::memory::sqlite_manager::TaskRow {
        content: "New only".into(), active_form: "Doing new".into(), status: "in_progress".into(),
    }];
    mgr.set_tasks_for_conversation("c1", &replacement).unwrap();

    let got = mgr.get_tasks_for_conversation("c1").unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].content, "New only");
}

#[test]
fn test_set_tasks_conversation_isolation() {
    let mgr = fresh_mgr();
    mgr.set_tasks_for_conversation("c1", &vec![crate::memory::sqlite_manager::TaskRow {
        content: "A".into(), active_form: "Doing A".into(), status: "pending".into(),
    }]).unwrap();
    mgr.set_tasks_for_conversation("c2", &vec![crate::memory::sqlite_manager::TaskRow {
        content: "B".into(), active_form: "Doing B".into(), status: "pending".into(),
    }, crate::memory::sqlite_manager::TaskRow {
        content: "C".into(), active_form: "Doing C".into(), status: "pending".into(),
    }]).unwrap();

    assert_eq!(mgr.get_tasks_for_conversation("c1").unwrap().len(), 1);
    assert_eq!(mgr.get_tasks_for_conversation("c2").unwrap().len(), 2);
}

#[test]
fn test_set_tasks_empty_clears_list() {
    let mgr = fresh_mgr();
    mgr.set_tasks_for_conversation("c1", &vec![crate::memory::sqlite_manager::TaskRow {
        content: "Item".into(), active_form: "Doing item".into(), status: "pending".into(),
    }]).unwrap();
    mgr.set_tasks_for_conversation("c1", &vec![]).unwrap();
    assert!(mgr.get_tasks_for_conversation("c1").unwrap().is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tauri-appnebula --lib memory::sqlite_manager::tests::test_set 2>&1 | tail -20`
Expected: FAIL (types `TaskRow` and methods `set_tasks_for_conversation` / `get_tasks_for_conversation` do not exist).

- [ ] **Step 3: Add the input and output types + methods**

Insert near the top of `src-tauri/src/memory/sqlite_manager.rs` (right after the existing `use` imports, before `pub struct SqliteManager`):

```rust
/// Plain input row for replacing a conversation's task list.
#[derive(Debug, Clone)]
pub struct TaskRow {
    pub content: String,
    pub active_form: String,
    pub status: String, // "pending" | "in_progress" | "completed"
}

/// Persisted task as returned to callers.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PersistedTask {
    pub id: String,
    pub conversation_id: String,
    pub position: i32,
    pub content: String,
    pub active_form: String,
    pub status: String,
    pub updated_at: String,
}
```

Then, inside `impl SqliteManager`, add (placement: alongside the other write methods, e.g. after `save_message_with_timestamp`):

```rust
/// Replace the entire task list for a conversation in one transaction.
pub fn set_tasks_for_conversation(
    &self,
    conversation_id: &str,
    tasks: &[TaskRow],
) -> Result<Vec<PersistedTask>> {
    let now = chrono::Utc::now().to_rfc3339();

    self.conn.execute("BEGIN", [])?;

    let delete_result = self.conn.execute(
        "DELETE FROM tasks WHERE conversation_id = ?1",
        rusqlite::params![conversation_id],
    );
    if let Err(e) = delete_result {
        let _ = self.conn.execute("ROLLBACK", []);
        return Err(e.into());
    }

    let mut out = Vec::with_capacity(tasks.len());
    for (i, t) in tasks.iter().enumerate() {
        let id = uuid::Uuid::new_v4().to_string();
        let insert_result = self.conn.execute(
            "INSERT INTO tasks (id, conversation_id, position, content, active_form, status, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                id,
                conversation_id,
                i as i32,
                t.content,
                t.active_form,
                t.status,
                now,
            ],
        );
        if let Err(e) = insert_result {
            let _ = self.conn.execute("ROLLBACK", []);
            return Err(e.into());
        }
        out.push(PersistedTask {
            id,
            conversation_id: conversation_id.to_string(),
            position: i as i32,
            content: t.content.clone(),
            active_form: t.active_form.clone(),
            status: t.status.clone(),
            updated_at: now.clone(),
        });
    }

    self.conn.execute("COMMIT", [])?;
    Ok(out)
}

pub fn get_tasks_for_conversation(&self, conversation_id: &str) -> Result<Vec<PersistedTask>> {
    let mut stmt = self.conn.prepare(
        "SELECT id, conversation_id, position, content, active_form, status, updated_at \
         FROM tasks WHERE conversation_id = ?1 ORDER BY position ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![conversation_id], |r| {
        Ok(PersistedTask {
            id: r.get(0)?,
            conversation_id: r.get(1)?,
            position: r.get(2)?,
            content: r.get(3)?,
            active_form: r.get(4)?,
            status: r.get(5)?,
            updated_at: r.get(6)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tauri-appnebula --lib memory::sqlite_manager::tests::test_set 2>&1 | tail -20`
Expected: all four PASS.

- [ ] **Step 5: Run full SqliteManager test suite to confirm no regressions**

Run: `cargo test -p tauri-appnebula --lib memory::sqlite_manager`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/memory/sqlite_manager.rs
git commit -m "Add SqliteManager task CRUD with replace-all semantics"
```

---

## Task 3: Create `tasks` module — types, context formatter, tool descriptor

**Files:**
- Create: `src-tauri/src/tasks/mod.rs`.
- Modify: `src-tauri/src/lib.rs` near line 24 — add `pub mod tasks;`.

- [ ] **Step 1: Create the module with types and formatter**

Create `src-tauri/src/tasks/mod.rs`:

```rust
use crate::llm::provider::ToolDefinition;
use crate::memory::sqlite_manager::PersistedTask;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInput {
    pub content: String,
    pub active_form: String,
    pub status: String, // validated against pending/in_progress/completed
}

/// Build a markdown checklist for injection into the model's system context.
/// Returns None if the list is empty.
pub fn format_task_list_for_context(tasks: &[PersistedTask]) -> Option<String> {
    if tasks.is_empty() {
        return None;
    }
    let mut out = String::from("Current task checklist:\n");
    for t in tasks {
        let (marker, text) = match t.status.as_str() {
            "completed" => ("[\u{2713}]", t.content.as_str()),
            "in_progress" => ("[\u{2192}]", t.active_form.as_str()),
            _ => ("[ ]", t.content.as_str()),
        };
        out.push_str(&format!("- {} {}\n", marker, text));
    }
    Some(out)
}

/// Construct the LLM-facing `update_tasks` tool descriptor.
pub fn build_update_tasks_tool() -> ToolDefinition {
    ToolDefinition {
        name: "update_tasks".to_string(),
        description: "Update the user-visible task checklist for the current conversation. \
            Pass the COMPLETE updated list each time — this call REPLACES the entire list. \
            Use this tool when working on multi-step requests so the user can see your plan \
            and your progress. Mark exactly one task as `in_progress` at a time. Mark a task \
            `completed` the moment it is done; do not batch completions. \
            `content` is the imperative form (e.g. 'Build the parser'); \
            `active_form` is the present-progressive form (e.g. 'Building the parser')."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "description": "The complete updated task list, in display order.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "Imperative form, e.g. 'Build the parser'."
                            },
                            "active_form": {
                                "type": "string",
                                "description": "Present-progressive form, e.g. 'Building the parser'."
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"]
                            }
                        },
                        "required": ["content", "active_form", "status"]
                    }
                }
            },
            "required": ["tasks"]
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::sqlite_manager::PersistedTask;

    fn t(content: &str, active: &str, status: &str) -> PersistedTask {
        PersistedTask {
            id: "x".into(),
            conversation_id: "c".into(),
            position: 0,
            content: content.into(),
            active_form: active.into(),
            status: status.into(),
            updated_at: "2026-05-20T00:00:00Z".into(),
        }
    }

    #[test]
    fn empty_list_returns_none() {
        assert!(format_task_list_for_context(&[]).is_none());
    }

    #[test]
    fn formatter_uses_active_form_for_in_progress_and_content_otherwise() {
        let tasks = vec![
            t("Look up data model", "Looking up data model", "completed"),
            t("Build the parser", "Building the parser", "in_progress"),
            t("Wire the UI", "Wiring the UI", "pending"),
        ];
        let out = format_task_list_for_context(&tasks).unwrap();
        assert!(out.contains("[\u{2713}] Look up data model"));
        assert!(out.contains("[\u{2192}] Building the parser"));
        assert!(out.contains("[ ] Wire the UI"));
        // No bleed-through of the wrong form.
        assert!(!out.contains("Looking up data model"));
        assert!(!out.contains("Build the parser\n"));
    }

    #[test]
    fn tool_descriptor_has_required_shape() {
        let tool = build_update_tasks_tool();
        assert_eq!(tool.name, "update_tasks");
        let schema = &tool.input_schema;
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["tasks"]["items"]["properties"]["status"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "in_progress"));
    }
}
```

- [ ] **Step 2: Register the module in lib.rs**

In `src-tauri/src/lib.rs` near line 24, add `pub mod tasks;` next to the other `pub mod` lines. The block becomes:

```rust
pub mod llm;
pub use llm::capabilities;
pub mod mcp;
pub mod memory;
pub mod security;
pub mod tasks;
```

- [ ] **Step 3: Run the new tests**

Run: `cargo test -p tauri-appnebula --lib tasks::tests`
Expected: 3 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/tasks/mod.rs src-tauri/src/lib.rs
git commit -m "Add tasks module: types, context formatter, and update_tasks tool descriptor"
```

---

## Task 4: Librarian pass-throughs for tasks

**Files:**
- Modify: `src-tauri/src/memory/librarian.rs` — add `set_tasks`, `get_tasks`, `format_tasks_for_context` methods.

- [ ] **Step 1: Write the failing test**

Append to the `#[cfg(test)] mod tests` block in `src-tauri/src/memory/librarian.rs` (if no `tests` block exists, create one at the end of the file using the same pattern as `sqlite_manager.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::sqlite_manager::TaskRow;

    fn fresh_lib() -> Librarian {
        let dir = std::env::temp_dir().join(format!("nebula_lib_tasks_{}", uuid::Uuid::new_v4()));
        let lib = Librarian::new(&dir).unwrap();
        lib.sqlite.conn.execute(
            "INSERT INTO conversations (id, title, created_at) VALUES ('c1','C','2026-05-20T00:00:00Z')",
            [],
        ).unwrap();
        lib
    }

    #[test]
    fn librarian_set_and_get_tasks_round_trip() {
        let lib = fresh_lib();
        let saved = lib.set_tasks("c1", &[
            TaskRow { content: "A".into(), active_form: "Doing A".into(), status: "pending".into() },
            TaskRow { content: "B".into(), active_form: "Doing B".into(), status: "in_progress".into() },
        ]).unwrap();
        assert_eq!(saved.len(), 2);

        let got = lib.get_tasks("c1").unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].status, "in_progress");
    }

    #[test]
    fn librarian_format_tasks_for_context_returns_none_when_empty() {
        let lib = fresh_lib();
        assert!(lib.format_tasks_for_context("c1").unwrap().is_none());
    }

    #[test]
    fn librarian_format_tasks_for_context_returns_markdown() {
        let lib = fresh_lib();
        lib.set_tasks("c1", &[TaskRow {
            content: "Ship it".into(), active_form: "Shipping it".into(), status: "completed".into(),
        }]).unwrap();
        let s = lib.format_tasks_for_context("c1").unwrap().unwrap();
        assert!(s.contains("Ship it"));
        assert!(s.contains("[\u{2713}]"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tauri-appnebula --lib memory::librarian::tests::librarian_ 2>&1 | tail -20`
Expected: FAIL (methods `set_tasks`, `get_tasks`, `format_tasks_for_context` do not exist on `Librarian`).

- [ ] **Step 3: Add Librarian pass-through methods**

In `src-tauri/src/memory/librarian.rs`, inside `impl Librarian`, add (placement: alongside the other persistence methods, e.g. after the existing message-save methods):

```rust
/// Replace the entire task list for a conversation.
pub fn set_tasks(
    &self,
    conversation_id: &str,
    tasks: &[crate::memory::sqlite_manager::TaskRow],
) -> anyhow::Result<Vec<crate::memory::sqlite_manager::PersistedTask>> {
    self.sqlite.set_tasks_for_conversation(conversation_id, tasks)
}

pub fn get_tasks(
    &self,
    conversation_id: &str,
) -> anyhow::Result<Vec<crate::memory::sqlite_manager::PersistedTask>> {
    self.sqlite.get_tasks_for_conversation(conversation_id)
}

/// Returns the markdown task list for context injection, or None if empty.
pub fn format_tasks_for_context(
    &self,
    conversation_id: &str,
) -> anyhow::Result<Option<String>> {
    let tasks = self.sqlite.get_tasks_for_conversation(conversation_id)?;
    Ok(crate::tasks::format_task_list_for_context(&tasks))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tauri-appnebula --lib memory::librarian::tests::librarian_`
Expected: 3 PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/memory/librarian.rs
git commit -m "Add Librarian pass-through methods for tasks"
```

---

## Task 5: `disable_builtin_task_tool` setting field

**Files:**
- Modify: `src-tauri/src/mcp/config.rs` — add field on `Settings`.

- [ ] **Step 1: Write the failing test**

Append to the existing `#[cfg(test)] mod tests` block in `src-tauri/src/mcp/config.rs`:

```rust
#[test]
fn disable_builtin_task_tool_defaults_false() {
    let s = Settings::default();
    assert!(!s.disable_builtin_task_tool);
}

#[test]
fn disable_builtin_task_tool_round_trips_via_serde() {
    let s = Settings {
        disable_builtin_task_tool: true,
        ..Settings::default()
    };
    let json = serde_json::to_string(&s).unwrap();
    let back: Settings = serde_json::from_str(&json).unwrap();
    assert!(back.disable_builtin_task_tool);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tauri-appnebula --lib mcp::config::tests::disable_builtin_task_tool 2>&1 | tail -10`
Expected: FAIL — field does not exist.

- [ ] **Step 3: Add the field**

In `src-tauri/src/mcp/config.rs`, inside the `Settings` struct, add (place it near other behavioral toggles such as `context_inspection_enabled` around line 140):

```rust
    /// When true, the built-in `update_tasks` tool is hidden from the LLM.
    #[serde(default = "default_false_bool")]
    pub disable_builtin_task_tool: bool,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tauri-appnebula --lib mcp::config::tests::disable_builtin_task_tool`
Expected: 2 PASS.

- [ ] **Step 5: Run full settings test suite to confirm no regression in defaults / migrations**

Run: `cargo test -p tauri-appnebula --lib mcp::config`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/mcp/config.rs
git commit -m "Add disable_builtin_task_tool setting (default false)"
```

---

## Task 6: Wire built-in tool + context injection into `send_message`

**Files:**
- Modify: `src-tauri/src/lib.rs` — in `send_message`, after `state.mcp_manager.get_all_tools().await` (around line 798) and where context messages are assembled.

This task is structural — no easy unit test for the full `send_message` path. We rely on the upstream unit tests (Task 3, Task 4) for the helpers and verify integration manually in Task 11.

- [ ] **Step 1: Read current state of `send_message` tool list assembly**

Run: `grep -n "get_all_tools\|disabled_tools" src-tauri/src/lib.rs | head`
Expected output should show the existing tool-list filter near line 798–848.

- [ ] **Step 2: Inject the built-in tool into the tool list**

In `src-tauri/src/lib.rs`, find the block (currently near line 798–848):

```rust
        tracing::debug!("[DEBUG] Getting tools from MCP Manager...");
        let all_tools = state.mcp_manager.get_all_tools().await;

        let tools: Vec<_> = all_tools
            .into_iter()
            .filter(|t| !settings.disabled_tools.contains(&t.name))
            .collect();
        tracing::debug!("[DEBUG] Final tool count: {}", tools.len());
```

Replace with:

```rust
        tracing::debug!("[DEBUG] Getting tools from MCP Manager...");
        let all_tools = state.mcp_manager.get_all_tools().await;

        let mut tools: Vec<_> = all_tools
            .into_iter()
            .filter(|t| !settings.disabled_tools.contains(&t.name))
            .collect();

        // Inject the built-in update_tasks tool unless the user has disabled it.
        if !settings.disable_builtin_task_tool {
            tools.push(crate::tasks::build_update_tasks_tool());
        }

        tracing::debug!("[DEBUG] Final tool count: {}", tools.len());
```

- [ ] **Step 3: Inject the task-context system message**

Locate the existing context-injection block in `send_message` that handles `context_text` (currently a `system` message reading "You have access to the following long-term memories:..."). Immediately after that block, add a parallel block for tasks. Specifically, find:

```rust
        if !context_text.is_empty() {
            let context_msg = Message {
                id: None,
                role: "system".to_string(),
                content: Some(format!(
                    "You have access to the following long-term memories:\n{}",
                    context_text
                )),
                reasoning_content: None,
                tool_calls: None,
                tool_call_id: None,
                attachments: None,
                created_at: None,
            };
            final_messages.insert(0, context_msg);
        }
```

Immediately after that closing brace, add:

```rust
        // Inject current task checklist (if any) so the model knows what's pending vs. done.
        let task_context = {
            let lib = state.librarian.lock().await;
            lib.format_tasks_for_context(&conversation_id)
                .unwrap_or(None)
        };
        if let Some(text) = task_context {
            let task_msg = Message {
                id: None,
                role: "system".to_string(),
                content: Some(text),
                reasoning_content: None,
                tool_calls: None,
                tool_call_id: None,
                attachments: None,
                created_at: None,
            };
            final_messages.insert(0, task_msg);
        }
```

NOTE: `conversation_id` here refers to the parameter of `send_message`. Verify its exact name (`conversation_id` or `conv_id`) by checking the function signature at the top of `send_message` and adjust if needed.

- [ ] **Step 4: Build and verify the project compiles**

Run: `cargo build -p tauri-appnebula 2>&1 | tail -15`
Expected: `Finished` with no errors.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "Inject update_tasks tool and task-context into send_message"
```

---

## Task 7: `execute_tool` short-circuit for `update_tasks`

**Files:**
- Modify: `src-tauri/src/lib.rs` — `execute_tool` function around line 1263.

- [ ] **Step 1: Add the short-circuit at the top of `execute_tool`**

In `src-tauri/src/lib.rs`, inside `execute_tool` (starts around line 1263), immediately after `let settings = Settings::load_migrated(&settings_path);` (line ~1274), insert:

```rust
    // Short-circuit: the built-in `update_tasks` tool is handled in-process,
    // bypasses MCP routing, and bypasses the per-tool approval flow because
    // it has no external side effects — it only updates local UI state.
    if name == "update_tasks" {
        use tauri::Emitter;

        // Parse arguments.
        let tasks_arr = args
            .get("tasks")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "update_tasks: missing 'tasks' array".to_string())?;

        let mut rows = Vec::with_capacity(tasks_arr.len());
        for t in tasks_arr {
            let content = t.get("content").and_then(|v| v.as_str())
                .ok_or_else(|| "update_tasks: task missing 'content'".to_string())?
                .to_string();
            let active_form = t.get("active_form").and_then(|v| v.as_str())
                .ok_or_else(|| "update_tasks: task missing 'active_form'".to_string())?
                .to_string();
            let status = t.get("status").and_then(|v| v.as_str())
                .ok_or_else(|| "update_tasks: task missing 'status'".to_string())?
                .to_string();
            if !["pending", "in_progress", "completed"].contains(&status.as_str()) {
                return Err(format!("update_tasks: invalid status '{}'", status));
            }
            rows.push(crate::memory::sqlite_manager::TaskRow { content, active_form, status });
        }

        // Require a conversation_id — tasks are scoped to a conversation.
        let cid = conversation_id
            .as_ref()
            .ok_or_else(|| "update_tasks: conversation_id is required".to_string())?
            .clone();

        let saved = {
            let lib = state.librarian.lock().await;
            lib.set_tasks(&cid, &rows).map_err(|e| e.to_string())?
        };

        // Notify the frontend.
        let _ = app.emit(
            "tasks-updated",
            serde_json::json!({ "conversation_id": cid, "tasks": saved }),
        );

        return Ok(serde_json::json!({
            "ok": true,
            "count": rows.len(),
            "message": "Task list updated."
        }));
    }
```

- [ ] **Step 2: Build to verify compile**

Run: `cargo build -p tauri-appnebula 2>&1 | tail -10`
Expected: `Finished` with no errors.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "Short-circuit update_tasks in execute_tool (auto-approved, emits tasks-updated)"
```

---

## Task 8: `get_conversation_tasks` Tauri command

**Files:**
- Modify: `src-tauri/src/lib.rs` — add command + register in `invoke_handler`.

- [ ] **Step 1: Add the command**

In `src-tauri/src/lib.rs`, alongside the other Tauri command definitions (e.g. near `get_tool_execution` around line 1359), add:

```rust
#[tauri::command]
async fn get_conversation_tasks(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<crate::memory::sqlite_manager::PersistedTask>, String> {
    let lib = state.librarian.lock().await;
    lib.get_tasks(&conversation_id).map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Register the command in `invoke_handler`**

In `src-tauri/src/lib.rs` (around line 2540 in the `tauri::generate_handler![...]` macro call), add `get_conversation_tasks,` to the list. For example, near `execute_tool, get_tool_execution,`:

```rust
            execute_tool,
            get_tool_execution,
            get_conversation_tasks,
```

- [ ] **Step 3: Build**

Run: `cargo build -p tauri-appnebula 2>&1 | tail -10`
Expected: `Finished` with no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "Add get_conversation_tasks Tauri command"
```

---

## Task 9: Integration test — full round-trip via Librarian

**Files:**
- Create: `src-tauri/tests/tasks_integration_test.rs`.

This test exercises the same code path `execute_tool` runs (parse → Librarian::set_tasks → Librarian::get_tasks → format), without the Tauri shell.

- [ ] **Step 1: Write the integration test**

Create `src-tauri/tests/tasks_integration_test.rs`:

```rust
use tauri_appnebula_lib::memory::librarian::Librarian;
use tauri_appnebula_lib::memory::sqlite_manager::TaskRow;

#[test]
fn update_tasks_round_trip_writes_and_formats() {
    let dir = std::env::temp_dir().join(format!(
        "nebula_tasks_int_{}",
        uuid::Uuid::new_v4()
    ));
    let lib = Librarian::new(&dir).unwrap();
    lib.sqlite
        .conn
        .execute(
            "INSERT INTO conversations (id, title, created_at) VALUES ('conv-1','Test','2026-05-20T00:00:00Z')",
            [],
        )
        .unwrap();

    // First update: 3 tasks.
    let initial = vec![
        TaskRow {
            content: "Look up the data model".into(),
            active_form: "Looking up the data model".into(),
            status: "completed".into(),
        },
        TaskRow {
            content: "Build the parser".into(),
            active_form: "Building the parser".into(),
            status: "in_progress".into(),
        },
        TaskRow {
            content: "Wire up the UI".into(),
            active_form: "Wiring up the UI".into(),
            status: "pending".into(),
        },
    ];
    let saved = lib.set_tasks("conv-1", &initial).unwrap();
    assert_eq!(saved.len(), 3);

    // Read back via Librarian — positions correct, status preserved.
    let got = lib.get_tasks("conv-1").unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(got[1].status, "in_progress");
    assert_eq!(got[2].position, 2);

    // Context formatter uses active_form for the in-progress row.
    let ctx = lib.format_tasks_for_context("conv-1").unwrap().unwrap();
    assert!(ctx.contains("Building the parser"));
    assert!(ctx.contains("[\u{2713}] Look up the data model"));
    assert!(ctx.contains("[ ] Wire up the UI"));

    // Second update: replace-all.
    let replacement = vec![TaskRow {
        content: "All done".into(),
        active_form: "Doing all done".into(),
        status: "completed".into(),
    }];
    lib.set_tasks("conv-1", &replacement).unwrap();
    let got2 = lib.get_tasks("conv-1").unwrap();
    assert_eq!(got2.len(), 1);
    assert_eq!(got2[0].content, "All done");
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p tauri-appnebula --test tasks_integration_test`
Expected: 1 test PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/tests/tasks_integration_test.rs
git commit -m "Add integration test: update_tasks round-trip via Librarian"
```

---

## Task 10: TasksPanel.tsx frontend component

**Files:**
- Create: `src/components/TasksPanel.tsx`.

- [ ] **Step 1: Create the component**

Create `src/components/TasksPanel.tsx`:

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface PersistedTask {
    id: string;
    conversation_id: string;
    position: number;
    content: string;
    active_form: string;
    status: "pending" | "in_progress" | "completed";
    updated_at: string;
}

interface TasksUpdatedPayload {
    conversation_id: string;
    tasks: PersistedTask[];
}

interface TasksPanelProps {
    conversationId: string | null;
    onClose: () => void;
}

export default function TasksPanel({ conversationId, onClose }: TasksPanelProps) {
    const [tasks, setTasks] = useState<PersistedTask[]>([]);

    // Load when conversation changes.
    useEffect(() => {
        if (!conversationId) {
            setTasks([]);
            return;
        }
        let cancelled = false;
        invoke<PersistedTask[]>("get_conversation_tasks", { conversationId })
            .then((rows) => {
                if (!cancelled) setTasks(rows);
            })
            .catch((e) => console.error("get_conversation_tasks failed", e));
        return () => {
            cancelled = true;
        };
    }, [conversationId]);

    // Live updates from the backend.
    useEffect(() => {
        const unlistenPromise = listen<TasksUpdatedPayload>("tasks-updated", (event) => {
            if (event.payload.conversation_id === conversationId) {
                setTasks(event.payload.tasks);
            }
        });
        return () => {
            unlistenPromise.then((unlisten) => unlisten());
        };
    }, [conversationId]);

    return (
        <div className="tasks-panel">
            <div className="tasks-panel-header">
                <span>Tasks</span>
                <button onClick={onClose} aria-label="Close tasks panel">×</button>
            </div>
            {tasks.length === 0 ? (
                <div className="tasks-empty">No tasks yet for this conversation.</div>
            ) : (
                <ul className="tasks-list">
                    {tasks.map((t) => {
                        const marker =
                            t.status === "completed" ? "✓" : t.status === "in_progress" ? "▶" : "☐";
                        const text = t.status === "in_progress" ? t.active_form : t.content;
                        const className = `task-item task-${t.status}`;
                        return (
                            <li key={t.id} className={className}>
                                <span className="task-marker" aria-hidden="true">
                                    {marker}
                                </span>
                                <span className="task-text">{text}</span>
                            </li>
                        );
                    })}
                </ul>
            )}
        </div>
    );
}
```

- [ ] **Step 2: Add minimal CSS for the panel**

Append to whichever stylesheet `MemoryPanel` already uses (most likely `src/App.css` — confirm via `grep -rn "memory-panel" src/`). Add:

```css
.tasks-panel {
    display: flex;
    flex-direction: column;
    min-width: 280px;
    max-width: 360px;
    border-left: 1px solid var(--border-color, #ddd);
    background: var(--panel-bg, #fafafa);
    overflow-y: auto;
}
.tasks-panel-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 12px;
    font-weight: 600;
    border-bottom: 1px solid var(--border-color, #ddd);
}
.tasks-empty {
    padding: 16px;
    color: var(--muted-fg, #888);
    font-size: 0.9em;
}
.tasks-list {
    list-style: none;
    margin: 0;
    padding: 0;
}
.task-item {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border-color-subtle, #eee);
}
.task-marker {
    font-family: var(--mono-font, monospace);
    width: 1.2em;
    text-align: center;
}
.task-completed .task-text {
    text-decoration: line-through;
    opacity: 0.6;
}
.task-in_progress {
    font-weight: 600;
}
```

If `MemoryPanel`'s styles live in a different file, place the CSS there instead.

- [ ] **Step 3: Type-check the frontend**

Run: `npm run build 2>&1 | tail -10`
Expected: build succeeds (Vite + TS).

- [ ] **Step 4: Commit**

```bash
git add src/components/TasksPanel.tsx src/App.css
git commit -m "Add TasksPanel React component"
```

(If you added CSS to a different file, adjust the `git add` accordingly.)

---

## Task 11: Mount TasksPanel inside ChatInterface

**Files:**
- Modify: `src/components/ChatInterface.tsx` — import, state, toggle button, render block.

- [ ] **Step 1: Locate the existing MemoryPanel mount**

Run: `grep -n "MemoryPanel" src/components/ChatInterface.tsx`
Expected output: an import line near the top, and a render block around line 1523.

- [ ] **Step 2: Add the import**

In `src/components/ChatInterface.tsx`, near the existing `import MemoryPanel from "./MemoryPanel";` line, add:

```tsx
import TasksPanel from "./TasksPanel";
```

- [ ] **Step 3: Add toggle state alongside the existing panel-toggle states**

In `ChatInterface`'s function body, near the other `useState` calls for panel visibility (search for `showMemory` or similar), add:

```tsx
const [showTasks, setShowTasks] = useState<boolean>(() => {
    return localStorage.getItem("nebula.showTasks") === "1";
});
useEffect(() => {
    localStorage.setItem("nebula.showTasks", showTasks ? "1" : "0");
}, [showTasks]);
```

- [ ] **Step 4: Add the toggle button to the chat header**

Locate the existing toggle buttons that show/hide `MemoryPanel` (likely in a header bar near the top of the chat layout). Next to the memory toggle, add a Tasks toggle:

```tsx
<button
    type="button"
    onClick={() => setShowTasks((v) => !v)}
    title={showTasks ? "Hide tasks" : "Show tasks"}
    aria-pressed={showTasks}
>
    ☑ Tasks
</button>
```

Match the styling/class names used by the existing memory toggle button for consistency.

- [ ] **Step 5: Render TasksPanel**

Where `MemoryPanel` is currently rendered (around line 1523), add a sibling render for `TasksPanel`. The exact structure depends on the existing layout — match the pattern. Example:

```tsx
{showTasks && (
    <TasksPanel
        conversationId={currentConversationId}
        onClose={() => setShowTasks(false)}
    />
)}
```

`currentConversationId` should be whichever variable holds the active conversation's ID in this component (often `conversationId`, `activeConversationId`, or similar — match what `MemoryPanel`'s sibling code uses).

- [ ] **Step 6: Type-check**

Run: `npm run build 2>&1 | tail -10`
Expected: build succeeds.

- [ ] **Step 7: Commit**

```bash
git add src/components/ChatInterface.tsx
git commit -m "Mount TasksPanel in ChatInterface with toggle + persisted visibility"
```

---

## Task 12: Settings checkbox for `disable_builtin_task_tool`

**Files:**
- Modify: `src/components/SettingsPage.tsx` — add a checkbox bound to the setting.

- [ ] **Step 1: Locate a suitable section in SettingsPage**

Run: `grep -n "context_inspection_enabled\|disabled_tools\|memory_enabled" src/components/SettingsPage.tsx | head`
This finds where existing behavioral toggles are rendered. Pick the most appropriate section — likely the same group as `context_inspection_enabled` or under a Tools heading.

- [ ] **Step 2: Add the checkbox**

In `src/components/SettingsPage.tsx`, near an existing similar toggle, add (adjust the JSX wrappers to match the surrounding code's style):

```tsx
<label className="setting-row">
    <input
        type="checkbox"
        checked={!!settings.disable_builtin_task_tool}
        onChange={(e) =>
            setSettings({
                ...settings,
                disable_builtin_task_tool: e.target.checked,
            })
        }
    />
    <span>Disable built-in task checklist tool</span>
    <small>Hides the <code>update_tasks</code> tool from the model. The TasksPanel keeps any tasks already saved.</small>
</label>
```

If the local `Settings` type in TypeScript is defined explicitly (not derived from a backend type), add `disable_builtin_task_tool?: boolean;` to that interface. Search for the TS settings type with: `grep -n "interface Settings\|type Settings" src/`.

- [ ] **Step 3: Type-check**

Run: `npm run build 2>&1 | tail -10`
Expected: build succeeds.

- [ ] **Step 4: Commit**

```bash
git add src/components/SettingsPage.tsx
git commit -m "Add settings checkbox to disable built-in task tool"
```

(Include any frontend Settings type file in the `git add` if you edited one.)

---

## Task 13: End-to-end manual verification

**Files:** none modified.

This task walks through the acceptance criteria from the spec.

- [ ] **Step 1: Launch the dev app**

Run: `npm run tauri dev`
Expected: Tauri window opens.

- [ ] **Step 2: Trigger a multi-step LLM request**

Start a new conversation. Send: *"Plan and execute a 4-step refactor of a small Python script: 1) read it, 2) propose changes, 3) apply them, 4) verify. Use the update_tasks tool to track progress."*

Expected:
- TasksPanel (toggled on) populates with 4 items.
- Exactly one item shows status `in_progress` at a time.
- Items mark `completed` as the model progresses.

- [ ] **Step 3: Reopen the conversation**

Close and reopen the conversation from the sidebar.

Expected: The same task list is restored from SQLite.

- [ ] **Step 4: Conversation isolation**

Open a new conversation.

Expected: TasksPanel is empty. Tasks from the previous conversation are NOT visible.

- [ ] **Step 5: Model sees state on a second turn**

Within the first conversation, ask: *"Which tasks are still pending?"*

Expected: The model answers based on the injected task-context (proves Task 6's context injection works).

- [ ] **Step 6: Disable toggle**

In Settings, enable "Disable built-in task checklist tool". Send a fresh message that would normally trigger the tool.

Expected: The model does not call `update_tasks` (it isn't in the tool list). Existing tasks remain visible in the panel; only writes are gated.

- [ ] **Step 7: Re-enable toggle**

Uncheck the setting. Confirm the model can call `update_tasks` again.

- [ ] **Step 8: Commit (no code, but tag completion)**

Optional — only if you want a marker commit:

```bash
git commit --allow-empty -m "Task checklist feature: manual verification complete"
```

---

## Done

The feature is implemented and verified. Re-read the spec at `docs/superpowers/specs/2026-05-20-tasks-checklist-design.md` and confirm each acceptance-criterion item is satisfied.
