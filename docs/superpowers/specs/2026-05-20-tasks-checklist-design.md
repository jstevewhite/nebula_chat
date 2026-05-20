# Per-Conversation Task Checklist

**Status:** Design approved, awaiting implementation plan
**Date:** 2026-05-20
**Author:** Brainstormed with Claude

## Motivation

Nebula currently has no way to surface the LLM's multi-step plan to the user. When a model works on a request that decomposes into several sub-tasks, the user can only infer progress from the streaming response itself. A visible, live-updating checklist makes the model's plan legible, gives the user confidence the model is on track, and lets the model "check off" sub-tasks as it completes them — the proven Claude Code TodoWrite pattern.

## Goals

- LLM can publish a checklist of tasks for the current conversation.
- User sees the list update in real time as the model marks items in progress / completed.
- List is persisted per-conversation in SQLite and restored when the user reopens the chat.
- Model sees the current checklist as context on every send, so it knows what's already done.
- Built-in tool — no MCP server setup, no per-update user approval prompts.

## Non-Goals (V1)

- User-editable tasks (user only views; LLM owns writes).
- Reordering / drag-and-drop.
- Multi-conversation task views or global task list.
- Due dates, priorities, tags.
- Carrying tasks between conversations.
- Notifications when tasks complete.

## Architecture

A new `tasks` module in the Rust backend, parallel to `memory/`, owns the data. A built-in tool (`update_tasks`) is injected into the LLM's tool list alongside MCP tools. When the model calls the tool, the backend routes the call to the tasks store (not to `McpManager`), persists it, and emits a Tauri event that the frontend's new `TasksPanel` listens for. The current task list is also injected into the LLM's context on every send so the model can see what's pending/done.

```
LLM ──tool call──> lib.rs::send_message tool dispatch
                       │
                       ├─ name == "update_tasks" ──> TaskManager.set_tasks() ──> SQLite
                       │                                    │
                       │                                    └──> emit "tasks-updated" event ──> TasksPanel.tsx
                       │
                       └─ otherwise ──> McpManager (existing flow)
```

## Data Model

New table on the existing `Librarian` SQLite connection:

```sql
CREATE TABLE tasks (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  position        INTEGER NOT NULL,
  content         TEXT NOT NULL,
  active_form     TEXT NOT NULL,
  status          TEXT NOT NULL CHECK(status IN ('pending','in_progress','completed')),
  updated_at      TEXT NOT NULL
);
CREATE INDEX idx_tasks_conv ON tasks(conversation_id, position);
```

Field semantics:
- `position` — 0-based index in the list. Determines display order.
- `content` — imperative form ("Build the parser"). Shown when status is `pending` or `completed`.
- `active_form` — present-progressive form ("Building the parser"). Shown while status is `in_progress`.
- `status` — mirrors the Claude Code TodoWrite vocabulary: `pending` / `in_progress` / `completed`.
- `updated_at` — ISO-8601 timestamp, set on every write. For display and debugging.

Schema migration: added as `migrate_v4()` in `sqlite_manager.rs`, following the existing migration pattern.

## Tool Definition

Single tool with replace-all semantics:

```
Tool name: update_tasks
Description: |
  Update the visible task checklist for the current conversation. Pass the
  COMPLETE updated list every time — this replaces the entire list.

  Use this tool when working on multi-step requests so the user can see your
  plan and progress. Mark exactly one task as "in_progress" at a time. Mark a
  task "completed" the moment it is done — do not batch completions.

  - content: imperative form ("Build the parser")
  - active_form: present-progressive form ("Building the parser")
  - status: "pending" | "in_progress" | "completed"

Parameters:
  tasks: array of objects, each with:
    content: string (required)
    active_form: string (required)
    status: enum [pending, in_progress, completed] (required)
```

Replace-all is chosen over fine-grained ops to eliminate ID drift, missing-id errors, and partial-state bugs. The token cost of resending the full list is small (lists are usually under 15 items).

## Backend Integration

### New module: `src-tauri/src/tasks/mod.rs`

```rust
pub struct TaskManager { /* holds Arc<Mutex<SqliteManager>> shared with Librarian */ }

impl TaskManager {
    pub fn set_tasks(&self, conv_id: &str, tasks: Vec<TaskInput>) -> Result<Vec<Task>>;
    pub fn get_tasks(&self, conv_id: &str) -> Result<Vec<Task>>;
    pub fn format_for_context(&self, conv_id: &str) -> Result<Option<String>>;
}

pub struct Task {
    pub id: String,
    pub position: i32,
    pub content: String,
    pub active_form: String,
    pub status: TaskStatus,
    pub updated_at: String,
}

pub enum TaskStatus { Pending, InProgress, Completed }
```

`set_tasks` does the replace-all in one transaction: `DELETE WHERE conversation_id = ?`, then insert each new row with assigned position.

