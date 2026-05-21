//! Auto-injection layer (memory3 Phase 2).
//!
//! Per user turn, produces a compact system-prompt block consisting of:
//! - top recall hit's full doc body (clipped to `MAX_DOC_CHARS`)
//! - up to `MAX_FACTS` KG facts rendered as prose
//!
//! The recent conversation is **not** duplicated here — the LLM already sees
//! the full message history. This layer adds the curated long-term memory.
//!
//! The whole block is hard-capped at a configurable token budget. If the
//! budget is exceeded, the doc body is trimmed first, then facts.

use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::llm::tokenizer::Tokenizer;
use crate::memory::librarian::Librarian;
use crate::memory::{Fact, RelevantFact};

use super::api::{RecallInput, RecallOutput};
use super::DocStore;

/// Default cap on the auto-inject block in tokens. Matches the design doc.
pub const DEFAULT_TOKEN_BUDGET: usize = 4000;
/// Default minimum recall score required to inject a doc.
pub const DEFAULT_DOC_SCORE_FLOOR: f32 = 0.20;

const MAX_DOC_CHARS: usize = 4000;
const MAX_FACTS: usize = 8;
const MAX_FACT_ENTITIES: usize = 3;

#[derive(Debug, Clone)]
pub struct AutoInjectResult {
    /// Ready-to-inject system text. Empty when nothing relevant was found.
    pub text: String,
    /// Doc id whose body was injected, when one was selected.
    pub doc_id: Option<String>,
    /// Fact ids that contributed to the prose block.
    pub fact_ids: Vec<String>,
}

