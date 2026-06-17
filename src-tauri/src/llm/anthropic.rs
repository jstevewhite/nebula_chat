use crate::llm::provider::{
    GenerationOptions, LlmProvider, Message, StreamContent, ToolDefinition,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};

const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";

fn sanitize_anthropic_base_url(input: Option<String>) -> String {
    let mut base = input
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_ANTHROPIC_BASE_URL.to_string());
    while base.ends_with('/') {
        base.pop();
    }
    // Strip trailing /v1 so callers can paste either form; we append `/v1/messages` ourselves.
    if base.ends_with("/v1") {
        base.truncate(base.len() - 3);
    }
    base
}

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, base_url: Option<String>, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: sanitize_anthropic_base_url(base_url),
            model,
        }
    }

    fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.base_url)
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        options: Option<GenerationOptions>,
    ) -> Result<Message> {
        let anthropic_tools: Vec<Value> = tools
            .into_iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema
                })
            })
            .collect();

        // Convert messages to Anthropic format using the helper function
        let (system_prompt, filtered_messages) = convert_messages(messages);

        let mut body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": filtered_messages,
        });

        apply_sampling_options(&mut body, options.as_ref());

        if !system_prompt.is_empty() {
            body.as_object_mut()
                .unwrap()
                .insert("system".to_string(), json!(system_prompt));
        }

        if !anthropic_tools.is_empty() {
            body.as_object_mut()
                .unwrap()
                .insert("tools".to_string(), json!(anthropic_tools));
        }

        // DEBUG LOGGING
        // println!("Anthropic Request: {}", serde_json::to_string_pretty(&body).unwrap());

        let resp = self
            .client
            .post(self.messages_url())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to send request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let error_text = resp.text().await?;
            println!("Anthropic API Error ({}): {}", status, error_text);
            return Err(anyhow::anyhow!(
                "Anthropic API Error ({}): {}",
                status,
                error_text
            ));
        }

        let json: Value = resp.json().await?;
        // println!("Anthropic Response: {:?}", json);

        // Parse response to Message
        let mut final_content = String::new();
        let mut tool_calls = Vec::new();

        if let Some(content_arr) = json.get("content").and_then(|c| c.as_array()) {
            for item in content_arr {
                if item["type"] == "text" {
                    final_content.push_str(item["text"].as_str().unwrap_or(""));
                } else if item["type"] == "tool_use" {
                    // Convert to OpenAI tool call format for internal consistency
                    let args = item["input"].to_string(); // Keep as string for consistency with OpenAI json string
                    tool_calls.push(json!({
                        "id": item["id"],
                        "type": "function",
                        "function": {
                            "name": item["name"],
                            "arguments": args
                        }
                    }));
                }
            }
        }

        let id = json["id"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let role = json["role"].as_str().unwrap_or("assistant");
        let content = if final_content.is_empty() {
            None
        } else {
            Some(final_content)
        };
        let tool_calls = if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        };

        Ok(Message {
            id: Some(id),
            role: role.to_string(),
            content,
            reasoning_content: None, // Anthropic doesn't use this field yet (unless via thinking blocks in content)
            tool_calls,
            tool_call_id: None,
            attachments: None,
            created_at: None,
        })
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        options: Option<GenerationOptions>,
        on_token: Box<dyn Fn(StreamContent) + Send + Sync>,
    ) -> Result<Message> {
        let anthropic_tools: Vec<Value> = tools
            .into_iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema
                })
            })
            .collect();

        // Convert messages to Anthropic format using the helper function
        let (system_prompt, filtered_messages) = convert_messages(messages);

        let mut body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": filtered_messages,
            "stream": true,
        });

        apply_sampling_options(&mut body, options.as_ref());

        if !system_prompt.is_empty() {
            body.as_object_mut()
                .unwrap()
                .insert("system".to_string(), json!(system_prompt));
        }

        if !anthropic_tools.is_empty() {
            body.as_object_mut()
                .unwrap()
                .insert("tools".to_string(), json!(anthropic_tools));
        }

        let resp = self
            .client
            .post(self.messages_url())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .context("Failed to send stream request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let error_text = resp.text().await.unwrap_or_default();
            eprintln!("Anthropic stream error ({}): {}", status, error_text);
            return Err(anyhow::anyhow!(
                "Anthropic stream error ({}): {}",
                status,
                error_text
            ));
        }

        let mut stream = resp.bytes_stream();

        let mut full_content = String::new();
        let mut full_reasoning = String::new();
        let mut sse_buffer = String::new();

        // Track in-progress content blocks. Anthropic streams identify blocks by
        // a sequential `index`; we map index → (block kind, accumulator) so a
        // text and a tool_use block can interleave without colliding.
        #[derive(Debug)]
        enum BlockKind {
            Text,
            Thinking,
            ToolUse {
                id: String,
                name: String,
                args: String,
            },
        }
        let mut blocks: std::collections::HashMap<u64, BlockKind> =
            std::collections::HashMap::new();
        let mut tool_calls_acc: Vec<Value> = Vec::new();

        'outer: while let Some(item) = stream.next().await {
            let chunk = item?;
            let chunk_norm = String::from_utf8_lossy(&chunk).replace("\r\n", "\n");
            sse_buffer.push_str(&chunk_norm);

            // SSE events are separated by a blank line. Drain whole events out
            // of the buffer; anything left over is a partial event and stays
            // for the next chunk.
            loop {
                let Some(idx) = sse_buffer.find("\n\n") else {
                    break;
                };
                let event = sse_buffer[..idx].to_string();
                sse_buffer.drain(..idx + 2);

                for line in event.lines() {
                    // SSE has both `event: <name>` and `data: <json>` lines.
                    // The `data:` payload already includes a `"type"` field
                    // for Anthropic, so we only need to parse data lines.
                    if !line.starts_with("data: ") {
                        continue;
                    }
                    let data = line[6..].trim();
                    if data.is_empty() {
                        continue;
                    }

                    let Ok(json) = serde_json::from_str::<Value>(data) else {
                        tracing::debug!("Anthropic stream: dropped unparseable data line");
                        continue;
                    };

                    let Some(event_type) = json["type"].as_str() else {
                        continue;
                    };

                    match event_type {
                        "content_block_start" => {
                            let index = json["index"].as_u64().unwrap_or(0);
                            let block = &json["content_block"];
                            let kind = block["type"].as_str().unwrap_or("");
                            match kind {
                                "text" => {
                                    blocks.insert(index, BlockKind::Text);
                                }
                                "thinking" => {
                                    blocks.insert(index, BlockKind::Thinking);
                                }
                                "tool_use" => {
                                    let id = block["id"].as_str().unwrap_or("").to_string();
                                    let name = block["name"].as_str().unwrap_or("").to_string();
                                    blocks.insert(
                                        index,
                                        BlockKind::ToolUse {
                                            id,
                                            name,
                                            args: String::new(),
                                        },
                                    );
                                }
                                _ => {}
                            }
                        }
                        "content_block_delta" => {
                            let index = json["index"].as_u64().unwrap_or(0);
                            let delta = &json["delta"];
                            let delta_type = delta["type"].as_str().unwrap_or("");

                            match delta_type {
                                "text_delta" => {
                                    if let Some(text) = delta["text"].as_str() {
                                        on_token(StreamContent::Text(text.to_string()));
                                        full_content.push_str(text);
                                    }
                                }
                                "thinking_delta" => {
                                    if let Some(text) = delta["thinking"].as_str() {
                                        on_token(StreamContent::Reasoning(text.to_string()));
                                        full_reasoning.push_str(text);
                                    }
                                }
                                "input_json_delta" => {
                                    if let Some(partial) = delta["partial_json"].as_str() {
                                        if let Some(BlockKind::ToolUse { args, .. }) =
                                            blocks.get_mut(&index)
                                        {
                                            args.push_str(partial);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        "content_block_stop" => {
                            let index = json["index"].as_u64().unwrap_or(0);
                            if let Some(BlockKind::ToolUse { id, name, args }) =
                                blocks.remove(&index)
                            {
                                // Empty input is valid; serialize to `{}` so the
                                // downstream tool dispatch always sees parseable JSON.
                                let args_str = if args.is_empty() {
                                    "{}".to_string()
                                } else {
                                    args
                                };
                                tool_calls_acc.push(json!({
                                    "id": id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": args_str,
                                    }
                                }));
                            }
                        }
                        "message_stop" => {
                            sse_buffer.clear();
                            break 'outer;
                        }
                        "error" => {
                            let msg = json["error"]["message"]
                                .as_str()
                                .unwrap_or("unknown stream error");
                            return Err(anyhow::anyhow!("Anthropic stream error: {}", msg));
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(Message {
            id: None,
            role: "assistant".to_string(),
            content: if full_content.is_empty() {
                None
            } else {
                Some(full_content)
            },
            tool_calls: if tool_calls_acc.is_empty() {
                None
            } else {
                Some(tool_calls_acc)
            },
            tool_call_id: None,
            attachments: None,
            reasoning_content: if full_reasoning.is_empty() {
                None
            } else {
                Some(full_reasoning)
            },
            created_at: None,
        })
    }
}

/// Anthropic rejects `temperature` and `top_p` together on newer models with
/// `invalid_request_error`. The UI defaults both to non-None values, so we
/// can't pass them through verbatim — pick one. Temperature wins because it's
/// what users actually adjust; top_p is left at its slider default (1.0) far
/// more often.
///
/// `presence_penalty`, `frequency_penalty`, and `reasoning_effort` are
/// OpenAI-only and silently ignored here.
fn apply_sampling_options(body: &mut Value, options: Option<&GenerationOptions>) {
    let Some(opts) = options else {
        return;
    };
    let obj = body.as_object_mut().expect("body must be a JSON object");

    match (opts.temperature, opts.top_p) {
        (Some(temp), _) => {
            obj.insert("temperature".to_string(), json!(temp));
        }
        (None, Some(top_p)) => {
            obj.insert("top_p".to_string(), json!(top_p));
        }
        (None, None) => {}
    }

    if let Some(max_tokens) = opts.max_tokens {
        obj.insert("max_tokens".to_string(), json!(max_tokens));
    }
}

fn convert_messages(messages: Vec<Message>) -> (String, Vec<Value>) {
    let mut system_prompt = String::new();
    let mut filtered_messages = Vec::new();

    for msg in messages {
        if msg.role == "system" {
            if !system_prompt.is_empty() {
                system_prompt.push_str("\n\n");
            }
            system_prompt.push_str(msg.content.as_deref().unwrap_or(""));
        } else {
            // If there are tool call results (tool_call_id present), we need to format them correctly
            // Anthropic expects tool_result blocks
            if let Some(tool_call_id) = &msg.tool_call_id {
                // This is a tool RESULT from the user/tool
                filtered_messages.push(json!({
                   "role": "user",
                   "content": [{
                       "type": "tool_result",
                       "tool_use_id": tool_call_id,
                       "content": msg.content.unwrap_or_default()
                   }]
                }));
                continue;
            }

            // If message has tool_calls (assistant requesting)
            if let Some(calls) = &msg.tool_calls {
                let mut parts = Vec::new();
                // First add text thought if any
                if let Some(text) = &msg.content {
                    parts.push(json!({
                       "type": "text",
                       "text": text
                    }));
                }

                for call in calls {
                    let f = &call["function"];
                    parts.push(json!({
                        "type": "tool_use",
                        "id": call["id"],
                        "name": f["name"],
                        "input": serde_json::from_str::<Value>(f["arguments"].as_str().unwrap_or("{}")).unwrap_or(json!({}))
                     }));
                }

                filtered_messages.push(json!({
                    "role": msg.role,
                    "content": parts
                }));
                continue;
            }

            // Normal message
            let content = msg.content.clone().unwrap_or_default();
            let effective_content = if content.is_empty() {
                " ".to_string()
            } else {
                content
            };

            // Check for attachments
            if let Some(attachments) = &msg.attachments {
                let mut parts = Vec::new();

                // Text Content (User input + Text Attachments)
                let mut text_content = effective_content.clone();
                for att in attachments {
                    if !att.is_binary {
                        text_content.push_str(&format!(
                            "\n\nFile: {}\n```\n{}\n```",
                            att.name, att.data
                        ));
                    }
                }

                parts.push(json!({
                    "type": "text",
                    "text": text_content
                }));

                // Image Attachments
                for att in attachments {
                    if att.is_binary {
                        let img = &att.data;
                        // Extract base64. Format is data:image/png;base64,....
                        // We need media_type and data
                        if let Some(comma_pos) = img.find(',') {
                            let meta = &img[0..comma_pos]; // data:image/png;base64
                            let data = &img[comma_pos + 1..];

                            let media_type =
                                meta.trim_start_matches("data:").trim_end_matches(";base64");

                            parts.push(json!({
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": media_type,
                                    "data": data
                                }
                            }));
                        }
                    }
                }

                if !parts.is_empty() {
                    filtered_messages.push(json!({
                        "role": msg.role,
                        "content": parts
                    }));
                    continue;
                }
            }

            filtered_messages.push(json!({
                "role": msg.role,
                "content": effective_content
            }));
        }
    }

    (system_prompt, filtered_messages)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // Locks the invariant prompt-caching Phase 0b depends on: every system-role
    // message is folded into the flat system string regardless of its position in
    // the vec — even one that sits *after* a user message. This is why the volatile
    // trailing reminder must be role:"user"; a role:"system" reminder placed after
    // the history would be pulled back into the (cached) system prefix.
    #[test]
    fn system_messages_are_folded_into_system_prefix_regardless_of_position() {
        let (system_prompt, filtered) = convert_messages(vec![
            msg("system", "STABLE PROMPT"),
            msg("user", "hello"),
            msg("system", "TRAILING SYSTEM"),
        ]);

        assert_eq!(system_prompt, "STABLE PROMPT\n\nTRAILING SYSTEM");
        // Only the user turn survives in the messages array.
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0]["role"], "user");
        // The trailing system content must NOT remain in the messages array.
        assert!(!filtered
            .iter()
            .any(|m| m["content"].to_string().contains("TRAILING SYSTEM")));
    }

    // The Phase 0b trailing reminder (role:"user") stays in the messages array,
    // after the history, and out of the cached system prefix.
    #[test]
    fn user_role_trailing_reminder_stays_in_messages() {
        let reminder = "<system-reminder>\nThe current local date and time is ...\n</system-reminder>";
        let (system_prompt, filtered) = convert_messages(vec![
            msg("system", "STABLE PROMPT"),
            msg("user", "what time is it?"),
            msg("user", reminder),
        ]);

        // Volatile content is not in the cached prefix.
        assert_eq!(system_prompt, "STABLE PROMPT");
        assert!(!system_prompt.contains("system-reminder"));
        // It is the last message, content preserved verbatim (plain user messages
        // serialize content as a string, not a text-block array).
        assert_eq!(filtered.len(), 2);
        let last = filtered.last().unwrap();
        assert_eq!(last["role"], "user");
        assert_eq!(last["content"], reminder);
    }
}

