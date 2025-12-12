use crate::llm::provider::Message;
use crate::llm::tokenizer::Tokenizer;
use anyhow::Result;

pub struct ContextManager;

impl ContextManager {
    pub fn prune_messages(messages: Vec<Message>, max_tokens: usize) -> Result<Vec<Message>> {
        let mut pruned = Vec::new();
        let mut budget = max_tokens;

        // 1. Always keep system prompt if present (usually first)
        let mut msgs_to_process = messages.clone();
        if let Some(first) = msgs_to_process.first() {
            if first.role == "system" {
                let system_msg = msgs_to_process.remove(0);
                let count = Tokenizer::count_tokens(system_msg.content.as_deref().unwrap_or(""))?;
                if count < budget {
                    pruned.push(system_msg);
                    budget -= count;
                }
            }
        }

        // 2. Process remaining in reverse (newest first)
        let mut history = Vec::new();
        for msg in msgs_to_process.into_iter().rev() {
            let content = msg.content.as_deref().unwrap_or("");
            let count = Tokenizer::count_tokens(content)?;
            
            if count <= budget {
                history.push(msg);
                budget -= count;
            } else {
                // Budget exceeded
                break;
            }
        }

        // 3. Reassemble
        history.reverse();
        pruned.extend(history);

        Ok(pruned)
    }
}
