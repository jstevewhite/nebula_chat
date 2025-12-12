use crate::llm::provider::{LlmProvider, Message, ToolDefinition};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model,
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(&self, messages: Vec<Message>, tools: Vec<ToolDefinition>) -> Result<Message> {
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

        // Convert messages to Anthropic format
        // System prompt is separate in Anthropic API
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

        let mut body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": filtered_messages,
        });

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
            .post("https://api.anthropic.com/v1/messages")
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

        Ok(Message {
            id: None,
            role: "assistant".to_string(),
            content: if final_content.is_empty() {
                None
            } else {
                Some(final_content)
            },
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: None,
            attachments: None,
        })
    }
}
