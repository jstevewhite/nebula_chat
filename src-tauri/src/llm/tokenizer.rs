use crate::llm::provider::Message;
use anyhow::Result;
use tiktoken_rs::cl100k_base;

pub struct Tokenizer;

impl Tokenizer {
    pub fn count_tokens(text: &str) -> Result<usize> {
        let bpe = cl100k_base()?;
        let tokens = bpe.encode_with_special_tokens(text);
        Ok(tokens.len())
    }

    pub fn truncate(text: &str, max_tokens: usize) -> Result<String> {
        let bpe = cl100k_base()?;
        let tokens = bpe.encode_with_special_tokens(text);
        if tokens.len() <= max_tokens {
            return Ok(text.to_string());
        }

        let truncated_tokens = &tokens[..max_tokens];
        let decoded = bpe.decode(truncated_tokens.to_vec())?;
        Ok(decoded)
    }

    pub fn count_message_tokens(msg: &Message) -> Result<usize> {
        let mut total = 0;

        // 1. Content
        if let Some(content) = &msg.content {
            total += Self::count_tokens(content)?;
        }

        // 2. Tool Calls (Assistant role)
        if let Some(tool_calls) = &msg.tool_calls {
            for tool in tool_calls {
                // Approximate by counting the JSON string representation
                let json = serde_json::to_string(tool).unwrap_or_default();
                total += Self::count_tokens(&json)?;
            }
        }

        // 3. Tool Call ID (Tool role)
        if let Some(id) = &msg.tool_call_id {
            total += Self::count_tokens(id)?;
        }

        // 4. Attachments (User role)
        if let Some(attachments) = &msg.attachments {
            for att in attachments {
                if !att.is_binary {
                    // Count text content
                    total += Self::count_tokens(&att.data)?;
                } else {
                    // TODO: Logic for image tokens if needed.
                    // For now, treat binary as 0 or small constant metadata?
                    // Let's assume 0 for text-only context limits.
                }
            }
        }

        Ok(total)
    }
}
