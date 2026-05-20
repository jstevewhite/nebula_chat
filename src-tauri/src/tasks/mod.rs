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