impl AutoInjectResult {
    pub fn empty() -> Self {
        Self {
            text: String::new(),
            doc_id: None,
            fact_ids: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

impl DocStore {
    /// Build the auto-injection block for this turn. Caller is responsible for
    /// deciding whether to inject at all (typically via the `memory_enabled`
    /// + `memory_auto_inject_docs` settings).
    pub async fn auto_inject(
        &self,
        query: &str,
        librarian: Arc<Mutex<Librarian>>,
        token_budget: usize,
        doc_score_floor: f32,
    ) -> Result<AutoInjectResult> {
        let token_budget = token_budget.max(256);

        // 1. Recall top doc.
        let recall: RecallOutput = self
            .recall(RecallInput {
                query: query.to_string(),
                k: 1,
                tags: Vec::new(),
            })
            .await
            .unwrap_or(RecallOutput { hits: Vec::new() });

        let mut doc_section = String::new();
        let mut doc_id: Option<String> = None;
        if let Some(hit) = recall.hits.first() {
            if hit.score >= doc_score_floor {
                if let Some(rec) = self.fetch(&hit.doc_id).await? {
                    let body = clip_chars(&rec.content, MAX_DOC_CHARS);
                    doc_section = format!("## Relevant document: {}\n{}", rec.title, body);
                    doc_id = Some(rec.id);
                }
            }
        }

        // 2. Collect KG facts.
        let facts = {
            let lib = librarian.lock().await;
            collect_relevant_facts(&lib, query)?
        };
        let mut fact_ids: Vec<String> = facts.iter().map(|f| f.fact.id.clone()).collect();
        let facts_section = render_facts_prose(&facts);

        // 3. Assemble.
        let mut sections: Vec<String> = Vec::new();
        if !doc_section.is_empty() {
            sections.push(doc_section.clone());
        }
        if !facts_section.is_empty() {
            sections.push(facts_section.clone());
        }
        let mut text = sections.join("\n\n");

        // 4. Enforce token budget. Drop the doc first if we're over.
        if !text.is_empty() && Tokenizer::count_tokens(&text).unwrap_or(0) > token_budget {
            if !doc_section.is_empty() {
                // Try clipping the doc tighter rather than dropping it outright.
                let mut tighter_doc = doc_section.clone();
                while Tokenizer::count_tokens(&text).unwrap_or(0) > token_budget
                    && tighter_doc.len() > 200
                {
                    let new_len = tighter_doc.len() / 2;
                    tighter_doc.truncate(new_len);
                    tighter_doc.push_str("\n…[truncated]");
                    let mut next: Vec<String> = vec![tighter_doc.clone()];
                    if !facts_section.is_empty() {
                        next.push(facts_section.clone());
                    }
                    text = next.join("\n\n");
                }
                // Still over? Drop the doc entirely.
                if Tokenizer::count_tokens(&text).unwrap_or(0) > token_budget {
                    doc_id = None;
                    text = facts_section.clone();
                }
            }
            // Still over? Trim facts from the tail.
            if !text.is_empty()
                && Tokenizer::count_tokens(&text).unwrap_or(0) > token_budget
                && !fact_ids.is_empty()
            {
                let mut trimmed = facts.clone();
                while Tokenizer::count_tokens(&text).unwrap_or(0) > token_budget
                    && !trimmed.is_empty()
                {
                    trimmed.pop();
                    let trimmed_section = render_facts_prose(&trimmed);
                    let mut next: Vec<String> = Vec::new();
                    if doc_id.is_some() {
                        next.push(doc_section.clone());
                    }
                    if !trimmed_section.is_empty() {
                        next.push(trimmed_section);
                    }
                    text = next.join("\n\n");
                }
                fact_ids = trimmed.iter().map(|f| f.fact.id.clone()).collect();
            }
        }

        if text.is_empty() {
            return Ok(AutoInjectResult::empty());
        }

        Ok(AutoInjectResult {
            text,
            doc_id,
            fact_ids,
        })
    }
}

fn clip_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("\n…[truncated]");
    out
}

/// Collect a bounded set of relevant facts: user-profile facts plus a few
/// per-entity facts for entities mentioned in the query.
pub(crate) fn collect_relevant_facts(
    librarian: &Librarian,
    query: &str,
) -> Result<Vec<RelevantFact>> {
    let mut results: Vec<RelevantFact> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let profile = librarian.get_user_profile_facts(MAX_FACTS)?;
    for f in profile {
        if seen.insert(f.id.clone()) {
            results.push(RelevantFact::from_fact(f));
            if results.len() >= MAX_FACTS {
                return Ok(results);
            }
        }
    }

    let entities = extract_candidate_entities(query, MAX_FACT_ENTITIES);
    for entity in entities {
        if results.len() >= MAX_FACTS {
            break;
        }
        let remaining = MAX_FACTS - results.len();
        let per_entity_limit = remaining.min(3);
        let facts = librarian.get_facts_about_entity(&entity, per_entity_limit)?;
        for f in facts {
            if seen.insert(f.id.clone()) {
                results.push(RelevantFact::from_fact(f));
                if results.len() >= MAX_FACTS {
                    break;
                }
            }
        }
    }

    Ok(results)
}

/// Stopword-aware token extractor. Picks up the most plausible "entity-ish"
/// words in the query (3+ chars, alphanumeric, not stopwords).
fn extract_candidate_entities(query: &str, max_entities: usize) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "the", "and", "for", "with", "that", "this", "from", "have", "about", "your", "you",
        "are", "was", "were", "will", "would", "should", "could", "into", "what", "when",
        "where", "how", "why", "can", "please", "just", "like", "memory", "doc", "tool",
    ];
    let lower = query.to_lowercase();
    let mut entities = Vec::new();
    let mut seen = HashSet::new();
    for raw in lower.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        let token = raw.trim();
        if token.len() < 3 {
            continue;
        }
        if STOPWORDS.contains(&token) {
            continue;
        }
        if seen.insert(token.to_string()) {
            entities.push(token.to_string());
            if entities.len() >= max_entities {
                break;
            }
        }
    }
    entities
}

