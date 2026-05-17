use crate::llm::anthropic::AnthropicProvider;
use crate::llm::ollama::OllamaProvider;
use crate::llm::openai::OpenAiProvider;
use crate::llm::provider::{LlmProvider, Message};
use crate::mcp::config::{ProviderType, Settings};
use anyhow::{anyhow, Context, Result};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct Compactor;

pub struct CompactionState {
    conversation_id: String,
    phase: String,
    messages_before: Vec<Message>,
    original_summary: Option<(String, String)>,
}

impl Compactor {
    pub async fn compact(
        messages: Vec<Message>,
        settings: &Settings,
        conversation_id: Option<&str>,
        librarian_arc: Arc<Mutex<crate::memory::librarian::Librarian>>,
    ) -> Result<(Option<String>, Vec<Message>)> {
        let mut backup_state: Option<CompactionState> = None;
        let mut compact_conv_id: Option<&str> = conversation_id;

        if let Some(conv_id) = conversation_id {
            match Self::backup_state(conv_id, &messages, &librarian_arc).await {
                Ok(state) => {
                    backup_state = Some(state);
                    compact_conv_id = Some(&backup_state.as_ref().unwrap().conversation_id);
                }
                Err(e) => {
                    tracing::error!("Failed to backup state for compaction: {}", e);
                }
            }
        }

        let result = Self::compact_inner(messages, settings, compact_conv_id, librarian_arc.clone()).await;

        if let Some(state) = backup_state {
            if result.is_err() {
                tracing::error!("Compaction failed, initiating rollback");
                Self::rollback(state, &librarian_arc).await;
            }
        }

        result
    }

    async fn backup_state(
        conversation_id: &str,
        messages: &[Message],
        librarian_arc: &Arc<Mutex<crate::memory::librarian::Librarian>>,
    ) -> Result<CompactionState> {
        let lib = librarian_arc.lock().await;
        let existing_summary = lib.sqlite.get_conversation_summary(conversation_id)?;
        Ok(CompactionState {
            conversation_id: conversation_id.to_string(),
            phase: "compact".to_string(),
            messages_before: messages.to_vec(),
            original_summary: existing_summary,
        })
    }

    async fn rollback(
        state: CompactionState,
        librarian_arc: &Arc<Mutex<crate::memory::librarian::Librarian>>,
    ) {
        let lib = librarian_arc.lock().await;
        tracing::warn!(
            "Rolling back compaction for conversation {}, restoring {} messages",
            state.conversation_id,
            state.messages_before.len()
        );
        if let Some((msg_id, summary)) = state.original_summary {
            let _ = lib.sqlite.save_conversation_summary(&state.conversation_id, &msg_id, &summary);
        }
    }

