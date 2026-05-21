use crate::llm::provider::{LlmProvider, Message};
use crate::memory::librarian::Librarian;
use crate::memory::{NewFact, ObjectKind};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Extracts structured facts from messages and saves them to the knowledge graph.
pub struct FactExtractor;

impl FactExtractor {
    /// Extract facts from a message and save them to the librarian, tagging
    /// them with the supplied source message id for provenance.
    pub async fn extract(
        librarian: Arc<Mutex<Librarian>>,
        provider: &dyn LlmProvider,
        role: &str,
        content: &str,
        source_message_id: &str,
    ) -> Result<String> {
        Self::extract_with_source(
            librarian,
            provider,
            role,
            content,
            Some(source_message_id.to_string()),
        )
        .await
    }

    /// Same as `extract`, but with an optional `source_message_id`. Used by
    /// the `/remember` chat command and other arbitrary-text triggers where
    /// there is no single backing message.
    pub async fn extract_with_source(
        librarian: Arc<Mutex<Librarian>>,
        provider: &dyn LlmProvider,
        role: &str,
        content: &str,
        source_message_id: Option<String>,
    ) -> Result<String> {
        let prompt = format!(
            r#"You are a long-term memory extraction model for a coding assistant.

Your job:
- Read the following {role} message.
- Decide which facts should be stored as **long-term memory** because they are likely to matter in future conversations with this same user.
- Only extract facts that would still be useful days or weeks from now.

What to STORE (examples):
- User identity and role: jobs, seniority, areas of expertise.
- Stable preferences: editors, languages, frameworks, libraries, tools, workflows.
- Persistent environment details: OS, CPU/GPU, main devices, IDEs, hosting platforms, CI/CD systems.
- Long-lived projects: project names, repositories, tech stacks, key decisions.
- Stable account / organization context (without secrets): "company uses GitHub Enterprise", "deploys to GKE".
- Clear decisions and constraints: "we decided to use Postgres, not MySQL", "production cluster is Kubernetes 1.30".

What to IGNORE (do NOT store):
- One-off questions or search queries (e.g. "look up the specs for X", "what is the error in this log?").
- Ephemeral status: "I'm tired", "today", "this week", "right now I'm running a benchmark".
- Raw tool outputs, logs, stack traces, and long URLs unless they define a stable choice (e.g. a canonical docs URL).
- Speculative statements or guesses ("I think maybe we should use Rust").
- Anything that looks like a secret, password, token, or private key.

Return format:
Return a JSON object with a single key "facts" whose value is an array of objects with this schema:
{{
  "subject": "normalized_subject_key",
  "predicate": "normalized_predicate_key",
  "object": "raw object value or entity key",
  "object_kind": "entity" | "literal",
  "confidence": 0.0-1.0
}}

Normalization:
- subject and predicate should be lowercase, snake_case keys for stable lookup (e.g. "user", "owns_device", "main_editor").
- object may be a normalized key (for entities) or a free-form literal string.

If there are **no** facts that meet the criteria above, return:
{{"facts": []}}
"#,
            role = role
        );

        let messages = vec![
            Message {
                id: None,
                role: "user".to_string(),
                content: Some(prompt),
                tool_calls: None,
                tool_call_id: None,
                attachments: None,
                reasoning_content: None,
                created_at: None,
            },
            Message {
                id: None,
                role: role.to_string(),
                content: Some(content.to_string()),
                tool_calls: None,
                tool_call_id: None,
                attachments: None,
                reasoning_content: None,
                created_at: None,
            },
        ];

        let options = crate::llm::provider::GenerationOptions {
            temperature: Some(0.1),
            ..Default::default()
        };
        let response = provider.chat(messages, vec![], Some(options)).await?;
        let output = response.content.unwrap_or_default();
        let extracted_facts: Vec<ExtractedFactDto> = Self::extract_and_parse_json(&output);
        let count = extracted_facts.len();

        if count == 0 {
            return Ok("No facts extracted".to_string());
        }

        let lib = librarian.lock().await;
        for dto in extracted_facts {
            let subject = Self::normalize_key(&dto.subject);
            let predicate = Self::normalize_key(&dto.predicate);
            let object = dto.object.trim().to_string();

            let object_kind = match dto
                .object_kind
                .as_deref()
                .map(|s| s.to_lowercase())
                .as_deref()
            {
                Some("entity") => ObjectKind::Entity,
                _ => ObjectKind::Literal,
            };

            let mut confidence = dto.confidence.unwrap_or(1.0);
            if !confidence.is_finite() {
                confidence = 1.0;
            } else {
                confidence = confidence.clamp(0.0, 1.0);
            }

            let new_fact = NewFact::new(
                subject,
                predicate,
                object,
                object_kind,
                confidence,
                source_message_id.clone(),
            );

            if let Err(e) = lib.upsert_fact(new_fact) {
                tracing::warn!("Failed to upsert fact: {}", e);
            }
        }

        Ok(format!("Extracted and saved {} facts", count))
    }

    fn extract_and_parse_json(content: &str) -> Vec<ExtractedFactDto> {
        let trimmed = content.trim();
        let without_fence = if trimmed.starts_with("```") {
            let mut body = trimmed.trim_start_matches('`');
            if body.starts_with("json") {
                body = body.trim_start_matches("json");
            }
            if let Some(end) = body.rfind("```") {
                &body[..end]
            } else {
                body
            }
        } else {
            trimmed
        };

        let json_part = if let Some(start) = without_fence.find('{') {
            if let Some(end) = without_fence.rfind('}') {
                &without_fence[start..=end]
            } else {
                without_fence
            }
        } else if let Some(start) = without_fence.find('[') {
            if let Some(end) = without_fence.rfind(']') {
                &without_fence[start..=end]
            } else {
                without_fence
            }
        } else {
            without_fence
        };

        // Try envelope
        if let Ok(env) = serde_json::from_str::<ExtractedFactsEnvelope>(json_part) {
            return env.facts;
        }

        // Try bare array
        if let Ok(list) = serde_json::from_str::<Vec<ExtractedFactDto>>(json_part) {
            return list;
        }

        Vec::new()
    }