`format_for_context` returns something like:

```
Current task checklist:
- [✓] Look up the data model
- [→] Build the parser
- [ ] Wire up the UI
- [ ] Write tests
```

…or `None` if there are no tasks.

### Tool registration

In `lib.rs::send_message`, before the tool list is sent to the provider:

1. Build the `update_tasks` tool descriptor in the existing `Tool` shape.
2. Append it to the list `McpManager::get_all_tools()` returns, unless the user has disabled it via a new `disable_builtin_task_tool` setting (default `false`).

### Tool dispatch

The existing tool-call handler intercepts before MCP routing:

```rust
if tool_call.name == "update_tasks" {
    let tasks = parse_tasks(&tool_call.arguments)?;
    let result = state.task_manager.set_tasks(&conversation_id, tasks)?;
    app_handle.emit("tasks-updated", serde_json::json!({
        "conversation_id": conversation_id,
        "tasks": result,
    }))?;
    return Ok(ToolCallResult::success("Task list updated."));
}
// else fall through to McpManager dispatch
```

### Context injection

In `send_message`, after the existing memory-context injection and before the LLM call, call `task_manager.format_for_context(&conversation_id)` and if it returns `Some(text)`, prepend a system message with that text. Placement: after the active system prompt, before or alongside the current-date system message.

### Auto-approval

The built-in `update_tasks` tool **bypasses the standard tool-approval flow**. Rationale: it has no external side effects, only updates local UI state. Per-update approval prompts would make the feature unusable. This is encoded as a hardcoded short-circuit in the tool-approval logic — the tool name `update_tasks` is always auto-allowed regardless of the user's approval settings.

## Frontend

### New component: `src/components/TasksPanel.tsx`

Modeled on `MemoryPanel.tsx`:

- On mount: invoke a new `get_conversation_tasks` Tauri command for the current conversation.
- Subscribes to the `tasks-updated` Tauri event; filters by `conversation_id == current`; replaces local list state.
- Renders a vertical list:
  - `☐` for `pending` (shows `content`)
  - `▶` for `in_progress` (shows `active_form`, bolded)
  - `✓` for `completed` (shows `content`, struck through)
- Empty state: "No tasks yet for this conversation."

### Mounting

Added to the same auxiliary-panel region as `ToolsPanel` and `MemoryPanel`. Toggleable via a header button (icon: checklist). Visibility state persisted in localStorage like the existing panels.

### Conversation switching

When the user switches conversations (existing event), `TasksPanel` re-fetches via `get_conversation_tasks` for the new conversation.

## New Tauri commands

- `get_conversation_tasks(conversation_id: String) -> Result<Vec<Task>, String>` — returns the current list for the panel.

The replace-all path doesn't need a `set` command from the frontend — only the LLM writes.

## Settings

Single new field added to `Settings` (`mcp/config.rs`):

```rust
/// When true, the built-in update_tasks tool is hidden from the LLM.
#[serde(default)]
pub disable_builtin_task_tool: bool,
```

Default `false` (tool enabled). Surface as a checkbox under an appropriate Settings section (likely "Tools" or "Behavior").

## Testing

- **Rust unit tests** (`tasks/mod.rs`):
  - `set_tasks` replaces existing rows (no leftover positions).
  - `get_tasks` returns rows ordered by `position`.
  - Conversation isolation: rows for conv A invisible to conv B.
  - `format_for_context` produces correct markdown for each status mix; returns `None` when list is empty.
- **Rust integration test** (`tests/`):
  - Round-trip: simulate an LLM tool call with `name=update_tasks`, verify DB state and that a `tasks-updated` event was emitted.
- **Frontend**: manual via `npm run tauri dev`, per project convention (no frontend test suite per CLAUDE.md).

## Open questions deferred to V1.1

- Should the panel auto-open the first time a task arrives in a conversation that's never had tasks? (Lean yes, but not in V1.)
- Should completed tasks fade after N seconds? (V1: no, keep them visible.)
- Should there be a "clear all tasks" affordance? (V1: no — LLM can clear by sending an empty array.)

## Acceptance criteria

1. User opens a chat. Asks the model to do a multi-step task. The model emits `update_tasks` with 3-5 items, one marked `in_progress`. The `TasksPanel` lights up live with the list.
2. The model finishes step 1, calls `update_tasks` again with step 1 `completed` and step 2 `in_progress`. The panel updates without reloading the conversation.
3. User closes the conversation and reopens it from the sidebar. The task list reappears in the same state.
4. User starts a new conversation. The `TasksPanel` is empty — tasks did not leak across conversations.
5. The model, on a second turn within the same conversation, can correctly reference which tasks are done — confirmation that context injection works.
6. User enables `disable_builtin_task_tool` in Settings (sets it to `true`). On the next send, the model no longer sees `update_tasks` in its tool list (verified by inspecting the tool list in dev logs).
