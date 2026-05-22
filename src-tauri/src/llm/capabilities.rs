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
            // The Anthropic provider's stream() now accumulates `tool_use`
            // blocks across `content_block_start` / `content_block_delta` /
            // `content_block_stop` events, so streaming with tools is safe.
            supports_streaming_tools: true,
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

// Reasoning capability detection based on model name patterns
pub fn supports_reasoning_effort(model: &str) -> bool {
    let m = model.to_lowercase();
    // OpenAI o1/o3 series models support reasoning_effort parameter
    m.contains("o1-") || m.contains("o3-") || m.starts_with("o1") || m.starts_with("o3")
}

pub fn supports_thinking_mode(model: &str) -> bool {
    let m = model.to_lowercase();
    // DeepSeek R1 and reasoner models support thinking mode
    m.contains("deepseek-r1") 
        || m.contains("deepseek-reasoner") 
        || m.contains("deepseek-v3.1") 
        || m.contains("deepseek") && (m.contains("thinking") || m.contains("reasoner"))
}

pub fn supports_extended_thinking(model: &str) -> bool {
    let m = model.to_lowercase();
    // Anthropic Claude 4 models support extended thinking
    m.contains("claude-4")
        || m.contains("claude-opus-4")
        || m.contains("claude-sonnet-4")
        || m.contains("claude-4.5")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_openai_supports_full_feature_set() {
        let c = get_capabilities(&ProviderType::OpenAI, "gpt-4o");
        assert!(c.supports_tools);
        assert!(c.supports_streaming);
        assert!(c.supports_streaming_tools);
        assert!(c.supports_multimodal);
    }

    #[test]
    fn capabilities_anthropic_enables_streaming_tools() {
        let c = get_capabilities(&ProviderType::Anthropic, "claude-3-5-sonnet");
        assert!(c.supports_tools);
        assert!(c.supports_streaming);
        assert!(
            c.supports_streaming_tools,
            "Anthropic streaming+tools is supported by the SSE parser"
        );
        assert!(c.supports_multimodal);
    }

    #[test]
    fn capabilities_ollama_disables_tools_and_multimodal() {
        let c = get_capabilities(&ProviderType::Ollama, "llama3");
        assert!(!c.supports_tools);
        assert!(c.supports_streaming);
        assert!(!c.supports_streaming_tools);
        assert!(!c.supports_multimodal);
    }

    #[test]
    fn capabilities_openai_compatible_matches_openai() {
        let c = get_capabilities(&ProviderType::OpenAICompatible, "openrouter/some-model");
        assert!(c.supports_tools);
        assert!(c.supports_streaming);
        assert!(c.supports_streaming_tools);
        assert!(c.supports_multimodal);
    }

    #[test]
    fn context_window_openai_models() {
        assert_eq!(get_model_context_window("gpt-4o-2024-05-13"), Some(128_000));
        assert_eq!(get_model_context_window("gpt-4-turbo"), Some(128_000));
        assert_eq!(get_model_context_window("gpt-4-0125-preview"), Some(128_000));
        assert_eq!(get_model_context_window("gpt-4-1106-preview"), Some(128_000));
        assert_eq!(get_model_context_window("gpt-3.5-turbo-0125"), Some(16_385));
        // Plain gpt-4 (after turbo branches) falls through to 8192
        assert_eq!(get_model_context_window("gpt-4"), Some(8_192));
    }

    #[test]
    fn context_window_o_series_models() {
        assert_eq!(get_model_context_window("o1-preview"), Some(128_000));
        assert_eq!(get_model_context_window("o1-mini"), Some(128_000));
        // Generic o1 (no preview/mini suffix) falls into the 200k branch
        assert_eq!(get_model_context_window("o1"), Some(200_000));
    }

    #[test]
    fn context_window_anthropic_models() {
        assert_eq!(get_model_context_window("claude-3-5-sonnet-20240620"), Some(200_000));
        assert_eq!(get_model_context_window("claude-3.5-sonnet"), Some(200_000));
        assert_eq!(get_model_context_window("claude-3-haiku"), Some(200_000));
        assert_eq!(get_model_context_window("claude-4.5-sonnet"), Some(200_000));
    }

    #[test]
    fn context_window_gemini_models() {
        assert_eq!(get_model_context_window("gemini-1.5-pro-latest"), Some(2_000_000));
        assert_eq!(get_model_context_window("gemini-1.5-flash"), Some(1_000_000));
        assert_eq!(get_model_context_window("gemini-2.0-flash"), Some(2_000_000));
    }

    #[test]
    fn context_window_llama_models() {
        assert_eq!(get_model_context_window("llama-3.1-70b"), Some(128_000));
        assert_eq!(get_model_context_window("llama3.2"), Some(128_000));
        assert_eq!(get_model_context_window("llama3-8b"), Some(8_192));
    }

    #[test]
    fn context_window_is_case_insensitive() {
        assert_eq!(get_model_context_window("GPT-4O"), Some(128_000));
        assert_eq!(get_model_context_window("Claude-3-Opus"), Some(200_000));
    }

    #[test]
    fn context_window_unknown_models_return_none() {
        assert_eq!(get_model_context_window("totally-unknown-model"), None);
        assert_eq!(get_model_context_window(""), None);
    }

    #[test]
    fn reasoning_effort_only_for_o_series() {
        assert!(supports_reasoning_effort("o1-preview"));
        assert!(supports_reasoning_effort("o1-mini"));
        assert!(supports_reasoning_effort("o3-mini"));
        assert!(supports_reasoning_effort("o1"));
        assert!(supports_reasoning_effort("O3-Mini"));
        assert!(!supports_reasoning_effort("gpt-4o"));
        assert!(!supports_reasoning_effort("claude-3-5-sonnet"));
    }

    #[test]
    fn thinking_mode_for_deepseek_reasoners() {
        assert!(supports_thinking_mode("deepseek-r1"));
        assert!(supports_thinking_mode("DeepSeek-Reasoner"));
        assert!(supports_thinking_mode("deepseek-v3.1"));
        assert!(supports_thinking_mode("deepseek-thinking"));
        assert!(!supports_thinking_mode("deepseek-v3"));
        assert!(!supports_thinking_mode("gpt-4o"));
    }

    #[test]
    fn extended_thinking_for_claude_4_family() {
        assert!(supports_extended_thinking("claude-4"));
        assert!(supports_extended_thinking("claude-opus-4"));
        assert!(supports_extended_thinking("claude-sonnet-4-20251022"));
        assert!(supports_extended_thinking("claude-4.5-sonnet"));
        assert!(!supports_extended_thinking("claude-3-5-sonnet"));
        assert!(!supports_extended_thinking("gpt-4"));
    }
}