    pub fn normalize_key(value: &str) -> String {
        let lower = value.to_lowercase();
        let parts: Vec<&str> = lower.split_whitespace().collect();
        parts.join("_")
    }
}

#[derive(serde::Deserialize, Debug)]
struct ExtractedFactsEnvelope {
    facts: Vec<ExtractedFactDto>,
}

#[derive(serde::Deserialize, Debug)]
struct ExtractedFactDto {
    subject: String,
    predicate: String,
    object: String,
    object_kind: Option<String>,
    confidence: Option<f32>,
}

#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct ExtractedFact {
    subject: String,
    predicate: String,
    object: String,
    kind: String,
    confidence: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_key_lowercases_and_snake_cases() {
        assert_eq!(FactExtractor::normalize_key("Main Editor"), "main_editor");
        assert_eq!(FactExtractor::normalize_key("USER"), "user");
        assert_eq!(FactExtractor::normalize_key("owns device"), "owns_device");
    }

    #[test]
    fn normalize_key_collapses_runs_of_whitespace() {
        // split_whitespace handles tabs, newlines, repeated spaces.
        assert_eq!(FactExtractor::normalize_key("a   b\tc\nd"), "a_b_c_d");
        assert_eq!(FactExtractor::normalize_key("  leading and trailing  "), "leading_and_trailing");
    }

    #[test]
    fn normalize_key_empty_string() {
        assert_eq!(FactExtractor::normalize_key(""), "");
        assert_eq!(FactExtractor::normalize_key("   "), "");
    }

    #[test]
    fn parse_json_handles_envelope_form() {
        let raw = r#"{"facts":[{"subject":"user","predicate":"likes","object":"rust","object_kind":"literal","confidence":0.9}]}"#;
        let parsed = FactExtractor::extract_and_parse_json(raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].subject, "user");
        assert_eq!(parsed[0].predicate, "likes");
        assert_eq!(parsed[0].object, "rust");
        assert_eq!(parsed[0].object_kind.as_deref(), Some("literal"));
        assert_eq!(parsed[0].confidence, Some(0.9));
    }

    #[test]
    fn parse_json_bare_array_of_objects_is_not_recovered() {
        // The parser searches for the first `{` before the first `[`, so a bare array
        // of objects collapses to one of its inner objects, fails envelope parsing,
        // and returns empty. This documents (rather than endorses) the current
        // limitation — the "bare array" fallback only matches arrays of literals.
        let raw = r#"[{"subject":"user","predicate":"uses","object":"vim"}]"#;
        let parsed = FactExtractor::extract_and_parse_json(raw);
        assert!(parsed.is_empty(), "bare array form is currently unsupported for object elements");
    }

    #[test]
    fn parse_json_strips_markdown_fence_with_language_tag() {
        let raw = "```json\n{\"facts\":[{\"subject\":\"user\",\"predicate\":\"likes\",\"object\":\"rust\"}]}\n```";
        let parsed = FactExtractor::extract_and_parse_json(raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].object, "rust");
    }

    #[test]
    fn parse_json_strips_markdown_fence_without_language_tag() {
        let raw = "```\n{\"facts\":[{\"subject\":\"a\",\"predicate\":\"b\",\"object\":\"c\"}]}\n```";
        let parsed = FactExtractor::extract_and_parse_json(raw);
        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn parse_json_extracts_envelope_from_chatty_prose() {
        let raw = r#"Sure! Here are the facts I extracted:

{"facts": [{"subject": "user", "predicate": "lives_in", "object": "Berlin"}]}

Let me know if you need more."#;
        let parsed = FactExtractor::extract_and_parse_json(raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].object, "Berlin");
    }

    #[test]
    fn parse_json_empty_envelope_returns_empty() {
        let parsed = FactExtractor::extract_and_parse_json(r#"{"facts":[]}"#);
        assert!(parsed.is_empty());
    }

    #[test]
    fn parse_json_garbage_returns_empty() {
        assert!(FactExtractor::extract_and_parse_json("not json at all").is_empty());
        assert!(FactExtractor::extract_and_parse_json("").is_empty());
        // Schema mismatch (missing required fields) also yields empty
        assert!(FactExtractor::extract_and_parse_json(r#"{"foo":"bar"}"#).is_empty());
    }

    #[test]
    fn parse_json_envelope_preferred_over_array_when_both_could_parse() {
        let raw = r#"{"facts":[{"subject":"s","predicate":"p","object":"o"}]}"#;
        let parsed = FactExtractor::extract_and_parse_json(raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].subject, "s");
    }

    #[test]
    fn parse_json_accepts_multiple_facts_in_envelope() {
        let raw = r#"{"facts":[
            {"subject":"user","predicate":"owns","object":"laptop","object_kind":"entity","confidence":1.0},
            {"subject":"user","predicate":"likes","object":"coffee","object_kind":"literal","confidence":0.7}
        ]}"#;
        let parsed = FactExtractor::extract_and_parse_json(raw);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].object, "coffee");
    }
}
