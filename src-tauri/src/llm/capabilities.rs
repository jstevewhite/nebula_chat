// src/llm/capabilities.rs
use crate::mcp::config::ProviderType;

#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    pub supports_tools: bool,
    pub supports_streaming: bool,
    pub supports_streaming_tools: bool,
    pub supports_multimodal: bool,
}

pub fn get_capabilities(provider: &ProviderType, _model: &str) -> Capabilities {
    match provider {
        ProviderType::OpenAI => Capabilities {
            supports_tools: true,
            supports_streaming: true,
            supports_streaming_tools: true, // OpenAI handles this well
            supports_multimodal: true,      // Generally true for GPT-4o, etc.
        },
        ProviderType::Anthropic => Capabilities {
            supports_tools: true,
            supports_streaming: true,
            // Anthropic streaming with tools can be tricky or unsupported in some client impls,
            // but for now we'll flag it as false to be safe per the plan,
            // or true if we are confident. The plan says "If provider does not support streaming tools..."
            // Let's be conservative for now as per Phase 1.4 requirements.
            supports_streaming_tools: false,
            supports_multimodal: true,
        },
        ProviderType::Ollama => Capabilities {
            supports_tools: false, // Often varies by model, but safe default is false
            supports_streaming: true,
            supports_streaming_tools: false,
            supports_multimodal: false,
        },
        ProviderType::OpenAICompatible => Capabilities {
            supports_tools: true, // OpenAI-compatible APIs (like OpenRouter) support tools
            supports_streaming: true,
            supports_streaming_tools: true, // They use the same streaming format as OpenAI
            supports_multimodal: true, // Many support vision models
        },
    }
}
