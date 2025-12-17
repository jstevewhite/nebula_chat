use crate::llm::anthropic::AnthropicProvider;
use crate::llm::ollama::OllamaProvider;
use crate::llm::openai::OpenAiProvider;
use crate::llm::provider::LlmProvider;
use crate::llm::provider::Message;
use crate::mcp::config::Settings;
use crate::memory::MemoryHit;
use anyhow::Result;

pub struct ContextAssembler;

impl ContextAssembler {
    pub async fn assemble(
        query: &str,
        memory_hits: &[MemoryHit],
        conversation_history: &[Message],
        context_turns: usize,
        context_model_id: &str,
        settings: &Settings,
    ) -> Result<String> {
        if memory_hits.is_empty() {
            return Ok(String::new());
        }

        // 1. Identify Provider for the Strategy Model
        let parts: Vec<&str> = context_model_id.split("::").collect();
        let (provider_id, model_name) = if parts.len() == 2 {
            (parts[0], parts[1])
        } else {
            // Fallback or error
            return Ok(memory_hits
                .iter()
                .map(|h| h.snippet.clone())
                .collect::<Vec<_>>()
                .join("\n"));
        };

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
                // If provider config not found, fallback to raw snippets
                return Ok(memory_hits
                    .iter()
                    .map(|h| h.snippet.clone())
                    .collect::<Vec<_>>()
                    .join("\n"));
            };

        // 3. Construct Prompt
        let memory_block = memory_hits
            .iter()
            .map(|hit| {
                format!(
                    "[{}] {} (score: {:.2})\n{}",
                    hit.role, hit.created_at, hit.score, hit.snippet
                )
            })
            .collect::<Vec<_>>()
            .join("\n---\n");

        let recent_context = if context_turns == 0 {
            String::new()
        } else {
            let max_msgs = context_turns.saturating_mul(2);
            let mut recent: Vec<String> = Vec::new();

            for m in conversation_history.iter().rev() {
                if recent.len() >= max_msgs {
                    break;
                }
                if m.role != "user" && m.role != "assistant" {
                    continue;
                }
                let content = m.content.clone().unwrap_or_default();
                if content.trim().is_empty() {
                    continue;
                }
                recent.push(format!("{}: {}", m.role, content.trim()));
            }
            recent.reverse();

            if recent.is_empty() {
                String::new()
            } else {
                format!(
                    "\nRECENT CONVERSATION (last {} turns max):\n---\n{}\n---\n",
                    context_turns,
                    recent.join("\n")
                )
            }
        };

        let system_prompt = "You are a helpful assistant acting as a Memory Manager.
Your goal is to prepare a concise context block for the main LLM.

INSTRUCTIONS:
1. Use the user query AND the recent conversation to determine relevance.
2. Analyze the memories in relation to the query.
3. Filter out duplicates or irrelevant noise.
4. Summarize the relevant facts into a single coherent block of text.
5. If nothing is relevant, just say 'No relevant context.'
6. Do NOT output conversational filler. Just the facts."
            .to_string();

        let user_prompt = format!(
            "USER QUERY: {}{}
            
RETRIEVED MEMORIES (Fragments):
---
{}
---",
            query, recent_context, memory_block
        );

        let mut msgs = Vec::new();

        if !system_prompt.is_empty() {
            msgs.push(Message {
                id: None,
                role: "system".to_string(),
                content: Some(system_prompt),
                reasoning_content: None,
                tool_calls: None,
                tool_call_id: None,
                attachments: None,
            });
        }

        msgs.push(Message {
            id: None,
            role: "user".to_string(),
            content: Some(user_prompt),
            reasoning_content: None,
            tool_calls: None,
            attachments: None,
            tool_call_id: None,
        });

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
                // Fallback to raw snippets
                Ok(memory_hits
                    .iter()
                    .map(|h| h.snippet.clone())
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
    }
}
