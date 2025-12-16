use crate::llm::openai::OpenAiProvider;
use crate::llm::provider::{Attachment, GenerationOptions, LlmProvider, Message};
use crate::mcp::config::Settings;
use crate::mcp::config::{McpServerConfig, ModelConfig, ProviderType};
use crate::mcp::manager::McpManager;
use crate::memory::tantivy_index::SearchResult;
use anyhow::Result;
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

#[cfg(test)]
mod tests {
    #[test]
    fn test_tool_call_validation_integration() {
        // This is a placeholder for integration tests
        // The actual database validation is tested in the sqlite_manager tests
    }
}

#[derive(Clone)]
pub struct AppState {
    mcp_manager: Arc<McpManager>,
    librarian: Arc<Mutex<crate::memory::librarian::Librarian>>,
    active_task: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
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

#[derive(serde::Serialize)]
struct Conversation {
    id: String,
    title: String,
    created_at: String,
}

#[tauri::command]
async fn list_conversations(state: State<'_, AppState>) -> Result<Vec<Conversation>, String> {
    let lib = state.librarian.lock().await;
    let list = lib.list_conversations().map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|(id, title, created_at)| Conversation {
            id,
            title,
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
    for (_, (_, role, content, _, _, _, _)) in history.iter().enumerate().take(6) {
        if let Some(c) = content {
            prompt.push_str(&format!("{}: {}\n", role, c));
        }
    }

    prompt.push_str("\n\nInstructions: Generate a very brief (max 5 words) title for this conversation. Do not use quotes.");

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
        tool_calls: None,
        tool_call_id: None,
        attachments: None,
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

    let new_title = response
        .content
        .unwrap_or("New Chat".to_string())
        .trim()
        .trim_matches('"')
        .to_string();

    // Re-acquire lock to save
    let lib = state.librarian.lock().await;
    lib.rename_conversation(&conversation_id, &new_title)
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

    let mut messages = Vec::new();
    for (id, role, content, tool_calls_json, tool_call_id, _, attachments_json) in raw {
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

        messages.push(Message {
            id: Some(id),
            role,
            content,
            tool_calls,
            tool_call_id,
            attachments,
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
) -> Result<Message, String> {
    // Clone necessary data for the async task
    let state_owned = state.inner().clone();
    let app_handle_clone = app_handle.clone();
    let provider_id_clone = provider_id.clone();
    let model_clone = model.clone();
    let conversation_id_clone = conversation_id.clone();
    let messages_clone = messages.clone();
    let attachments_clone = attachments.clone();

    // Spawn the generation task
    let task = tokio::spawn(async move {
        // --- ORIGINAL LOGIC START ---

        // We're working on clones now
        let mut messages = messages_clone;
        let provider_id = provider_id_clone;
        let model = model_clone;
        let conversation_id = conversation_id_clone;
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

                    // Save full message
                    let _ = lib.save_full_message(
                        conv_id,
                        &last.role,
                        last.content.as_deref(),
                        tool_calls_json.as_deref(),
                        last.tool_call_id.as_deref(),
                        last.attachments.as_deref(), // Use last.attachments to avoid borrow error
                    );
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

        // Retrieve Context (Long-term memory)
        let mut context_text = String::new();
        if settings.memory_enabled && !query.is_empty() {
            let lib = state.librarian.lock().await;
            if let Ok(results) = lib.search(&query) {
                if !results.is_empty() {
                    // Emit Memory Context Event
                    let memory_list_preview: Vec<String> =
                        results.iter().map(|res| res.content.clone()).collect();

                    if let Some(ctx_model) = &settings.context_model {
                        let assembled = crate::llm::context_assembler::ContextAssembler::assemble(
                            &query,
                            &memory_list_preview,
                            &messages,
                            settings.context_turns,
                            ctx_model,
                            &settings,
                        )
                        .await
                        .unwrap_or_else(|_| memory_list_preview.join("\n"));

                        context_text = format!("Refined Context:\n{}\n", assembled);
                        use tauri::Emitter;
                        if let Err(e) = app_handle.emit("memory-context", &vec![assembled]) {
                            tracing::error!("Failed to emit memory-context: {}", e);
                        }
                    } else {
                        use tauri::Emitter;
                        if let Err(e) = app_handle.emit("memory-context", &memory_list_preview) {
                            tracing::error!("Failed to emit memory-context: {}", e);
                        }

                        context_text = "Relevant Memories:\n".to_string();
                        for res in results {
                            context_text.push_str(&format!("- {}\n", res.content));
                        }
                    }
                }
            }
        }

        // Inject Context
        let mut final_messages = messages.clone();
        if !context_text.is_empty() {
            let context_msg = Message {
                id: None,
                role: "system".to_string(),
                content: Some(format!(
                    "You have access to the following long-term memories:\n{}",
                    context_text
                )),
                tool_calls: None,
                tool_call_id: None,
                attachments: None,
            };
            final_messages.insert(0, context_msg);
        }

        // Inject System Prompt
        tracing::debug!("[DEBUG] Loading settings for system prompt...");

        if let Some(active_id) = settings.active_system_prompt_id {
            if let Some(prompt) = settings.system_prompts.iter().find(|p| p.id == active_id) {
                tracing::debug!("[DEBUG] Injecting system prompt: {}", prompt.name);
                final_messages.insert(
                    0,
                    Message {
                        id: Some(uuid::Uuid::new_v4().to_string()),
                        role: "system".to_string(),
                        content: Some(prompt.content.clone()),
                        tool_call_id: None,
                        tool_calls: None,
                        attachments: None,
                    },
                );
            }
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

        let options = Some(GenerationOptions {
            temperature,
            top_p,
            stream,
        });

        let app_handle_for_stream = app_handle.clone();
        let on_token = Box::new(move |token: String| {
            use tauri::Emitter;
            tracing::debug!("🔊 Emitting stream chunk: {} chars", token.len());
            // Emit standard stream chunk event
            if let Err(e) = app_handle_for_stream.emit("stream-chunk", &token) {
                tracing::error!("Failed to emit stream chunk: {}", e);
            } else {
                tracing::debug!("✅ Stream chunk emitted successfully");
            }
        });

        // Provider Execution
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
                        .stream(pruned_messages, tools, options, on_token)
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
                        .stream(pruned_messages, tools, options, on_token)
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
                        .stream(pruned_messages, tools, options, on_token)
                        .await
                } else {
                    provider.chat(pruned_messages, tools, options).await
                }
            }
        };

        let response = response_result.map_err(|e| e.to_string())?;

        // Handle Response (Save & Prune triggers)
        if let Some(conv_id) = conversation_id {
            let lib = state.librarian.lock().await;
            let tool_calls_json = if let Some(tc) = &response.tool_calls {
                serde_json::to_string(tc).ok()
            } else {
                None
            };

            // Note: If streaming, 'response' contains the FULL aggregated content at the end, so saving works fine.

            let _ = lib.save_full_message(
                &conv_id,
                "assistant",
                response.content.as_deref(),
                tool_calls_json.as_deref(),
                response.tool_call_id.as_deref(),
                None,
            );

            // Trigger Background Pruning (Fire & Forget)
            tauri::async_runtime::spawn(async move {
                if let Err(e) = attempt_pruning(
                    app_handle,
                    librarian_arc,
                    provider_id_bg,
                    model_bg,
                    conv_id_bg.unwrap_or_default(),
                )
                .await
                {
                    tracing::error!("Pruning Error: {}", e);
                }
            });
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
                        models.push(ModelConfig {
                            id: id.to_string(),
                            name: id.to_string(),
                            visible: true,
                            context_window: None,
                            max_tokens: None,
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
                                context_window: None,
                                max_tokens: None,
                            });
                        }
                    }
                }
            } else {
                // Fallback if fetch fails (e.g. key invalid or network issue), but updated with latest models
                // This ensures the dropdown isn't empty even if the API call fails.
                let fallback_models = vec![
                    "claude-3-opus-20240229",
                    "claude-3-sonnet-20240229",
                    "claude-3-haiku-20240307",
                    "claude-3-5-sonnet-20240620",
                ];
                for id in fallback_models {
                    models.push(ModelConfig {
                        id: id.to_string(),
                        name: id.to_string(),
                        visible: true,
                        context_window: None,
                        max_tokens: None,
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
                            context_window: None,
                            max_tokens: None,
                        });
                    }
                }
            }
        }
    }

    Ok(models)
}

