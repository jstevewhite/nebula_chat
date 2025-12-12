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
        let openai_tools: Vec<Value> = tools.into_iter().map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema
                }
            })
        }).collect();

        let mut body = json!({
            "model": self.model,
            "messages": messages,
        });

        if !openai_tools.is_empty() {
             body.as_object_mut().unwrap().insert("tools".to_string(), json!(openai_tools));
        }

        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));

        let resp = self.client.post(&url)
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
        let tool_calls = choice.get("tool_calls").cloned().and_then(|v| v.as_array().cloned());

        Ok(Message {
            role: "assistant".to_string(),
            content,
            tool_calls,
            tool_call_id: None,
        })
    }
}
