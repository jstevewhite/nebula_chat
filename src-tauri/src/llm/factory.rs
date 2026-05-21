//! Helpers for instantiating an `LlmProvider` from settings. Lives in `llm`
//! (not `memory`) because the strategist module that previously housed this
//! factory was deleted in memory3 Phase 2.

use anyhow::Result;

use crate::llm::{
    anthropic::AnthropicProvider, ollama::OllamaProvider, openai::OpenAiProvider,
    provider::LlmProvider,
};
use crate::mcp::config::{ProviderType, Settings};

/// Build a provider for the given `(provider_id, model_name)` pair using the
/// `providers` table from settings. Returns an error if the provider id is
/// missing.
pub fn create_provider(
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
