use serde_json::Value;

pub const MAX_PREVIEW_CHARS: usize = 50000;

pub fn shape_tool_output(output: &Value) -> (String, String) {
    let full_json = serde_json::to_string(output).unwrap_or_default();

    // If it's a string, we can truncate it nicely.
    // If it's an object/array, we might want to keep structure but truncate content.
    // For simplicity in Phase 3, we'll just treat it as a string.

    let preview = if full_json.len() > MAX_PREVIEW_CHARS {
        let mut truncated = full_json
            .chars()
            .take(MAX_PREVIEW_CHARS)
            .collect::<String>();
        truncated.push_str("\n... (truncated)");
        truncated
    } else {
        full_json.clone()
    };

    (preview, full_json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn short_output_preview_equals_full() {
        let v = json!({"ok": true, "count": 3});
        let (preview, full) = shape_tool_output(&v);
        assert_eq!(preview, full);
        assert!(!preview.contains("(truncated)"));
    }

    #[test]
    fn string_value_is_serialized_with_quotes() {
        let v = json!("hello");
        let (preview, full) = shape_tool_output(&v);
        assert_eq!(full, "\"hello\"");
        assert_eq!(preview, full);
    }

    #[test]
    fn null_value_serializes_to_null() {
        let (preview, full) = shape_tool_output(&serde_json::Value::Null);
        assert_eq!(full, "null");
        assert_eq!(preview, "null");
    }

    #[test]
    fn oversized_output_is_truncated_with_marker() {
        let big = "a".repeat(MAX_PREVIEW_CHARS + 100);
        let v = json!(big);
        let (preview, full) = shape_tool_output(&v);
        assert!(full.len() > MAX_PREVIEW_CHARS);
        assert!(preview.ends_with("(truncated)"));
        // Truncation works in chars; the visible body is at most MAX_PREVIEW_CHARS chars,
        // then the marker is appended.
        let marker = "\n... (truncated)";
        let body_chars = preview.chars().count() - marker.chars().count();
        assert_eq!(body_chars, MAX_PREVIEW_CHARS);
    }

    #[test]
    fn truncation_handles_multibyte_utf8_safely() {
        // Each ☃ snowman is 1 char / 3 bytes. With MAX_PREVIEW_CHARS+10 snowmen we
        // exceed the char budget; truncation must still produce valid UTF-8.
        let snowmen: String = "☃".repeat(MAX_PREVIEW_CHARS + 10);
        let v = json!(snowmen);
        let (preview, _full) = shape_tool_output(&v);
        assert!(preview.is_char_boundary(preview.len()));
        assert!(preview.ends_with("(truncated)"));
    }

    #[test]
    fn full_payload_is_unmodified_for_large_inputs() {
        let big = "x".repeat(MAX_PREVIEW_CHARS + 50);
        let v = json!(big);
        let (preview, full) = shape_tool_output(&v);
        // Full should never carry the truncation marker; only preview does.
        assert!(!full.contains("(truncated)"));
        assert_ne!(preview, full);
    }

    #[test]
    fn exactly_at_limit_is_not_truncated() {
        // serde_json::to_string adds two quotes around a JSON string. To make the
        // serialized form land exactly on MAX_PREVIEW_CHARS, body should be
        // MAX_PREVIEW_CHARS - 2 ASCII chars.
        let body = "a".repeat(MAX_PREVIEW_CHARS - 2);
        let v = json!(body);
        let (preview, full) = shape_tool_output(&v);
        assert_eq!(full.chars().count(), MAX_PREVIEW_CHARS);
        assert_eq!(preview, full);
        assert!(!preview.contains("(truncated)"));
    }
}
