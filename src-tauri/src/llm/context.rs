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
                let count = Tokenizer::count_message_tokens(&system_msg)?;
                if count < budget {
                    pruned.push(system_msg);
                    budget = budget.saturating_sub(count);
                }
            }
        }

        // 2. Squash Old Tool Outputs
        // Heuristic: If msg.role == "tool" and it is NOT in the last 3 messages, truncate content.
        let len = msgs_to_process.len();
        for (i, msg) in msgs_to_process.iter_mut().enumerate() {
            let is_recent = i >= len.saturating_sub(3);
            if msg.role == "tool" && !is_recent {
                if let Some(content) = &msg.content {
                    if let Ok(truncated) = Tokenizer::truncate(content, 100) {
                        msg.content = Some(format!("{}... (truncated by context)", truncated));
                    }
                }
            }
        }

        // 3. Process remaining in reverse (newest first)
        let mut history = Vec::new();
        for msg in msgs_to_process.into_iter().rev() {
            let count = Tokenizer::count_message_tokens(&msg)?;

            if count <= budget {
                history.push(msg);
                budget -= count;
            } else {
                // Budget exceeded
                break;
            }
        }

        // 4. Reassemble
        history.reverse();
        pruned.extend(history);

        Ok(pruned)
    }
}
