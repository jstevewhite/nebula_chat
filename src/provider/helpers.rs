// src/provider/helpers.rs

use crate::llm::anthropic::AnthropicProvider;
use crate::llm::ollama::OllamaProvider;
use crate::llm::openai::OpenAiProvider;
use crate::llm::provider::LlmProvider;
use crate::mcp::config::{ProviderConfig, ProviderType, Settings};

/// Select the appropriate LLM provider based on the provider configuration.
/// Returns a boxed trait object implementing `LlmProvider`.
pub fn select_provider(config: &ProviderConfig) -> Box<dyn LlmProvider> {
    match config.provider_type {
        ProviderType::OpenAI => {
            let api_key = config.api_key.clone().unwrap_or_default();
            Box::new(OpenAiProvider::new(
                api_key,
                None,
                config.models[0].id.clone(),
            ))
        }
        ProviderType::OpenAICompatible => {
            let api_key = config.api_key.clone().unwrap_or_default();
            let base_url = config.base_url.clone();
            Box::new(OpenAiProvider::new(
                api_key,
                base_url,
                config.models[0].id.clone(),
            ))
        }
        ProviderType::Anthropic => {
            let api_key = config.api_key.clone().unwrap_or_default();
            Box::new(AnthropicProvider::new(api_key, config.models[0].id.clone()))
        }
        ProviderType::Ollama => {
            let base_url = config
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            Box::new(OllamaProvider::new(base_url, config.models[0].id.clone()))
        }
    }
}
