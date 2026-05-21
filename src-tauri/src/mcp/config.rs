use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum McpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    Sse {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    StreamableHttp {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ServerPermissions {
    #[serde(default)]
    pub allowlist: Vec<String>,
    #[serde(default)]
    pub denylist: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpServerConfig {
    #[serde(flatten)]
    pub transport: McpTransport,
    #[serde(default)]
    pub auto_approve: bool,
    #[serde(default)]
    pub auto_approve_tools: Vec<String>,
    #[serde(default)]
    pub permissions: ServerPermissions,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ProviderType {
    OpenAI,
    Anthropic,
    Ollama,
    OpenAICompatible, // For LMStudio, etc
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ModelConfig {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub visible: bool,
    pub context_window: Option<usize>,
    pub max_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cost: Option<String>, // Cost per token for prompt (as string to preserve precision)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_cost: Option<String>, // Cost per token for completion
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<u64>, // Number of model parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>, // Model description
    // Reasoning capabilities (auto-detected from OpenRouter or manually overridden)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_reasoning_effort: Option<bool>, // OpenAI o1/o3 style reasoning_effort parameter
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_thinking_mode: Option<bool>, // DeepSeek style thinking parameter
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_extended_thinking: Option<bool>, // Anthropic Claude 4 style extended thinking
}

fn default_true() -> bool {
    true
}

fn default_true_bool() -> bool {
    true
}
fn default_false_bool() -> bool {
    false
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProviderConfig {
    pub enabled: bool,
    pub provider_type: ProviderType,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Settings {
    // Legacy fields (kept optional for migration, or remove if brave)
    // We will keep them private or unused, but for now let's just use the map
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,

    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,

    #[serde(default)]
    pub default_model: Option<String>,

    #[serde(default)]
    pub disabled_tools: Vec<String>,

    #[serde(default)]
    pub system_prompts: Vec<crate::mcp::config::SystemPrompt>,

    #[serde(default)]
    pub active_system_prompt_id: Option<String>,

    #[serde(default)]
    pub context_model: Option<String>,

    // Number of recent conversation turns (user/assistant pairs) to include when assembling memory context.
    // 0 preserves prior behavior (no conversation included).
    #[serde(default)]
    pub context_turns: usize,

    // Enables long-term memory retrieval/injection (Tantivy search + context injection).
    // Disabling does NOT disable chat history persistence.
    #[serde(default = "default_true_bool")]
    pub memory_enabled: bool,

    #[serde(default = "default_true_bool")]
    pub enable_keychain: bool,

    // Enables context inspection mode: shows full context before sending to model
    // with Cancel/OK dialog for debugging and transparency.
    #[serde(default)]
    pub context_inspection_enabled: bool,

    /// When true, the built-in `update_tasks` tool is hidden from the LLM.
    #[serde(default = "default_false_bool")]
    pub disable_builtin_task_tool: bool,

    /// When true, the six memory_* tools (memory_remember / fetch / edit /
    /// forget / recall / link_context) skip the per-call approval popup and
    /// execute immediately. Default true: tool calls only touch local audit-
    /// visible markdown docs, and prompting on every call drowns the UX.
    #[serde(default = "default_true_bool")]
    pub memory_tools_auto_approve: bool,

    /// When true, every user turn prefixes the system context with the most
    /// relevant memory doc + a few KG facts. When false, the LLM still has
    /// the six memory_* tools and can fetch on demand, but nothing is pushed
    /// up front. Default true.
    #[serde(default = "default_true_bool")]
    pub memory_auto_inject_docs: bool,

    /// Hard token cap on the auto-injected memory block. Defaults to 4000.
    #[serde(default = "default_auto_inject_budget")]
    pub memory_auto_inject_token_budget: usize,

    /// Minimum fusion score for a doc to be auto-injected. Defaults to 0.20.
    #[serde(default = "default_recall_floor")]
    pub memory_recall_score_floor: f32,
    // Show per-message timestamps in the chat UI.
    #[serde(default = "default_false_bool")]
    pub show_message_timestamps: bool,

    // Theme preference: "light", "dark", "solarized-light", "solarized-dark"
    #[serde(default = "default_theme")]
    pub theme: String,

    #[serde(default = "default_font_interface")]
    pub interface_font: String,

    #[serde(default = "default_size_interface")]
    pub interface_font_size: u32,

    #[serde(default = "default_weight")]
    pub interface_font_weight: String,

    #[serde(default = "default_font_chat")]
    pub chat_font: String,

    #[serde(default = "default_size_chat")]
    pub chat_font_size: u32,

    #[serde(default = "default_weight")]
    pub chat_font_weight: String,

    // Number of recent messages to keep uncompressed.
    // If messages exceed this count, older ones are summarized.
    #[serde(default = "default_uncompressed_count")]
    pub context_uncompressed_msg_count: usize,
}

fn default_auto_inject_budget() -> usize {
    4000
}
fn default_recall_floor() -> f32 {
    0.20
}

fn default_uncompressed_count() -> usize {
    20
}

fn default_font_interface() -> String {
    "Inter".to_string()
}
fn default_size_interface() -> u32 {
    14
}
fn default_weight() -> String {
    "400".to_string()
}
fn default_font_chat() -> String {
    "Inter".to_string()
}
fn default_size_chat() -> u32 {
    14
}

fn default_theme() -> String {
    "dark".to_string()
}

#[derive(Clone, serde::Serialize, serde::Deserialize, Debug)]
pub struct SystemPrompt {
    pub id: String,
    pub name: String,
    pub content: String,
}

impl Settings {
    pub fn load(path: &std::path::Path) -> Self {
        // This is a stub for logic that might be used elsewhere,
        // or we can redirect to load_migrated if we want to enforce migration always.
        // For strict backward compat without migration, simplistic:
        Self::load_migrated(path)
    }

    // Helper for safe loading including migration from old format
    pub fn load_migrated(path: &std::path::Path) -> Self {
        if path.exists() {
            let content = std::fs::read_to_string(path).unwrap_or_default();
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                // Check if it has old keys
                let mut settings = if val.get("providers").is_some() {
                    // Start migration logic for MCP Servers
                    let s = serde_json::from_value::<Settings>(val.clone());

                    if s.is_err() {
                        eprintln!(
                            "Settings deserialization failed: {}",
                            s.as_ref().err().unwrap()
                        );
                        // Fallback: Try to migrate legacy MCP config manually
                        // Only try migration if we can at least parse Providers, otherwise we might be reading garbage
                        if val.get("providers").is_some() {
                            return Self::migrate_legacy_json(val);
                        }
                        // If we can't even find providers, let it error or return default
                        return s.unwrap_or_default();
                    }
                    s.unwrap_or_default()
                } else {
                    // Start fresh or migrate very old format
                    let s = Settings::default();
                    // ... (keep existing hardcoded migration logic if needed) ...
                    s
                };

                // Env var overrides
                if let Ok(key) = std::env::var("NEBULA_OPENAI_KEY") {
                    if let Some(p) = settings.providers.get_mut("openai") {
                        p.api_key = Some(key);
                    }
                }
                if let Ok(key) = std::env::var("NEBULA_ANTHROPIC_KEY") {
                    if let Some(p) = settings.providers.get_mut("anthropic") {
                        p.api_key = Some(key);
                    }
                }

                // Load keys from keychain if enabled
                if settings.enable_keychain {
                    for (name, provider) in settings.providers.iter_mut() {
                        if let Ok(Some(secret)) =
                            crate::security::keychain::get_secret("nebula_chat", name)
                        {
                            if !secret.is_empty() {
                                provider.api_key = Some(secret);
                            }
                        }
                    }
                }

                return settings;
            } else {
                eprintln!("Failed to parse settings.json as JSON value");
            }
        }
        Settings::default()
    }

    fn migrate_legacy_json(val: serde_json::Value) -> Settings {
        eprintln!("Migrating legacy settings JSON...");
        // Best effort migration from partial new or old state
        let mut s = Settings::default();

        if let Some(providers) = val
            .get("providers")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
        {
            s.providers = providers;
        }

        // Recover other fields
        if let Some(default_model) = val.get("default_model").and_then(|v| v.as_str()) {
            s.default_model = Some(default_model.to_string());
        }
        if let Some(disabled) = val
            .get("disabled_tools")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
        {
            s.disabled_tools = disabled;
        }
        if let Some(prompts) = val
            .get("system_prompts")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
        {
            s.system_prompts = prompts;
        }
        if let Some(active_prompt) = val.get("active_system_prompt_id").and_then(|v| v.as_str()) {
            s.active_system_prompt_id = Some(active_prompt.to_string());
        }
        if let Some(ctx_model) = val.get("context_model").and_then(|v| v.as_str()) {
            s.context_model = Some(ctx_model.to_string());
        }
        if let Some(ctx_turns) = val.get("context_turns").and_then(|v| v.as_u64()) {
            s.context_turns = ctx_turns as usize;
        }
        if let Some(mem_enabled) = val.get("memory_enabled").and_then(|v| v.as_bool()) {
            s.memory_enabled = mem_enabled;
        }
        if let Some(b) = val
            .get("memory_tools_auto_approve")
            .and_then(|v| v.as_bool())
        {
            s.memory_tools_auto_approve = b;
        }
        if let Some(b) = val.get("memory_auto_inject_docs").and_then(|v| v.as_bool()) {
            s.memory_auto_inject_docs = b;
        }
        if let Some(n) = val
            .get("memory_auto_inject_token_budget")
            .and_then(|v| v.as_u64())
        {
            s.memory_auto_inject_token_budget = n as usize;
        }
        if let Some(f) = val
            .get("memory_recall_score_floor")
            .and_then(|v| v.as_f64())
        {
            s.memory_recall_score_floor = f as f32;
        }
        if let Some(theme) = val.get("theme").and_then(|v| v.as_str()) {
            s.theme = theme.to_string();
        }
        if let Some(font) = val.get("interface_font").and_then(|v| v.as_str()) {
            s.interface_font = font.to_string();
        }
        if let Some(size) = val.get("interface_font_size").and_then(|v| v.as_u64()) {
            s.interface_font_size = size as u32;
        }
        if let Some(weight) = val.get("interface_font_weight").and_then(|v| v.as_str()) {
            s.interface_font_weight = weight.to_string();
        }
        if let Some(font) = val.get("chat_font").and_then(|v| v.as_str()) {
            s.chat_font = font.to_string();
        }
        if let Some(size) = val.get("chat_font_size").and_then(|v| v.as_u64()) {
            s.chat_font_size = size as u32;
        }
        if let Some(weight) = val.get("chat_font_weight").and_then(|v| v.as_str()) {
            s.chat_font_weight = weight.to_string();
        }
        if let Some(count) = val
            .get("context_uncompressed_msg_count")
            .and_then(|v| v.as_u64())
        {
            s.context_uncompressed_msg_count = count as usize;
        }

        if let Some(mcp) = val.get("mcp_servers").and_then(|v| v.as_object()) {
            for (name, server_val) in mcp {
                // Try New Format first
                if let Ok(config) = serde_json::from_value::<McpServerConfig>(server_val.clone()) {
                    s.mcp_servers.insert(name.clone(), config);
                } else {
                    eprintln!(
                        "Failed to parse MCP server '{}' as standard config, trying legacy...",
                        name
                    );
                    // Try Legacy Stdio format: { "command": "...", "args": ... }
                    // Only for Stdio, SSE usually matches standard config if type present
                    if let Some(cmd) = server_val.get("command").and_then(|v| v.as_str()) {
                        let args = server_val
                            .get("args")
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                            .unwrap_or_default();
                        let env = server_val
                            .get("env")
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                            .unwrap_or_default();
                        let auto_approve = server_val
                            .get("auto_approve")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        let config = McpServerConfig {
                            transport: McpTransport::Stdio {
                                command: cmd.to_string(),
                                args,
                                env,
                            },
                            auto_approve,
                            auto_approve_tools: vec![],
                            permissions: ServerPermissions::default(),
                        };
                        s.mcp_servers.insert(name.clone(), config);
                    }
                }
            }
        }

        s
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut to_save = self.clone();

        if self.enable_keychain {
            for (name, provider) in self.providers.iter() {
                if let Some(key) = &provider.api_key {
                    if !key.trim().is_empty() {
                        if let Err(e) =
                            crate::security::keychain::set_secret("nebula_chat", name, key)
                        {
                            eprintln!("Failed to save key for {} to keychain: {}", name, e);
                        } else {
                            // Strip from file payload
                            if let Some(p) = to_save.providers.get_mut(name) {
                                p.api_key = None;
                            }
                        }
                    }
                }
            }
        }

        let content = serde_json::to_string_pretty(&to_save)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disable_builtin_task_tool_defaults_false() {
        let s = Settings::default();
        assert!(!s.disable_builtin_task_tool);
    }

    #[test]
    fn disable_builtin_task_tool_round_trips_via_serde() {
        let s = Settings {
            disable_builtin_task_tool: true,
            ..Settings::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert!(back.disable_builtin_task_tool);
    }
}
