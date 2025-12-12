use crate::llm::provider::{LlmProvider, Message, ToolDefinition};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

pub struct OllamaProvider {
    client: Client,
    base_url: String,
    model: String,
}

impl OllamaProvider {
    pub fn new(base_url: String, model: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
            model,
        }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
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

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "stream": false
        });

        if !openai_tools.is_empty() {
            body.as_object_mut()
                .unwrap()
                .insert("tools".to_string(), json!(openai_tools));
            body.as_object_mut()
                .unwrap()
                .insert("tool_choice".to_string(), json!("auto"));
        }

        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );

        let resp = self
            .client
            .post(&url)
            .header("Authorization", "Bearer ollama") // Dummy key required by some compat layers
            .json(&body)
            .send()
            .await
            .context("Failed to send request to Ollama")?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow::anyhow!("Ollama API Error: {}", error_text));
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
        })
    }
}
