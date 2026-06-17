use crate::llm::openai::OpenAiProvider;
use crate::llm::provider::{Attachment, GenerationOptions, LlmProvider, Message};
use crate::mcp::config::Settings;
use crate::mcp::config::{McpServerConfig, ModelConfig, ProviderType};
use crate::mcp::manager::McpManager;
use crate::memory::tantivy_index::SearchResult;
use crate::memory::{Fact as MemoryFact, ObjectKind};
use crate::memory::extraction::FactExtractor;
use anyhow::Result;
use serde::Serialize;
use std::sync::Arc;
use tauri::{Manager, State};
use tokio::sync::Mutex;

use crate::llm::anthropic::AnthropicProvider;
use crate::llm::context::ContextManager;
use crate::llm::ollama::OllamaProvider;
use std::collections::HashMap;

pub mod llm;
pub use llm::capabilities;
pub mod mcp;
pub mod memory;
pub mod security;
pub mod skills;
pub mod tasks;

#[derive(Clone)]
pub struct AppState {
    mcp_manager: Arc<McpManager>,
    librarian: Arc<Mutex<crate::memory::librarian::Librarian>>,
    skills: Arc<crate::skills::SkillStore>,
    /// memory3 Phase 1: botmem-style doc store. None when docs are disabled or
    /// initialization failed (the rest of the app should keep working).
    doc_store: Arc<Mutex<Option<Arc<crate::memory::docs::DocStore>>>>,
    active_task: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
    // For context inspection: maps request_id -> oneshot sender for approval
    context_approvals: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>,
}

/// Instantiate the configured embedding provider. Returns `None` when the
/// selected provider is unavailable in this build (e.g. `fastembed` chosen but
/// the `local-embeddings` feature is off) or fails to initialise — recall then
/// falls back to BM25-only.
async fn init_embedder(
    settings: &Settings,
) -> Option<Arc<dyn crate::memory::docs::embedding::EmbeddingProvider>> {
    match settings.memory_embedding_provider.as_str() {
        "remote" => {
            let provider_id = match settings.memory_remote_embedding_provider_id.as_deref() {
                Some(id) if !id.is_empty() => id,
                _ => {
                    tracing::warn!(
                        "memory_embedding_provider=remote but no memory_remote_embedding_provider_id configured"
                    );
                    return None;
                }
            };
            let config = match settings.providers.get(provider_id) {
                Some(c) => c.clone(),
                None => {
                    tracing::warn!(
                        "memory_remote_embedding_provider_id '{provider_id}' is not in providers"
                    );
                    return None;
                }
            };
            match crate::memory::docs::embedding::RemoteEmbeddingProvider::try_new(
                provider_id.to_string(),
                settings.memory_remote_embedding_model.clone(),
                &config,
            )
            .await
            {
                Ok(p) => Some(
                    Arc::new(p)
                        as Arc<dyn crate::memory::docs::embedding::EmbeddingProvider>,
                ),
                Err(e) => {
                    tracing::warn!(
                        "Remote embedding init failed; docs recall will be BM25-only: {e}"
                    );
                    None
                }
            }
        }
        // "fastembed" or anything unknown falls through to the local provider.
        _ => {
            #[cfg(feature = "local-embeddings")]
            {
                match crate::memory::docs::embedding::FastembedProvider::try_default().await {
                    Ok(p) => return Some(Arc::new(p) as Arc<dyn crate::memory::docs::embedding::EmbeddingProvider>),
                    Err(e) => {
                        tracing::warn!("Fastembed init failed; docs recall will be BM25-only: {e}");
                    }
                }
            }
            None
        }
    }
}

#[tauri::command]
async fn stop_generation(state: State<'_, AppState>) -> Result<(), String> {
    let mut handle_guard = state.active_task.lock().await;
    if let Some(handle) = handle_guard.take() {
        tracing::info!("[Backend] Stopping generation...");
        handle.abort();
    }
    Ok(())
}

#[tauri::command]
async fn respond_to_context_inspection(
    state: State<'_, AppState>,
    request_id: String,
    approved: bool,
) -> Result<(), String> {
    let mut approvals = state.context_approvals.lock().await;
    if let Some(sender) = approvals.remove(&request_id) {
        let _ = sender.send(approved);
    }
    Ok(())
}

#[derive(Clone, Serialize)]
struct StreamChunkEvent {
    request_id: Option<String>,
    conversation_id: Option<String>,
    chunk: String,
    chunk_type: String, // "text" or "reasoning"
}

#[derive(Clone, Serialize)]
struct StreamStatsEvent {
    request_id: Option<String>,
    conversation_id: Option<String>,
    tokens_per_second: f64,
    total_tokens: usize,
    duration_ms: u64,
}

#[derive(serde::Serialize)]
struct Conversation {
    id: String,
    title: String,
    icon: Option<String>,
    created_at: String,
}

#[tauri::command]
async fn list_conversations(state: State<'_, AppState>) -> Result<Vec<Conversation>, String> {
    let lib = state.librarian.lock().await;
    let list = lib.list_conversations().map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|(id, title, icon, created_at)| Conversation {
            id,
            title,
            icon,
            created_at,
        })
        .collect())
}

