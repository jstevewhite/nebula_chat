// src/settings/mod.rs

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct FeatureFlags {
    /// Enable OS keychain storage for provider secrets
    pub enable_keychain: bool,
    /// Enable new logging system (tracing)
    pub enable_tracing: bool,
    /// Enable versioned SQLite migrations
    pub enable_migrations: bool,
    /// Enable timing metrics collection
    pub enable_timing: bool,
    // Add more flags as needed
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Settings {
    pub providers: std::collections::HashMap<String, crate::mcp::config::ProviderConfig>,
    pub mcp_servers: std::collections::HashMap<String, crate::mcp::config::McpServerConfig>,
    pub system_prompts: Vec<crate::mcp::config::SystemPrompt>,
    pub active_system_prompt_id: Option<String>,
    pub disabled_tools: Vec<String>,
    pub default_model: Option<String>,
    pub feature_flags: FeatureFlags,
    // other fields remain unchanged
}

impl Settings {
    /// Load settings from the given path, performing any required migrations.
    pub fn load_migrated<P: AsRef<Path>>(path: P) -> Self {
        // Load the JSON file (fallback to defaults on error)
        let raw = fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
        let mut settings: Settings = serde_json::from_str(&raw).unwrap_or_default();
        // Run migrations if the flag is enabled
        if settings.feature_flags.enable_migrations {
            crate::migration::run_pending(&mut settings);
        }
        // Load provider API keys from the OS keychain if enabled
        if settings.feature_flags.enable_keychain {
            for (name, provider) in settings.providers.iter_mut() {
                if let Some(key) = crate::security::keychain::get_secret("nebula_chat", name)
                    .ok()
                    .flatten()
                {
                    provider.api_key = Some(key);
                }
            }
        }
        settings
    }

    /// Save the current settings back to the given path, omitting provider API keys.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), std::io::Error> {
        // Clone and strip secrets before writing
        let mut sanitized = self.clone();
        for provider in sanitized.providers.values_mut() {
            provider.api_key = None;
        }
        let json = serde_json::to_string_pretty(&sanitized)?;
        fs::write(path, json)
    }
}

// Public API for the rest of the codebase
pub fn load() -> Settings {
    let config_dir = tauri::api::path::app_config_dir(&tauri::Config::default()).unwrap();
    let settings_path = config_dir.join("settings.json");
    Settings::load_migrated(settings_path)
}

pub fn save(settings: &Settings) {
    let config_dir = tauri::api::path::app_config_dir(&tauri::Config::default()).unwrap();
    let settings_path = config_dir.join("settings.json");
    let _ = settings.save(settings_path);
}
