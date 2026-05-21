//! Skill types exposed to the frontend and the LLM dispatch layer.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
}

impl From<&Skill> for SkillSummary {
    fn from(s: &Skill) -> Self {
        Self {
            slug: s.slug.clone(),
            name: s.name.clone(),
            description: s.description.clone(),
            built_in: s.built_in,
            path: s.path.to_string_lossy().into_owned(),
        }
    }
}