#[tauri::command]
async fn add_mcp_server(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
    transport_type: String, // "stdio" or "sse"
    command: Option<String>,
    args: Option<Vec<String>>,
    env: Option<HashMap<String, String>>,
    url: Option<String>,
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
    let lib = state.librarian.lock().await;

    // 1. Clear Tantivy
    lib.clear_search_index().map_err(|e| e.to_string())?;

    // 2. Read all messages from SQLite
    // We need a method to iterate all messages, or just list convs and get messages for each
    let conversations = lib.list_conversations().map_err(|e| e.to_string())?;

    for (conv_id, _, _) in conversations {
        let messages = lib
            .get_complete_history(&conv_id)
            .map_err(|e| e.to_string())?;

        for (msg_id, role, content, _, _, created_at, _) in messages {
            if let Some(text) = content {
                lib.index_existing_message(&conv_id, &role, &text, &msg_id, &created_at)
                    .map_err(|e| e.to_string())?;
            }
        }
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
    let lib = state.librarian.lock().await;

    // Get metadata
    let convs = lib.list_conversations().map_err(|e| e.to_string())?;
    let (_, title, created_at) = convs
        .into_iter()
        .find(|(id, _, _)| id == &conversation_id)
        .ok_or("Conversation not found")?;

    // Get messages
    let raw_msgs = lib
        .get_complete_history(&conversation_id)
        .map_err(|e| e.to_string())?;

    let mut messages = Vec::new();
    for (id, role, content, tool_calls_txt, tool_call_id, _created, attachments_json) in raw_msgs {
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

        messages.push(Message {
            id: Some(id),
            role,
            content,
            tool_calls,
            tool_call_id,
            attachments,
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
    settings.save(&settings_path).map_err(|e| e.to_string())
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
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let mut settings = Settings::load_migrated(&settings_path);

    let new_id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let new_prompt = crate::mcp::config::SystemPrompt {
        id: new_id.clone(),
        name,
        content,
    };

    if let Some(idx) = settings.system_prompts.iter().position(|p| p.id == new_id) {
        settings.system_prompts[idx] = new_prompt;
    } else {
        settings.system_prompts.push(new_prompt);
    }

    settings.save(&settings_path).map_err(|e| e.to_string())
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

    settings.save(&settings_path).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_active_mcp_servers(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(state.mcp_manager.list_servers().await)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            list_conversations,
            create_conversation,
            delete_conversation,
            rename_conversation,
            delete_message,
            generate_title,
            get_chat_history,
            send_message,
            get_settings,
            save_settings,
            fetch_models,
            execute_tool,
            get_tool_execution,
            add_mcp_server,
            edit_mcp_server,
            delete_mcp_server,
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
            search_messages,
            export_conversation,
            get_tool_policies,
            toggle_mcp_server_auto_approve,
            toggle_tool_auto_approve,
            import_conversation
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
