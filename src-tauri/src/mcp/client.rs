use crate::mcp::config::McpTransport;
use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

#[async_trait::async_trait]
trait Transport: Send + Sync {
    async fn send(&self, req: JsonRpcRequest) -> Result<()>;
    // Optional: for manual shutdown or checks
    fn is_connected(&self) -> bool {
        true
    }
}

struct StdioTransport {
    tx: tokio::sync::mpsc::Sender<JsonRpcRequest>,
}

#[async_trait::async_trait]
impl Transport for StdioTransport {
    async fn send(&self, req: JsonRpcRequest) -> Result<()> {
        self.tx.send(req).await.context("Transport closed")?;
        Ok(())
    }
    fn is_connected(&self) -> bool {
        !self.tx.is_closed()
    }
}

struct SseTransport {
    url: String, // Base URL
    client: HttpClient,
    session_id: Arc<Mutex<Option<String>>>,
    sse_handle: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value>>>>>,
}

impl SseTransport {
    fn start_sse_loop(&self, session_id_val: String) {
        let client = self.client.clone();
        let url = self.url.clone();
        let pending = self.pending.clone();

        let mut handle_guard = self.sse_handle.lock().unwrap(); // specific tokio mutex might be better but std is fine here for simple guard or just use async mutex if deeper
                                                                // actually we are in async context, let's use the one from struct
                                                                // But to call blocking lock we need std::sync::Mutex or await.
                                                                // Let's use synchronous mutex for the handle option as it's quick.
                                                                // For simplicity, let's just spawn and overwrite.
        if handle_guard.is_some() {
            // Already running? Maybe we should abort old one if session ID changed?
            // For now, assume one session per transport lifetime for simplicity.
            return;
        }

        let handle = tokio::spawn(async move {
            let req_builder = client
                .get(&url)
                .header("Accept", "text/event-stream")
                .header("mcp-session-id", session_id_val);

            // Retry logic could go here
            let response = match req_builder.send().await {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Failed to connect to SSE endpoint: {}", e);
                    return;
                }
            };

            if !response.status().is_success() {
                eprintln!("SSE connection failed: {}", response.status());
                return;
            }

            let mut event_source = response.bytes_stream();

            while let Some(item) = event_source.next().await {
                match item {
                    Ok(bytes) => {
                        let s = String::from_utf8_lossy(&bytes);
                        for line in s.lines() {
                            if line.starts_with("data: ") {
                                let data = &line[6..];
                                if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(data) {
                                    McpClient::handle_response(resp, &pending).await;
                                }
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        *handle_guard = Some(handle);
    }
}

#[async_trait::async_trait]
impl Transport for SseTransport {
    async fn send(&self, req: JsonRpcRequest) -> Result<()> {
        let client = self.client.clone();
        let session_id_str = {
            let guard = self.session_id.lock().await;
            guard.clone()
        };

        let mut req_builder = client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .json(&req);
        if let Some(sid) = &session_id_str {
            req_builder = req_builder.header("mcp-session-id", sid);
        }

        // We need to await the response to capture headers
        let response = req_builder.send().await?;

        // Check for session ID in headers
        if let Some(val) = response.headers().get("mcp-session-id") {
            if let Ok(s) = val.to_str() {
                let new_sid = s.to_string();
                let mut guard = self.session_id.lock().await;
                if guard.as_ref() != Some(&new_sid) {
                    *guard = Some(new_sid.clone());
                    // Start/Restart SSE loop
                    self.start_sse_loop(new_sid);
                }
            }
        }

        // We can't return the response body here because `send` returns Result<()>.
        // But McpClient::request waits for the response via the `pending` channel.
        // IF the response is a standard JSON-RPC response in the body (which it is for POST),
        // we must process it here!
        if response.status().is_success() {
            // For "Streamable HTTP", the POST response might be empty (202 Accepted) if it's processing async?
            // Or it might be the actual JSON-RPC response (application/json).
            // The spec says: "Content-Type: application/json, to return one JSON object."
            if let Some(ct) = response.headers().get("content-type") {
                if ct.to_str().unwrap_or("").contains("application/json") {
                    // It's a direct response
                    if let Ok(resp) = response.json::<JsonRpcResponse>().await {
                        McpClient::handle_response(resp, &self.pending).await;
                    }
                }
            }
        }

        Ok(())
    }
}

pub struct McpClient {
    transport: Box<dyn Transport>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value>>>>>,
}

impl McpClient {
    pub async fn new(config: &McpTransport) -> Result<Self> {
        let pending = Arc::new(Mutex::new(
            HashMap::<String, oneshot::Sender<Result<Value>>>::new(),
        ));
        let pending_clone = pending.clone();

        let transport: Box<dyn Transport> = match config {
            McpTransport::Stdio { command, args, env } => {
                let mut cmd = Command::new(command);
                cmd.args(args);
                cmd.envs(env);
                cmd.stdin(Stdio::piped());
                cmd.stdout(Stdio::piped());
                cmd.stderr(Stdio::inherit());

                let mut child = cmd
                    .spawn()
                    .context(format!("Failed to spawn command: {}", command))?;

                let mut stdin = child.stdin.take().context("Failed to open stdin")?;
                let stdout = child.stdout.take().context("Failed to open stdout")?;

                let (tx, mut rx) = tokio::sync::mpsc::channel::<JsonRpcRequest>(32);

                // Writer Loop
                tokio::spawn(async move {
                    while let Some(req) = rx.recv().await {
                        if let Ok(json) = serde_json::to_string(&req) {
                            if stdin.write_all(json.as_bytes()).await.is_err() {
                                break;
                            }
                            if stdin.write_all(b"\n").await.is_err() {
                                break;
                            }
                        }
                    }
                });

                // Reader Loop
                let pending_monitor = pending_clone.clone();
                tokio::spawn(async move {
                    let reader = BufReader::new(stdout);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(&line) {
                            Self::handle_response(resp, &pending_monitor).await;
                        }
                    }
                });

                Box::new(StdioTransport { tx })
            }
            McpTransport::Sse { url } => {
                let client = HttpClient::new();
                let session_id = Arc::new(Mutex::new(None));
                let sse_handle = Arc::new(std::sync::Mutex::new(None)); // Use std mutex for handle option

                Box::new(SseTransport {
                    url: url.clone(),
                    client,
                    session_id,
                    sse_handle,
                    pending: pending_clone,
                })
            }
        };

        Ok(Self { transport, pending })
    }

    async fn handle_response(
        resp: JsonRpcResponse,
        pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value>>>>>,
    ) {
        if let Some(id) = resp.id {
            let id_str = match id {
                Value::String(s) => s,
                Value::Number(n) => n.to_string(),
                _ => id.to_string(),
            };

            let mut map = pending.lock().await;
            if let Some(sender) = map.remove(&id_str) {
                if let Some(err) = resp.error {
                    let _ = sender.send(Err(anyhow::anyhow!(
                        "RPC Error {}: {}",
                        err.code,
                        err.message
                    )));
                } else {
                    let _ = sender.send(Ok(resp.result.unwrap_or(Value::Null)));
                }
            }
        }
    }

    pub async fn request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        // Default timeout to avoid hanging indefinitely (e.g. during initialize).
        self.request_with_timeout(std::time::Duration::from_secs(30), method, params)
            .await
    }

    pub async fn request_with_timeout(
        &self,
        timeout: std::time::Duration,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value> {
        let id_val = Uuid::new_v4().to_string();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::String(id_val.clone())),
            method: method.to_string(),
            params,
        };

        let (resp_tx, resp_rx) = oneshot::channel();

        {
            let mut map = self.pending.lock().await;
            map.insert(id_val.clone(), resp_tx);
        }

        if let Err(e) = self.transport.send(req).await {
            let mut map = self.pending.lock().await;
            map.remove(&id_val);
            return Err(anyhow::anyhow!("Transport Error: {}", e));
        }

        match tokio::time::timeout(timeout, resp_rx).await {
            Ok(r) => r.context("Response channel closed")?,
            Err(_) => {
                // Ensure we don't leak a pending entry on timeout.
                let mut map = self.pending.lock().await;
                map.remove(&id_val);
                Err(anyhow::anyhow!(
                    "RPC timeout after {:?} ({})",
                    timeout,
                    method
                ))
            }
        }
    }

    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<()> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: method.to_string(),
            params,
        };
        self.transport.send(req).await
    }

    pub fn is_connected(&self) -> bool {
        self.transport.is_connected()
    }
}
