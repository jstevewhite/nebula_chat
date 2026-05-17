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

#[derive(Clone)]
pub struct AppState {
    mcp_manager: Arc<McpManager>,
    librarian: Arc<Mutex<crate::memory::librarian::Librarian>>,
    active_task: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
    // For context inspection: maps request_id -> oneshot sender for approval
    context_approvals: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>,
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
            let provider = AnthropicProvider::new(api_key, model.clone());
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
                                    // Trigger Background Fact Extraction
                                    let app_handle_bg = app_handle.clone();
                                    let lib_clone = state.librarian.clone();
                                    let content_clone = last.content.clone().unwrap_or_default();
                                    let msg_id_clone = msg_id.clone();

                                    tokio::spawn(async move {
                                         let config_dir = app_handle_bg.path().app_config_dir().unwrap_or_default();
                                         let settings_path = config_dir.join("settings.json");
                                         let settings = Settings::load_migrated(&settings_path);

                                        if let Some(model_id) = &settings.context_model {
                                            let parts: Vec<&str> = model_id.split("::").collect();
                                            if parts.len() == 2 {
                                                if let Ok(provider) = crate::memory::StrategistMemoryOrchestrator::create_provider(parts[0], parts[1], &settings) {
                                                    if let Err(e) = FactExtractor::extract(lib_clone, provider.as_ref(), "user", &content_clone, &msg_id_clone).await {
                                                        tracing::warn!("Fact extraction failed: {}", e);
                                                    }
                                                }
                                            }
                                        }
                                    });
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
        let extraction_model = settings.context_model.clone();
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

