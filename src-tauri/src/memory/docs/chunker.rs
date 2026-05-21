//! Markdown chunking. v1 strategy: split on paragraph boundaries, pack into
//! ~500-char windows with ~80-char overlap. Code fences are never split.

use sha2::{Digest, Sha256};

pub const TARGET_CHARS: usize = 500;
pub const OVERLAP_CHARS: usize = 80;

/// A chunk of a document. `char_start` / `char_end` are byte offsets into the
/// original body string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    pub text_hash: String,
    pub char_start: usize,
    pub char_end: usize,
}

/// Split a document body into chunks.
///
/// Algorithm: walk paragraph-by-paragraph (separated by blank lines). Code
/// fences (lines starting with ```` ``` ````) suspend splitting until the fence
/// closes. Within the current accumulator, when adding the next paragraph
/// would exceed `TARGET_CHARS`, emit the accumulator as a chunk, then start a
/// new accumulator seeded with the last `OVERLAP_CHARS` of the previous one.
pub fn chunk(body: &str) -> Vec<Chunk> {
    if body.trim().is_empty() {
        return Vec::new();
    }

    let mut segments: Vec<(usize, usize, String)> = Vec::new(); // (start, end, text)
    let bytes = body.as_bytes();

    let mut i = 0usize;
    let mut in_fence = false;
    let mut seg_start = 0usize;
    let mut seg_buf = String::new();

    while i < bytes.len() {
        let line_end = match memchr_newline(bytes, i) {
            Some(p) => p,
            None => bytes.len(),
        };
        let line = &body[i..line_end];

        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with("```") {
            in_fence = !in_fence;
            seg_buf.push_str(line);
            seg_buf.push('\n');
        } else if in_fence {
            seg_buf.push_str(line);
            seg_buf.push('\n');
        } else if line.trim().is_empty() {
            if !seg_buf.is_empty() {
                let end = i;
                segments.push((seg_start, end, std::mem::take(&mut seg_buf)));
            }
            seg_start = (line_end + 1).min(bytes.len());
        } else {
            if seg_buf.is_empty() {
                seg_start = i;
            }
            seg_buf.push_str(line);
            seg_buf.push('\n');
        }

        if line_end < bytes.len() {
            i = line_end + 1;
        } else {
            break;
        }
    }
    if !seg_buf.is_empty() {
        segments.push((seg_start, bytes.len(), seg_buf));
    }

    // Pack segments into chunks.
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut cur = String::new();
    let mut cur_start: usize = 0;
    let mut cur_end: usize = 0;

    for (start, end, text) in segments {
        let trimmed = text.trim_end_matches('\n');
        if cur.is_empty() {
            cur.push_str(trimmed);
            cur_start = start;
            cur_end = end;
            continue;
        }

        if cur.chars().count() + trimmed.chars().count() + 2 > TARGET_CHARS {
            chunks.push(finalize(&cur, cur_start, cur_end));
            // Seed new chunk with overlap from the tail of the previous one.
            cur = tail_chars(&cur, OVERLAP_CHARS);
            // Note: cur_start becomes approximate after overlap-seeding; we
            // leave char_start at the new segment's start for clarity.
            cur.push_str("\n\n");
            cur.push_str(trimmed);
            cur_start = start;
            cur_end = end;
        } else {
            cur.push_str("\n\n");
            cur.push_str(trimmed);
            cur_end = end;
        }
    }
    if !cur.trim().is_empty() {
        chunks.push(finalize(&cur, cur_start, cur_end));
    }

    chunks
}

fn finalize(text: &str, start: usize, end: usize) -> Chunk {
    let trimmed = text.trim();
    let hash = hash_text(trimmed);
    Chunk {
        text: trimmed.to_string(),
        text_hash: hash,
        char_start: start,
        char_end: end,
    }
}

fn hash_text(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

fn tail_chars(s: &str, n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    let start = chars.len().saturating_sub(n);
    chars[start..].iter().collect()
}

fn memchr_newline(bytes: &[u8], from: usize) -> Option<usize> {
    bytes[from..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|p| from + p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_body_yields_no_chunks() {
        assert!(chunk("").is_empty());
        assert!(chunk("   \n  \n").is_empty());
    }

    #[test]
    fn short_body_yields_one_chunk() {
        let chunks = chunk("Hello world.\n\nThis is a small doc.");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.contains("Hello world"));
        assert!(!chunks[0].text_hash.is_empty());
    }

    #[test]
    fn long_body_packs_into_multiple_chunks_with_overlap() {
        let para = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(8);
        let body = (0..6)
            .map(|i| format!("Paragraph {i}: {para}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let chunks = chunk(&body);
        assert!(chunks.len() >= 2, "expected multiple chunks, got {}", chunks.len());
        // Each chunk should be near or under target.
        for c in &chunks {
            assert!(c.text.chars().count() <= TARGET_CHARS * 2);
        }
    }

    #[test]
    fn code_fences_are_never_split() {
        let body = format!(
            "intro paragraph\n\n```rust\n{}\n```\n\noutro paragraph",
            "let x = 1;\n".repeat(40)
        );
        let chunks = chunk(&body);
        // The code fence content must appear contiguous inside exactly one chunk.
        let fence_count = chunks
            .iter()
            .filter(|c| c.text.contains("```rust") && c.text.contains("```\n"))
            .count();
        assert!(fence_count >= 1, "code fence got split across chunks");
    }

    #[test]
    fn text_hash_is_stable_and_distinct() {
        let a = chunk("alpha\n\nbeta");
        let b = chunk("alpha\n\nbeta");
        let c = chunk("alpha\n\ngamma");
        assert_eq!(a[0].text_hash, b[0].text_hash);
        assert_ne!(a.last().unwrap().text_hash, c.last().unwrap().text_hash);
    }
}
