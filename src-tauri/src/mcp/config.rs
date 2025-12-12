use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub auto_approve: bool,
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
}

fn default_true() -> bool { true }

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
                    serde_json::from_value::<Settings>(val).unwrap_or_default()
                } else {
                    // MIGRATE
                    let mut s = Settings::default();
                    
                    // OpenAI
                    if let Some(k) = val.get("api_key").and_then(|v| v.as_str()) {
                         let mut models = vec![
                             ModelConfig { id: "gpt-4o".to_string(), name: "GPT-4o".to_string(), visible: true }
                         ];
                         s.providers.insert("openai".to_string(), ProviderConfig {
                             enabled: true,
                             provider_type: ProviderType::OpenAI,
                             base_url: None,
                             api_key: Some(k.to_string()),
                             models
                         });
                    }
                    
                    // Anthropic
                    if let Some(k) = val.get("anthropic_key").and_then(|v| v.as_str()) {
                         s.providers.insert("anthropic".to_string(), ProviderConfig {
                             enabled: true,
                             provider_type: ProviderType::Anthropic,
                             base_url: None,
                             api_key: Some(k.to_string()),
                             models: vec![
                                ModelConfig { id: "claude-3-5-sonnet-20240620".to_string(), name: "Claude 3.5 Sonnet".to_string(), visible: true }
                             ]
                         });
                    }

                    // Ollama
                    let url = val.get("ollama_url").and_then(|v| v.as_str()).unwrap_or("http://localhost:11434");
                    s.providers.insert("ollama".to_string(), ProviderConfig {
                         enabled: true,
                         provider_type: ProviderType::Ollama,
                         base_url: Some(url.to_string()),
                         api_key: None,
                         models: vec![]
                    });

                    // MCP
                    if let Some(mcp) = val.get("mcp_servers") {
                        if let Ok(servers) = serde_json::from_value(mcp.clone()) {
                            s.mcp_servers = servers;
                        }
                    }
                    
                    s
                };
                
                // Env var overrides
                if let Ok(key) = std::env::var("NEBULA_OPENAI_KEY") {
                    if let Some(p) = settings.providers.get_mut("openai") { p.api_key = Some(key); }
                }
                if let Ok(key) = std::env::var("NEBULA_ANTHROPIC_KEY") {
                     if let Some(p) = settings.providers.get_mut("anthropic") { p.api_key = Some(key); }
                }
                
                return settings;
            }
        }
        
        // Fallback default
        let mut s = Self::default();
        s.providers.insert("openai".to_string(), ProviderConfig {
                enabled: true, provider_type: ProviderType::OpenAI, base_url: None, api_key: None, 
                models: vec![ModelConfig { id: "gpt-4o".to_string(), name: "GPT-4o".to_string(), visible: true }]
        });
        s.providers.insert("anthropic".to_string(), ProviderConfig {
                enabled: true, provider_type: ProviderType::Anthropic, base_url: None, api_key: None, 
                models: vec![ModelConfig { id: "claude-3-5-sonnet-20240620".to_string(), name: "Claude 3.5 Sonnet".to_string(), visible: true }]
        });
        s.providers.insert("ollama".to_string(), ProviderConfig {
                enabled: true, provider_type: ProviderType::Ollama, base_url: Some("http://localhost:11434".to_string()), api_key: None, models: vec![]
        });
        s
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}
