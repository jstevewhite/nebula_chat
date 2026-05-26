use crate::llm::provider::{
    GenerationOptions, LlmProvider, Message, StreamContent, ToolDefinition,
};
use crate::llm::think_tag::ThinkTagSplitter;
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
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

    /// POST to the OpenAI-compatible chat endpoint, returning a successful
    /// response or a descriptive error.
    ///
    /// Two behaviours matter here and were previously missing from the stream
    /// path, surfacing as a silent "no response":
    /// 1. The HTTP status is always checked — on error Ollama returns a JSON
    ///    body, not SSE, which the stream loop would otherwise drop on the
    ///    floor and return an empty message.
    /// 2. Tool-less local models (e.g. gemma3) reject *any* request carrying a
    ///    `tools` array with 400 "... does not support tools". Nebula always
    ///    attaches built-in tools (update_tasks, memory_*, skills), so we
    ///    transparently retry once without tools. Chat then works; tool-calling
    ///    is simply unavailable for that model.
    async fn post_with_tool_fallback(
        &self,
        body: &mut Value,
        has_tools: bool,
    ) -> Result<reqwest::Response> {
        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));

        let resp = self
            .client
            .post(&url)
            .header("Authorization", "Bearer ollama") // Dummy key required by some compat layers
            .json(&*body)
            .send()
            .await
            .context("Failed to send request to Ollama")?;

        if resp.status().is_success() {
            return Ok(resp);
        }

        let status = resp.status();
        let error_text = resp.text().await.unwrap_or_default();

        if status == reqwest::StatusCode::BAD_REQUEST
            && has_tools
            && error_text.contains("does not support tools")
        {
            tracing::warn!(
                "Ollama model '{}' does not support tools; retrying without tools",
                self.model
            );
            if let Some(obj) = body.as_object_mut() {
                obj.remove("tools");
                obj.remove("tool_choice");
            }

            let retry = self
                .client
                .post(&url)
                .header("Authorization", "Bearer ollama")
                .json(&*body)
                .send()
                .await
                .context("Failed to send request to Ollama")?;

            if !retry.status().is_success() {
                let retry_status = retry.status();
                let retry_text = retry.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!(
                    "Ollama API Error ({}): {}",
                    retry_status,
                    retry_text
                ));
            }
            return Ok(retry);
        }

        Err(anyhow::anyhow!("Ollama API Error ({}): {}", status, error_text))
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
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
            "stream": false
        });

        if let Some(opts) = options {
            // Ollama options are usually under "options" key or top level depending on version/compat
            // Standard /v1/chat/completions (OpenAI compat) uses top level for temp/top_p
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
            if let Some(max_tokens) = opts.max_tokens {
                body.as_object_mut()
                    .unwrap()
                    .insert("max_tokens".to_string(), json!(max_tokens));
            }
            if let Some(presence_penalty) = opts.presence_penalty {
                body.as_object_mut()
                    .unwrap()
                    .insert("presence_penalty".to_string(), json!(presence_penalty));
            }
            if let Some(frequency_penalty) = opts.frequency_penalty {
                body.as_object_mut()
                    .unwrap()
                    .insert("frequency_penalty".to_string(), json!(frequency_penalty));
            }
            // Note: reasoning_effort is specific to certain models (DeepSeek, OpenAI o1)
            // Ollama may or may not support it depending on the model
            if let Some(reasoning_effort) = opts.reasoning_effort {
                body.as_object_mut()
                    .unwrap()
                    .insert("reasoning_effort".to_string(), json!(reasoning_effort));
            }
        }

        let has_tools = !openai_tools.is_empty();
        if has_tools {
            body.as_object_mut()
                .unwrap()
                .insert("tools".to_string(), json!(openai_tools));
            body.as_object_mut()
                .unwrap()
                .insert("tool_choice".to_string(), json!("auto"));
        }

        let resp = self.post_with_tool_fallback(&mut body, has_tools).await?;

        let json: Value = resp.json().await?;
        let choice = &json["choices"][0]["message"];

        let raw_content = choice["content"].as_str().unwrap_or_default();
        let (clean_content, inline_reasoning) = {
            let mut splitter = ThinkTagSplitter::new();
            let (mut text, mut reasoning) = splitter.push(raw_content);
            let (text_tail, reasoning_tail) = splitter.flush();
            text.push_str(&text_tail);
            reasoning.push_str(&reasoning_tail);
            (text, reasoning)
        };
        let content = if clean_content.is_empty() {
            None
        } else {
            Some(clean_content)
        };
        let reasoning_content = if inline_reasoning.is_empty() {
            None
        } else {
            Some(inline_reasoning)
        };
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
            reasoning_content,
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
            if let Some(max_tokens) = opts.max_tokens {
                body.as_object_mut()
                    .unwrap()
                    .insert("max_tokens".to_string(), json!(max_tokens));
            }
            if let Some(presence_penalty) = opts.presence_penalty {
                body.as_object_mut()
                    .unwrap()
                    .insert("presence_penalty".to_string(), json!(presence_penalty));
            }
            if let Some(frequency_penalty) = opts.frequency_penalty {
                body.as_object_mut()
                    .unwrap()
                    .insert("frequency_penalty".to_string(), json!(frequency_penalty));
            }
            if let Some(reasoning_effort) = opts.reasoning_effort {
                body.as_object_mut()
                    .unwrap()
                    .insert("reasoning_effort".to_string(), json!(reasoning_effort));
            }
        }

        let has_tools = !openai_tools.is_empty();
        if has_tools {
            body.as_object_mut()
                .unwrap()
                .insert("tools".to_string(), json!(openai_tools));
            // Don't force tool_choice in stream if not robustly supported by all ollama versions, but standard is auto
            body.as_object_mut()
                .unwrap()
                .insert("tool_choice".to_string(), json!("auto"));
        }

        let resp = self.post_with_tool_fallback(&mut body, has_tools).await?;

        let mut stream = resp.bytes_stream();

        let mut full_content = String::new();
        let mut full_reasoning = String::new();
        let mut think_splitter = ThinkTagSplitter::new();
        // Ollama streaming tool calls support varies; implement basic accumulation similar to OpenAI
        let mut tool_calls_acc: Vec<Value> = Vec::new();
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
                                if let Some(content) = delta["content"].as_str() {
                                    let (text_out, reasoning_out) =
                                        think_splitter.push(content);
                                    if !text_out.is_empty() {
                                        on_token(StreamContent::Text(text_out.clone()));
                                        full_content.push_str(&text_out);
                                    }
                                    if !reasoning_out.is_empty() {
                                        on_token(StreamContent::Reasoning(
                                            reasoning_out.clone(),
                                        ));
                                        full_reasoning.push_str(&reasoning_out);
                                    }
                                }

                                if let Some(delta_tool_calls) = delta["tool_calls"].as_array() {
                                    for tc in delta_tool_calls {
                                        // `index` is absent in some compat layers; default to 0
                                        // rather than panicking and killing the stream task.
                                        let index = tc["index"].as_u64().unwrap_or(0) as usize;
                                        if current_tool_index != Some(index) {
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

        let (text_tail, reasoning_tail) = think_splitter.flush();
        if !text_tail.is_empty() {
            on_token(StreamContent::Text(text_tail.clone()));
            full_content.push_str(&text_tail);
        }
        if !reasoning_tail.is_empty() {
            on_token(StreamContent::Reasoning(reasoning_tail.clone()));
            full_reasoning.push_str(&reasoning_tail);
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

#[cfg(test)]
mod tests {
    use super::OllamaProvider;
    use crate::llm::provider::{LlmProvider, Message, StreamContent, ToolDefinition};
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    struct MockResponse {
        status: u16,
        content_type: &'static str,
        body: String,
    }

    impl MockResponse {
        fn json(status: u16, body: &str) -> Self {
            Self {
                status,
                content_type: "application/json",
                body: body.to_string(),
            }
        }
        fn sse(body: &str) -> Self {
            Self {
                status: 200,
                content_type: "text/event-stream",
                body: body.to_string(),
            }
        }
    }

    /// Minimal one-request-per-connection HTTP server. Serves `responses` in
    /// order (one per incoming request) and records each request body so tests
    /// can assert on what the provider actually sent. Returns the base URL and
    /// the shared list of captured request bodies.
    async fn start_mock(responses: Vec<MockResponse>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_task = requests.clone();
        let count = responses.len();
        let mut queue: VecDeque<MockResponse> = responses.into();

        tokio::spawn(async move {
            for _ in 0..count {
                let (mut sock, _) = listener.accept().await.unwrap();
                let body = read_request_body(&mut sock).await;
                requests_task.lock().unwrap().push(body);
                let resp = queue.pop_front().unwrap();
                write_response(&mut sock, &resp).await;
            }
        });

        (format!("http://{}", addr), requests)
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    async fn read_request_body(sock: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        loop {
            let n = sock.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            let Some(pos) = find_subslice(&buf, b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
            let content_len = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let body_start = pos + 4;
            while buf.len() - body_start < content_len {
                let n = sock.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let end = (body_start + content_len).min(buf.len());
            return String::from_utf8_lossy(&buf[body_start..end]).to_string();
        }
        String::new()
    }

    async fn write_response(sock: &mut TcpStream, resp: &MockResponse) {
        let reason = match resp.status {
            200 => "OK",
            400 => "Bad Request",
            500 => "Internal Server Error",
            _ => "Status",
        };
        let raw = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            resp.status,
            reason,
            resp.content_type,
            resp.body.len(),
            resp.body
        );
        sock.write_all(raw.as_bytes()).await.unwrap();
        sock.flush().await.unwrap();
        let _ = sock.shutdown().await;
    }

    fn user_msg(text: &str) -> Message {
        Message {
            id: None,
            role: "user".to_string(),
            content: Some(text.to_string()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            attachments: None,
            created_at: None,
        }
    }

    fn a_tool() -> ToolDefinition {
        ToolDefinition {
            name: "update_tasks".to_string(),
            description: "x".to_string(),
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }

    const TOOLS_UNSUPPORTED: &str =
        r#"{"error":{"message":"registry.ollama.ai/library/gemma3:4b does not support tools","type":"invalid_request_error"}}"#;

    #[tokio::test]
    async fn chat_retries_without_tools_when_model_rejects_them() {
        let (url, requests) = start_mock(vec![
            MockResponse::json(400, TOOLS_UNSUPPORTED),
            MockResponse::json(200, r#"{"choices":[{"message":{"content":"hello there"}}]}"#),
        ])
        .await;

        let provider = OllamaProvider::new(url, "gemma3:4b".to_string());
        let msg = provider
            .chat(vec![user_msg("hi")], vec![a_tool()], None)
            .await
            .expect("chat should succeed after retrying without tools");

        assert_eq!(msg.content.as_deref(), Some("hello there"));

        let reqs = requests.lock().unwrap();
        assert_eq!(reqs.len(), 2, "should have made an initial + retry request");
        assert!(reqs[0].contains("\"tools\""), "first request carries tools");
        assert!(
            !reqs[1].contains("\"tools\""),
            "retry must strip the tools array"
        );
        assert!(!reqs[1].contains("tool_choice"), "retry must strip tool_choice");
    }

    #[tokio::test]
    async fn chat_surfaces_other_400_errors_without_retrying() {
        let (url, requests) = start_mock(vec![MockResponse::json(
            400,
            r#"{"error":{"message":"context length exceeded"}}"#,
        )])
        .await;

        let provider = OllamaProvider::new(url, "gemma3:4b".to_string());
        let err = provider
            .chat(vec![user_msg("hi")], vec![a_tool()], None)
            .await
            .expect_err("a non-tools 400 should surface as an error");

        assert!(err.to_string().contains("context length exceeded"));
        assert_eq!(
            requests.lock().unwrap().len(),
            1,
            "must not retry on unrelated errors"
        );
    }

    #[tokio::test]
    async fn stream_returns_error_instead_of_silent_empty_message() {
        // The original bug: stream() never checked status, so an error body
        // (not SSE) yielded an empty Ok message — "no response" in the UI.
        let (url, _requests) = start_mock(vec![MockResponse::json(500, "boom")]).await;

        let provider = OllamaProvider::new(url, "gemma3:4b".to_string());
        let result = provider
            .stream(
                vec![user_msg("hi")],
                vec![],
                None,
                Box::new(|_| {}),
            )
            .await;

        assert!(
            result.is_err(),
            "stream must surface HTTP errors, not return an empty message"
        );
    }

    #[tokio::test]
    async fn stream_parses_sse_and_retries_without_tools() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n\
                   data: [DONE]\n\n";
        let (url, requests) = start_mock(vec![
            MockResponse::json(400, TOOLS_UNSUPPORTED),
            MockResponse::sse(sse),
        ])
        .await;

        let collected = Arc::new(Mutex::new(String::new()));
        let sink = collected.clone();
        let provider = OllamaProvider::new(url, "gemma3:4b".to_string());
        let msg = provider
            .stream(
                vec![user_msg("hi")],
                vec![a_tool()],
                None,
                Box::new(move |c| {
                    if let StreamContent::Text(t) = c {
                        sink.lock().unwrap().push_str(&t);
                    }
                }),
            )
            .await
            .expect("stream should succeed after retrying without tools");

        assert_eq!(msg.content.as_deref(), Some("Hello"));
        assert_eq!(*collected.lock().unwrap(), "Hello");

        let reqs = requests.lock().unwrap();
        assert_eq!(reqs.len(), 2);
        assert!(!reqs[1].contains("\"tools\""), "retry must strip tools");
    }
}
