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
            supports_multimodal: true,      // Many support vision models
        },
    }
}

pub fn get_model_context_window(model: &str) -> Option<usize> {
    let m = model.to_lowercase();

    // OpenAI
    if m.contains("gpt-4o") || m.contains("gpt-4-turbo") {
        return Some(128_000);
    }
    if m.contains("gpt-4-0125") || m.contains("gpt-4-1106") {
        return Some(128_000);
    }
    if m.contains("gpt-4") {
        return Some(8_192);
    }
    if m.contains("gpt-3.5-turbo") {
        return Some(16_385);
    }
    if m.contains("o1-preview") || m.contains("o1-mini") {
        return Some(128_000);
    }
    if m.contains("o1") {
        // Future proofing generic o1
        return Some(200_000);
    }

    // Anthropic
    // User mentioned "Claude Sonnet is at 4.5" - possibly referring to recent leaks or a specific proxy setup.
    // We will support checking for 4.5 or 3.5 explicitly.
    if m.contains("claude-4.5") || m.contains("claude-4.5-sonnet") {
        return Some(200_000); // Assumed
    }
    if m.contains("claude-3-5") || m.contains("claude-3.5") {
        return Some(200_000);
    }
    if m.contains("claude-3") {
        return Some(200_000);
    }

    // DeepSeek
    if m.contains("deepseek-v3") || m.contains("deepseek-3") {
        return Some(128_000); // V3 is often 128k
    }
    if m.contains("deepseek") {
        // Fallback for generic deepseek
        return Some(32_000);
    }

    // Google
    if m.contains("gemini-1.5-pro") || m.contains("gemini-pro-1.5") {
        return Some(2_000_000);
    }
    if m.contains("gemini-1.5-flash") {
        return Some(1_000_000);
    }
    if m.contains("gemini-2.0") || m.contains("gemini-2") {
        // User mentioned 3.0? Assuming 2.0 Flash/Pro imminent or user meant 2.0
        return Some(2_000_000);
    }

    // Meta / Open Source (Ollama often uses these names)
    if m.contains("llama-3.1") || m.contains("llama3.1") {
        return Some(128_000);
    }
    if m.contains("llama-3.2") || m.contains("llama3.2") {
        return Some(128_000);
    }
    if m.contains("llama-3") || m.contains("llama3") {
        return Some(8_192);
    }
    if m.contains("mistral-large") {
        return Some(32_000);
    }
    if m.contains("mistral-nemo") {
        return Some(128_000);
    }
    if m.contains("qwen2.5") || m.contains("qwen-2.5") {
        return Some(32_768); // Often 32k default, though supports more
    }

    // Fallback for generic "gpt-4" strings if not caught above
    if m.contains("gpt-4") {
        return Some(8_192);
    }

    None
}
