use crate::llm::anthropic::AnthropicProvider;
use crate::llm::ollama::OllamaProvider;
use crate::llm::openai::OpenAiProvider;
use crate::llm::provider::LlmProvider;
use crate::llm::provider::Message;
use crate::mcp::config::Settings;
use anyhow::Result;

pub struct ContextAssembler;

impl ContextAssembler {
    pub async fn assemble(
        query: &str,
        raw_memories: &[String],
        _conversation_history: &[Message],
        context_model_id: &str,
        settings: &Settings,
    ) -> Result<String> {
        if raw_memories.is_empty() {
            return Ok(String::new());
        }

        // 1. Identify Provider for the Strategy Model
        let parts: Vec<&str> = context_model_id.split("::").collect();
        let (provider_id, model_name) = if parts.len() == 2 {
            (parts[0], parts[1])
        } else {
            // Fallback or error
            return Ok(raw_memories.join("\n"));
        };

        // 2. Instantiate local provider (stateless for this call)
        // Note: We need the API keys/URL from settings.
        // This is a bit duplicative of McpManager logic but simpler for this isolated task.
        // 2. Instantiate local provider (stateless for this call)
        let provider: Box<dyn LlmProvider + Send + Sync> =
            if let Some(config) = settings.providers.get(provider_id) {
                match config.provider_type {
                    crate::mcp::config::ProviderType::OpenAI
                    | crate::mcp::config::ProviderType::OpenAICompatible => {
                        let key = config.api_key.clone().unwrap_or_default();
                        let base_url = config.base_url.clone();
                        Box::new(OpenAiProvider::new(key, base_url, model_name.to_string()))
                    }
                    crate::mcp::config::ProviderType::Anthropic => {
                        let key = config.api_key.clone().unwrap_or_default();
                        Box::new(AnthropicProvider::new(key, model_name.to_string()))
                    }
                    crate::mcp::config::ProviderType::Ollama => {
                        let base_url = config
                            .base_url
                            .clone()
                            .unwrap_or("http://localhost:11434".to_string());
                        Box::new(OllamaProvider::new(base_url, model_name.to_string()))
                    }
                }
            } else {
                // If provider config not found, fallback to raw
                return Ok(raw_memories.join("\n"));
            };

        // 3. Construct Prompt
        // Minimizing context: Just the user query and the memories. History is expensive.
        // If query is vague, maybe last 2 messages.
        let memory_block = raw_memories.join("\n---\n");
        let prompt = format!(
            "You are a helpful assistant acting as a Memory Manager.
Your goal is to prepare a concise context block for the main LLM.

USER QUERY: {}

RETRIEVED MEMORIES (Fragments):
---
{}
---

INSTRUCTIONS:
1. Analyze the memories in relation to the query.
2. Filter out duplicates or irrelevant noise.
3. Summarize the relevant facts into a single coherent block of text.
4. If nothing is relevant, just say 'No relevant context.'
5. Do NOT output conversational filler. Just the facts.
",
            query, memory_block
        );

        let msgs = vec![Message {
            id: None,
            role: "user".to_string(),
            content: Some(prompt),
            tool_calls: None,
            attachments: None,
            tool_call_id: None,
        }];

        // 4. Call Model
        match provider.chat(msgs, vec![], None).await {
            Ok(response) => {
                let content = response.content.unwrap_or_default();
                if content.contains("No relevant context") {
                    Ok(String::new())
                } else {
                    Ok(content)
                }
            }
            Err(e) => {
                eprintln!("Context Assembly Failed: {}", e);
                Ok(raw_memories.join("\n")) // Fallback to raw
            }
        }
    }
}
