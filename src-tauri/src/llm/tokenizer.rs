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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::provider::Attachment;

    fn msg(role: &str, content: Option<&str>) -> Message {
        Message {
            id: None,
            role: role.to_string(),
            content: content.map(|s| s.to_string()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            attachments: None,
            created_at: None,
        }
    }

    #[test]
    fn count_tokens_empty_is_zero() {
        assert_eq!(Tokenizer::count_tokens("").unwrap(), 0);
    }

    #[test]
    fn count_tokens_monotonic_with_length() {
        let short = Tokenizer::count_tokens("hello").unwrap();
        let long = Tokenizer::count_tokens("hello hello hello hello hello").unwrap();
        assert!(short > 0);
        assert!(long > short);
    }

    #[test]
    fn truncate_short_text_is_unchanged() {
        let text = "hello world";
        let n = Tokenizer::count_tokens(text).unwrap();
        let out = Tokenizer::truncate(text, n + 10).unwrap();
        assert_eq!(out, text);
    }

    #[test]
    fn truncate_long_text_respects_budget() {
        let text = "one two three four five six seven eight nine ten ".repeat(20);
        let out = Tokenizer::truncate(&text, 5).unwrap();
        let out_tokens = Tokenizer::count_tokens(&out).unwrap();
        assert!(out_tokens <= 5, "truncated text had {} tokens, expected <= 5", out_tokens);
        assert!(out.len() < text.len(), "truncated text should be shorter");
    }

    #[test]
    fn truncate_to_zero_returns_empty() {
        let out = Tokenizer::truncate("hello world", 0).unwrap();
        assert_eq!(Tokenizer::count_tokens(&out).unwrap(), 0);
    }

    #[test]
    fn count_message_tokens_content_only() {
        let m = msg("user", Some("hello"));
        let direct = Tokenizer::count_tokens("hello").unwrap();
        assert_eq!(Tokenizer::count_message_tokens(&m).unwrap(), direct);
    }

    #[test]
    fn count_message_tokens_no_content_is_zero() {
        let m = msg("assistant", None);
        assert_eq!(Tokenizer::count_message_tokens(&m).unwrap(), 0);
    }

    #[test]
    fn count_message_tokens_includes_tool_calls() {
        let mut m = msg("assistant", Some("calling tool"));
        m.tool_calls = Some(vec![serde_json::json!({
            "id": "call_1",
            "type": "function",
            "function": {"name": "lookup", "arguments": "{\"q\":\"x\"}"}
        })]);
        let with_calls = Tokenizer::count_message_tokens(&m).unwrap();
        let content_only = Tokenizer::count_tokens("calling tool").unwrap();
        assert!(with_calls > content_only, "tool_calls should add to token count");
    }

    #[test]
    fn count_message_tokens_includes_tool_call_id() {
        let mut m = msg("tool", Some("result"));
        m.tool_call_id = Some("call_abc123".to_string());
        let with_id = Tokenizer::count_message_tokens(&m).unwrap();
        let content_only = Tokenizer::count_tokens("result").unwrap();
        assert!(with_id > content_only);
    }

    #[test]
    fn count_message_tokens_text_attachments_count_but_binary_does_not() {
        let mut text_msg = msg("user", Some("see file"));
        text_msg.attachments = Some(vec![Attachment {
            name: "doc.txt".to_string(),
            media_type: "text/plain".to_string(),
            data: "the quick brown fox jumps over the lazy dog".to_string(),
            is_binary: false,
        }]);

        let mut bin_msg = msg("user", Some("see file"));
        bin_msg.attachments = Some(vec![Attachment {
            name: "pic.png".to_string(),
            media_type: "image/png".to_string(),
            data: "base64payloadbase64payloadbase64payload".to_string(),
            is_binary: true,
        }]);

        let text_total = Tokenizer::count_message_tokens(&text_msg).unwrap();
        let bin_total = Tokenizer::count_message_tokens(&bin_msg).unwrap();
        let content_only = Tokenizer::count_tokens("see file").unwrap();

        assert!(text_total > content_only, "text attachment should add tokens");
        assert_eq!(bin_total, content_only, "binary attachment currently contributes 0");
    }
}
