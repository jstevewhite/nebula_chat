//! The LLM-facing `use_skill` tool. The system prompt is already injected
//! with the list of available skills (name + description) by
//! `SkillStore::render_for_system_prompt` so the model knows which slugs are
//! callable.

use crate::llm::provider::ToolDefinition;
use serde_json::json;

pub const TOOL_USE_SKILL: &str = "use_skill";

pub fn is_skill_tool(name: &str) -> bool {
    name == TOOL_USE_SKILL
}

pub fn build_all() -> Vec<ToolDefinition> {
    vec![use_skill_tool()]
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