    async fn compact_inner(
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
        tracing::debug!(
            "Compactor: {} total messages, limit is {}, will compact if > limit",
            messages.len(),
            limit
        );
        
        if messages.len() <= limit {
            tracing::debug!("Compactor: Message count within limit, no compaction needed");
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
        // Tool chains follow this pattern:
        //   assistant (with tool_calls) -> tool -> tool -> ... (multiple tool responses)
        // We must keep these together. Move split_idx backward until we find a safe split point.
        
        // Step 1: If we land on tool messages, move back past all consecutive tool messages
        while split_idx > 0 && messages[split_idx].role == "tool" {
            split_idx -= 1;
        }
        
        // Step 2: If we're now at an assistant message with tool_calls, move back one more
        // to ensure we don't separate the assistant's tool_calls from the tool responses
        if split_idx > 0 && messages[split_idx].role == "assistant" {
            if let Some(ref tool_calls) = messages[split_idx].tool_calls {
                if !tool_calls.is_empty() {
                    // This assistant message has tool calls, so tool responses likely follow.
                    // Move split point back to before this assistant message.
                    split_idx -= 1;
                }
            }
        }
        
        // Step 3: Safety check - if we ended up at another tool message after step 2,
        // continue moving backward (handles edge cases with nested tool chains)
        while split_idx > 0 && messages[split_idx].role == "tool" {
            split_idx -= 1;
        }
        
        // Step 4: CRITICAL - After all adjustments, ensure we don't start keep_raw with a tool message
        // because the summary (system message) would be prepended, creating invalid sequence:
        // [system (summary), tool] which is invalid.
        // Move back to include the assistant message that owns these tool responses.
        if split_idx < messages.len() && messages[split_idx].role == "tool" {
            tracing::warn!(
                "Split would start keep_raw with tool message at index {}. Moving back to find owning assistant.",
                split_idx
            );
            // Move backward to find the assistant message with tool_calls
            while split_idx > 0 {
                split_idx -= 1;
                if messages[split_idx].role == "assistant" {
                    if let Some(ref tool_calls) = messages[split_idx].tool_calls {
                        if !tool_calls.is_empty() {
                            // Found the assistant, keep it in keep_raw
                            break;
                        }
                    }
                }
            }
        }
        let to_compact = &messages[..split_idx];
        let keep_raw = &messages[split_idx..];
        
        tracing::debug!(
            "Compactor: Split at index {}. To compact: {} messages, Keep raw: {} messages",
            split_idx,
            to_compact.len(),
            keep_raw.len()
        );

        // Filter messages that are already covered by the previous summary
        // Usage: if last_id is present, skip messages up to and including that ID.
        let mut new_chunk_msgs = Vec::new();

        // Safety validation: If last_id is provided but not found in to_compact,
        // reset it to None to process all messages from the beginning.
        // This prevents silent data loss when the marker message is outside the compaction range.
        let mut effective_last_id = last_id.clone();
        if let Some(ref lid) = last_id {
            let id_exists = to_compact.iter().any(|msg| {
                msg.id.as_ref() == Some(lid)
            });
            if !id_exists {
                tracing::warn!(
                    "Compaction: last_id '{}' not found in to_compact range ({} messages). \
                     Starting compaction from beginning to prevent data loss.",
                    lid,
                    to_compact.len()
                );
                effective_last_id = None;
            } else {
                tracing::debug!(
                    "Compaction: Found last_id '{}' in to_compact range",
                    lid
                );
            }
        }

        // Initialize found_last from effective_last_id (post-safety-check). If
        // effective_last_id is None — either because no last_id was supplied OR
        // because the safety check above reset it — we begin processing from the
        // first message. Previously this used `last_id.is_none()` which left
        // found_last=false when the safety check fired, defeating the recovery
        // and silently emitting an unchanged old summary.
        let mut found_last = effective_last_id.is_none();

        // If we have a last_id, we need to find where it is in `to_compact`.
        // If it's not found in `to_compact`, it might be that our 'last_id' is actually *older*
        // than the start of `to_compact` (which shouldn't happen if we strictly append),
        // OR it might be that we are re-compacting a range.
        // Simplified approach: scan `to_compact`.

        for msg in to_compact {
            if !found_last {
                if let Some(id) = &msg.id {
                    if Some(id.clone()) == effective_last_id {
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

        tracing::debug!(
            "Compaction filtering complete: {} messages selected for compaction out of {} in range",
            new_chunk_msgs.len(),
            to_compact.len()
        );

        // If nothing new to compact, just assemble result
        if new_chunk_msgs.is_empty() {
            tracing::info!(
                "Compaction: No new messages to compact. Summary: '{}' ({} chars), Raw messages: {}",
                if last_summary.is_empty() { "(none)" } else { &last_summary[..last_summary.len().min(100)] },
                last_summary.len(),
                keep_raw.len()
            );
            // We still need to return the summary message + kept raw messages
            let mut result = Vec::new();
            if !last_summary.is_empty() {
                result.push(Self::create_summary_message(last_summary.clone()));
            }
            
            // Validate and add keep_raw messages
            for msg in keep_raw.iter() {
                // Validate tool messages have required tool_call_id
                if msg.role == "tool" {
                    if msg.tool_call_id.is_none() || msg.tool_call_id.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true) {
                        tracing::error!(
                            "Compactor (early exit): Dropping tool message without valid tool_call_id (id: {:?})",
                            msg.id
                        );
                        continue; // Skip invalid tool messages
                    }
                }
                result.push(msg.clone());
            }
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
                "Summarize this conversation fragment in 2-4 concise sentences. Preserve key facts, decisions, and context. Output ONLY the summary.\n\nCONVERSATION:\n{}\n\nSUMMARY:",
                chunk_text
            )
        } else {
            format!(
                "Update the summary below by incorporating the new conversation fragment. Output ONLY the updated summary in 2-4 concise sentences.\n\nCURRENT SUMMARY:\n{}\n\nNEW CONVERSATION:\n{}\n\nUPDATED SUMMARY:",
                last_summary, chunk_text
            )
        };

        let sys_msg = Message {
            id: None,
            role: "system".to_string(),
            content: Some(
                "You are a Conversation Summarizer. Your ONLY job is to produce a concise factual summary of conversation history. Output ONLY the summary text with no commentary, no meta-discussion, no markdown formatting, and no preamble.".to_string(),
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
        
        tracing::debug!(
            "Compactor: Generated summary of {} chars for {} compacted messages",
            new_summary.len(),
            new_chunk_msgs.len()
        );

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
        
        // Clone and validate keep_raw messages
        for msg in keep_raw.iter() {
            // Validate tool messages have required tool_call_id
            if msg.role == "tool" {
                if msg.tool_call_id.is_none() || msg.tool_call_id.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true) {
                    tracing::error!(
                        "Compactor: Dropping tool message without valid tool_call_id (id: {:?}, content preview: {:?})",
                        msg.id,
                        msg.content.as_ref().map(|c| &c[..c.len().min(50)])
                    );
                    continue; // Skip invalid tool messages
                }
            }
            result.push(msg.clone());
        }

        // Final validation: Log message sequence for debugging
        let role_sequence: Vec<&str> = result.iter().map(|m| m.role.as_str()).collect();
        tracing::debug!(
            "Compactor returning {} messages. Role sequence: {:?}",
            result.len(),
            role_sequence
        );
        
        // Validate no tool messages immediately follow system message
        if result.len() >= 2 && result[0].role == "system" && result[1].role == "tool" {
            tracing::error!(
                "CRITICAL: Compactor produced invalid sequence [system, tool]. This should never happen!"
            );
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::librarian::Librarian;

    fn create_test_settings() -> Settings {
        Settings {
            context_uncompressed_msg_count: 10,
            context_model: Some("openai::gpt-4".to_string()),
            default_model: Some("openai::gpt-4".to_string()),
            providers: std::collections::HashMap::new(),
            ..Default::default()
        }
    }

    fn create_test_message(id: &str, role: &str, content: &str) -> Message {
        Message {
            id: Some(id.to_string()),
            role: role.to_string(),
            content: Some(content.to_string()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            attachments: None,
            created_at: None,
        }
    }

    async fn create_test_librarian() -> Arc<Mutex<Librarian>> {
        // Use a process-unique temp dir; passing ":memory:" as a Path historically
        // caused Librarian to create a real ":memory:" directory in the working
        // directory (which then got committed to git).
        let tmp = tempfile::tempdir().unwrap();
        let librarian = Librarian::new(tmp.path()).unwrap();
        // Intentionally leak the TempDir so it survives for the lifetime of the
        // test; cleanup happens when the test process exits.
        std::mem::forget(tmp);
        Arc::new(Mutex::new(librarian))
    }

    #[tokio::test]
    async fn test_compaction_disabled_when_count_is_zero() {
        let mut settings = Settings::default();
        settings.context_uncompressed_msg_count = 0;
        let messages = vec![
            create_test_message("1", "user", "Hello"),
            create_test_message("2", "assistant", "Hi there"),
        ];
        let librarian: Arc<Mutex<Librarian>> = create_test_librarian().await;

        let result = Compactor::compact(messages, &settings, Some("test"), librarian).await;
        assert!(result.is_ok());
        let (summary, msgs) = result.unwrap();
        assert!(summary.is_none());
        assert_eq!(msgs.len(), 2);
    }

    #[tokio::test]
    async fn test_compaction_skip_when_within_limit() {
        let settings = create_test_settings();
        let messages = vec![
            create_test_message("1", "user", "Hello"),
            create_test_message("2", "assistant", "Hi there"),
        ];
        let librarian: Arc<Mutex<Librarian>> = create_test_librarian().await;

        let result = Compactor::compact(messages, &settings, Some("test"), librarian).await;
        assert!(result.is_ok());
        let (summary, msgs) = result.unwrap();
        assert!(summary.is_none());
        assert_eq!(msgs.len(), 2);
    }

    #[tokio::test]
    async fn test_last_id_not_in_range_safety() {
        // Scenario: a stored summary references last_id="100" but the actual messages
        // being compacted have ids 1..=15 (the marker was pruned away). The safety
        // recovery must reset to "compact from the start" — previously, a bug in
        // found_last initialization left it false even after the safety reset,
        // causing the loop to skip every message and silently emit the stale old
        // summary. Here we use an invalid model id so the compactor falls through
        // to pass-through after the (now-correctly) recognized need to re-compact.
        let librarian: Arc<Mutex<Librarian>> = create_test_librarian().await;

        let mut settings = Settings::default();
        settings.context_uncompressed_msg_count = 5;
        settings.context_model = Some("invalid_model_id".to_string());
        settings.default_model = Some("invalid_model_id".to_string());

        let messages: Vec<Message> = (1..=15)
            .map(|i| create_test_message(&i.to_string(), "user", &format!("Message {}", i)))
            .collect();

        let conv_id = {
            let lib = librarian.lock().await;
            lib.sqlite.init_conversation("test_conv").unwrap()
        };

        {
            let lib = librarian.lock().await;
            lib.sqlite.save_conversation_summary(&conv_id, "100", "Old summary").unwrap();
        }

        let result = Compactor::compact(messages.clone(), &settings, Some(&conv_id), librarian).await;

        assert!(result.is_ok(), "Compaction should not error when last_id is unfindable");
        let (summary, msgs) = result.unwrap();
        // With the safety recovery in effect AND an invalid model id, the function
        // proceeds past the empty-chunk fallback (proving new_chunk_msgs was NOT
        // empty) and falls through to the invalid-model pass-through, returning the
        // original messages with no new summary. The pre-fix behaviour incorrectly
        // returned (Some(stale_summary), keep_raw) — exactly the silent data loss
        // remediation.md flagged.
        assert!(summary.is_none(), "Invalid model id should skip compaction, not emit a stale summary");
        assert_eq!(msgs.len(), 15, "Pass-through must return all original messages");
    }

    #[tokio::test]
    async fn test_empty_messages() {
        let settings = create_test_settings();
        let messages = vec![];
        let librarian: Arc<Mutex<Librarian>> = create_test_librarian().await;

        let result = Compactor::compact(messages, &settings, Some("test"), librarian).await;
        assert!(result.is_ok());
        let (summary, msgs) = result.unwrap();
        assert!(summary.is_none());
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn test_no_conversation_id() {
        let settings = create_test_settings();
        let messages = vec![
            create_test_message("1", "user", "Hello"),
            create_test_message("2", "assistant", "Hi there"),
        ];
        let librarian: Arc<Mutex<Librarian>> = create_test_librarian().await;

        let result = Compactor::compact(messages, &settings, None, librarian).await;
        assert!(result.is_ok());
        let (summary, msgs) = result.unwrap();
        assert!(summary.is_none());
        assert_eq!(msgs.len(), 2);
    }
}
