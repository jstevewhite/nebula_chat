use crate::llm::provider::{GenerationOptions, LlmProvider, Message, ToolDefinition};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};

fn sanitize_base_url(input: Option<String>, default_: &str) -> String {
    let mut base = input.unwrap_or_else(|| default_.to_string());
    // Trim trailing slashes
    while base.ends_with('/') {
        base.pop();
    }
    // Strip trailing /v1 if present
    if base.ends_with("/v1") {
        base.truncate(base.len() - 3);
    }
    base
}

fn extract_text_from_content(msg_content: &Value) -> String {
    // Handles OpenAI message.content which can be a string or an array of parts
    if let Some(s) = msg_content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = msg_content.as_array() {
        let mut out = String::new();
        for part in arr {
            if let Some(t) = part.get("type").and_then(|v| v.as_str()) {
                match t {
                    "text" => {
                        if let Some(txt) = part.get("text").and_then(|v| v.as_str()) {
                            out.push_str(txt);
                        }
                    }
                    _ => {}
                }
            }
        }
        return out;
    }
    String::new()
}

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenAiProvider {
    pub fn new(api_key: String, base_url: Option<String>, model: String) -> Self {
        let base = sanitize_base_url(base_url, "https://api.openai.com");
        Self {
            client: Client::new(),
            api_key,
            base_url: base,
            model,
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        options: Option<GenerationOptions>,
    ) -> Result<Message> {
        let openai_tools: Vec<Value> = tools
            .into_iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema
                    }
                })
            })
            .collect();

        let formatted_messages: Vec<Value> = messages
            .into_iter()
            .map(|msg| {
                if let Some(attachments) = &msg.attachments {
                    let mut content_parts = Vec::new();
                    let mut text_content = msg.content.clone().unwrap_or_default();

                    // Process Text Attachments
                    for att in attachments {
                        if !att.is_binary {
                            text_content.push_str(&format!(
                                "\n\nFile: {}\n```\n{}\n```",
                                att.name, att.data
                            ));
                        }
                    }

                    // Add Main Text Part (with appended text attachments)
                    if !text_content.is_empty() {
                        content_parts.push(json!({
                            "type": "text",
                            "text": text_content
                        }));
                    }

                    // Process Image Attachments
                    for att in attachments {
                        if att.is_binary {
                            content_parts.push(json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": att.data
                                }
                            }));
                        }
                    }

                    if !content_parts.is_empty() {
                        return json!({
                            "role": msg.role,
                            "content": content_parts
                        });
                    }
                }

                // Default handling
                json!(msg)
            })
            .collect();

        let mut body = json!({
            "model": self.model,
            "messages": formatted_messages,
        });

        if let Some(opts) = options {
            if let Some(temp) = opts.temperature {
                body.as_object_mut()
                    .unwrap()
                    .insert("temperature".to_string(), json!(temp));
            }
            if let Some(top_p) = opts.top_p {
                body.as_object_mut()
                    .unwrap()
                    .insert("top_p".to_string(), json!(top_p));
            }
        }

        if !openai_tools.is_empty() {
            body.as_object_mut()
                .unwrap()
                .insert("tools".to_string(), json!(openai_tools));
        }

        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .context("Failed to send request")?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow::anyhow!("OpenAI API Error: {}", error_text));
        }

        let json: Value = resp.json().await?;
        let choice = &json["choices"][0]["message"];

        let content_text = extract_text_from_content(&choice["content"]);
        let tool_calls = choice
            .get("tool_calls")
            .cloned()
            .and_then(|v| v.as_array().cloned());

        if content_text.is_empty()
            && (tool_calls.is_none() || tool_calls.as_ref().unwrap().is_empty())
        {
            let raw = json.to_string();
            let truncated = if raw.len() > 2000 {
                format!("{}...", &raw[..2000])
            } else {
                raw
            };
            return Err(anyhow::anyhow!(
                "OpenAI returned empty message. Raw response: {}",
                truncated
            ));
        }

        let content = if content_text.is_empty() {
            None
        } else {
            Some(content_text)
        };

        Ok(Message {
            id: None,
            role: "assistant".to_string(),
            content,
            tool_calls,
            tool_call_id: None,
            attachments: None,
        })
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        options: Option<GenerationOptions>,
        on_token: Box<dyn Fn(String) + Send + Sync>,
    ) -> Result<Message> {
        let openai_tools: Vec<Value> = tools
            .into_iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema
                    }
                })
            })
            .collect();

        let formatted_messages: Vec<Value> = messages
            .into_iter()
            .map(|msg| {
                // Simplified attachment handling for streaming reuse (same as chat)
                if let Some(attachments) = &msg.attachments {
                    let mut content_parts = Vec::new();
                    let mut text_content = msg.content.clone().unwrap_or_default();

                    for att in attachments {
                        if !att.is_binary {
                            text_content.push_str(&format!(
                                "\n\nFile: {}\n```\n{}\n```",
                                att.name, att.data
                            ));
                        }
                    }

                    if !text_content.is_empty() {
                        content_parts.push(json!({"type": "text", "text": text_content}));
                    }

                    for att in attachments {
                        if att.is_binary {
                            content_parts
                                .push(json!({"type": "image_url", "image_url": {"url": att.data}}));
                        }
                    }

                    if !content_parts.is_empty() {
                        return json!({"role": msg.role, "content": content_parts});
                    }
                }
                json!(msg)
            })
            .collect();

        let mut body = json!({
            "model": self.model,
            "messages": formatted_messages,
            "stream": true
        });

        if let Some(opts) = options {
            if let Some(temp) = opts.temperature {
                body.as_object_mut()
                    .unwrap()
                    .insert("temperature".to_string(), json!(temp));
            }
            if let Some(top_p) = opts.top_p {
                body.as_object_mut()
                    .unwrap()
                    .insert("top_p".to_string(), json!(top_p));
            }
        }

        if !openai_tools.is_empty() {
            body.as_object_mut()
                .unwrap()
                .insert("tools".to_string(), json!(openai_tools));
        }

        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .context("Failed to send stream request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "OpenAI stream error: {} — {}",
                status,
                body_text
            ));
        }

        let mut stream = response.bytes_stream();

        let mut full_content = String::new();
        let mut tool_calls_acc: Vec<Value> = Vec::new(); // Accumulate tool calls logic if needed (complex)

        // Maintain an SSE buffer across chunks to avoid losing partial lines
        let mut sse_buffer = String::new();
        let mut saw_any_delta = false;

        // OpenAI streaming tool calls send parts we need to reassemble.
        let mut current_tool_index: Option<usize> = None;
        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_args = String::new();

        'outer: while let Some(item) = stream.next().await {
            let chunk = item?;
            // Normalize CRLF to LF to make splitting robust across platforms
            let chunk_norm = String::from_utf8_lossy(&chunk).replace("\r\n", "\n");
            sse_buffer.push_str(&chunk_norm);

            // Process complete SSE events separated by double newlines
            loop {
                if let Some(idx) = sse_buffer.find("\n\n") {
                    let event = sse_buffer[..idx].to_string();
                    sse_buffer.drain(..idx + 2);

                    // Each event may contain multiple lines; extract data lines
                    for line in event.lines() {
                        if !line.starts_with("data: ") {
                            continue;
                        }
                        let data = line[6..].trim();
                        if data == "[DONE]" {
                            // End of stream
                            sse_buffer.clear();
                            break 'outer;
                        }

                        if let Ok(json) = serde_json::from_str::<Value>(data) {
                            if let Some(choices) = json["choices"].as_array() {
                                if let Some(choice) = choices.first() {
                                    if let Some(delta) = choice.get("delta") {
                                        // Handle Content (string or array of parts)
                                        if let Some(content_str) = delta["content"].as_str() {
                                            on_token(content_str.to_string());
                                            full_content.push_str(content_str);
                                            saw_any_delta = true;
                                        } else if let Some(parts) = delta["content"].as_array() {
                                            for part in parts {
                                                if part.get("type").and_then(|v| v.as_str())
                                                    == Some("text")
                                                {
                                                    if let Some(txt) =
                                                        part.get("text").and_then(|v| v.as_str())
                                                    {
                                                        on_token(txt.to_string());
                                                        full_content.push_str(txt);
                                                        saw_any_delta = true;
                                                    }
                                                }
                                            }
                                        }

                                        // Handle Tool Calls (Accumulation)
                                        if let Some(delta_tool_calls) =
                                            delta["tool_calls"].as_array()
                                        {
                                            for tc in delta_tool_calls {
                                                let index =
                                                    tc["index"].as_u64().unwrap_or(0) as usize;

                                                // New Tool Call?
                                                if current_tool_index != Some(index) {
                                                    // Push previous if exists
                                                    if !current_tool_id.is_empty() {
                                                        tool_calls_acc.push(json!({
                                                            "id": current_tool_id,
                                                            "type": "function",
                                                            "function": {
                                                                "name": current_tool_name,
                                                                "arguments": current_tool_args
                                                            }
                                                        }));
                                                    }

                                                    // Reset
                                                    current_tool_index = Some(index);
                                                    current_tool_id =
                                                        tc["id"].as_str().unwrap_or("").to_string();
                                                    current_tool_name = tc["function"]["name"]
                                                        .as_str()
                                                        .unwrap_or("")
                                                        .to_string();
                                                    current_tool_args = tc["function"]["arguments"]
                                                        .as_str()
                                                        .unwrap_or("")
                                                        .to_string();
                                                } else {
                                                    // Append args
                                                    if let Some(args) =
                                                        tc["function"]["arguments"].as_str()
                                                    {
                                                        current_tool_args.push_str(args);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    break; // wait for more data
                }
            }
        }

        // Push last tool call if any
        if !current_tool_id.is_empty() {
            tool_calls_acc.push(json!({
                "id": current_tool_id,
                "type": "function",
                "function": {
                    "name": current_tool_name,
                    "arguments": current_tool_args
                }
            }));
        }

        if full_content.is_empty() && tool_calls_acc.is_empty() && !saw_any_delta {
            return Err(anyhow::anyhow!("OpenAI streaming returned no content"));
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
        })
    }
}