        // Retrieve Context (Long-term memory) via Strategist Orchestrator
        let mut context_text = String::new();
        if memory_enabled && !query.is_empty() {
            // Use strategist orchestrator for intelligent context assembly
            match crate::memory::StrategistMemoryOrchestrator::assemble_context(
                &query,
                &messages_for_context, // Use compacted history
                state.librarian.clone(),
                settings.context_turns,
                settings.context_model.as_deref(),
                &settings,
                conversation_id.as_deref(),
            )
            .await
            {
                Ok(result) => {
                    if !result.context_text.is_empty() {
                        context_text = format!("Refined Context:\n{}\n", result.context_text);

                        // Emit memory-context event for UI
                        use tauri::Emitter;
                        if let Err(e) =
                            app_handle.emit("memory-context", &vec![result.context_text.clone()])
                        {
                            tracing::error!("Failed to emit memory-context: {}", e);
                        }

                        // Emit memory-hits event with selected IDs (for debugging/UI)
                        if let Err(e) =
                            app_handle.emit("memory-selected-ids", &result.selected_message_ids)
                        {
                            tracing::error!("Failed to emit memory-selected-ids: {}", e);
                        }

                        // Log search plan if present (for debugging)
                        if let Some(plan) = &result.search_plan {
                            tracing::debug!("Strategist plan: {} queries", plan.queries.len());
                            if let Some(notes) = &plan.notes {
                                tracing::debug!("Plan notes: {}", notes);
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Strategist memory assembly failed: {}", e);
                    // Continue without memory context rather than failing the entire request
                }
            }
        }

        // Inject Context
        let mut final_messages = messages_for_context; // Use compacted messages

        // iterate over messages to inject timestamp, UNLESS it's the last message (current user query)
        // or specifically, we want the LLM to know when previous messages were sent.
        // The last message might not have a created_at yet if it's new (or it might be passed from frontend?)
        // Actually, send_message receives `messages` which includes history.
        let len = final_messages.len();
        for (i, msg) in final_messages.iter_mut().enumerate() {
            let ts = if let Some(t) = msg.created_at {
                t
            } else if i == len - 1 {
                // Current message likely has no timestamp yet, use (now)
                chrono::Utc::now().timestamp()
            } else {
                // Old message with missing timestamp - skip
                0
            };

            if ts > 0 {
                if let Some(utc_dt) = chrono::DateTime::from_timestamp(ts, 0) {
                    let local_dt: chrono::DateTime<chrono::Local> = chrono::DateTime::from(utc_dt);
                    let formatted = local_dt.format("%Y:%m:%d:%H:%M:%S").to_string();
                    // Prefix content with timestamp
                    if let Some(content) = &mut msg.content {
                        *content = format!("<timestamp: {}>\n{}", formatted, content);
                    }
                }
            }
        }

        if !context_text.is_empty() {
            let context_msg = Message {
                id: None,
                role: "system".to_string(),
                content: Some(format!(
                    "You have access to the following long-term memories:\n{}",
                    context_text
                )),
                reasoning_content: None,
                tool_calls: None,
                tool_call_id: None,
                attachments: None,
                created_at: None,
            };
            final_messages.insert(0, context_msg);
        }

        // Inject System Prompt
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

        // Inject Current Date/Time
        let now = chrono::Local::now();
        let date_msg = Message {
            id: None,
            role: "system".to_string(),
            content: Some(format!(
                "CURRENT DATE: {}",
                now.format("%A, %B %d, %Y %H:%M")
            )),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            attachments: None,
            created_at: None,
        };

        let has_system_prompt = settings
            .active_system_prompt_id
            .as_ref()
            .and_then(|id| settings.system_prompts.iter().find(|p| &p.id == id))
            .is_some();

        if has_system_prompt {
            final_messages.insert(1, date_msg);
        } else {
            final_messages.insert(0, date_msg);
        }

        tracing::debug!("[DEBUG] Getting tools from MCP Manager...");
        let all_tools = state.mcp_manager.get_all_tools().await;

        let tools: Vec<_> = all_tools
            .into_iter()
            .filter(|t| !settings.disabled_tools.contains(&t.name))
            .collect();
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
                let provider = AnthropicProvider::new(api_key, model.clone());

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
                let token_count = crate::llm::tokenizer::count_tokens(content)
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

            // Trigger Background Fact Extraction for assistant message
            if memory_enabled {
                if let (Some(context_model), Some(msg_id), Some(content)) = (
                    extraction_model.clone(),
                    assistant_message_id_for_save,
                    response.content.clone(),
                ) {
                    let app_for_facts = app_handle.clone();
                    let librarian_for_facts = state.librarian.clone();
                    
                    tauri::async_runtime::spawn(async move {
                         let config_dir = app_for_facts.path().app_config_dir().unwrap_or_default();
                         let settings_path = config_dir.join("settings.json");
                         let settings = Settings::load_migrated(&settings_path);

                         let parts: Vec<&str> = context_model.split("::").collect();
                         if parts.len() == 2 {
                             if let Ok(provider) = crate::memory::StrategistMemoryOrchestrator::create_provider(parts[0], parts[1], &settings) {
                                 if let Err(e) = FactExtractor::extract(librarian_for_facts, provider.as_ref(), "assistant", &content, &msg_id).await {
                                     tracing::error!("Assistant fact extraction failed: {}", e);
                                 }
                             }
                         }
                    });
                }
            }
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
            let url = "https://api.anthropic.com/v1/models";
            let key = api_key.unwrap_or_default();

            let resp = client
                .get(url)
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
                let provider = AnthropicProvider::new(api_key, model.clone());
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
        let parts: Vec<&str> = t.name.splitn(2, "__").collect();
        let server = if parts.len() == 2 {
            parts[0]
        } else {
            "unknown"
        };

        result.push(ToolStatus {
            name: t.name.clone(),
            description: t.description,
            server: server.to_string(),
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
        // Extract server name from tool name "server__tool"
        let parts: Vec<&str> = t.name.splitn(2, "__").collect();
        let auto_approve = if parts.len() == 2 {
            let server_name = parts[0];
            let tool_name = parts[1];
            settings
                .mcp_servers
                .get(server_name)
                .map(|c| c.auto_approve || c.auto_approve_tools.contains(&tool_name.to_string()))
                .unwrap_or(false)
        } else {
            false
        };
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
    let new_prompt = crate::mcp::config::SystemPrompt {
        id: new_id.clone(),
        name: name.clone(),
        content: content.clone(),
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
        std::env::set_var("GTK_IM_MODULE", "xim");
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");

        // Disable touch emulation to allow proper mouse/pointer events
        // This fixes right-click and text selection on Linux
        std::env::set_var("GDK_CORE_DEVICE_EVENTS", "1");
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
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

                // 1. Librarian
                let librarian = crate::memory::librarian::Librarian::new(&config_dir).unwrap();
                let librarian_arc = Arc::new(Mutex::new(librarian));

                // 2. AppState
                let state = AppState {
                    mcp_manager: mcp_manager.clone(),
                    librarian: librarian_arc.clone(),
                    active_task: Arc::new(Mutex::new(None)),
                    context_approvals: Arc::new(Mutex::new(HashMap::new())),
                };
                app_handle.manage(state);

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
            execute_tool,
            get_tool_execution,
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
            cleanup_invalid_tool_messages
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
