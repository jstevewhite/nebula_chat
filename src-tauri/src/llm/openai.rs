use crate::llm::provider::{
    GenerationOptions, LlmProvider, Message, StreamContent, ToolDefinition,
};
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

    fn sanitize_messages(messages: Vec<Message>) -> Vec<Message> {
        let mut healed = messages;

        // Pass 1: Heal IDs (Block-based)
        // We iterate through the messages. When we find an Assistant message with tool_calls,
        // we look ahead to see how many Tool messages follow it.
        // We then try to assign missing IDs to those Tool messages sequentially.
        let len = healed.len();
        for i in 0..len {
            if healed[i].role == "assistant" {
                if let Some(calls) = &healed[i].tool_calls {
                    if !calls.is_empty() {
                        // Gather IDs from this assistant message
                        let valid_ids: Vec<Option<String>> = calls
                            .iter()
                            .map(|c| c.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                            .collect();

                        // Look ahead for sequential tool messages
                        let mut tool_idx = 0;
                        for j in (i + 1)..len {
                            if healed[j].role == "tool" {
                                // If this tool message has no ID (or empty), try to assign one
                                let needs_id = match &healed[j].tool_call_id {
                                    Some(id) => id.trim().is_empty(),
                                    None => true,
                                };

                                if needs_id {
                                    if let Some(Some(target_id)) = valid_ids.get(tool_idx) {
                                        tracing::warn!("Healed missing tool_call_id for message at index {} using assistant call index {}", j, tool_idx);
                                        healed[j].tool_call_id = Some(target_id.clone());
                                    }
                                }
                                tool_idx += 1;
                            } else {
                                // Stop if we hit a non-tool message (e.g. user or another assistant)
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Pass 2: Prune invalid
        let mut final_msgs: Vec<Message> = Vec::new();
        let len = healed.len();

        for i in 0..len {
            let mut msg = healed[i].clone();

            if msg.role == "tool" {
                // 1. Must have valid ID
                let has_valid_id = match &msg.tool_call_id {
                    Some(id) => !id.trim().is_empty(),
                    None => false,
                };

                if !has_valid_id {
                    tracing::warn!("Dropping invalid tool message without ID at index {}", i);
                    continue;
                }

                // 2. Must be preceded by Assistant or Tool (Sequential) in the FINAL stream
                // If it's the first message, it's definitely an orphan (unless system msg exists? No, system usually at 0)
                // Actually, if final_msgs is empty, it's an orphan.
                // If final_msgs.last() is Not Assistant AND Not Tool, it's an orphan.

                let is_orphan = if let Some(last) = final_msgs.last() {
                    let r = last.role.as_str();
                    r != "assistant" && r != "tool"
                } else {
                    true // Orphan at start of queue
                };

                if is_orphan {
                    tracing::warn!(
                        "Dropping orphaned tool message at index {} (preceded by {:?})",
                        i,
                        final_msgs.last().map(|m| &m.role)
                    );
                    continue;
                }

                // 3. Validate that tool_call_id references an actual tool call in a preceding assistant message
                // Search backwards through final_msgs to find the assistant message with this tool_call_id
                let tool_call_id = msg.tool_call_id.as_ref().unwrap(); // Safe: validated above
                let mut found_matching_call = false;
                
                for prev_msg in final_msgs.iter().rev() {
                    if prev_msg.role == "assistant" {
                        if let Some(calls) = &prev_msg.tool_calls {
                            // Check if any tool_call has this ID
                            found_matching_call = calls.iter().any(|call| {
                                call.get("id")
                                    .and_then(|v| v.as_str())
                                    .map(|id| id == tool_call_id)
                                    .unwrap_or(false)
                            });
                            if found_matching_call {
                                break; // Found it, stop searching
                            }
                        }
                    }
                }

                if !found_matching_call {
                    tracing::error!(
                        "SANITIZE: Dropping tool message at index {} with invalid tool_call_id '{}' (not found in any preceding assistant message). Searched {} assistant messages.",
                        i,
                        tool_call_id,
                        final_msgs.iter().filter(|m| m.role == "assistant").count()
                    );
                    continue;
                }
            } else if msg.role == "assistant" {
                // 1. Validate internal integrity of tool_calls (must have IDs)
                if let Some(tc) = &mut msg.tool_calls {
                    tc.retain(|call| {
                         let has_id = call.get("id").and_then(|v| v.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false);
                         if !has_id {
                             tracing::warn!("Pruning malformed tool_call without ID inside assistant message at index {}", i);
                         }
                         has_id
                     });
                    // If filtered to empty, set to None
                    if tc.is_empty() {
                        msg.tool_calls = None;
                    }
                }

                // 2. Check for orphaned tool calls
                let mut has_tool_calls = false;
                if let Some(tc) = &msg.tool_calls {
                    if !tc.is_empty() {
                        has_tool_calls = true;
                    }
                }

                if has_tool_calls {
                    // Check if next is tool.
                    let next_is_valid_tool = if i + 1 < len {
                        let next = &healed[i + 1];
                        match &next.tool_call_id {
                            Some(id) => next.role == "tool" && !id.trim().is_empty(),
                            None => false,
                        }
                    } else {
                        false
                    };

                    // If it's the last message, it's fine (pending execution).
                    let is_last = i == len - 1;

                    if !is_last && !next_is_valid_tool {
                        if let Some(calls) = &msg.tool_calls {
                            let ids: Vec<String> = calls.iter()
                                .filter_map(|c| c.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                                .collect();
                            tracing::error!("SANITIZE: Removing orphaned tool_calls {:?} from assistant at index {} (no following tool messages)", ids, i);
                        }
                        msg.tool_calls = None;

                        // If content also empty, drop?
                        let has_content =
                            msg.content.as_ref().map(|s| !s.is_empty()).unwrap_or(false);
                        if !has_content {
                            tracing::error!("SANITIZE: Dropping empty assistant message at index {}", i);
                            continue;
                        }
                    }
                }
            }

            final_msgs.push(msg);
        }

        final_msgs
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
        // Log message roles for debugging
        let role_seq: Vec<String> = messages.iter().map(|m| {
            if m.role == "tool" {
                format!("tool(id:{:?})", m.tool_call_id)
            } else {
                m.role.clone()
            }
        }).collect();
        tracing::debug!("OpenAI chat called with {} messages: {:?}", messages.len(), role_seq);
        
        let messages = Self::sanitize_messages(messages);

        // Log final message count and roles after sanitization
        let sanitized_roles: Vec<&str> = messages.iter().map(|m| m.role.as_str()).collect();
        tracing::debug!("After sanitization: {} messages: {:?}", messages.len(), sanitized_roles);

        // Log detailed info about assistant messages with tool_calls
        for (idx, msg) in messages.iter().enumerate() {
            if msg.role == "assistant" && msg.tool_calls.is_some() {
                if let Some(calls) = &msg.tool_calls {
                    let ids: Vec<String> = calls.iter()
                        .filter_map(|c| c.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                        .collect();
                    tracing::warn!("Assistant at index {} has tool_calls with IDs: {:?}", idx, ids);
                }
            }
        }
        
        // Log and validate tool messages have valid IDs
        for (idx, msg) in messages.iter().enumerate() {
            if msg.role == "tool" {
                tracing::warn!(
                    "Tool message at index {}: tool_call_id = {:?}, content preview = {:?}",
                    idx,
                    msg.tool_call_id,
                    msg.content.as_ref().map(|c| &c[..c.len().min(100)])
                );

                // Validate tool message has valid tool_call_id
                match &msg.tool_call_id {
                    Some(id) if !id.trim().is_empty() => {
                        // Valid
                    }
                    Some(_) => {
                        return Err(anyhow::anyhow!(
                            "CRITICAL: Tool message at index {} has empty tool_call_id. This violates API requirements.",
                            idx
                        ));
                    }
                    None => {
                        return Err(anyhow::anyhow!(
                            "CRITICAL: Tool message at index {} missing tool_call_id field. This violates API requirements.",
                            idx
                        ));
                    }
                }
            }
        }

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

                // Default handling - exclude 'id' field as OpenAI doesn't accept it
                let mut obj = json!({
                    "role": msg.role,
                });

                if let Some(content) = msg.content {
                    obj.as_object_mut()
                        .unwrap()
                        .insert("content".to_string(), json!(content));
                }

                if let Some(tool_call_id) = msg.tool_call_id {
                    // Only insert if not empty/whitespace
                    if !tool_call_id.trim().is_empty() {
                        obj.as_object_mut()
                            .unwrap()
                            .insert("tool_call_id".to_string(), json!(tool_call_id));
                    }
                    // Note: Empty tool_call_id already validated above, won't reach here for tool messages
                }

                if let Some(tool_calls) = msg.tool_calls {
                    obj.as_object_mut()
                        .unwrap()
                        .insert("tool_calls".to_string(), json!(tool_calls));
                }

                obj
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

        let reasoning_content = choice
            .get("reasoning_content")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

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
            reasoning_content,
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
        let messages = Self::sanitize_messages(messages);
        
        // Log final message count and roles after sanitization
        let sanitized_roles: Vec<&str> = messages.iter().map(|m| m.role.as_str()).collect();
        tracing::debug!("Stream after sanitization: {} messages: {:?}", messages.len(), sanitized_roles);
        
        // Log and validate tool messages have valid IDs
        for (idx, msg) in messages.iter().enumerate() {
            if msg.role == "tool" {
                tracing::debug!(
                    "Stream tool message at index {}: tool_call_id = {:?}",
                    idx,
                    msg.tool_call_id
                );

                // Validate tool message has valid tool_call_id
                match &msg.tool_call_id {
                    Some(id) if !id.trim().is_empty() => {
                        // Valid
                    }
                    Some(_) => {
                        return Err(anyhow::anyhow!(
                            "CRITICAL: Tool message at index {} has empty tool_call_id. This violates API requirements.",
                            idx
                        ));
                    }
                    None => {
                        return Err(anyhow::anyhow!(
                            "CRITICAL: Tool message at index {} missing tool_call_id field. This violates API requirements.",
                            idx
                        ));
                    }
                }
            }
        }

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

                // Default handling - exclude 'id' field as OpenAI doesn't accept it
                let mut obj = json!({
                    "role": msg.role,
                });

                if let Some(content) = msg.content {
                    obj.as_object_mut()
                        .unwrap()
                        .insert("content".to_string(), json!(content));
                }

                if let Some(tool_call_id) = msg.tool_call_id {
                    // Only insert if not empty/whitespace
                    if !tool_call_id.trim().is_empty() {
                        obj.as_object_mut()
                            .unwrap()
                            .insert("tool_call_id".to_string(), json!(tool_call_id));
                    }
                    // Note: Empty tool_call_id already validated above, won't reach here for tool messages
                }

                if let Some(tool_calls) = msg.tool_calls {
                    obj.as_object_mut()
                        .unwrap()
                        .insert("tool_calls".to_string(), json!(tool_calls));
                }

                obj
            })
            .collect();

        let mut body = json!({
            "model": self.model,
            "messages": formatted_messages,
            "model": self.model,
            "messages": formatted_messages,
            "stream": true,
            // Attempt to request reasoning for compatible models (like DeepSeek via specialized providers)
            // Most standard providers ignore unknown fields, but strict ones might fail.
            // We'll add it if the model name suggests DeepSeek or similar, or just try it.
            // For now, let's only add it if it's NOT a standard OpenAI model to avoid validation errors.
            // "include_reasoning": true
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
        let mut full_reasoning = String::new();
        let mut tool_calls_acc: Vec<Value> = Vec::new();

        let mut sse_buffer = String::new();
        let mut saw_any_delta = false;

        let mut current_tool_index: Option<usize> = None;
        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_args = String::new();

        'outer: while let Some(item) = stream.next().await {
            let chunk = item?;
            let chunk_norm = String::from_utf8_lossy(&chunk).replace("\r\n", "\n");
            sse_buffer.push_str(&chunk_norm);

            loop {
                if let Some(idx) = sse_buffer.find("\n\n") {
                    let event = sse_buffer[..idx].to_string();
                    sse_buffer.drain(..idx + 2);

                    for line in event.lines() {
                        if !line.starts_with("data: ") {
                            continue;
                        }
                        let data = line[6..].trim();
                        if data == "[DONE]" {
                            sse_buffer.clear();
                            break 'outer;
                        }

                        if let Ok(json) = serde_json::from_str::<Value>(data) {
                            if let Some(choices) = json["choices"].as_array() {
                                if let Some(choice) = choices.first() {
                                    if let Some(delta) = choice.get("delta") {
                                        tracing::debug!("Raw delta: {:?}", delta);
                                        // Content
                                        if let Some(content_str) = delta["content"].as_str() {
                                            on_token(StreamContent::Text(content_str.to_string()));
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
                                                        on_token(StreamContent::Text(
                                                            txt.to_string(),
                                                        ));
                                                        full_content.push_str(txt);
                                                        saw_any_delta = true;
                                                    }
                                                }
                                            }
                                        }

                                        // Reasoning (DeepSeek/Qwen) - Check common fields
                                        let reasoning_chunk = delta
                                            .get("reasoning_content")
                                            .or_else(|| delta.get("reasoning"))
                                            .or_else(|| delta.get("thinking"))
                                            .and_then(|v| v.as_str());

                                        if let Some(reasoning_str) = reasoning_chunk {
                                            tracing::debug!(
                                                "🧩 Reasoning chunk: {}",
                                                reasoning_str
                                            );
                                            on_token(StreamContent::Reasoning(
                                                reasoning_str.to_string(),
                                            ));
                                            full_reasoning.push_str(reasoning_str);
                                            saw_any_delta = true;
                                        }

                                        // Tool Calls
                                        if let Some(delta_tool_calls) =
                                            delta["tool_calls"].as_array()
                                        {
                                            for tc in delta_tool_calls {
                                                let index =
                                                    tc["index"].as_u64().unwrap_or(0) as usize;

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
                    break;
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

        if full_content.is_empty()
            && full_reasoning.is_empty()
            && tool_calls_acc.is_empty()
            && !saw_any_delta
        {
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
            reasoning_content: if full_reasoning.is_empty() {
                None
            } else {
                Some(full_reasoning)
            },
            tool_calls: if tool_calls_acc.is_empty() {
                None
            } else {
                Some(tool_calls_acc)
            },
            tool_call_id: None,
            attachments: None,
            created_at: None,
        })
    }
}
