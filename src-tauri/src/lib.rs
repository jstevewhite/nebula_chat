use crate::llm::openai::OpenAiProvider;
use crate::llm::provider::{LlmProvider, Message};
use crate::mcp::config::{McpServerConfig, ModelConfig, ProviderType, Settings};
use crate::mcp::manager::McpManager;
use std::sync::Arc;
use tauri::{Manager, State};
use tokio::sync::Mutex;

use crate::llm::anthropic::AnthropicProvider;
use crate::llm::context::ContextManager;
use crate::llm::ollama::OllamaProvider;
use std::collections::HashMap;

pub mod llm;
pub mod mcp;
pub mod memory;

struct AppState {
    mcp_manager: Arc<McpManager>,
    librarian: Arc<Mutex<crate::memory::librarian::Librarian>>,
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
    for (_, (_, role, content, _, _)) in history.iter().enumerate().take(6) {
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
    }];

    let response = match provider_config.provider_type {
        ProviderType::OpenAI | ProviderType::OpenAICompatible => {
            let api_key = provider_config.api_key.clone().unwrap_or_default();
            let base_url = provider_config.base_url.clone();
            let provider = OpenAiProvider::new(api_key, base_url, model.clone());
            provider
                .chat(messages, tools)
                .await
                .map_err(|e| e.to_string())
        }
        ProviderType::Anthropic => {
            let api_key = provider_config.api_key.clone().unwrap_or_default();
            let provider = AnthropicProvider::new(api_key, model.clone());
            provider
                .chat(messages, tools)
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
                .chat(messages, tools)
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
    for (id, role, content, tool_calls_json, tool_call_id) in raw {
        let tool_calls = if let Some(json_str) = tool_calls_json {
            if !json_str.is_empty() {
                serde_json::from_str(&json_str).ok()
            } else {
                None
            }
        } else {
            None
        };

        messages.push(Message {
            id: Some(id),
            role,
            content,
            tool_calls,
            tool_call_id,
        });
    }
    Ok(messages)
}

#[tauri::command]
async fn send_message(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    messages: Vec<Message>,
    provider_id: String,
    model: String,
    conversation_id: String,
) -> Result<Message, String> {
    // Prepare Clones for Background Task
    let app_handle = app.clone();
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

            let lib = state.librarian.lock().await;
            let tool_calls_json = if let Some(tc) = &last.tool_calls {
                serde_json::to_string(tc).ok()
            } else {
                None
            };

            // Save full message
            let _ = lib.save_full_message(
                &conversation_id,
                &last.role,
                last.content.as_deref(),
                tool_calls_json.as_deref(),
                last.tool_call_id.as_deref(),
            );
        }
    }

    // Retrieve Context
    let mut context_text = String::new();
    if !query.is_empty() {
        let lib = state.librarian.lock().await;
        if let Ok(results) = lib.search(&query) {
            if !results.is_empty() {
                context_text = "Relevant Memories:\n".to_string();
                for (_id, content) in results {
                    context_text.push_str(&format!("- {}\n", content));
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
        };
        final_messages.insert(0, context_msg);
    }

    let all_tools = state.mcp_manager.get_all_tools().await;

    // Filter disabled tools
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    let settings = Settings::load_migrated(&settings_path);

    let tools: Vec<_> = all_tools
        .into_iter()
        .filter(|t| !settings.disabled_tools.contains(&t.name))
        .collect();

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

    // Prune context
    let pruned_messages =
        ContextManager::prune_messages(final_messages, 64000).map_err(|e| e.to_string())?;

    let response = match provider_config.provider_type {
        ProviderType::OpenAI | ProviderType::OpenAICompatible => {
            let api_key = provider_config.api_key.clone().unwrap_or_default();
            let base_url = provider_config.base_url.clone();
            let provider = OpenAiProvider::new(api_key, base_url, model.clone());
            provider
                .chat(pruned_messages, tools)
                .await
                .map_err(|e| e.to_string())
        }
        ProviderType::Anthropic => {
            let api_key = provider_config.api_key.clone().unwrap_or_default();
            let provider = AnthropicProvider::new(api_key, model.clone());
            provider
                .chat(pruned_messages, tools)
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
                .chat(pruned_messages, tools)
                .await
                .map_err(|e| e.to_string())
        }
    };

    match response {
        Ok(msg) => {
            // Save Assistant Message
            let lib = state.librarian.lock().await;
            let tool_calls_json = if let Some(tc) = &msg.tool_calls {
                serde_json::to_string(tc).ok()
            } else {
                None
            };

            let _ = lib.save_full_message(
                &conversation_id,
                "assistant",
                msg.content.as_deref(),
                tool_calls_json.as_deref(),
                msg.tool_call_id.as_deref(),
            );

            // Trigger Background Pruning (Fire & Forget)
            tauri::async_runtime::spawn(async move {
                if let Err(e) = attempt_pruning(
                    app_handle,
                    librarian_arc,
                    provider_id_bg,
                    model_bg,
                    conv_id_bg,
                )
                .await
                {
                    eprintln!("Pruning Error: {}", e);
                }
            });

            Ok(msg)
        }
        Err(e) => Err(e),
    }
}

#[tauri::command]
async fn execute_tool(
    state: State<'_, AppState>,
    name: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    state
        .mcp_manager
        .call_tool(&name, args)
        .await
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
            let url = format!(
                "{}/v1/models",
                base_url
                    .unwrap_or_else(|| "https://api.openai.com".to_string())
                    .trim_end_matches('/')
            );
            let key = api_key.unwrap_or_default();

            let resp = client
                .get(&url)
                .header("Authorization", format!("Bearer {}", key))
                .send()
                .await
                .map_err(|e| e.to_string())?;

            if !resp.status().is_success() {
                return Err(format!("Failed to fetch models: {}", resp.status()));
            }

            let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            if let Some(arr) = json["data"].as_array() {
                for m in arr {
                    if let Some(id) = m["id"].as_str() {
                        models.push(ModelConfig {
                            id: id.to_string(),
                            name: id.to_string(),
                            visible: true,
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
                            });
                        }
                    }
                }
            } else {
                // Fallback if fetch fails (e.g. key invalid or network issue), but updated with latest models
                models.push(ModelConfig {
                    id: "claude-3-5-sonnet-20241022".to_string(),
                    name: "Claude 3.5 Sonnet (New)".to_string(),
                    visible: true,
                });
                models.push(ModelConfig {
                    id: "claude-3-5-haiku-20241022".to_string(),
                    name: "Claude 3.5 Haiku".to_string(),
                    visible: true,
                });
                models.push(ModelConfig {
                    id: "claude-3-5-sonnet-20240620".to_string(),
                    name: "Claude 3.5 Sonnet (Old)".to_string(),
                    visible: true,
                });
                models.push(ModelConfig {
                    id: "claude-3-opus-20240229".to_string(),
                    name: "Claude 3 Opus".to_string(),
                    visible: true,
                });
                models.push(ModelConfig {
                    id: "claude-3-sonnet-20240229".to_string(),
                    name: "Claude 3 Sonnet".to_string(),
                    visible: true,
                });
                models.push(ModelConfig {
                    id: "claude-3-haiku-20240307".to_string(),
                    name: "Claude 3 Haiku".to_string(),
                    visible: true,
                });
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
    url: Option<String>,
) -> Result<(), String> {
    use crate::mcp::config::McpTransport;

    let transport = match transport_type.as_str() {
        "stdio" => McpTransport::Stdio {
            command: command.ok_or("Command required for Stdio")?,
            args: args.unwrap_or_default(),
            env: HashMap::new(),
        },
        "sse" => McpTransport::Sse {
            url: url.ok_or("URL required for SSE")?,
        },
        _ => return Err("Invalid transport type".to_string()),
    };

    let config = McpServerConfig {
        transport,
        auto_approve: false,
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
        .map_err(|e| e.to_string())
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
        .map_err(|e| e.to_string())
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
                let base_url = provider_config.base_url.clone();
                let provider = OpenAiProvider::new(api_key, base_url, model.clone());
                provider.chat(vec![Message {
                    id: None,
                    role: "user".to_string(),
                    content: Some(format!("Summarize the following conversation segment concisely, preserving key facts:\n\n{}", text_to_summarize)),
                    tool_calls: None,
                    tool_call_id: None
                }], vec![]).await
            }
            ProviderType::Anthropic => {
                let api_key = provider_config.api_key.clone().unwrap_or_default();
                let provider = AnthropicProvider::new(api_key, model.clone());
                provider.chat(vec![Message {
                    id: None,
                    role: "user".to_string(),
                    content: Some(format!("Summarize the following conversation segment concisely, preserving key facts:\n\n{}", text_to_summarize)),
                    tool_calls: None,
                    tool_call_id: None
                }], vec![]).await
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
                    tool_call_id: None
                }], vec![]).await
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
                    println!("Pruning complete. Summarized {} messages.", prune_count);
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

    settings.save(&settings_path).map_err(|e| e.to_string())
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

    settings.save(&settings_path).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mcp_manager = Arc::new(McpManager::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let config_dir = app.path().app_config_dir().unwrap();

            // Init Memory
            let memory_dir = config_dir.join("memory");
            let librarian = crate::memory::librarian::Librarian::new(&memory_dir)
                .expect("Failed to initialize memory");

            app.manage(AppState {
                mcp_manager: mcp_manager.clone(),
                librarian: Arc::new(Mutex::new(librarian)),
            });

            let settings_path = config_dir.join("settings.json");

            let settings = Settings::load_migrated(&settings_path);

            let manager = mcp_manager.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = manager.initialize(settings).await {
                    eprintln!("Failed to initialize from settings: {}", e);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            send_message,
            execute_tool,
            add_mcp_server,
            get_mcp_servers,
            get_settings,
            save_settings,
            fetch_models,
            list_conversations,
            create_conversation,
            get_chat_history,
            delete_conversation,
            rename_conversation,
            generate_title,
            edit_mcp_server,
            delete_message,
            set_default_model,
            get_tools,
            toggle_tool,
            toggle_tool_list
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
