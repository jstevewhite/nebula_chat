use crate::llm::provider::{LlmProvider, Message, ToolDefinition};
use anyhow::{Context, Result};
use async_trait::async_trait;
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
    async fn chat(&self, messages: Vec<Message>, tools: Vec<ToolDefinition>) -> Result<Message> {
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
}
