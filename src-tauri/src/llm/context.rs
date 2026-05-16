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

        // 2. Process remaining in reverse (newest first)
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

        // 3. Reassemble
        history.reverse();
        pruned.extend(history);

        Ok(pruned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::tokenizer::Tokenizer;

    fn msg(role: &str, content: &str) -> Message {
        Message {
            id: None,
            role: role.to_string(),
            content: Some(content.to_string()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            attachments: None,
            created_at: None,
        }
    }

    #[test]
    fn prune_empty_input_returns_empty() {
        let out = ContextManager::prune_messages(vec![], 1000).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn prune_with_huge_budget_keeps_everything() {
        let msgs = vec![
            msg("system", "you are a helpful assistant"),
            msg("user", "hello"),
            msg("assistant", "hi"),
            msg("user", "how are you?"),
        ];
        let out = ContextManager::prune_messages(msgs.clone(), 1_000_000).unwrap();
        assert_eq!(out.len(), msgs.len());
        assert_eq!(out[0].role, "system");
        assert_eq!(out.last().unwrap().content.as_deref(), Some("how are you?"));
    }

    #[test]
    fn prune_preserves_system_message_at_head() {
        let msgs = vec![
            msg("system", "system prompt"),
            msg("user", "u1"),
            msg("assistant", "a1"),
            msg("user", "u2"),
        ];
        let sys_tokens = Tokenizer::count_message_tokens(&msgs[0]).unwrap();
        let last_tokens = Tokenizer::count_message_tokens(&msgs[3]).unwrap();
        // Budget for system + last only.
        let out = ContextManager::prune_messages(msgs, sys_tokens + last_tokens + 1).unwrap();
        assert_eq!(out.first().unwrap().role, "system");
        assert_eq!(out.last().unwrap().content.as_deref(), Some("u2"));
    }

    #[test]
    fn prune_evicts_oldest_first_when_over_budget() {
        let msgs = vec![
            msg("user", "first message"),
            msg("assistant", "second message"),
            msg("user", "third message"),
            msg("assistant", "fourth message"),
        ];
        // Budget for last 2 messages only.
        let last_two_tokens: usize = msgs[2..]
            .iter()
            .map(|m| Tokenizer::count_message_tokens(m).unwrap())
            .sum();
        let out = ContextManager::prune_messages(msgs, last_two_tokens).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].content.as_deref(), Some("third message"));
        assert_eq!(out[1].content.as_deref(), Some("fourth message"));
    }

    #[test]
    fn prune_with_zero_budget_drops_everything() {
        let msgs = vec![msg("system", "sys"), msg("user", "u")];
        let out = ContextManager::prune_messages(msgs, 0).unwrap();
        assert!(out.is_empty(), "with zero budget nothing should fit");
    }

    #[test]
    fn prune_no_system_message_still_works() {
        let msgs = vec![
            msg("user", "u1"),
            msg("assistant", "a1"),
            msg("user", "u2"),
        ];
        let out = ContextManager::prune_messages(msgs, 1_000_000).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].role, "user");
    }

    #[test]
    fn prune_preserves_chronological_order_in_output() {
        let msgs = vec![
            msg("system", "sys"),
            msg("user", "1"),
            msg("assistant", "2"),
            msg("user", "3"),
            msg("assistant", "4"),
        ];
        let out = ContextManager::prune_messages(msgs, 1_000_000).unwrap();
        let order: Vec<&str> = out.iter().filter_map(|m| m.content.as_deref()).collect();
        assert_eq!(order, vec!["sys", "1", "2", "3", "4"]);
    }
}
