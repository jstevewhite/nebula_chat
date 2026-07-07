//! Skill types exposed to the frontend and the LLM dispatch layer.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where a skill came from. Native = authored in Nebula (user dir or built-in).
/// Claude = imported read-only from Claude Code's `~/.claude/skills/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SkillOrigin {
    #[default]
    Native,
    Claude,
}

/// A fully-loaded skill: identity, description, and the markdown body that
/// gets injected when the LLM calls `use_skill`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub body: String,
    pub built_in: bool,
    pub path: PathBuf,
    #[serde(default)]
    pub origin: SkillOrigin,
}

/// Lightweight view returned to the frontend's Skills tab and used in the
/// system-prompt rendering. Excludes the full body to keep responses small.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub built_in: bool,
    pub path: String,
    #[serde(default)]
    pub origin: SkillOrigin,
}

impl From<&Skill> for SkillSummary {
    fn from(s: &Skill) -> Self {
        Self {
            slug: s.slug.clone(),
            name: s.name.clone(),
            description: s.description.clone(),
            built_in: s.built_in,
            path: s.path.to_string_lossy().into_owned(),
            origin: s.origin,
        }
    }
}

/// One discovered Claude skill plus its computed state, for the Settings
/// checklist. `effective_enabled` is whether it is active in the cache right
/// now (`!shadowed && (override or heuristic_default)`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeSkillEntry {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub heuristic_default: bool,
    pub effective_enabled: bool,
    pub shadowed_by_native: bool,
}
