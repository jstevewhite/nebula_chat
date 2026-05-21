//! The LLM-facing `use_skill` tool. The system prompt is already injected
//! with the list of available skills (name + description) by
//! `SkillStore::render_for_system_prompt` so the model knows which slugs are
//! callable.

use crate::llm::provider::ToolDefinition;
use serde_json::json;

pub const TOOL_USE_SKILL: &str = "use_skill";
pub const TOOL_LIST_SKILLS: &str = "list_skills";

pub fn is_skill_tool(name: &str) -> bool {
    matches!(name, TOOL_USE_SKILL | TOOL_LIST_SKILLS)
}

pub fn build_all() -> Vec<ToolDefinition> {
    vec![use_skill_tool(), list_skills_tool()]
}

fn list_skills_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_LIST_SKILLS.to_string(),
        description:
            "Return the slug, name, and description of every skill currently installed. \
             Usually unnecessary — the same list is already in the system prompt under \
             \"Available skills\" — but useful when you want a fresh view (e.g. the user just \
             added a new skill mid-conversation via the file watcher) or want to enumerate \
             skills programmatically before deciding which to call with `use_skill`."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {}
        }),
    }
}

fn use_skill_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_USE_SKILL.to_string(),
        description:
            "Load a SKILL — a bundle of imperative instructions that tell you how to approach \
             a particular kind of task (code review, debugging, summarising, etc). The tool \
             returns the skill's body, which you should treat as authoritative guidance for \
             the remainder of the user's current request.\n\n\
             Available skills are listed in the system prompt as `slug — description`. Pass \
             the slug as the `name` argument. Calling an unknown skill returns an error.\n\n\
             Use a skill when the user's request matches one of the descriptions and your \
             default approach would benefit from the structured guidance. Don't load skills \
             pre-emptively for every turn — load when the task actually calls for it."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill slug, as listed in the system prompt's available-skills block."
                }
            }
        }),
    }
}
