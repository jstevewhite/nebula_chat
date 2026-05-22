//! Streaming-safe splitter for inline `<think>...</think>` blocks.
//!
//! Some OpenAI-compatible providers (MiniMax, certain Ollama models, etc.)
//! emit chain-of-thought inside the regular `content` stream wrapped in
//! `<think>` tags rather than in a dedicated `reasoning_content` field.
//! This splitter consumes streamed text chunks and yields the portions
//! that belong on the text channel vs. the reasoning channel, correctly
//! handling tags that straddle chunk boundaries.

const OPEN_TAG: &str = "<think>";
const CLOSE_TAG: &str = "</think>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Text,
    Reasoning,
}

pub struct ThinkTagSplitter {
    mode: Mode,
    buf: String,
}

impl ThinkTagSplitter {
    pub fn new() -> Self {
        Self {
            mode: Mode::Text,
            buf: String::new(),
        }
    }

    /// Feed a new chunk of streamed content. Returns `(text, reasoning)` —
    /// the substrings to route to each channel. Either may be empty.
    pub fn push(&mut self, chunk: &str) -> (String, String) {
        self.buf.push_str(chunk);
        let mut text_out = String::new();
        let mut reasoning_out = String::new();

        loop {
            let expected = match self.mode {
                Mode::Text => OPEN_TAG,
                Mode::Reasoning => CLOSE_TAG,
            };

            if let Some(pos) = self.buf.find(expected) {
                let before: String = self.buf.drain(..pos).collect();
                match self.mode {
                    Mode::Text => text_out.push_str(&before),
                    Mode::Reasoning => reasoning_out.push_str(&before),
                }
                self.buf.drain(..expected.len());
                self.mode = match self.mode {
                    Mode::Text => Mode::Reasoning,
                    Mode::Reasoning => Mode::Text,
                };
                continue;
            }

            // No complete tag — retain the longest trailing suffix of buf
            // that is a prefix of the expected tag, in case the rest of
            // the tag arrives in the next chunk.
            let max_keep = expected.len().saturating_sub(1).min(self.buf.len());
            let mut keep = 0;
            for k in (1..=max_keep).rev() {
                let start = self.buf.len() - k;
                if !self.buf.is_char_boundary(start) {
                    continue;
                }
                if expected.starts_with(&self.buf[start..]) {
                    keep = k;
                    break;
                }
            }
            let emit_len = self.buf.len() - keep;
            let to_emit: String = self.buf.drain(..emit_len).collect();
            match self.mode {
                Mode::Text => text_out.push_str(&to_emit),
                Mode::Reasoning => reasoning_out.push_str(&to_emit),
            }
            break;
        }

        (text_out, reasoning_out)
    }

    /// Flush any buffered content. Call once at end of stream.
    pub fn flush(&mut self) -> (String, String) {
        let remaining = std::mem::take(&mut self.buf);
        match self.mode {
            Mode::Text => (remaining, String::new()),
            Mode::Reasoning => (String::new(), remaining),
        }
    }
}

impl Default for ThinkTagSplitter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunks: &[&str]) -> (String, String) {
        let mut splitter = ThinkTagSplitter::new();
        let mut text = String::new();
        let mut reasoning = String::new();
        for c in chunks {
            let (t, r) = splitter.push(c);
            text.push_str(&t);
            reasoning.push_str(&r);
        }
        let (t, r) = splitter.flush();
        text.push_str(&t);
        reasoning.push_str(&r);
        (text, reasoning)
    }

    #[test]
    fn no_tags_passes_through_as_text() {
        let (t, r) = run(&["Hello, world!"]);
        assert_eq!(t, "Hello, world!");
        assert_eq!(r, "");
    }

    #[test]
    fn single_complete_block() {
        let (t, r) = run(&["<think>reasoning here</think>answer"]);
        assert_eq!(t, "answer");
        assert_eq!(r, "reasoning here");
    }

    #[test]
    fn open_tag_split_across_chunks() {
        let (t, r) = run(&["<thi", "nk>hidden</think>visible"]);
        assert_eq!(t, "visible");
        assert_eq!(r, "hidden");
    }

    #[test]
    fn close_tag_split_across_chunks() {
        let (t, r) = run(&["<think>hidden</thi", "nk>visible"]);
        assert_eq!(t, "visible");
        assert_eq!(r, "hidden");
    }

    #[test]
    fn tag_in_middle_of_chunks() {
        let (t, r) = run(&["prefix <think>secret", " stuff</think>", " suffix"]);
        assert_eq!(t, "prefix  suffix");
        assert_eq!(r, "secret stuff");
    }

    #[test]
    fn multiple_blocks() {
        let (t, r) = run(&["a<think>x</think>b<think>y</think>c"]);
        assert_eq!(t, "abc");
        assert_eq!(r, "xy");
    }

    #[test]
    fn unterminated_think_flushes_to_reasoning() {
        let (t, r) = run(&["<think>never closed"]);
        assert_eq!(t, "");
        assert_eq!(r, "never closed");
    }

    #[test]
    fn partial_open_tag_at_end_flushes_as_text() {
        let (t, r) = run(&["hello <thi"]);
        assert_eq!(t, "hello <thi");
        assert_eq!(r, "");
    }

    #[test]
    fn bare_lt_is_text() {
        let (t, r) = run(&["a < b"]);
        assert_eq!(t, "a < b");
        assert_eq!(r, "");
    }

    #[test]
    fn lt_inside_reasoning_is_kept() {
        let (t, r) = run(&["<think>cost < 5</think>done"]);
        assert_eq!(t, "done");
        assert_eq!(r, "cost < 5");
    }

    #[test]
    fn unicode_content() {
        let (t, r) = run(&["héllo <think>résumé</think>世界"]);
        assert_eq!(t, "héllo 世界");
        assert_eq!(r, "résumé");
    }

    #[test]
    fn streaming_char_by_char() {
        let input = "abc<think>xyz</think>def";
        let mut splitter = ThinkTagSplitter::new();
        let mut text = String::new();
        let mut reasoning = String::new();
        for ch in input.chars() {
            let mut buf = [0u8; 4];
            let chunk = ch.encode_utf8(&mut buf);
            let (t, r) = splitter.push(chunk);
            text.push_str(&t);
            reasoning.push_str(&r);
        }
        let (t, r) = splitter.flush();
        text.push_str(&t);
        reasoning.push_str(&r);
        assert_eq!(text, "abcdef");
        assert_eq!(reasoning, "xyz");
    }

    #[test]
    fn no_tag_in_stream_emits_text_immediately() {
        // A long run with no `<` should not get buffered.
        let mut splitter = ThinkTagSplitter::new();
        let (t, r) = splitter.push("plain text with no tags at all");
        assert_eq!(t, "plain text with no tags at all");
        assert_eq!(r, "");
    }
}
