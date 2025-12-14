use crate::llm::provider::{GenerationOptions, LlmProvider, Message, ToolDefinition};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenAiProvider {
    pub fn new(api_key: String, base_url: Option<String>, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com".to_string()),
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

        let content = choice["content"].as_str().map(|s| s.to_string());
        let tool_calls = choice
            .get("tool_calls")
            .cloned()
            .and_then(|v| v.as_array().cloned());

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

        let mut stream = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .context("Failed to send stream request")?
            .bytes_stream();

        let mut full_content = String::new();
        let mut tool_calls_acc: Vec<Value> = Vec::new(); // Accumulate tool calls logic if needed (complex)

        // For MVP streaming, we focus on content. Tool calls usually come in non-streamed or specific chunks.
        // OpenAI streaming tool calls send parts we need to reassemble.

        let mut current_tool_index: Option<usize> = None;
        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_args = String::new();

        while let Some(item) = stream.next().await {
            let chunk = item?;
            let chunk_str = String::from_utf8_lossy(&chunk);

            for line in chunk_str.lines() {
                if !line.starts_with("data: ") {
                    continue;
                }
                let data = line.trim_start_matches("data: ");
                if data == "[DONE]" {
                    break;
                }

                if let Ok(json) = serde_json::from_str::<Value>(data) {
                    if let Some(choices) = json["choices"].as_array() {
                        if let Some(choice) = choices.first() {
                            if let Some(delta) = choice.get("delta") {
                                // Handle Content
                                if let Some(content) = delta["content"].as_str() {
                                    on_token(content.to_string());
                                    full_content.push_str(content);
                                }

                                // Handle Tool Calls (Accumulation)
                                if let Some(delta_tool_calls) = delta["tool_calls"].as_array() {
                                    for tc in delta_tool_calls {
                                        let index = tc["index"].as_u64().unwrap() as usize;

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
                                            if let Some(args) = tc["function"]["arguments"].as_str()
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
