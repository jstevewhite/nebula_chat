use crate::llm::anthropic::AnthropicProvider;
use crate::llm::ollama::OllamaProvider;
use crate::llm::openai::OpenAiProvider;
use crate::llm::provider::{LlmProvider, Message};
use crate::mcp::config::{ProviderType, Settings};
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct Compactor;

impl Compactor {
    pub async fn compact(
        messages: Vec<Message>,
        settings: &Settings,
        conversation_id: Option<&str>,
        librarian_arc: Arc<Mutex<crate::memory::librarian::Librarian>>,
    ) -> Result<(Option<String>, Vec<Message>)> {
        // If disabled or no conversation context, return originals
        if settings.context_uncompressed_msg_count == 0 || conversation_id.is_none() {
            return Ok((None, messages));
        }
        let conversation_id = conversation_id.unwrap();

        let limit = settings.context_uncompressed_msg_count;
        if messages.len() <= limit {
            return Ok((None, messages));
        }

        let lib = librarian_arc.lock().await;

        // 1. Get existing summary
        let existing = lib.sqlite.get_conversation_summary(conversation_id)?;
        let (mut last_summary, mut last_id) = if let Some((msg_id, summary)) = existing {
            (summary, Some(msg_id))
        } else {
            (String::new(), None)
        };

        drop(lib); // Drop lock for async work

        // 2. Identify messages to compact
        // We want to keep the last `limit` messages RAW.
        // Everything before that should be compacted.
        let mut split_idx = messages.len().saturating_sub(limit);

        // Safety check: Don't split in the middle of a tool chain.
        // If split_idx lands on a "tool" message, we must include the preceding
        // assistant message (and any other related tool messages) in the "keep_raw" segment.
        // We move split_idx backwards until we find a non-tool message.
        // This effectively pulls the entire tool interaction into the "recent/raw" buffer.
        while split_idx > 0 {
            if messages[split_idx].role == "tool" {
                split_idx -= 1;
            } else {
                break;
            }
        }
        let to_compact = &messages[..split_idx];
        let keep_raw = &messages[split_idx..];

        // Filter messages that are already covered by the previous summary
        // Usage: if last_id is present, skip messages up to and including that ID.
        let mut new_chunk_msgs = Vec::new();
        let mut found_last = last_id.is_none(); // If no last_id, we start from beginning

        // If we have a last_id, we need to find where it is in `to_compact`.
        // If it's not found in `to_compact`, it might be that our 'last_id' is actually *older*
        // than the start of `to_compact` (which shouldn't happen if we strictly append),
        // OR it might be that we are re-compacting a range.
        // Simplified approach: scan `to_compact`.

        for msg in to_compact {
            if !found_last {
                if let Some(id) = &msg.id {
                    if Some(id.clone()) == last_id {
                        found_last = true;
                        // Don't include this one, it was the last one summarized (inclusive).
                        continue;
                    }
                }
                // If we haven't found the marker yet, we assume this message was already summarized
                // (assuming strict chronological order and no holes).
                // Wait... if last_id is configured, it means everything UP TO last_id is done.
                // So we actually SKIP until we find last_id.
                continue;
            }
            // found_last is true (or was initially true), so these are new valid messages to compact
            new_chunk_msgs.push(msg);
        }

        // If nothing new to compact, just assemble result
        if new_chunk_msgs.is_empty() {
            // We still need to return the summary message + kept raw messages
            let mut result = Vec::new();
            if !last_summary.is_empty() {
                result.push(Self::create_summary_message(last_summary.clone()));
            }
            result.extend(keep_raw.iter().cloned());
            return Ok((Some(last_summary), result));
        }

        // 3. Compact the new chunk
        // We need an LLM provider. Use the memory strategist model if defined, else default model.
        let model_id = settings
            .context_model
            .as_deref()
            .or(settings.default_model.as_deref());

        let (provider, model_name) = if let Some(mid) = model_id {
            if let Some((p, m)) = Self::parse_model_id(mid) {
                (Self::create_provider(p, m, settings)?, m)
            } else {
                // Fallback to first available or error?
                // Let's try to infer if it's a simple string
                tracing::warn!(
                    "Compactor: Invalid model ID format '{}', skipping compaction",
                    mid
                );
                return Ok((None, messages));
            }
        } else {
            tracing::warn!("Compactor: No model selected for compaction");
            return Ok((None, messages));
        };

        // Format prompt
        let chunk_text = new_chunk_msgs
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content.as_deref().unwrap_or("[empty]")))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = if last_summary.is_empty() {
            format!(
                "Summarize the following conversation fragment concisely, preserving key facts, decisions, and context.\n\nFRAGMENT:\n{}",
                chunk_text
            )
        } else {
            format!(
                "Update the existing conversation summary with the new conversation fragment.\n\nEXISTING SUMMARY:\n{}\n\nNEW FRAGMENT:\n{}\n\nTASK:\nMerge the new information into the summary. Keep it concise but comprehensive. Output ONLY the new summary.",
                last_summary, chunk_text
            )
        };

        let sys_msg = Message {
            id: None,
            role: "system".to_string(),
            content: Some(
                "You are a helpful assistant acting as a Conversation Summarizer.".to_string(),
            ),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            attachments: None,
            created_at: None,
        };
        let user_msg = Message {
            id: None,
            role: "user".to_string(),
            content: Some(prompt),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            attachments: None,
            created_at: None,
        };

        let response = provider
            .chat(vec![sys_msg, user_msg], vec![], None)
            .await
            .context("Compaction LLM call failed")?;

        let new_summary = response.content.unwrap_or_default();

        // 4. Save new summary
        // The last message ID we compacted is the ID of the last message in new_chunk_msgs
        if let Some(last_msg) = new_chunk_msgs.last() {
            if let Some(lid) = &last_msg.id {
                let lib = librarian_arc.lock().await;
                lib.sqlite
                    .save_conversation_summary(conversation_id, lid, &new_summary)?;
            }
        }

        // 5. Return Result
        let mut result = Vec::new();
        result.push(Self::create_summary_message(new_summary.clone()));
        result.extend(keep_raw.iter().cloned());

        Ok((Some(new_summary), result))
    }

    fn create_summary_message(summary: String) -> Message {
        Message {
            id: Some("summary-inject".to_string()),
            role: "system".to_string(),
            content: Some(format!("PREVIOUS CONVERSATION SUMMARY:\n{}", summary)),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            attachments: None,
            created_at: None,
        }
    }

    fn parse_model_id(id: &str) -> Option<(&str, &str)> {
        let parts: Vec<&str> = id.split("::").collect();
        if parts.len() == 2 {
            Some((parts[0], parts[1]))
        } else {
            // Handle simple cases if needed, but for now expect nebula format
            None
        }
    }

    fn create_provider(
        provider_id: &str,
        model_name: &str,
        settings: &Settings,
    ) -> Result<Box<dyn LlmProvider + Send + Sync>> {
        let config = settings
            .providers
            .get(provider_id)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_id))?;

        let provider: Box<dyn LlmProvider + Send + Sync> = match config.provider_type {
            ProviderType::OpenAI | ProviderType::OpenAICompatible => {
                let key = config.api_key.clone().unwrap_or_default();
                let base_url = config.base_url.clone();
                Box::new(OpenAiProvider::new(key, base_url, model_name.to_string()))
            }
            ProviderType::Anthropic => {
                let key = config.api_key.clone().unwrap_or_default();
                Box::new(AnthropicProvider::new(key, model_name.to_string()))
            }
            ProviderType::Ollama => {
                let base_url = config
                    .base_url
                    .clone()
                    .unwrap_or("http://localhost:11434".to_string());
                Box::new(OllamaProvider::new(base_url, model_name.to_string()))
            }
        };

        Ok(provider)
    }
}