/// Render facts as a natural-language prose block. User-profile facts collapse
/// into a single "About the user: ..." paragraph; entity-centric facts get one
/// paragraph per subject.
pub(crate) fn render_facts_prose(facts: &[RelevantFact]) -> String {
    if facts.is_empty() {
        return String::new();
    }

    let mut user_clauses: Vec<String> = Vec::new();
    let mut by_subject: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();

    for rf in facts {
        let f = &rf.fact;
        let clause = predicate_object_phrase(f);
        if f.subject == "user" {
            user_clauses.push(clause);
        } else {
            by_subject
                .entry(f.subject.clone())
                .or_default()
                .push(clause);
        }
    }

    let mut paragraphs: Vec<String> = Vec::new();
    if !user_clauses.is_empty() {
        paragraphs.push(format!(
            "## About the user\n{}.",
            join_clauses(&user_clauses)
        ));
    }
    for (subject, clauses) in by_subject {
        paragraphs.push(format!(
            "## About `{}`\n{}.",
            subject,
            join_clauses(&clauses)
        ));
    }

    paragraphs.join("\n\n")
}

fn join_clauses(clauses: &[String]) -> String {
    match clauses.len() {
        0 => String::new(),
        1 => clauses[0].clone(),
        2 => format!("{} and {}", clauses[0], clauses[1]),
        _ => {
            let head = clauses[..clauses.len() - 1].join(", ");
            format!("{}, and {}", head, clauses[clauses.len() - 1])
        }
    }
}

fn predicate_object_phrase(f: &Fact) -> String {
    // Normalise "snake_case" predicates back to spaces; surface the object
    // verbatim.
    let predicate = f.predicate.replace('_', " ");
    format!("{} {}", predicate, f.object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{Fact, ObjectKind};

    fn fact(subject: &str, predicate: &str, object: &str) -> RelevantFact {
        RelevantFact::from_fact(Fact {
            id: format!("{subject}-{predicate}-{object}"),
            subject: subject.into(),
            predicate: predicate.into(),
            object: object.into(),
            object_kind: ObjectKind::Literal,
            confidence: 0.9,
            source_message_id: None,
            created_at: "2026-05-21T00:00:00Z".into(),
            updated_at: "2026-05-21T00:00:00Z".into(),
        })
    }

    #[test]
    fn render_user_facts_collapses_to_one_paragraph() {
        let facts = vec![
            fact("user", "prefers", "dark mode"),
            fact("user", "uses", "rust"),
            fact("user", "working_on", "nebula chat"),
        ];
        let s = render_facts_prose(&facts);
        assert!(s.starts_with("## About the user"));
        assert!(s.contains("prefers dark mode"));
        assert!(s.contains("uses rust"));
        assert!(s.contains("working on nebula chat"));
        assert!(s.contains(" and "));
    }

    #[test]
    fn render_entity_facts_groups_by_subject() {
        let facts = vec![
            fact("nebula_chat", "built_with", "tauri"),
            fact("nebula_chat", "uses", "sqlite"),
            fact("rust", "is", "a systems language"),
        ];
        let s = render_facts_prose(&facts);
        assert!(s.contains("## About `nebula_chat`"));
        assert!(s.contains("## About `rust`"));
        assert!(s.contains("built with tauri"));
    }

    #[test]
    fn entity_extractor_drops_stopwords() {
        let q = "how does the nebula_chat memory tool work with sqlite";
        let entities = extract_candidate_entities(q, 5);
        assert!(entities.iter().any(|e| e == "nebula_chat"));
        assert!(entities.iter().any(|e| e == "sqlite"));
        assert!(!entities.iter().any(|e| e == "the"));
        assert!(!entities.iter().any(|e| e == "memory")); // stopword
    }

    #[test]
    fn clip_chars_truncates_and_marks() {
        let s: String = "a".repeat(10_000);
        let clipped = clip_chars(&s, 100);
        assert!(clipped.len() < s.len());
        assert!(clipped.contains("[truncated]"));
    }

    #[test]
    fn render_empty_returns_empty() {
        assert!(render_facts_prose(&[]).is_empty());
    }
}
