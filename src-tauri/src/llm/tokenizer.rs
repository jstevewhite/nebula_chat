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
}
