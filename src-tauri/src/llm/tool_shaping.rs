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