#[tauri::command]
async fn create_conversation(state: State<'_, AppState>, title: String) -> Result<String, String> {
    let lib = state.librarian.lock().await;
    lib.create_conversation(&title).map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<(), String> {
    let lib = state.librarian.lock().await;
    lib.delete_conversation(&conversation_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_message(state: State<'_, AppState>, message_id: String) -> Result<(), String> {
    let lib = state.librarian.lock().await;
    lib.delete_messages(&[message_id])
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_messages(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
) -> Result<(), String> {
    let lib = state.librarian.lock().await;
    lib.delete_messages(&message_ids).map_err(|e| e.to_string())
}

#[tauri::command]
async fn rename_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
    new_title: String,
) -> Result<(), String> {
    let lib = state.librarian.lock().await;
    lib.rename_conversation(&conversation_id, &new_title)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn update_conversation_icon(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    icon: Option<String>,
) -> Result<(), String> {
    let lib = state.librarian.lock().await;
    lib.update_conversation_icon(&conversation_id, icon.as_deref())
        .map_err(|e| e.to_string())?;

    use tauri::Emitter;
    let _ = app.emit("conversations-updated", ());

    Ok(())
}

#[tauri::command]
async fn generate_title(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    provider_id: String,
    model: String,
) -> Result<String, String> {
    let lib_guard = state.librarian.lock().await;

    // Get first few messages
    let history = lib_guard
        .get_complete_history(&conversation_id)
        .map_err(|e| e.to_string())?;

    // Drop guard to allow async IO (request to LLM)
    drop(lib_guard);

    if history.is_empty() {
        return Ok("New Chat".to_string());
    }

    let mut prompt = String::new();
    for (_, (_, role, content, _, _, _, _, _)) in history.iter().enumerate().take(6) {
        if let Some(c) = content {
            prompt.push_str(&format!("{}: {}\n", role, c));
        }
    }

    prompt.push_str("\n\nInstructions: Generate a very brief (max 5 words) title and a single appropriate emoji for this conversation based on its content and main topic.\n\nFormat your response as:\nTitle: [title here]\nEmoji: [single emoji here]\n\nChoose an emoji that best represents the conversation's theme, topic, or purpose. Use only one emoji character.");

    // Select Provider
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let settings = Settings::load_migrated(&settings_path);

    let provider_config = settings
        .providers
        .get(&provider_id)
        .ok_or_else(|| format!("Provider '{}' not found in settings", provider_id))?;

    if !provider_config.enabled {
        return Err(format!("Provider '{}' is disabled", provider_id));
    }

    let tools = vec![];
    let messages = vec![Message {
        id: None,
        role: "user".to_string(),
        content: Some(prompt),
        reasoning_content: None,
        tool_calls: None,
        tool_call_id: None,
        attachments: None,
        created_at: None,
    }];

    let response = match provider_config.provider_type {
        ProviderType::OpenAI | ProviderType::OpenAICompatible => {
            let api_key = provider_config.api_key.clone().unwrap_or_default();
            let base_url = match provider_config.provider_type {
                ProviderType::OpenAI => None,
                _ => provider_config.base_url.clone(),
            };
            let provider = OpenAiProvider::new(api_key, base_url, model.clone());
            provider
                .chat(messages, tools, None)
                .await
                .map_err(|e| e.to_string())
        }
        ProviderType::Anthropic => {
            let api_key = provider_config.api_key.clone().unwrap_or_default();
            let base_url = provider_config.base_url.clone();
            let provider = AnthropicProvider::new(api_key, base_url, model.clone());
            provider
                .chat(messages, tools, None)
                .await
                .map_err(|e| e.to_string())
        }
        ProviderType::Ollama => {
            let base_url = provider_config
                .base_url
                .clone()
                .unwrap_or("http://localhost:11434".to_string());
            let provider = OllamaProvider::new(base_url, model.clone());
            provider
                .chat(messages, tools, None)
                .await
                .map_err(|e| e.to_string())
        }
    }?;

    let response_text = response
        .content
        .unwrap_or("Title: New Chat\nEmoji: 💬".to_string());

    // Parse title and emoji from response
    let mut new_title = "New Chat".to_string();
    let mut new_icon: Option<String> = None;

    for line in response_text.lines() {
        let line = line.trim();
        if line.starts_with("Title:") {
            new_title = line
                .strip_prefix("Title:")
                .unwrap_or("New Chat")
                .trim()
                .trim_matches('"')
                .to_string();
        } else if line.starts_with("Emoji:") {
            let emoji = line.strip_prefix("Emoji:").unwrap_or("").trim().to_string();
            if !emoji.is_empty() {
                new_icon = Some(emoji);
            }
        }
    }

    // Fallback: if parsing failed and response doesn't contain expected format, use the whole response as title
    if new_title == "New Chat" && !response_text.contains("Title:") {
        new_title = response_text.trim().trim_matches('"').to_string();
    }

    // Re-acquire lock to save
    let lib = state.librarian.lock().await;
    lib.update_conversation_title_and_icon(&conversation_id, &new_title, new_icon.as_deref())
        .map_err(|e| e.to_string())?;

    use tauri::Emitter;
    let _ = app.emit("conversations-updated", ());

    Ok(new_title)
}

#[tauri::command]
async fn get_chat_history(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<Message>, String> {
    let lib = state.librarian.lock().await;
    let raw = lib
        .get_complete_history(&conversation_id)
        .map_err(|e| e.to_string())?;
    // Collect valid tool_call_ids from assistant messages first
    let mut assistant_tool_call_ids = std::collections::HashSet::new();
    for (
        _id,
        role,
        _content,
        tool_calls_json,
        _tool_call_id,
        _reasoning_content,
        _created_at,
        _attachments_json,
    ) in raw.iter()
    {
        if role == "assistant" {
            if let Some(json_str) = tool_calls_json {
                if !json_str.is_empty() {
                    if let Ok(calls) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                        for call in calls {
                            if let Some(cid) = call.get("id").and_then(|v| v.as_str()) {
                                assistant_tool_call_ids.insert(cid.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    let mut messages = Vec::new();
    for (
        id,
        role,
        content,
        tool_calls_json,
        tool_call_id,
        reasoning_content,
        created_at,
        attachments_json,
    ) in raw
    {
        let tool_calls = if let Some(json_str) = tool_calls_json {
            if !json_str.is_empty() {
                serde_json::from_str(&json_str).ok()
            } else {
                None
            }
        } else {
            None
        };

        let attachments = if !attachments_json.is_empty() {
            serde_json::from_str(&attachments_json).ok()
        } else {
            None
        };

        // Parse created_at (assuming it's compatible with chrono, otherwise None)
        let created_at_ts = if let Ok(dt) =
            chrono::NaiveDateTime::parse_from_str(&created_at, "%Y-%m-%d %H:%M:%S")
        {
            Some(dt.and_utc().timestamp())
        } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&created_at) {
            Some(dt.timestamp())
        } else {
            // Try parsing as raw timestamp (i64) if it's just digits
            if let Ok(ts) = created_at.parse::<i64>() {
                Some(ts)
            } else {
                tracing::error!("Failed to parse timestamp: '{}'", created_at);
                None
            }
        };

        // Validate tool messages have valid tool_call_id before adding to history
        if role == "tool" {
            let has_valid_id = match &tool_call_id {
                Some(id) => !id.trim().is_empty(),
                None => false,
            };
            
            if !has_valid_id {
                tracing::warn!(
                    "get_chat_history: Skipping tool message (id: {}) without valid tool_call_id",
                    id
                );
                continue; // Skip this message
            }

            if let Some(tcid) = &tool_call_id {
                if !assistant_tool_call_ids.contains(tcid) {
                    tracing::warn!(
                        "get_chat_history: Skipping orphan tool message (id: {}, tool_call_id: {})",
                        id,
                        tcid
                    );
                    continue;
                }
            }
        }

        messages.push(Message {
            id: Some(id),
            role,
            content,
            reasoning_content,
            tool_calls,
            tool_call_id,
            attachments,
            created_at: created_at_ts,
        });
    }
    Ok(messages)
}

#[tauri::command]
async fn send_message(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    messages: Vec<Message>,
    provider_id: String,
    model: String,
    conversation_id: Option<String>,
    attachments: Option<Vec<Attachment>>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    stream: bool,
    max_tokens: Option<u32>,
    presence_penalty: Option<f32>,
    frequency_penalty: Option<f32>,
    reasoning_effort: Option<String>,
    request_id: Option<String>,
) -> Result<Message, String> {
    // Clone necessary data for the async task
    let state_owned = state.inner().clone();
    let app_handle_clone = app_handle.clone();
    let provider_id_clone = provider_id.clone();
    let model_clone = model.clone();
    let conversation_id_clone = conversation_id.clone();
    let request_id_clone = request_id.clone();
    let messages_clone = messages.clone();
    let attachments_clone = attachments.clone();

    // Spawn the generation task
    let task = tokio::spawn(async move {
        // --- ORIGINAL LOGIC START ---

        // We're working on clones now
        let mut messages = messages_clone;
        
        // Filter out invalid tool messages and detect duplicates
        let mut seen_tool_call_ids = std::collections::HashSet::new();
        let mut seen_assistant_tool_call_ids = std::collections::HashSet::new();
        
        // First pass: collect all tool_call IDs from assistant messages
        for msg in messages.iter() {
            if msg.role == "assistant" {
                if let Some(calls) = &msg.tool_calls {
                    for call in calls {
                        if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                            seen_assistant_tool_call_ids.insert(id.to_string());
                        }
                    }
                }
            }
        }
        
        messages.retain(|msg| {
            if msg.role == "tool" {
                let has_valid_id = match &msg.tool_call_id {
                    Some(id) => !id.trim().is_empty(),
                    None => false,
                };
                
                if !has_valid_id {
                    tracing::warn!(
                        "send_message: Filtering out tool message without valid tool_call_id (id: {:?})",
                        msg.id
                    );
                    return false;
                }
                
                let tool_call_id = msg.tool_call_id.as_ref().unwrap();
                
                // Check if this tool_call_id has been seen before (duplicate)
                if !seen_tool_call_ids.insert(tool_call_id.clone()) {
                    tracing::warn!(
                        "send_message: Filtering out duplicate tool message with tool_call_id: {}",
                        tool_call_id
                    );
                    return false;
                }
                
                // Check if this tool_call_id has a matching assistant message
                if !seen_assistant_tool_call_ids.contains(tool_call_id) {
                    tracing::warn!(
                        "send_message: Filtering out orphaned tool message with tool_call_id: {} (no matching assistant)",
                        tool_call_id
                    );
                    return false;
                }
            }
            true
        });
        let provider_id = provider_id_clone;
        let model = model_clone;
        let conversation_id = conversation_id_clone;
        let request_id = request_id_clone;
        let attachments = attachments_clone;
        let state = state_owned;
        let app_handle = app_handle_clone;

        if let Some(atts) = attachments {
            if let Some(last_msg) = messages.last_mut() {
                if last_msg.role == "user" {
                    last_msg.attachments = Some(atts);
                }
            }
        }
        let librarian_arc = state.librarian.clone();
        let provider_id_bg = provider_id.clone();
        let model_bg = model.clone();
        let conv_id_bg = conversation_id.clone();

        // Save User or Tool Message & Capture Query
        let mut query = String::new();
        if let Some(last) = messages.last() {
            if last.role == "user" || last.role == "tool" {
                if last.role == "user" {
                    query = last.content.clone().unwrap_or_default();
                }

                if let Some(conv_id) = &conversation_id {
                    let lib = state.librarian.lock().await;
                    let tool_calls_json = if let Some(tc) = &last.tool_calls {
                        serde_json::to_string(tc).ok()
                    } else {
                        None
                    };

                    // Phase 1.3: Tool-call integrity check
                    if last.role == "tool" {
                        if let Some(tid) = &last.tool_call_id {
                            // Reject empty tool_call_ids
                            if tid.trim().is_empty() {
                                tracing::error!(
                                    "Security: Tool message submitted with empty tool_call_id"
                                );
                                return Err("Tool message has empty tool_call_id".to_string());
                            }

                            // Verify this ID exists in the conversation history using database-backed validation
                            let tool_call_exists = match lib.tool_call_id_exists(conv_id, tid) {
                                Ok(exists) => exists,
                                Err(e) => {
                                    tracing::error!("Error checking tool_call_id: {}", e);
                                    return Err(format!("Error validating tool_call_id: {}", e));
                                }
                            };
                            if !tool_call_exists {
                                tracing::error!("Security: Attempted to submit tool result with invalid tool_call_id: {}", tid);
                                return Err(format!("Invalid tool_call_id: {}", tid));
                            }
                        } else {
                            tracing::error!(
                                "Security: Tool message submitted without tool_call_id"
                            );
                            return Err("Tool message must have tool_call_id".to_string());
                        }
                    }

                    // Save full message (only if not already saved - check for existing ID)
                    // When regenerating, messages already have IDs and shouldn't be saved again
                    if last.id.is_none() {
                        if last.role == "user" {
                            match lib.save_full_message_returning_id(
                                conv_id,
                                &last.role,
                                last.content.as_deref(),
                                tool_calls_json.as_deref(),
                                last.tool_call_id.as_deref(),
                                None, // reasoning_content (users don't have reasoning)
                                last.attachments.as_deref(),
                            ) {
                                Err(e) => {
                                    tracing::error!("Failed to save user message: {}", e);
                                }
                                Ok(msg_id) => {
                                    // Tell the frontend the persisted id of the
                                    // user message it just sent. Without this the
                                    // message stays id=None in the UI, so a later
                                    // Regenerate replays it id-less — defeating the
                                    // `last.id.is_none()` guard above and inserting
                                    // a duplicate user row on every regenerate.
                                    use tauri::Emitter;
                                    let _ = app_handle.emit(
                                        "user-message-saved",
                                        serde_json::json!({
                                            "request_id": request_id,
                                            "conversation_id": conv_id,
                                            "message_id": msg_id,
                                        }),
                                    );
                                    // memory3 Phase 3: per-turn fact extraction
                                    // has been removed. Facts are now written
                                    // only via /remember, "Save as fact", the
                                    // session-end pass, or the LLM tool
                                    // `memory_remember_fact`.
                                }
                            }
                        } else {
                            let _ = lib.save_full_message(
                                conv_id,
                                &last.role,
                                last.content.as_deref(),
                                tool_calls_json.as_deref(),
                                last.tool_call_id.as_deref(),
                                None, // reasoning_content (tool messages don't have reasoning)
                                last.attachments.as_deref(),
                            );
                        }
                    }
                }
            }
        }

        // Load settings once for this request
        let config_dir = app_handle
            .path()
            .app_config_dir()
            .map_err(|e| e.to_string())?;
        let settings_path = config_dir.join("settings.json");
        let settings = Settings::load_migrated(&settings_path);
        let memory_enabled = settings.memory_enabled;

        // --- CONTEXT COMPACTION ---
        // Compact messages before generating context.
        // We do this if compaction is enabled (count > 0).
        let (_compacted_summary, effective_messages) = if settings.context_uncompressed_msg_count > 0
        {
            match crate::llm::compactor::Compactor::compact(
                messages.clone(),
                &settings,
                conversation_id.as_deref(),
                state.librarian.clone(),
            )
            .await
            {
                Ok((summary, compacted_msgs)) => {
                    if let Some(s) = &summary {
                        tracing::debug!("Context compaction applied. Summary length: {}", s.len());
                    }
                    (summary, compacted_msgs)
                }
                Err(e) => {
                    tracing::error!("Context compaction failed: {}. Using original messages.", e);
                    (None, messages.clone())
                }
            }
        } else {
            (None, messages.clone())
        };

        // Use effective_messages for context retrieval and final generation
        let messages_for_context = effective_messages.clone();

        // Retrieve Context (Long-term memory) via the DocStore auto-injection
        // layer (memory3 Phase 2). Replaces the verbose strategist
        // planner/synthesizer with a deterministic recall + KG-prose block.
        let mut context_text = String::new();
        if memory_enabled
            && settings.memory_auto_inject_docs
            && !query.is_empty()
        {
            let store = {
                let slot = state.doc_store.lock().await;
                slot.clone()
            };
            if let Some(store) = store {
                match store
                    .auto_inject(
                        &query,
                        state.librarian.clone(),
                        settings.memory_auto_inject_token_budget,
                        settings.memory_recall_score_floor,
                    )
                    .await
                {
                    Ok(result) if !result.is_empty() => {
                        context_text = result.text.clone();

                        use tauri::Emitter;
                        if let Err(e) = app_handle.emit("memory-context", &vec![context_text.clone()])
                        {
                            tracing::error!("Failed to emit memory-context: {}", e);
                        }
                        if let Err(e) =
                            app_handle.emit("memory-selected-doc", &result.doc_id)
                        {
                            tracing::error!("Failed to emit memory-selected-doc: {}", e);
                        }
                        if let Err(e) =
                            app_handle.emit("memory-selected-fact-ids", &result.fact_ids)
                        {
                            tracing::error!("Failed to emit memory-selected-fact-ids: {}", e);
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("DocStore auto-inject failed: {}", e);
                    }
                }
            }
        }

        // Inject Context
        let mut final_messages = messages_for_context; // Use compacted messages

        // === Stable system prefix (cacheable) — prompt-caching Phase 0b ===
        // Stable system content (active system prompt + skills block) forms one
        // contiguous block at the FRONT so the [tools + system + history] prefix is
        // byte-stable across turns. Per-turn volatile content (datetime, task
        // checklist, long-term memory) is appended AFTER the history as a trailing
        // <system-reminder> instead (see below), so it never invalidates the cached
        // prefix.

        // Skills block (stable: changes only when a skill is added/removed). Only
        // injected when skills exist (slug + description per skill — bodies are
        // pulled on demand by the tool). Inserted before the system prompt so the
        // final front order is [system-prompt, skills, ...history].
        if let Some(skills_block) = state.skills.render_for_system_prompt().await {
            final_messages.insert(
                0,
                Message {
                    id: None,
                    role: "system".to_string(),
                    content: Some(skills_block),
                    reasoning_content: None,
                    tool_calls: None,
                    tool_call_id: None,
                    attachments: None,
                    created_at: None,
                },
            );
        }

        // Active system prompt (stable). Inserted at 0 so it precedes the skills block.
        tracing::debug!("[DEBUG] Loading settings for system prompt...");
        if let Some(active_id) = &settings.active_system_prompt_id {
            if let Some(prompt) = settings.system_prompts.iter().find(|p| &p.id == active_id) {
                tracing::debug!("[DEBUG] Injecting system prompt: {}", prompt.name);
                final_messages.insert(
                    0,
                    Message {
                        id: Some(uuid::Uuid::new_v4().to_string()),
                        role: "system".to_string(),
                        content: Some(prompt.content.clone()),
                        reasoning_content: None,
                        tool_call_id: None,
                        tool_calls: None,
                        attachments: None,
                        created_at: None,
                    },
                );
            }
        }

        // === Volatile trailing reminder (uncached) — prompt-caching Phase 0b ===
        // Collect per-turn content and append it AFTER the history as a single
        // role:"user" <system-reminder>. It MUST be role:"user": convert_messages
        // (anthropic.rs) folds every role:"system" message into the flat system
        // prefix regardless of position, which would pull this volatile content back
        // into the cached prefix and defeat the split. Placing it after the history
        // also reads more correctly — "now" is relative to the latest turn.
        let mut volatile_sections: Vec<String> = Vec::new();

        // Current date/time (always present). Replaces the per-message
        // <timestamp: ...> prefix that was previously attached to each user message.
        let now = chrono::Local::now();
        let tz = now.format("%Z").to_string();
        volatile_sections.push(format!(
            "The current local date and time is {} ({}). \
             Treat this as the authoritative reference for \"now\" when the user \
             asks about today, yesterday, recent events, deadlines, or any other \
             time-relative question. Prior messages in this conversation may have \
             been sent at earlier times; if the exact timing of a previous message \
             matters, ask the user to confirm rather than assuming it is current.",
            now.format("%A, %B %d, %Y %H:%M:%S"),
            tz,
        ));

        // Current task checklist (if any) so the model knows what's pending vs. done.
        if let Some(conv_id) = &conversation_id {
            let task_context = {
                let lib = state.librarian.lock().await;
                lib.format_tasks_for_context(conv_id).unwrap_or(None)
            };
            if let Some(text) = task_context {
                volatile_sections.push(text);
            }
        }

        // Long-term memory recall for this turn (if any).
        if !context_text.is_empty() {
            volatile_sections.push(format!("Long-term memory for this turn:\n\n{}", context_text));
        }

        final_messages.push(Message {
            id: None,
            role: "user".to_string(),
            content: Some(format!(
                "<system-reminder>\n{}\n</system-reminder>",
                volatile_sections.join("\n\n")
            )),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            attachments: None,
            created_at: None,
        });

        tracing::debug!("[DEBUG] Getting tools from MCP Manager...");
        let all_tools = state.mcp_manager.get_all_tools().await;

        let mut tools: Vec<_> = all_tools
            .into_iter()
            .filter(|t| !settings.disabled_tools.contains(&t.name))
            .collect();

        // Inject the built-in update_tasks tool unless the user has disabled it.
        if !settings.disable_builtin_task_tool {
            tools.push(crate::tasks::build_update_tasks_tool());
        }

        // Inject the memory_* built-in tools (memory3 Phase 1). They are
        // exposed only when the DocStore has finished initialising.
        let doc_store_ready = {
            let slot = state.doc_store.lock().await;
            slot.is_some()
        };
        if doc_store_ready {
            for t in crate::memory::docs::tools::build_all() {
                if !settings.disabled_tools.contains(&t.name) {
                    tools.push(t);
                }
            }
        }

        // Inject the `use_skill` built-in tool. Only exposed when at least one
        // skill exists on disk — otherwise it would just be a noisy entry.
        let skill_summaries = state.skills.list().await;
        if !skill_summaries.is_empty() {
            for t in crate::skills::tools::build_all() {
                if !settings.disabled_tools.contains(&t.name) {
                    tools.push(t);
                }
            }
        }

        // Deterministic tool order (prompt-caching Phase 0a). Sort AFTER the
        // builtins are appended so the whole array is canonical. get_all_tools
        // iterates a HashMap (unordered), so without this the tools block — which
        // renders first in the prefix — never caches, and a later cache_control
        // breakpoint on the last tool would land on a nondeterministic entry.
        tools.sort_by(|a, b| a.name.cmp(&b.name));

        tracing::debug!("[DEBUG] Final tool count: {}", tools.len());

        // Phase 1.4: Streaming parity safety
        // Check capabilities
        let provider_type = match provider_id.as_str() {
            "openai" => ProviderType::OpenAI,
            "anthropic" => ProviderType::Anthropic,
            "ollama" => ProviderType::Ollama,
            _ => ProviderType::OpenAICompatible,
        };

        let caps = crate::llm::capabilities::get_capabilities(&provider_type, &model);

        // If streaming is requested but not supported for tools, and we have tools, force stream=false
        let mut effective_stream = stream;
        if stream && !caps.supports_streaming_tools && !tools.is_empty() {
            tracing::warn!(
                "Provider {:?} does not support streaming with tools. Forcing non-streaming.",
                provider_type
            );
            effective_stream = false;
        }
        tracing::info!(
            "🎬 Streaming requested: {}, effective: {}, tools count: {}",
            stream,
            effective_stream,
            tools.len()
        );

        // 5. Instantiate Provider
        let provider_config = settings.providers.get(&provider_id).ok_or_else(|| {
            tracing::error!("Provider not found in settings: {}", provider_id);
            format!("Provider {} not found in settings", provider_id)
        })?;
        if !provider_config.enabled {
            return Err(format!("Provider '{}' is disabled", provider_id));
        }

        // Determine context limit
        let model_config = provider_config.models.iter().find(|m| m.id == model);
        let context_limit = model_config
            .and_then(|m| m.context_window)
            .unwrap_or(128000); // Default to 128k (standard for modern models), fallback was 64k

        // Prune context
        let pruned_messages = ContextManager::prune_messages(final_messages, context_limit)
            .map_err(|e| e.to_string())?;
        tracing::debug!("[DEBUG] Messages pruned. Calling provider...");

        // Context Inspection: If enabled, emit context and wait for user approval
        if settings.context_inspection_enabled {
            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = tokio::sync::oneshot::channel::<bool>();

            // Store the approval sender
            {
                let mut approvals = state.context_approvals.lock().await;
                approvals.insert(request_id.clone(), tx);
            }

            // Format the context for display
            let context_preview: Vec<serde_json::Value> = pruned_messages
                .iter()
                .map(|msg| {
                    serde_json::json!({
                        "role": msg.role,
                        "content": msg.content.clone().unwrap_or_default(),
                        "reasoning_content": msg.reasoning_content.clone().unwrap_or_default(),
                        "tool_calls": msg.tool_calls.as_ref().map(|tc| serde_json::to_string(tc).unwrap_or_default()),
                        "tool_call_id": msg.tool_call_id.clone(),
                    })
                })
                .collect();

            // Emit context inspection event
            use tauri::Emitter;
            if let Err(e) = app_handle.emit(
                "context-inspection-request",
                serde_json::json!({
                    "request_id": request_id,
                    "messages": context_preview,
                    "tools_count": tools.len(),
                    "provider": provider_id,
                    "model": model,
                }),
            ) {
                tracing::error!("Failed to emit context-inspection-request: {}", e);
                return Err("Failed to emit context inspection request".to_string());
            }

            // Wait for user approval (with timeout)
            match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
                Ok(Ok(true)) => {
                    tracing::info!("Context inspection approved");
                }
                Ok(Ok(false)) => {
                    tracing::info!("Context inspection rejected");
                    return Err("Context inspection rejected by user".to_string());
                }
                Ok(Err(_)) => {
                    tracing::error!("Context inspection channel closed unexpectedly");
                    return Err("Context inspection failed".to_string());
                }
                Err(_) => {
                    // Timeout - clean up
                    let mut approvals = state.context_approvals.lock().await;
                    approvals.remove(&request_id);
                    tracing::error!("Context inspection timed out");
                    return Err("Context inspection timed out".to_string());
                }
            }
        }

        let options = Some(GenerationOptions {
            temperature,
            top_p,
            stream,
            max_tokens,
            presence_penalty,
            frequency_penalty,
            reasoning_effort,
        });

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let app_handle_for_stream = app_handle.clone();
        let stream_conversation_id = conversation_id.clone();
        let stream_request_id = request_id.clone();

        // Aggregator task: Buffers tokens and emits events every 20ms to avoid flooding the IPC/event loop
        let aggregator_handle = tauri::async_runtime::spawn(async move {
            use tauri::Emitter;
            let mut text_buf = String::new();
            let mut reason_buf = String::new();
            let mut last_emit = std::time::Instant::now();
            let throttle = std::time::Duration::from_millis(20);

            while let Some(content) = rx.recv().await {
                match content {
                    crate::llm::provider::StreamContent::Text(s) => text_buf.push_str(&s),
                    crate::llm::provider::StreamContent::Reasoning(s) => reason_buf.push_str(&s),
                }

                if last_emit.elapsed() >= throttle {
                    if !text_buf.is_empty() {
                        let _ = app_handle_for_stream.emit(
                            "stream-chunk",
                            StreamChunkEvent {
                                request_id: stream_request_id.clone(),
                                conversation_id: stream_conversation_id.clone(),
                                chunk: text_buf.clone(),
                                chunk_type: "text".to_string(),
                            },
                        );
                        text_buf.clear();
                    }
                    if !reason_buf.is_empty() {
                        let _ = app_handle_for_stream.emit(
                            "stream-chunk",
                            StreamChunkEvent {
                                request_id: stream_request_id.clone(),
                                conversation_id: stream_conversation_id.clone(),
                                chunk: reason_buf.clone(),
                                chunk_type: "reasoning".to_string(),
                            },
                        );
                        reason_buf.clear();
                    }
                    last_emit = std::time::Instant::now();
                }
            }

            // Final flush
            if !text_buf.is_empty() {
                let _ = app_handle_for_stream.emit(
                    "stream-chunk",
                    StreamChunkEvent {
                        request_id: stream_request_id.clone(),
                        conversation_id: stream_conversation_id.clone(),
                        chunk: text_buf,
                        chunk_type: "text".to_string(),
                    },
                );
            }
            if !reason_buf.is_empty() {
                let _ = app_handle_for_stream.emit(
                    "stream-chunk",
                    StreamChunkEvent {
                        request_id: stream_request_id.clone(),
                        conversation_id: stream_conversation_id.clone(),
                        chunk: reason_buf,
                        chunk_type: "reasoning".to_string(),
                    },
                );
            }
        });

        let on_token = Box::new(move |content: crate::llm::provider::StreamContent| {
            let _ = tx.send(content);
        });

        let mut on_token_opt = Some(on_token);

        let gen_start = std::time::Instant::now();

        let response_result = match provider_config.provider_type {
            ProviderType::OpenAI | ProviderType::OpenAICompatible => {
                let api_key = provider_config.api_key.clone().unwrap_or_default();
                let base_url = match provider_config.provider_type {
                    ProviderType::OpenAI => None, // always use official base
                    _ => provider_config.base_url.clone(),
                };
                let provider = OpenAiProvider::new(api_key, base_url, model.clone());

                if effective_stream {
                    provider
                        .stream(
                            pruned_messages,
                            tools,
                            options,
                            on_token_opt.take().unwrap(),
                        )
                        .await
                } else {
                    provider.chat(pruned_messages, tools, options).await
                }
            }
            ProviderType::Anthropic => {
                let api_key = provider_config.api_key.clone().unwrap_or_default();
                let base_url = provider_config.base_url.clone();
                let provider = AnthropicProvider::new(api_key, base_url, model.clone());

                if effective_stream {
                    provider
                        .stream(
                            pruned_messages,
                            tools,
                            options,
                            on_token_opt.take().unwrap(),
                        )
                        .await
                } else {
                    provider.chat(pruned_messages, tools, options).await
                }
            }
            ProviderType::Ollama => {
                let base_url = provider_config
                    .base_url
                    .clone()
                    .unwrap_or("http://localhost:11434".to_string());
                let provider = OllamaProvider::new(base_url, model.clone());

                if effective_stream {
                    provider
                        .stream(
                            pruned_messages,
                            tools,
                            options,
                            on_token_opt.take().unwrap(),
                        )
                        .await
                } else {
                    provider.chat(pruned_messages, tools, options).await
                }
            }
        };

        // Ensure all tokens are emitted by the aggregator before continuing
        drop(on_token_opt); // Explicitly drop sender if it wasn't used to close the channel
        let _ = aggregator_handle.await;

        if effective_stream {
            if let Ok(ref resp) = response_result {
                let duration_ms = gen_start.elapsed().as_millis() as u64;
                let content = resp.content.as_deref().unwrap_or("");
                let token_count = crate::llm::tokenizer::Tokenizer::count_tokens(content)
                    .unwrap_or_else(|_| content.len() / 4);
                let tokens_per_second = if duration_ms > 0 {
                    token_count as f64 * 1000.0 / duration_ms as f64
                } else {
                    0.0
                };
                use tauri::Emitter;
                let _ = app_handle.emit(
                    "stream-stats",
                    StreamStatsEvent {
                        request_id: request_id.clone(),
                        conversation_id: conversation_id.clone(),
                        tokens_per_second,
                        total_tokens: token_count,
                        duration_ms,
                    },
                );
            }
        }

        let mut response = response_result.map_err(|e| e.to_string())?;

        // Handle Response (Save & Prune triggers)
        if let Some(conv_id) = conversation_id.clone() {
            let lib = state.librarian.lock().await;
            let tool_calls_json = if let Some(tc) = &response.tool_calls {
                serde_json::to_string(tc).ok()
            } else {
                None
            };

            // Note: If streaming, 'response' contains the FULL aggregated content at the end, so saving works fine.

            let assistant_message_id_for_save = match lib.save_full_message_returning_id(
                &conv_id,
                "assistant",
                response.content.as_deref(),
                tool_calls_json.as_deref(),
                response.tool_call_id.as_deref(),
                response.reasoning_content.as_deref(),
                None,
            ) {
                Ok(id) => Some(id),
                Err(e) => {
                    tracing::error!("Failed to save assistant message for facts: {}", e);
                    None
                }
            };

            if let Some(saved_id) = &assistant_message_id_for_save {
                response.id = Some(saved_id.clone());
            }

            // Trigger Background Pruning (Fire & Forget)
            let app_handle_for_pruning = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = attempt_pruning(
                    app_handle_for_pruning,
                    librarian_arc.clone(),
                    provider_id_bg.clone(),
                    model_bg.clone(),
                    conv_id_bg.clone().unwrap_or_default(),
                )
                .await
                {
                    tracing::error!("Pruning Error: {}", e);
                }
            });

            // memory3 Phase 3: per-turn assistant fact extraction has been
            // removed. Facts are only written via the explicit / session_end
            // policies or the `memory_remember_fact` LLM tool. The captured
            // `assistant_message_id_for_save` is still used above to set
            // `response.id`; nothing else is needed here.
            let _ = &assistant_message_id_for_save;
        }

        Ok(response)
    });

    // Store abort handle
    let abort_handle = task.abort_handle();
    {
        let mut handle_guard = state.active_task.lock().await;
        *handle_guard = Some(abort_handle);
    }

    // Await task
    match task.await {
        Ok(res) => {
            // Clear handle
            let mut handle_guard = state.active_task.lock().await;
            *handle_guard = None;
            res
        }
        Err(e) => {
            // Task fail (cancelled or panic)
            let mut handle_guard = state.active_task.lock().await;
            *handle_guard = None;
            if e.is_cancelled() {
                Err("Generation cancelled by user.".to_string())
            } else {
                Err(format!("Task execution failed: {}", e))
            }
        }
    }
}




// ---------- memory3 Phase 1: docs subsystem dispatch + UI commands ----------

async fn dispatch_memory_tool(
    store: &Arc<crate::memory::docs::DocStore>,
    librarian: &Arc<Mutex<crate::memory::librarian::Librarian>>,
    name: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    use crate::memory::docs::api::*;
    use crate::memory::docs::tools::*;

    fn err_value(e: &DocsError) -> serde_json::Value {
        serde_json::to_value(e).unwrap_or_else(|_| serde_json::json!({"code": e.code, "message": e.message}))
    }

    match name {
        n if n == TOOL_DOC_REMEMBER => {
            let input: RememberInput = serde_json::from_value(args)
                .map_err(|e| format!("memory_doc_remember: invalid args: {e}"))?;
            match store.remember(input).await {
                Ok(out) => Ok(serde_json::to_value(out).unwrap()),
                Err(e) => Ok(err_value(&e)),
            }
        }
        n if n == TOOL_DOC_FETCH => {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "memory_doc_fetch: missing id".to_string())?;
            match store.fetch(id).await.map_err(|e| e.to_string())? {
                Some(doc) => Ok(serde_json::to_value(doc).unwrap()),
                None => Ok(err_value(&DocsError::new("NOT_FOUND", "no such doc"))),
            }
        }
        n if n == TOOL_DOC_EDIT => {
            let input: EditInput = serde_json::from_value(args)
                .map_err(|e| format!("memory_doc_edit: invalid args: {e}"))?;
            match store.edit(input).await {
                Ok(out) => Ok(serde_json::to_value(out).unwrap()),
                Err(e) => Ok(err_value(&e)),
            }
        }
        n if n == TOOL_DOC_FORGET => {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "memory_doc_forget: missing id".to_string())?;
            match store.forget(id).await {
                Ok(()) => Ok(serde_json::json!({"ok": true, "id": id})),
                Err(e) => Ok(err_value(&e)),
            }
        }
        n if n == TOOL_DOC_RECALL => {
            let input: RecallInput = serde_json::from_value(args)
                .map_err(|e| format!("memory_doc_recall: invalid args: {e}"))?;
            let out = store.recall(input).await.map_err(|e| e.to_string())?;
            Ok(serde_json::to_value(out).unwrap())
        }
        n if n == TOOL_DOC_LINK_CONTEXT => {
            let input: LinkContextInput = serde_json::from_value(args)
                .map_err(|e| format!("memory_doc_link_context: invalid args: {e}"))?;
            let out = store.link_context(input).await.map_err(|e| e.to_string())?;
            Ok(serde_json::to_value(out).unwrap())
        }
        n if n == TOOL_FACT_REMEMBER => {
            let subject = args
                .get("subject")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "memory_fact_remember: missing subject".to_string())?
                .trim();
            let predicate = args
                .get("predicate")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "memory_fact_remember: missing predicate".to_string())?
                .trim();
            let object = args
                .get("object")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "memory_fact_remember: missing object".to_string())?
                .trim();
            if subject.is_empty() || predicate.is_empty() || object.is_empty() {
                return Err("memory_fact_remember: subject/predicate/object must be non-empty".into());
            }
            let object_kind = match args
                .get("object_kind")
                .and_then(|v| v.as_str())
                .map(|s| s.to_lowercase())
                .as_deref()
            {
                Some("entity") => ObjectKind::Entity,
                _ => ObjectKind::Literal,
            };
            let confidence = args
                .get("confidence")
                .and_then(|v| v.as_f64())
                .map(|f| f as f32)
                .unwrap_or(0.9)
                .clamp(0.0, 1.0);

            let new_fact = crate::memory::NewFact::new(
                crate::memory::extraction::FactExtractor::normalize_key(subject),
                crate::memory::extraction::FactExtractor::normalize_key(predicate),
                object.to_string(),
                object_kind,
                confidence,
                None,
            );
            let lib = librarian.lock().await;
            let id = lib
                .upsert_fact(new_fact)
                .map_err(|e| format!("memory_fact_remember: {e}"))?;
            Ok(serde_json::json!({ "id": id, "ok": true }))
        }
        n if n == TOOL_FACT_RECALL => {
            let subject = args.get("subject").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
            let predicate = args.get("predicate").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
            let object = args.get("object").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(20)
                .clamp(1, 100);
            let lib = librarian.lock().await;
            let facts = lib
                .sqlite
                .search_facts_like(subject, predicate, object, limit)
                .map_err(|e| format!("memory_fact_recall: {e}"))?;
            Ok(serde_json::json!({ "facts": facts }))
        }
        n if n == TOOL_FACT_FORGET => {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "memory_fact_forget: missing id".to_string())?;
            let lib = librarian.lock().await;
            lib.delete_fact(id).map_err(|e| format!("memory_fact_forget: {e}"))?;
            Ok(serde_json::json!({ "ok": true, "id": id }))
        }
        _ => Err(format!("memory dispatch: unknown tool '{name}'")),
    }
}

#[tauri::command]
async fn list_memory_docs(
    state: State<'_, AppState>,
) -> Result<Vec<crate::memory::docs::api::DocSummary>, String> {
    let store = {
        let slot = state.doc_store.lock().await;
        slot.clone()
    };
    let store = store.ok_or_else(|| "Memory subsystem is not ready yet.".to_string())?;
    store.list_docs().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn fetch_memory_doc(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<crate::memory::docs::api::DocRecord>, String> {
    let store = {
        let slot = state.doc_store.lock().await;
        slot.clone()
    };
    let store = store.ok_or_else(|| "Memory subsystem is not ready yet.".to_string())?;
    store.fetch(&id).await.map_err(|e| e.to_string())
}

/// Hybrid (cosine + BM25) search across memory docs. Exposed for the
/// `/recall` slash command so the user can search directly without an
/// LLM round-trip through the `memory_doc_recall` tool.
#[tauri::command]
async fn recall_memory_docs(
    state: State<'_, AppState>,
    query: String,
    k: Option<usize>,
) -> Result<crate::memory::docs::api::RecallOutput, String> {
    let store = {
        let slot = state.doc_store.lock().await;
        slot.clone()
    };
    let store = store.ok_or_else(|| "Memory subsystem is not ready yet.".to_string())?;
    let input = crate::memory::docs::api::RecallInput {
        query,
        k: k.unwrap_or(5),
        tags: Vec::new(),
    };
    store.recall(input).await.map_err(|e| e.to_string())
}

#[derive(Clone, Serialize)]
struct StoragePaths {
    config_dir: String,
    settings_path: String,
    sqlite_db: String,
    message_index: String,
    docs_dir: String,
    docs_index: String,
    skills_dir: String,
}

/// Return the resolved filesystem paths for every place Nebula writes data on
/// this machine. Surfaced in Settings → Long-term Memory → Storage locations.
#[tauri::command]
async fn get_storage_paths(app: tauri::AppHandle) -> Result<StoragePaths, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let memory = config_dir.join("memory");
    Ok(StoragePaths {
        config_dir: config_dir.to_string_lossy().into_owned(),
        settings_path: config_dir.join("settings.json").to_string_lossy().into_owned(),
        sqlite_db: config_dir.join("nebula.db").to_string_lossy().into_owned(),
        message_index: config_dir
            .join("fulltext_index")
            .to_string_lossy()
            .into_owned(),
        docs_dir: memory.join("docs").to_string_lossy().into_owned(),
        docs_index: memory.join("docs_index").to_string_lossy().into_owned(),
        skills_dir: config_dir.join("skills").to_string_lossy().into_owned(),
    })
}

// ---------- Skills commands ----------

#[tauri::command]
async fn list_skills(
    state: State<'_, AppState>,
) -> Result<Vec<crate::skills::SkillSummary>, String> {
    Ok(state.skills.list().await)
}

#[tauri::command]
async fn get_skill(
    state: State<'_, AppState>,
    slug: String,
) -> Result<Option<crate::skills::Skill>, String> {
    Ok(state.skills.get(&slug).await)
}

#[tauri::command]
async fn create_skill(
    state: State<'_, AppState>,
    slug: String,
    name: String,
    description: String,
    body: String,
) -> Result<(), String> {
    state
        .skills
        .create(&slug, &name, &description, &body)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn update_skill(
    state: State<'_, AppState>,
    slug: String,
    name: String,
    description: String,
    body: String,
) -> Result<(), String> {
    state
        .skills
        .update(&slug, &name, &description, &body)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_skill(state: State<'_, AppState>, slug: String) -> Result<(), String> {
    state.skills.delete(&slug).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn reload_skills(state: State<'_, AppState>) -> Result<(), String> {
    state.skills.reload().await.map_err(|e| e.to_string())
}

#[derive(Clone, Serialize)]
struct ExtractionResult {
    extracted: usize,
    message: String,
}

/// Drive a one-shot fact extraction over arbitrary text. Used by the
/// `/remember <text>` chat command.
#[tauri::command]
async fn extract_facts_from_text(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> Result<ExtractionResult, String> {
    if text.trim().is_empty() {
        return Err("empty text".into());
    }
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let settings = Settings::load_migrated(&settings_path);

    let context_model = settings
        .context_model
        .clone()
        .ok_or_else(|| "No context_model configured; set one in Settings".to_string())?;
    let parts: Vec<&str> = context_model.split("::").collect();
    if parts.len() != 2 {
        return Err(format!("Malformed context_model '{}'", context_model));
    }
    let provider = crate::llm::factory::create_provider(parts[0], parts[1], &settings)
        .map_err(|e| e.to_string())?;
    let msg = FactExtractor::extract_with_source(
        state.librarian.clone(),
        provider.as_ref(),
        "user",
        &text,
        None,
    )
    .await
    .map_err(|e| e.to_string())?;

    let extracted = parse_extract_count(&msg);
    Ok(ExtractionResult {
        extracted,
        message: msg,
    })
}

/// Extract facts from an existing stored message, e.g. when the user clicks
/// "Save as fact" on an assistant turn.
#[tauri::command]
async fn extract_facts_for_message(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    message_id: String,
) -> Result<ExtractionResult, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let settings = Settings::load_migrated(&settings_path);

    let context_model = settings
        .context_model
        .clone()
        .ok_or_else(|| "No context_model configured; set one in Settings".to_string())?;
    let parts: Vec<&str> = context_model.split("::").collect();
    if parts.len() != 2 {
        return Err(format!("Malformed context_model '{}'", context_model));
    }
    let provider = crate::llm::factory::create_provider(parts[0], parts[1], &settings)
        .map_err(|e| e.to_string())?;

    let (role, content) = {
        let lib = state.librarian.lock().await;
        let rows = lib
            .sqlite
            .get_messages_by_ids(&[message_id.clone()])
            .map_err(|e| e.to_string())?;
        let (id, role, _tc, _tcid) = rows
            .into_iter()
            .next()
            .ok_or_else(|| format!("message {message_id} not found"))?;
        // get_messages_by_ids does not return content; re-read via the conversation lookup.
        let _ = id;
        let content = lib
            .sqlite
            .get_message_content(&message_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("message {message_id} has no content"))?;
        (role, content)
    };

    let msg = FactExtractor::extract_with_source(
        state.librarian.clone(),
        provider.as_ref(),
        &role,
        &content,
        Some(message_id.clone()),
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(ExtractionResult {
        extracted: parse_extract_count(&msg),
        message: msg,
    })
}

/// Run the session-end extraction pass for a conversation, when the
/// `fact_extraction_policy` setting is `"session_end"`. Walks messages added
/// since the per-conversation checkpoint (stored in `memory_meta`) and runs a
/// single extraction over their concatenation. Returns the number of facts
/// written.
#[tauri::command]
async fn extract_session_end(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<ExtractionResult, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let settings = Settings::load_migrated(&settings_path);
    if settings.fact_extraction_policy != "session_end" {
        return Ok(ExtractionResult {
            extracted: 0,
            message: "policy is not session_end; skipping".into(),
        });
    }

    let context_model = settings
        .context_model
        .clone()
        .ok_or_else(|| "No context_model configured; set one in Settings".to_string())?;
    let parts: Vec<&str> = context_model.split("::").collect();
    if parts.len() != 2 {
        return Err(format!("Malformed context_model '{}'", context_model));
    }
    let provider = crate::llm::factory::create_provider(parts[0], parts[1], &settings)
        .map_err(|e| e.to_string())?;

    let checkpoint_key = format!("fact_extraction_last_msg:{conversation_id}");

    let (text, last_id) = {
        let lib = state.librarian.lock().await;
        let prev = lib
            .sqlite
            .meta_get(&checkpoint_key)
            .map_err(|e| e.to_string())?;
        let messages = lib
            .sqlite
            .get_conversation_messages(&conversation_id)
            .map_err(|e| e.to_string())?;
        let mut after: bool = prev.is_none();
        let mut buf = String::new();
        let mut last_id: Option<String> = None;
        for (id, role, content, _tc, _tcid, _r, _created, _att) in messages {
            if !after {
                if Some(&id) == prev.as_ref() {
                    after = true;
                }
                continue;
            }
            if let Some(c) = content {
                buf.push_str(&format!("{role}: {c}\n\n"));
            }
            last_id = Some(id);
        }
        (buf, last_id)
    };

    if text.trim().is_empty() {
        return Ok(ExtractionResult {
            extracted: 0,
            message: "no new messages since last checkpoint".into(),
        });
    }

    let msg = FactExtractor::extract_with_source(
        state.librarian.clone(),
        provider.as_ref(),
        "conversation",
        &text,
        None,
    )
    .await
    .map_err(|e| e.to_string())?;

    if let Some(id) = last_id {
        let lib = state.librarian.lock().await;
        let _ = lib.sqlite.meta_set(&checkpoint_key, &id);
    }

    Ok(ExtractionResult {
        extracted: parse_extract_count(&msg),
        message: msg,
    })
}

fn parse_extract_count(msg: &str) -> usize {
    // FactExtractor returns "Extracted and saved N facts" or "No facts extracted".
    msg.split_whitespace()
        .find_map(|tok| tok.parse::<usize>().ok())
        .unwrap_or(0)
}

#[tauri::command]
async fn list_user_facts(state: State<'_, AppState>) -> Result<Vec<MemoryFact>, String> {
    let lib = state.librarian.lock().await;
    lib.get_user_profile_facts(200).map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_fact_entities(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<String>, String> {
    let lib = state.librarian.lock().await;
    lib.list_fact_entities(limit.unwrap_or(100))
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn update_fact(
    state: State<'_, AppState>,
    id: String,
    subject: String,
    predicate: String,
    object: String,
    object_kind: String,
    confidence: f32,
) -> Result<(), String> {
    let kind = match object_kind.to_lowercase().as_str() {
        "entity" => ObjectKind::Entity,
        _ => ObjectKind::Literal,
    };

    let lib = state.librarian.lock().await;
    lib.update_fact(&id, &subject, &predicate, &object, kind, confidence)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_fact(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let lib = state.librarian.lock().await;
    lib.delete_fact(&id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_facts_for_entity(
    state: State<'_, AppState>,
    entity: String,
    limit: Option<usize>,
) -> Result<Vec<MemoryFact>, String> {
    let lib = state.librarian.lock().await;
    lib.get_facts_about_entity(&entity, limit.unwrap_or(50))
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn execute_tool(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
    args: serde_json::Value,
    conversation_id: Option<String>,
    tool_call_id: Option<String>,
) -> Result<serde_json::Value, String> {
    // 1. Load Settings for Permissions
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let settings = Settings::load_migrated(&settings_path);

    // Short-circuit: the built-in `update_tasks` tool is handled in-process,
    // bypasses MCP routing, and bypasses the per-tool approval flow because
    // it has no external side effects — it only updates local UI state.
    if name == "update_tasks" {
        use tauri::Emitter;

        // Parse arguments.
        let tasks_arr = args
            .get("tasks")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "update_tasks: missing 'tasks' array".to_string())?;

        let mut rows = Vec::with_capacity(tasks_arr.len());
        for t in tasks_arr {
            let content = t.get("content").and_then(|v| v.as_str())
                .ok_or_else(|| "update_tasks: task missing 'content'".to_string())?
                .to_string();
            let active_form = t.get("active_form").and_then(|v| v.as_str())
                .ok_or_else(|| "update_tasks: task missing 'active_form'".to_string())?
                .to_string();
            let status = t.get("status").and_then(|v| v.as_str())
                .ok_or_else(|| "update_tasks: task missing 'status'".to_string())?
                .to_string();
            if !["pending", "in_progress", "completed"].contains(&status.as_str()) {
                return Err(format!("update_tasks: invalid status '{}'", status));
            }
            rows.push(crate::memory::sqlite_manager::TaskRow { content, active_form, status });
        }

        // Require a conversation_id — tasks are scoped to a conversation.
        let cid = conversation_id
            .as_ref()
            .ok_or_else(|| "update_tasks: conversation_id is required".to_string())?
            .clone();

        let saved = {
            let lib = state.librarian.lock().await;
            lib.set_tasks(&cid, &rows).map_err(|e| e.to_string())?
        };

        // Notify the frontend.
        let _ = app.emit(
            "tasks-updated",
            serde_json::json!({ "conversation_id": cid, "tasks": saved }),
        );

        return Ok(serde_json::json!({
            "ok": true,
            "count": rows.len(),
            "message": "Task list updated."
        }));
    }

    // Memory tools are in-process, bypass MCP routing, and bypass the per-tool
    // approval flow (they only touch local docs the user can audit on disk).
    if crate::memory::docs::tools::is_memory_tool(&name) {
        let store = {
            let slot = state.doc_store.lock().await;
            slot.clone()
        };
        let Some(store) = store else {
            return Err("Memory subsystem is not ready yet. Try again in a moment.".into());
        };
        let result = dispatch_memory_tool(&store, &state.librarian, &name, args.clone()).await;

        // memory3 Phase 4: log every memory-tool call to the audit log so the
        // user can see exactly what the LLM wrote into their local stores.
        if let Some(cid) = conversation_id.as_deref() {
            let (status, preview, full) = match &result {
                Ok(v) => {
                    let s = v.to_string();
                    let preview: String = s.chars().take(240).collect();
                    ("success", preview, s)
                }
                Err(e) => ("error", e.clone(), e.clone()),
            };
            let lib = state.librarian.lock().await;
            let _ = lib.audit.log_execution(
                cid,
                tool_call_id.as_deref().unwrap_or("memory-tool"),
                &name,
                "memory",
                &args.to_string(),
                &preview,
                &full,
                status,
            );
        }

        return result;
    }

    // The `use_skill` tool is in-process: load the skill body and return it as
    // the tool result. Audit-logged like memory tools so the user can see
    // which skills the model has pulled into context.
    if crate::skills::tools::is_skill_tool(&name) {
        use crate::skills::tools::{TOOL_LIST_SKILLS, TOOL_USE_SKILL};
        let result: Result<serde_json::Value, String> = match name.as_str() {
            n if n == TOOL_USE_SKILL => {
                let slug = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "use_skill: missing name".to_string())?
                    .trim()
                    .to_string();
                match state.skills.get(&slug).await {
                    Some(skill) => Ok(serde_json::json!({
                        "slug": skill.slug,
                        "name": skill.name,
                        "description": skill.description,
                        "body": skill.body,
                    })),
                    None => Err(format!(
                        "use_skill: no such skill '{slug}'. Call list_skills to see what's available."
                    )),
                }
            }
            n if n == TOOL_LIST_SKILLS => {
                let summaries = state.skills.list().await;
                Ok(serde_json::json!({ "skills": summaries }))
            }
            other => Err(format!("skills dispatch: unknown tool '{other}'")),
        };
        if let Some(cid) = conversation_id.as_deref() {
            let (status, preview, full) = match &result {
                Ok(v) => {
                    let s = v.to_string();
                    let preview: String = s.chars().take(240).collect();
                    ("success", preview, s)
                }
                Err(e) => ("error", e.clone(), e.clone()),
            };
            let lib = state.librarian.lock().await;
            let _ = lib.audit.log_execution(
                cid,
                tool_call_id.as_deref().unwrap_or(&name),
                &name,
                "skills",
                &args.to_string(),
                &preview,
                &full,
                status,
            );
        }
        return result;
    }

    // 2. Identify Server
    let server_name = state
        .mcp_manager
        .get_server_for_tool(&name)
        .await
        .ok_or_else(|| format!("Tool not found: {}", name))?;

    // 3. Check Permissions (Gatekeeper)
    if let Some(server_config) = settings.mcp_servers.get(&server_name) {
        let perms = &server_config.permissions;

        // Denylist check
        if perms.denylist.contains(&name) {
            // Log denial
            if let Some(cid) = &conversation_id {
                let lib = state.librarian.lock().await;
                let _ = lib.audit.log_execution(
                    cid,
                    tool_call_id.as_deref().unwrap_or("unknown"),
                    &name,
                    &server_name,
                    &args.to_string(),
                    "Denied by policy",
                    "",
                    "denied",
                );
            }
            return Err(format!("Tool '{}' is in the denylist.", name));
        }

        // Allowlist check (if not empty, must be in it)
        if !perms.allowlist.is_empty() && !perms.allowlist.contains(&name) {
            // Log denial
            if let Some(cid) = &conversation_id {
                let lib = state.librarian.lock().await;
                let _ = lib.audit.log_execution(
                    cid,
                    tool_call_id.as_deref().unwrap_or("unknown"),
                    &name,
                    &server_name,
                    &args.to_string(),
                    "Denied (not in allowlist)",
                    "",
                    "denied",
                );
            }
            return Err(format!("Tool '{}' is not in the allowlist.", name));
        }
    }

    // 4. Execute
    let result = state
        .mcp_manager
        .call_tool(&name, args.clone())
        .await
        .map_err(|e| e.to_string())?;

    // 5. Shape Output
    let (preview, full_json) = crate::llm::tool_shaping::shape_tool_output(&result);

    // 6. Audit Log
    if let Some(cid) = &conversation_id {
        let lib = state.librarian.lock().await;
        let _ = lib.audit.log_execution(
            cid,
            tool_call_id.as_deref().unwrap_or("unknown"),
            &name,
            &server_name,
            &args.to_string(),
            &preview,
            &full_json,
            "success",
        );
    }

    // Return the result
    if preview != full_json {
        return Ok(serde_json::Value::String(preview));
    }

    Ok(result)
}

#[tauri::command]
async fn get_tool_execution(
    state: State<'_, AppState>,
    tool_call_id: String,
) -> Result<String, String> {
    let lib = state.librarian.lock().await;
    lib.audit
        .get_execution_by_tool_call_id(&tool_call_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_conversation_tasks(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<crate::memory::sqlite_manager::PersistedTask>, String> {
    let lib = state.librarian.lock().await;
    lib.get_tasks(&conversation_id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_settings(app: tauri::AppHandle) -> Result<Settings, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    Ok(Settings::load_migrated(&settings_path))
}

#[tauri::command]
async fn save_settings(app: tauri::AppHandle, settings: Settings) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    settings.save(&settings_path).map_err(|e| e.to_string())?;

    use tauri::Emitter;
    app.emit("settings-updated", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_theme(app: tauri::AppHandle) -> Result<String, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let settings = Settings::load_migrated(&settings_path);
    Ok(settings.theme)
}

#[tauri::command]
async fn set_theme(app: tauri::AppHandle, theme: String) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let mut settings = Settings::load_migrated(&settings_path);
    settings.theme = theme;
    settings.save(&settings_path).map_err(|e| e.to_string())
}

/// Fetches a remote image through the backend so the renderer never connects to
/// the remote host directly (no IP/cookie leak) and arbitrary outbound requests
/// are impossible (anti-exfiltration). The CSP keeps `img-src` at `'self' data:`;
/// this returns a base64 `data:` URI the webview can render.
///
/// Security controls: the URL must pass `image_url_allowed` against the
/// configured allowlist (https-only, host + optional path prefix), every
/// redirect hop is re-validated against the same allowlist, the response must
/// be `image/*`, and the payload is capped.
#[tauri::command]
async fn fetch_proxied_image(app: tauri::AppHandle, url: String) -> Result<String, String> {
    use base64::Engine;

    const MAX_BYTES: u64 = 10 * 1024 * 1024;

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let settings = Settings::load_migrated(&settings_path);
    let allowlist = settings.image_proxy_allowlist;

    if !crate::mcp::config::image_url_allowed(&url, &allowlist) {
        return Err(format!("Image not allowed by proxy allowlist: {}", url));
    }

    // Re-validate the destination on every redirect hop: a 3xx must not be able
    // to bounce the fetch to a non-allowlisted host (SSRF / exfiltration).
    let redirect_allowlist = allowlist.clone();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > 5 {
                attempt.error("too many redirects")
            } else if crate::mcp::config::image_url_allowed(attempt.url().as_str(), &redirect_allowlist) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Image fetch failed: HTTP {}", resp.status()));
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !content_type.starts_with("image/") {
        return Err(format!(
            "Refusing to proxy non-image content (content-type: {})",
            if content_type.is_empty() { "unknown" } else { &content_type }
        ));
    }

    if let Some(len) = resp.content_length() {
        if len > MAX_BYTES {
            return Err("Image exceeds 10 MB proxy limit".to_string());
        }
    }

    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Image exceeds 10 MB proxy limit".to_string());
    }

    let mime = content_type
        .split(';')
        .next()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("image/png");
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", mime, encoded))
}

#[tauri::command]
async fn fetch_models(
    provider_type: ProviderType,
    base_url: Option<String>,
    api_key: Option<String>,
) -> Result<Vec<ModelConfig>, String> {
    let client = reqwest::Client::new();
    let mut models = Vec::new();

    match provider_type {
        ProviderType::OpenAI | ProviderType::OpenAICompatible => {
            // For OpenAI, always use the official base URL; ignore any provided base_url.
            // For OpenAICompatible, use provided base_url if present (caller should supply it).
            let mut base = match provider_type {
                ProviderType::OpenAI => "https://api.openai.com".to_string(),
                _ => base_url.unwrap_or_else(|| "https://api.openai.com".to_string()),
            };
            // Sanitize: remove trailing slashes and a trailing /v1 if present
            while base.ends_with('/') {
                base.pop();
            }
            if base.ends_with("/v1") {
                base.truncate(base.len() - 3);
            }

            let url = format!("{}/v1/models", base);
            let key = api_key.unwrap_or_default();

            let resp = client
                .get(&url)
                .header("Authorization", format!("Bearer {}", key))
                .send()
                .await
                .map_err(|e| e.to_string())?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Failed to fetch models: {} — {}", status, body));
            }

            let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            if let Some(arr) = json["data"].as_array() {
                for m in arr {
                    if let Some(id) = m["id"].as_str() {
                        // Check if this is OpenRouter (has rich metadata) or standard OpenAI
                        let is_openrouter = base.contains("openrouter");
                        
                        let (context_window, prompt_cost, completion_cost, parameters, description, 
                             supports_reasoning_effort, supports_thinking_mode, supports_extended_thinking) = if is_openrouter {
                            // Extract OpenRouter metadata
                            let ctx_window = m["context_length"].as_u64().map(|v| v as usize);
                            let prompt_cost = m["pricing"]["prompt"].as_str().map(|s| s.to_string());
                            let completion_cost = m["pricing"]["completion"].as_str().map(|s| s.to_string());
                            let parameters = m["architecture"]["parameters"].as_u64();
                            let description = m["description"].as_str().map(|s| s.to_string());
                            
                            // Detect reasoning capabilities from supported_parameters
                            let supported_params = m["supported_parameters"].as_array();
                            let reasoning_effort = supported_params.map(|arr| {
                                arr.iter().any(|p| p.as_str() == Some("reasoning_effort"))
                            });
                            let thinking = supported_params.map(|arr| {
                                arr.iter().any(|p| {
                                    let s = p.as_str();
                                    s == Some("reasoning") || s == Some("thinking") || s == Some("include_reasoning")
                                })
                            });
                            
                            // Check architecture.instruct_type for model family hints
                            let instruct_type = m["architecture"]["instruct_type"].as_str();
                            let extended_thinking = instruct_type.map(|t| {
                                t.contains("claude") || id.contains("claude-4") || id.contains("claude-opus-4") || id.contains("claude-sonnet-4")
                            }).or(Some(false));
                            
                            (ctx_window, prompt_cost, completion_cost, parameters, description, 
                             reasoning_effort, thinking, extended_thinking)
                        } else {
                            // Fallback to capabilities lookup and pattern detection
                            let ctx = crate::llm::capabilities::get_model_context_window(id);
                            let reasoning_effort = crate::llm::capabilities::supports_reasoning_effort(id);
                            let thinking = crate::llm::capabilities::supports_thinking_mode(id);
                            let extended_thinking = crate::llm::capabilities::supports_extended_thinking(id);
                            (ctx, None, None, None, None, Some(reasoning_effort), Some(thinking), Some(extended_thinking))
                        };
                        
                        models.push(ModelConfig {
                            id: id.to_string(),
                            name: m["name"].as_str().unwrap_or(id).to_string(),
                            visible: true,
                            context_window,
                            max_tokens: None,
                            prompt_cost,
                            completion_cost,
                            parameters,
                            description,
                            supports_reasoning_effort,
                            supports_thinking_mode,
                            supports_extended_thinking,
                        });
                    }
                }
            }
        }
        ProviderType::Anthropic => {
            // Allow custom Anthropic-compatible endpoints. Strip trailing slashes and
            // a trailing `/v1` (we append `/v1/models` ourselves), mirroring the OpenAI
            // sanitizer so users can paste either form.
            let mut base = base_url
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "https://api.anthropic.com".to_string());
            while base.ends_with('/') {
                base.pop();
            }
            if base.ends_with("/v1") {
                base.truncate(base.len() - 3);
            }
            let url = format!("{}/v1/models", base);
            let key = api_key.unwrap_or_default();

            let resp = client
                .get(&url)
                .header("x-api-key", &key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .send()
                .await
                .map_err(|e| e.to_string())?;

            if resp.status().is_success() {
                let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
                if let Some(arr) = json["data"].as_array() {
                    for m in arr {
                        if let Some(id) = m["id"].as_str() {
                            let name = m["display_name"].as_str().unwrap_or(id).to_string();
                            models.push(ModelConfig {
                                id: id.to_string(),
                                name,
                                visible: true,
                                context_window: crate::llm::capabilities::get_model_context_window(
                                    &id,
                                ),
                                max_tokens: None,
                                prompt_cost: None,
                                completion_cost: None,
                                parameters: None,
                                description: None,
                                supports_reasoning_effort: Some(crate::llm::capabilities::supports_reasoning_effort(&id)),
                                supports_thinking_mode: Some(crate::llm::capabilities::supports_thinking_mode(&id)),
                                supports_extended_thinking: Some(crate::llm::capabilities::supports_extended_thinking(&id)),
                            });
                        }
                    }
                }
            } else {
                // Fallback if fetch fails (e.g. key invalid or network issue), but updated with latest models
                // This ensures the dropdown isn't empty even if the API call fails.
                let fallback_models = vec![
                    "claude-3-5-sonnet-20241022",
                    "claude-3-5-sonnet-20240620",
                    "claude-3-5-haiku-20241022",
                    "claude-3-opus-20240229",
                    "claude-3-sonnet-20240229",
                    "claude-3-haiku-20240307",
                    "gpt-4o",
                    "gpt-4o-mini",
                    "o1-preview",
                    "o1-mini",
                    "gemini-1.5-pro",
                    "gemini-1.5-flash",
                ];
                for id in fallback_models {
                    models.push(ModelConfig {
                        id: id.to_string(),
                        name: id.to_string(),
                        visible: true,
                        context_window: crate::llm::capabilities::get_model_context_window(id),
                        max_tokens: None,
                        prompt_cost: None,
                        completion_cost: None,
                        parameters: None,
                        description: None,
                        supports_reasoning_effort: Some(crate::llm::capabilities::supports_reasoning_effort(id)),
                        supports_thinking_mode: Some(crate::llm::capabilities::supports_thinking_mode(id)),
                        supports_extended_thinking: Some(crate::llm::capabilities::supports_extended_thinking(id)),
                    });
                }
            }
        }
        ProviderType::Ollama => {
            let url = format!(
                "{}/api/tags",
                base_url
                    .unwrap_or_else(|| "http://localhost:11434".to_string())
                    .trim_end_matches('/')
            );
            let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;

            if !resp.status().is_success() {
                return Err(format!("Failed to fetch models: {}", resp.status()));
            }

            let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            if let Some(arr) = json["models"].as_array() {
                for m in arr {
                    if let Some(name) = m["name"].as_str() {
                        models.push(ModelConfig {
                            id: name.to_string(),
                            name: name.to_string(),
                            visible: true,
                            context_window: crate::llm::capabilities::get_model_context_window(
                                name,
                            ),
                            max_tokens: None,
                            prompt_cost: None,
                            completion_cost: None,
                            parameters: None,
                            description: None,
                            supports_reasoning_effort: Some(crate::llm::capabilities::supports_reasoning_effort(name)),
                            supports_thinking_mode: Some(crate::llm::capabilities::supports_thinking_mode(name)),
                            supports_extended_thinking: Some(crate::llm::capabilities::supports_extended_thinking(name)),
                        });
                    }
                }
            }
        }
    }

    Ok(models)
}

#[tauri::command]
async fn count_conversation_tokens(messages: Vec<Message>) -> Result<usize, String> {
    let mut total = 0;
    for msg in messages {
        total += crate::llm::tokenizer::Tokenizer::count_message_tokens(&msg)
            .map_err(|e| e.to_string())?;
    }
    Ok(total)
}

#[tauri::command]
async fn add_mcp_server(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
    transport_type: String, // "stdio", "sse", or "streamable-http"
    command: Option<String>,
    args: Option<Vec<String>>,
    env: Option<HashMap<String, String>>,
    url: Option<String>,
    headers: Option<HashMap<String, String>>,
    auto_approve: Option<bool>,
) -> Result<(), String> {
    use crate::mcp::config::McpTransport;

    let transport = match transport_type.as_str() {
        "stdio" => McpTransport::Stdio {
            command: command.ok_or("Command required for Stdio")?,
            args: args.unwrap_or_default(),
            env: env.unwrap_or_default(),
        },
        "sse" => McpTransport::Sse {
            url: url.ok_or("URL required for SSE")?,
            headers: headers.unwrap_or_default(),
        },
        "streamable-http" => McpTransport::StreamableHttp {
            url: url.ok_or("URL required for StreamableHttp")?,
            headers: headers.unwrap_or_default(),
        },
        _ => return Err("Invalid transport type".to_string()),
    };

    let config = McpServerConfig {
        transport,
        auto_approve: auto_approve.unwrap_or(false),
        auto_approve_tools: vec![],
        permissions: Default::default(),
    };

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let mut settings = Settings::load_migrated(&settings_path);

    if settings.mcp_servers.contains_key(&name) {
        return Err(format!("Server '{}' already exists", name));
    }

    settings.mcp_servers.insert(name.clone(), config.clone());
    settings.save(&settings_path).map_err(|e| e.to_string())?;

    // Update runtime
    let mut runtime_config = Settings::default();
    runtime_config.mcp_servers.insert(name, config);
    state
        .mcp_manager
        .initialize(runtime_config)
        .await
        .map_err(|e| e.to_string())?;

    use tauri::Emitter;
    let _ = app.emit("tools-updated", ());
    Ok(())
}

#[tauri::command]
async fn edit_mcp_server(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    original_name: String, // In case we allow renaming later, but for now fixed
    new_config: McpServerConfig,
) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let mut settings = Settings::load_migrated(&settings_path);

    if !settings.mcp_servers.contains_key(&original_name) {
        return Err(format!("Server '{}' not found", original_name));
    }

    // Update config
    settings
        .mcp_servers
        .insert(original_name.clone(), new_config.clone());
    settings.save(&settings_path).map_err(|e| e.to_string())?;

    // Restart in manager
    state
        .mcp_manager
        .restart_server(original_name, new_config)
        .await
        .map_err(|e| e.to_string())?;

    use tauri::Emitter;
    let _ = app.emit("tools-updated", ());
    Ok(())
}

#[tauri::command]
async fn delete_mcp_server(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let mut settings = Settings::load_migrated(&settings_path);

    if settings.mcp_servers.remove(&name).is_none() {
        return Err(format!("Server '{}' not found", name));
    }

    settings.save(&settings_path).map_err(|e| e.to_string())?;

    // Remove from runtime (best-effort; may not kill an external stdio process today).
    state.mcp_manager.remove_server(&name).await;

    use tauri::Emitter;
    let _ = app.emit("tools-updated", ());

    Ok(())
}

#[tauri::command]
async fn rebuild_memory_index(state: State<'_, AppState>) -> Result<(), String> {
    // Release the librarian mutex between conversations so other commands (sends,
    // searches, deletes) aren't blocked for the entire rebuild — this can otherwise
    // stall the UI for minutes on large histories.

    {
        let lib = state.librarian.lock().await;
        lib.clear_search_index().map_err(|e| e.to_string())?;
    }

    let conversations = {
        let lib = state.librarian.lock().await;
        lib.list_conversations().map_err(|e| e.to_string())?
    };

    for (conv_id, _, _, _) in conversations {
        let lib = state.librarian.lock().await;
        let messages = lib.get_complete_history(&conv_id).map_err(|e| e.to_string())?;
        for (msg_id, role, content, _, _, _, created_at, _) in messages {
            if let Some(text) = content {
                lib.index_existing_message(&conv_id, &role, &text, &msg_id, &created_at)
                    .map_err(|e| e.to_string())?;
            }
        }
        drop(lib);
    }

    Ok(())
}

#[tauri::command]
async fn search_messages(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<SearchResult>, String> {
    let lib = state.librarian.lock().await;
    lib.search(&query).map_err(|e| e.to_string())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ExportData {
    conversation_id: String,
    title: String,
    created_at: String,
    messages: Vec<Message>,
}

#[tauri::command]
async fn export_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
    format: String, // "json" or "md"
) -> Result<String, String> {
    tracing::info!(
        "Exporting conversation: {} format: {}",
        conversation_id,
        format
    );

    // Snapshot DB rows under the lock, then release it before doing the (potentially
    // large) serialization work — exports of long conversations used to block every
    // other librarian-bound command for the duration of the JSON/MD build.
    let (title, created_at, raw_msgs) = {
        let lib = state.librarian.lock().await;
        let convs = lib.list_conversations().map_err(|e| e.to_string())?;
        let (_, title, _, created_at) = convs
            .into_iter()
            .find(|(id, _, _, _)| id == &conversation_id)
            .ok_or("Conversation not found")?;
        let raw_msgs = lib
            .get_complete_history(&conversation_id)
            .map_err(|e| e.to_string())?;
        (title, created_at, raw_msgs)
    };

    let mut messages = Vec::new();
    for (
        id,
        role,
        content,
        tool_calls_txt,
        tool_call_id,
        reasoning_content,
        created_at_str,
        attachments_json,
    ) in raw_msgs
    {
        let tool_calls = if let Some(json) = tool_calls_txt {
            if !json.is_empty() {
                serde_json::from_str(&json).ok()
            } else {
                None
            }
        } else {
            None
        };

        let attachments = if !attachments_json.is_empty() {
            serde_json::from_str(&attachments_json).ok()
        } else {
            None
        };

        // Parse created_at (assuming it's compatible with chrono, otherwise None)
        let created_at = if let Ok(dt) =
            chrono::NaiveDateTime::parse_from_str(&created_at_str, "%Y-%m-%d %H:%M:%S")
        {
            Some(dt.and_utc().timestamp())
        } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&created_at_str) {
            Some(dt.timestamp())
        } else {
            None
        };

        messages.push(Message {
            id: Some(id),
            role,
            content,
            reasoning_content,
            tool_calls,
            tool_call_id,
            attachments,
            created_at,
        });
    }

    match format.as_str() {
        "json" => {
            let export = ExportData {
                conversation_id,
                title,
                created_at,
                messages,
            };
            serde_json::to_string_pretty(&export).map_err(|e| e.to_string())
        }
        "md" => {
            let mut md = String::new();
            md.push_str(&format!("# {}\n\n", title));
            for msg in messages {
                let role_title = match msg.role.as_str() {
                    "user" => "User",
                    "assistant" => "Assistant",
                    "system" => "System",
                    "tool" => "Tool Output",
                    _ => &msg.role,
                };
                md.push_str(&format!("## {}\n", role_title));

                if let Some(content) = msg.content {
                    md.push_str(&content);
                    md.push_str("\n\n");
                }

                if let Some(tool_calls) = msg.tool_calls {
                    md.push_str("```json\n");
                    md.push_str(&serde_json::to_string_pretty(&tool_calls).unwrap_or_default());
                    md.push_str("\n```\n\n");
                }
            }
            Ok(md)
        }
        _ => Err("Unsupported format".to_string()),
    }
}

#[tauri::command]
async fn import_conversation(
    state: State<'_, AppState>,
    json_content: String,
) -> Result<String, String> {
    let data: ExportData = serde_json::from_str(&json_content).map_err(|e| e.to_string())?;

    // We create a NEW conversation to allow re-importing without ID collision logic for now
    // Or we could try to preserve ID if checking existing. Let's create new to be safe.
    let lib = state.librarian.lock().await;
    let new_id = lib
        .create_conversation(&data.title)
        .map_err(|e| e.to_string())?;

    for msg in data.messages {
        // Save using current logic which generates new message IDs
        // Ignoring msg.id from JSON to ensure unique IDs in our DB
        let valid_tool_calls = msg
            .tool_calls
            .map(|tc| serde_json::to_string(&tc).unwrap_or_default());
        let tool_calls_str = valid_tool_calls.as_deref();

        lib.save_full_message(
            &new_id,
            &msg.role,
            msg.content.as_deref(),
            tool_calls_str,
            msg.tool_call_id.as_deref(),
            msg.reasoning_content.as_deref(),
            None,
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(new_id)
}
async fn attempt_pruning(
    app: tauri::AppHandle,
    librarian: Arc<Mutex<crate::memory::librarian::Librarian>>,
    provider_id: String,
    model: String,
    conversation_id: String,
) -> Result<(), String> {
    let limit = 50;
    let prune_count = 20;

    // Check count
    let count = {
        let lib = librarian.lock().await;
        lib.get_message_count(&conversation_id)
            .map_err(|e| e.to_string())?
    };

    if count > limit {
        println!("Pruning memory: {} messages found (limit {})", count, limit);

        // Get oldest messages
        let oldest = {
            let lib = librarian.lock().await;
            lib.get_oldest_messages(&conversation_id, prune_count)
                .map_err(|e| e.to_string())?
        };

        if oldest.is_empty() {
            return Ok(());
        }

        // Format for summarization
        let mut text_to_summarize = String::new();
        let mut ids_to_delete = Vec::new();
        let mut last_timestamp = String::new();

        for (id, role, content, ts) in oldest {
            text_to_summarize.push_str(&format!("{}: {}\n", role, content));
            ids_to_delete.push(id);
            last_timestamp = ts;
        }

        // Retrieve Provider Config
        let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
        let settings_path = config_dir.join("settings.json");
        let settings = Settings::load_migrated(&settings_path);

        let provider_config = settings
            .providers
            .get(&provider_id)
            .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

        if !provider_config.enabled {
            return Ok(());
        } // Skip pruning if provider disabled

        // Create Provider
        let provider_response = match provider_config.provider_type {
            ProviderType::OpenAI | ProviderType::OpenAICompatible => {
                let api_key = provider_config.api_key.clone().unwrap_or_default();
                let base_url = match provider_config.provider_type {
                    ProviderType::OpenAI => None,
                    _ => provider_config.base_url.clone(),
                };
                let provider = OpenAiProvider::new(api_key, base_url, model.clone());
                provider.chat(vec![Message {
                    id: None,
                    role: "user".to_string(),
                    content: Some(format!("Summarize the following conversation segment concisely, preserving key facts:\n\n{}", text_to_summarize)),
                    tool_calls: None,
                    tool_call_id: None,
                    attachments: None,
                    reasoning_content: None,
                    created_at: None,
                }], vec![], None).await
            }
            ProviderType::Anthropic => {
                let api_key = provider_config.api_key.clone().unwrap_or_default();
                let base_url = provider_config.base_url.clone();
                let provider = AnthropicProvider::new(api_key, base_url, model.clone());
                provider.chat(vec![Message {
                    id: None,
                    role: "user".to_string(),
                    content: Some(format!("Summarize the following conversation segment concisely, preserving key facts:\n\n{}", text_to_summarize)),
                    tool_calls: None,
                    tool_call_id: None,
                    attachments: None,
                    reasoning_content: None,
                    created_at: None,
                }], vec![], None).await
            }
            ProviderType::Ollama => {
                let base_url = provider_config
                    .base_url
                    .clone()
                    .unwrap_or("http://localhost:11434".to_string());
                let provider = OllamaProvider::new(base_url, model.clone());
                provider.chat(vec![Message {
                    id: None,
                    role: "user".to_string(),
                    content: Some(format!("Summarize the following conversation segment concisely, preserving key facts:\n\n{}", text_to_summarize)),
                    tool_calls: None,
                    tool_call_id: None,
                    attachments: None,
                    reasoning_content: None,
                    created_at: None,
                }], vec![], None).await
            }
        };

        match provider_response {
            Ok(msg) => {
                if let Some(summary) = msg.content {
                    let lib = librarian.lock().await;
                    // Save summary with timestamp of the LAST deleted message so it sits in the right place
                    let formatted_summary = format!("[Old Memory Summary]: {}", summary);
                    lib.save_summary(&conversation_id, &formatted_summary, &last_timestamp)
                        .map_err(|e| e.to_string())?;
                    lib.delete_messages(&ids_to_delete)
                        .map_err(|e| e.to_string())?;
                    tracing::debug!("Pruned {} messages.", prune_count);
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

#[tauri::command]
async fn set_default_model(app: tauri::AppHandle, model_target: String) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let mut settings = Settings::load_migrated(&settings_path);

    settings.default_model = Some(model_target);
    settings.save(&settings_path).map_err(|e| e.to_string())?;

    use tauri::Emitter;
    app.emit("settings-updated", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(serde::Serialize)]
struct ToolStatus {
    name: String,
    description: String,
    server: String,
    enabled: bool,
}

#[tauri::command]
async fn get_tools(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ToolStatus>, String> {
    let all_tools = state.mcp_manager.get_all_tools().await;

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let settings = Settings::load_migrated(&settings_path);

    let mut result = Vec::new();
    for t in all_tools {
        // Resolve the real server config key via the manager rather than
        // splitting the advertised name: that name may be sanitized (e.g. a
        // server keyed "jina.ai" surfaces as "jina_ai__<tool>"), so a naive
        // split yields a key that doesn't exist in `mcp_servers`.
        let server = state
            .mcp_manager
            .get_server_for_tool(&t.name)
            .await
            .unwrap_or_else(|| "unknown".to_string());

        result.push(ToolStatus {
            name: t.name.clone(),
            description: t.description,
            server,
            enabled: !settings.disabled_tools.contains(&t.name),
        });
    }

    Ok(result)
}

#[tauri::command]
async fn get_tool_policies(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<HashMap<String, bool>, String> {
    let all_tools = state.mcp_manager.get_all_tools().await;
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let settings = Settings::load_migrated(&settings_path);

    let mut policies = HashMap::new();
    for t in all_tools {
        // Resolve the real server config key via the manager (the advertised
        // name may be sanitized, so splitting it can produce a key that isn't
        // in `mcp_servers`, e.g. "jina_ai" vs the real "jina.ai"). The simple
        // tool name stored in `auto_approve_tools` is the part after the first
        // "__", matching what the UI toggle writes.
        let server = state.mcp_manager.get_server_for_tool(&t.name).await;
        let simple_name = t.name.splitn(2, "__").nth(1).unwrap_or(t.name.as_str());
        let auto_approve = server
            .as_deref()
            .and_then(|s| settings.mcp_servers.get(s))
            .map(|c| c.auto_approve || c.auto_approve_tools.contains(&simple_name.to_string()))
            .unwrap_or(false);
        policies.insert(t.name.clone(), auto_approve);
    }

    Ok(policies)
}

#[tauri::command]
async fn toggle_mcp_server_auto_approve(
    app: tauri::AppHandle,
    _state: State<'_, AppState>,
    server_name: String,
    auto_approve: bool,
) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let mut settings = Settings::load_migrated(&settings_path);

    if let Some(config) = settings.mcp_servers.get_mut(&server_name) {
        config.auto_approve = auto_approve;
        settings.save(&settings_path).map_err(|e| e.to_string())?;

        use tauri::Emitter;
        let _ = app.emit("tools-updated", ());
        Ok(())
    } else {
        Err(format!("Server '{}' not found", server_name))
    }
}

#[tauri::command]
async fn toggle_tool_auto_approve(
    app: tauri::AppHandle,
    _state: State<'_, AppState>,
    server_name: String,
    tool_name: String,
    auto_approve: bool,
) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let mut settings = Settings::load_migrated(&settings_path);

    if let Some(config) = settings.mcp_servers.get_mut(&server_name) {
        if auto_approve {
            if !config.auto_approve_tools.contains(&tool_name) {
                config.auto_approve_tools.push(tool_name);
            }
        } else {
            config.auto_approve_tools.retain(|t| t != &tool_name);
        }
        settings.save(&settings_path).map_err(|e| e.to_string())?;

        use tauri::Emitter;
        let _ = app.emit("tools-updated", ());
        Ok(())
    } else {
        Err(format!("Server '{}' not found", server_name))
    }
}

#[tauri::command]
async fn toggle_tool(
    app: tauri::AppHandle,
    tool_name: String,
    enabled: bool,
) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let mut settings = Settings::load_migrated(&settings_path);

    if enabled {
        settings.disabled_tools.retain(|t| t != &tool_name);
    } else {
        if !settings.disabled_tools.contains(&tool_name) {
            settings.disabled_tools.push(tool_name);
        }
    }

    settings.save(&settings_path).map_err(|e| e.to_string())?;

    use tauri::Emitter;
    let _ = app.emit("tools-updated", ());
    Ok(())
}

#[tauri::command]
async fn restart_mcp_server(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let settings = Settings::load_migrated(&settings_path);

    let config = settings
        .mcp_servers
        .get(&name)
        .ok_or_else(|| format!("Server '{}' not found in settings", name))?
        .clone();

    state
        .mcp_manager
        .restart_server(name, config)
        .await
        .map_err(|e| e.to_string())?;

    use tauri::Emitter;
    let _ = app.emit("tools-updated", ());

    Ok(())
}

#[tauri::command]
async fn get_mcp_servers(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(state.mcp_manager.list_servers().await)
}

#[tauri::command]
async fn toggle_tool_list(
    app: tauri::AppHandle,
    tool_names: Vec<String>,
    enabled: bool,
) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let mut settings = Settings::load_migrated(&settings_path);

    for tool_name in tool_names {
        if enabled {
            settings.disabled_tools.retain(|t| t != &tool_name);
        } else {
            if !settings.disabled_tools.contains(&tool_name) {
                settings.disabled_tools.push(tool_name);
            }
        }
    }

    settings.save(&settings_path).map_err(|e| e.to_string())?;

    use tauri::Emitter;
    let _ = app.emit("tools-updated", ());
    Ok(())
}

#[tauri::command]
async fn get_system_prompts(
    app: tauri::AppHandle,
) -> Result<Vec<crate::mcp::config::SystemPrompt>, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let settings = Settings::load_migrated(&settings_path);
    Ok(settings.system_prompts)
}

#[tauri::command]
async fn save_system_prompt(
    app: tauri::AppHandle,
    id: Option<String>,
    name: String,
    content: String,
) -> Result<(), String> {
    tracing::info!("save_system_prompt called: id={:?}, name={}", id, name);
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let mut settings = Settings::load_migrated(&settings_path);

    let new_id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Refuse to overwrite built-ins via this command — they'd be
    // regenerated from the binary on next launch anyway, so the edit
    // would silently disappear. UI should hide Save for built-ins;
    // this is defense in depth.
    if let Some(existing) = settings.system_prompts.iter().find(|p| p.id == new_id) {
        if existing.built_in {
            return Err(format!(
                "'{}' is a built-in prompt and cannot be edited. Clone it to a new prompt instead.",
                existing.name
            ));
        }
    }

    let new_prompt = crate::mcp::config::SystemPrompt {
        id: new_id.clone(),
        name: name.clone(),
        content: content.clone(),
        built_in: false,
    };

    if let Some(idx) = settings.system_prompts.iter().position(|p| p.id == new_id) {
        tracing::info!("Updating existing prompt at index {}", idx);
        settings.system_prompts[idx] = new_prompt;
    } else {
        tracing::info!("Adding new prompt with id {}", new_id);
        settings.system_prompts.push(new_prompt);
    }

    tracing::info!("Total prompts: {}", settings.system_prompts.len());
    let result = settings.save(&settings_path).map_err(|e| e.to_string());
    if result.is_ok() {
        tracing::info!("Successfully saved settings to {:?}", settings_path);
    } else {
        tracing::error!("Failed to save settings: {:?}", result);
    }
    result
}

#[tauri::command]
async fn delete_system_prompt(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let mut settings = Settings::load_migrated(&settings_path);

    // Refuse to delete built-ins — they'd be regenerated from the binary
    // on next launch anyway. UI should hide the delete button for built-
    // ins; this is defense in depth.
    if let Some(existing) = settings.system_prompts.iter().find(|p| p.id == id) {
        if existing.built_in {
            return Err(format!(
                "'{}' is a built-in prompt and cannot be deleted.",
                existing.name
            ));
        }
    }

    settings.system_prompts.retain(|p| p.id != id);
    if settings.active_system_prompt_id.as_deref() == Some(&id) {
        settings.active_system_prompt_id = None;
    }

    settings.save(&settings_path).map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_active_system_prompt(app: tauri::AppHandle, id: Option<String>) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let mut settings = Settings::load_migrated(&settings_path);

    settings.active_system_prompt_id = id;

    settings.save(&settings_path).map_err(|e| e.to_string())?;

    use tauri::Emitter;
    app.emit("settings-updated", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_active_mcp_servers(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(state.mcp_manager.list_servers().await)
}

#[tauri::command]
async fn cleanup_invalid_tool_messages(state: State<'_, AppState>) -> Result<usize, String> {
    let lib = state.librarian.lock().await;
    
    // Get all conversations
    let conversations = lib.list_conversations().map_err(|e| e.to_string())?;
    
    let mut total_deleted = 0;
    
    for (conv_id, _title, _icon, _created_at) in conversations {
        // Get all messages for this conversation
        let messages_raw = lib
            .get_complete_history(&conv_id)
            .map_err(|e| e.to_string())?;
        
        let mut seen_tool_call_ids = std::collections::HashSet::new();
        let mut assistant_tool_call_ids = std::collections::HashSet::new();
        let mut ids_to_delete = Vec::new();
        
        // First pass: collect all tool_call IDs from assistant messages and detect duplicates
        for (id, role, _content, tool_calls_json, _tool_call_id, _reasoning, _created_at, _attachments) in &messages_raw {
            if role == "assistant" {
                if let Some(json_str) = tool_calls_json {
                    if !json_str.is_empty() {
                        if let Ok(calls) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                            for call in calls {
                                if let Some(call_id) = call.get("id").and_then(|v| v.as_str()) {
                                    // Check for duplicate tool_call_id in assistant messages
                                    if !assistant_tool_call_ids.insert(call_id.to_string()) {
                                        tracing::warn!("Found duplicate assistant tool_call_id: {}, marking message for deletion", call_id);
                                        ids_to_delete.push(id.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // Second pass: check tool messages
        for (id, role, _content, _tool_calls_json, tool_call_id, _reasoning, _created_at, _attachments) in &messages_raw {
            if role == "tool" {
                let has_valid_id = match tool_call_id {
                    Some(ref id) => !id.trim().is_empty(),
                    None => false,
                };
                
                if !has_valid_id {
                    tracing::info!("Deleting invalid tool message (no ID): id={}", id);
                    ids_to_delete.push(id.clone());
                } else {
                    let tcid = tool_call_id.as_ref().unwrap();
                    
                    // Check for duplicate tool messages
                    if !seen_tool_call_ids.insert(tcid.clone()) {
                        tracing::info!("Deleting duplicate tool message: tool_call_id={}", tcid);
                        ids_to_delete.push(id.clone());
                    }
                    // Check for orphaned tool messages
                    else if !assistant_tool_call_ids.contains(tcid) {
                        tracing::info!("Deleting orphaned tool message: tool_call_id={}", tcid);
                        ids_to_delete.push(id.clone());
                    }
                }
            }
        }
        
        // Delete all marked messages
        if !ids_to_delete.is_empty() {
            lib.sqlite.delete_messages(&ids_to_delete).map_err(|e| e.to_string())?;
            total_deleted += ids_to_delete.len();
        }
    }
    
    tracing::info!("Cleanup complete: deleted {} invalid/duplicate messages", total_deleted);
    Ok(total_deleted)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Set environment variables for Linux compatibility
    // Fixes IBus input issues and NVIDIA rendering problems
    #[cfg(target_os = "linux")]
    {
        std::env::set_var("IBUS_ENABLE_SYNC_MODE", "1");
        // Commented out to prevent WebKitGTK input focus deadlocks/hangs on modern Linux systems.
        // std::env::set_var("GTK_IM_MODULE", "xim");
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");

        // Force software rendering when GPU acceleration isn't available
        // (e.g. xrdp sessions where EGL/DRI2 authentication fails — visible as
        // `libEGL warning: DRI2: failed to authenticate` on startup). Without
        // these, WebKit falls into a half-broken compositor state that hangs
        // the renderer when a freshly-mounted input gets focus.
        if std::env::var_os("LIBGL_ALWAYS_SOFTWARE").is_none() {
            std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
        }
        if std::env::var_os("WEBKIT_FORCE_GL_FALLBACK").is_none() {
            std::env::set_var("WEBKIT_FORCE_GL_FALLBACK", "1");
        }

        // Disable touch emulation to allow proper mouse/pointer events.
        // This fixes right-click and text selection on Linux.
        std::env::set_var("GDK_CORE_DEVICE_EVENTS", "1");
        // NOTE: WEBKIT_DISABLE_COMPOSITING_MODE=1 was previously set here, but it
        // turns off WebKit's accelerated compositor and causes the renderer to
        // hang when a freshly-mounted input gets focus (e.g. the model dropdown
        // search field). Opening the inspector forces compositing back on, which
        // is why the freeze "disappears" while DevTools is open. Leave it unset.
    }

    let mcp_manager = Arc::new(McpManager::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let mcp_manager = mcp_manager.clone();

            // Initialize AppState with real dependencies
            tauri::async_runtime::block_on(async move {
                let config_dir = app_handle.path().app_config_dir().unwrap();
                std::fs::create_dir_all(&config_dir).unwrap();

                // 0. Materialise built-in system prompts. Binary is source
                //    of truth (same policy as skills::builtins) — edits to
                //    built-in prompts are overwritten on each launch.
                {
                    let settings_path = config_dir.join("settings.json");
                    let mut settings = Settings::load_migrated(&settings_path);
                    if settings.materialize_builtin_prompts() {
                        if let Err(e) = settings.save(&settings_path) {
                            tracing::warn!(
                                "failed to save built-in system prompts: {e}"
                            );
                        }
                    }
                }

                // 1. Librarian
                let librarian = crate::memory::librarian::Librarian::new(&config_dir).unwrap();
                let librarian_arc = Arc::new(Mutex::new(librarian));

                // 2. DocStore (memory3 Phase 1).
                let doc_store_slot: Arc<Mutex<Option<Arc<crate::memory::docs::DocStore>>>> =
                    Arc::new(Mutex::new(None));

                // 3. SkillStore — materialises built-ins on first run, then
                //    scans the skills dir into memory. Synchronous + cheap.
                let skills_dir = config_dir.join("skills");
                let skills_store = crate::skills::SkillStore::new(skills_dir)
                    .await
                    .expect("SkillStore init failed");

                // Arm the FS watcher so external edits (vim, git pull) live-
                // update without a manual reload. After each watcher-driven
                // reload we emit `skills-updated` so SettingsPage refreshes.
                {
                    let app_for_event = app_handle.clone();
                    if let Err(e) = skills_store.start_watcher(move || {
                        use tauri::Emitter;
                        let _ = app_for_event.emit("skills-updated", ());
                    }) {
                        tracing::warn!("skills watcher failed to start: {e}");
                    }
                }

                // 4. AppState
                let state = AppState {
                    mcp_manager: mcp_manager.clone(),
                    librarian: librarian_arc.clone(),
                    doc_store: doc_store_slot.clone(),
                    skills: skills_store,
                    active_task: Arc::new(Mutex::new(None)),
                    context_approvals: Arc::new(Mutex::new(HashMap::new())),
                };
                app_handle.manage(state);

                // 4. Initialise DocStore in the background so app start is fast
                //    even on the first run that downloads the embedding model.
                {
                    let librarian_for_docs = librarian_arc.clone();
                    let config_dir = config_dir.clone();
                    let app_handle_for_docs = app_handle.clone();
                    let doc_store_slot = doc_store_slot.clone();
                    tauri::async_runtime::spawn(async move {
                        let docs_dir = config_dir.join("memory").join("docs");
                        let docs_index_dir = config_dir.join("memory").join("docs_index");
                        let settings_path_for_embed = config_dir.join("settings.json");
                        let settings_for_embed =
                            Settings::load_migrated(&settings_path_for_embed);
                        let embedder = init_embedder(&settings_for_embed).await;
                        match crate::memory::docs::DocStore::new(
                            docs_dir,
                            librarian_for_docs,
                            docs_index_dir,
                            embedder,
                        )
                        .await
                        {
                            Ok(store) => {
                                // Forward DocStore progress events to the
                                // frontend so the user sees re-embed activity.
                                let app_for_progress = app_handle_for_docs.clone();
                                store.set_progress_sink(Arc::new(move |phase, current, total| {
                                    use tauri::Emitter;
                                    let _ = app_for_progress.emit(
                                        "memory:reembed-progress",
                                        serde_json::json!({
                                            "phase": phase,
                                            "current": current,
                                            "total": total,
                                        }),
                                    );
                                }));
                                // Materialise built-in nebula docs before
                                // reconcile so they flow through normal
                                // ingest (SQLite + Tantivy + embeddings).
                                match store.materialize_builtins() {
                                    Ok(n) if n > 0 => tracing::info!(
                                        "DocStore: materialised {} built-in doc(s)",
                                        n
                                    ),
                                    Ok(_) => {}
                                    Err(e) => tracing::warn!(
                                        "DocStore: materialise built-ins failed: {e}"
                                    ),
                                }
                                match store.startup_reconcile().await {
                                    Ok(summary) => tracing::info!(
                                        "DocStore reconcile: scanned={} ingested={} updated={} deleted={}",
                                        summary.scanned,
                                        summary.ingested,
                                        summary.updated,
                                        summary.deleted,
                                    ),
                                    Err(e) => tracing::warn!("DocStore reconcile failed: {e}"),
                                }
                                if let Err(e) = store.start_watcher() {
                                    tracing::warn!("DocStore watcher failed to start: {e}");
                                }
                                {
                                    let mut slot = doc_store_slot.lock().await;
                                    *slot = Some(store);
                                }
                                use tauri::Emitter;
                                let _ = app_handle_for_docs.emit("docs-ready", ());
                            }
                            Err(e) => {
                                tracing::error!("DocStore init failed: {e}");
                            }
                        }
                    });
                }

                // 3. Initialize Context & MCP
                let settings_path = config_dir.join("settings.json");
                let settings = Settings::load_migrated(&settings_path);

                // Start MCP Servers (background)
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = mcp_manager.initialize(settings).await {
                        eprintln!("Failed to initialize MCP servers: {}", e);
                    }
                    // Emit event to notify UI that tools (and servers) are ready
                    use tauri::Emitter;
                    if let Err(e) = app_handle.emit("tools-updated", ()) {
                        eprintln!("Failed to emit tools-updated: {}", e);
                    }
                });
            });

            // Note: MCP server cleanup happens automatically on process exit via Drop
            // The SSE transport's stop() method is called by McpManager::shutdown()
            // which is available but not currently hooked into window close events

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            stop_generation,
            respond_to_context_inspection,
            list_conversations,
            create_conversation,
            delete_conversation,
            rename_conversation,
            update_conversation_icon,
            delete_message,
            delete_messages,
            generate_title,
            get_chat_history,
            send_message,
            get_settings,
            save_settings,
            get_theme,
            set_theme,
            fetch_models,
            fetch_proxied_image,
            execute_tool,
            get_tool_execution,
            get_conversation_tasks,
            add_mcp_server,
            edit_mcp_server,
            delete_mcp_server,
            restart_mcp_server,
            get_mcp_servers,
            get_active_mcp_servers,
            set_default_model,
            get_tools,
            toggle_tool,
            toggle_tool_list,
            get_system_prompts,
            save_system_prompt,
            delete_system_prompt,
            set_active_system_prompt,
            rebuild_memory_index,
            count_conversation_tokens,
            search_messages,
            export_conversation,
            get_tool_policies,
            toggle_mcp_server_auto_approve,
            toggle_tool_auto_approve,
            import_conversation,
            list_user_facts,
            list_fact_entities,
            update_fact,
            delete_fact,
            list_facts_for_entity,
            cleanup_invalid_tool_messages,
            list_memory_docs,
            fetch_memory_doc,
            recall_memory_docs,
            get_storage_paths,
            extract_facts_from_text,
            extract_facts_for_message,
            extract_session_end,
            list_skills,
            get_skill,
            create_skill,
            update_skill,
            delete_skill,
            reload_skills
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
