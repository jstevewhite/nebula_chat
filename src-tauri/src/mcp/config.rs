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

    /// When true, all `memory_doc_*` and `memory_fact_*` tools skip the
    /// per-call approval popup and execute immediately. Default true: tool
    /// calls only touch local audit-visible markdown docs and the KG, and
    /// prompting on every call drowns the UX.
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

    /// Trigger policy for KG fact extraction. One of:
    /// - `"explicit"` (default): only via the `/remember` chat command, the
    ///   "Save as fact" message action, or the LLM-callable
    ///   `memory_fact_remember` tool. No implicit per-turn extraction.
    /// - `"session_end"`: in addition to explicit triggers, run a one-shot
    ///   extraction pass on the messages added since the last checkpoint when
    ///   the user switches conversations.
    /// - `"off"`: all automatic extraction disabled. Explicit triggers still
    ///   work, but no session-end pass.
    #[serde(default = "default_extraction_policy")]
    pub fact_extraction_policy: String,

    /// Embedding backend selector for the docs subsystem. One of `"fastembed"`
    /// (local ONNX via the `local-embeddings` feature) or `"remote"` (call the
    /// embeddings endpoint of a configured LLM provider).
    #[serde(default = "default_embedding_provider")]
    pub memory_embedding_provider: String,

    /// fastembed model identifier when `memory_embedding_provider = "fastembed"`.
    /// Currently only `bge-small-en-v1.5` is wired (384 dim).
    #[serde(default = "default_fastembed_model")]
    pub memory_fastembed_model: String,

    /// Key into `providers` that hosts the embedding endpoint when
    /// `memory_embedding_provider = "remote"`. None disables remote embeddings.
    #[serde(default)]
    pub memory_remote_embedding_provider_id: Option<String>,

    /// Remote embedding model name (e.g. `text-embedding-3-small`).
    #[serde(default = "default_remote_embedding_model")]
    pub memory_remote_embedding_model: String,
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
fn default_extraction_policy() -> String {
    "explicit".to_string()
}
fn default_embedding_provider() -> String {
    "fastembed".to_string()
}
fn default_fastembed_model() -> String {
    "bge-small-en-v1.5".to_string()
}
fn default_remote_embedding_model() -> String {
    "text-embedding-3-small".to_string()
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
    #[serde(default)]
    pub built_in: bool,
}

impl Settings {
    pub fn load(path: &std::path::Path) -> Self {
        // This is a stub for logic that might be used elsewhere,
        // or we can redirect to load_migrated if we want to enforce migration always.
        // For strict backward compat without migration, simplistic:
        Self::load_migrated(path)
    }

    /// Re-apply built-in system prompts. Always overwrites existing entries
    /// with matching IDs so the binary remains source of truth (parallels
    /// `skills::builtins` materialization). Inserts missing built-ins.
    /// Returns true if anything changed and the settings should be re-saved.
    ///
    /// On the very first run (no prompts yet AND no active prompt set), also
    /// activates the first built-in so the user gets the intended behaviour
    /// out of the box. Subsequent runs leave `active_system_prompt_id` alone
    /// — including the case where the user has explicitly deactivated.
    pub fn materialize_builtin_prompts(&mut self) -> bool {
        let mut changed = false;
        let was_empty = self.system_prompts.is_empty();

        for (id, name, content) in crate::mcp::builtin_prompts::ALL {
            let new_prompt = SystemPrompt {
                id: id.to_string(),
                name: name.to_string(),
                content: content.to_string(),
                built_in: true,
            };
            if let Some(existing) = self.system_prompts.iter_mut().find(|p| p.id == *id) {
                if existing.name != new_prompt.name
                    || existing.content != new_prompt.content
                    || !existing.built_in
                {
                    *existing = new_prompt;
                    changed = true;
                }
            } else {
                self.system_prompts.push(new_prompt);
                changed = true;
            }
        }

        if was_empty && self.active_system_prompt_id.is_none() {
            if let Some((first_id, _, _)) = crate::mcp::builtin_prompts::ALL.first() {
                self.active_system_prompt_id = Some(first_id.to_string());
                changed = true;
            }
        }

        changed
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
        if let Some(p) = val
            .get("fact_extraction_policy")
            .and_then(|v| v.as_str())
        {
            s.fact_extraction_policy = p.to_string();
        }
        if let Some(p) = val
            .get("memory_embedding_provider")
            .and_then(|v| v.as_str())
        {
            s.memory_embedding_provider = p.to_string();
        }
        if let Some(p) = val.get("memory_fastembed_model").and_then(|v| v.as_str()) {
            s.memory_fastembed_model = p.to_string();
        }
        if let Some(p) = val
            .get("memory_remote_embedding_provider_id")
            .and_then(|v| v.as_str())
        {
            s.memory_remote_embedding_provider_id = Some(p.to_string());
        }
        if let Some(p) = val
            .get("memory_remote_embedding_model")
            .and_then(|v| v.as_str())
        {
            s.memory_remote_embedding_model = p.to_string();
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
