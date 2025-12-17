use crate::mcp::config::McpTransport;
use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::Client as HttpClient;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
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
    // Synchronous check is preferred to avoid complexity in filters
    fn is_connected(&self) -> bool {
        true
    }
    // Stop the transport and clean up resources
    fn stop(&self) {}
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransportStatus {
    Connected,
    Disconnected(String),
    Reconnecting,
}

struct StdioTransport {
    tx: tokio::sync::mpsc::Sender<JsonRpcRequest>,
    status: Arc<Mutex<TransportStatus>>,
    #[allow(dead_code)] // Logs kept for diagnostics/future UI
    stderr_log: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl Transport for StdioTransport {
    async fn send(&self, req: JsonRpcRequest) -> Result<()> {
        if let TransportStatus::Disconnected(reason) = &*self.status.lock().await {
            return Err(anyhow::anyhow!("Transport disconnected: {}", reason));
        }
        self.tx.send(req).await.context("Transport closed")?;
        Ok(())
    }
    fn is_connected(&self) -> bool {
        if let Ok(status) = self.status.try_lock() {
            matches!(*status, TransportStatus::Connected)
        } else {
            !self.tx.is_closed()
        }
    }
}

struct SseTransport {
    url: String,
    client: HttpClient,
    headers: HeaderMap,
    session_id: Arc<Mutex<Option<String>>>,
    sse_handle: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value>>>>>,
    shutdown_flag: Arc<AtomicBool>,
    disconnected_flag: Arc<AtomicBool>,
}

fn build_header_map(headers: &HashMap<String, String>) -> Result<HeaderMap> {
    let mut map = HeaderMap::new();
    for (k, v) in headers {
        let name = HeaderName::from_bytes(k.as_bytes())
            .with_context(|| format!("Invalid header name: '{}'", k))?;
        let value = HeaderValue::from_str(v)
            .with_context(|| format!("Invalid header value for '{}': value is not valid HTTP header text", k))?;
        map.insert(name, value);
    }
    Ok(map)
}

impl SseTransport {
    fn start_sse_loop(&self, session_id_val: String) {
        let client = self.client.clone();
        let url = self.url.clone();
        let headers = self.headers.clone();
        let pending = self.pending.clone();
        let session_id_str = session_id_val.clone(); // Keep for retries
        let shutdown_flag = self.shutdown_flag.clone();

        let mut handle_guard = self.sse_handle.lock().unwrap();
        if handle_guard.is_some() {
            return;
        }

        let handle = tokio::spawn(async move {
            let mut retry_delay = std::time::Duration::from_millis(500);
            let max_delay = std::time::Duration::from_secs(15);

            loop {
                // Check if shutdown was requested
                if shutdown_flag.load(Ordering::Relaxed) {
                    tracing::info!("SSE loop stopped by request");
                    return;
                }

                let req_builder = client
                    .get(&url)
                    .headers(headers.clone())
                    .header("Accept", "text/event-stream")
                    .header("mcp-session-id", &session_id_str);

                match req_builder.send().await {
                    Ok(response) => {
                        if !response.status().is_success() {
                            tracing::error!("SSE connection failed: {}", response.status());
                        } else {
                            // Reset backoff on successful connection
                            retry_delay = std::time::Duration::from_millis(500);

                            let mut event_source = response.bytes_stream();
                            let mut buffer = Vec::new();

                            loop {
                                match event_source.next().await {
                                    Some(Ok(bytes)) => {
                                        buffer.extend_from_slice(&bytes);

                                        // Process buffer for events delimited by \n\n
                                        while let Some(idx) =
                                            buffer.windows(2).position(|w| w == b"\n\n")
                                        {
                                            let event_bytes =
                                                buffer.drain(0..idx + 2).collect::<Vec<u8>>();
                                            let s = String::from_utf8_lossy(&event_bytes);

                                            // Naive parser for "data: " lines within the chunk
                                            for line in s.lines() {
                                                if line.starts_with("data: ") {
                                                    let data = &line[6..];
                                                    // Handle [DONE] or other messages if relevant, mostly JSON
                                                    if let Ok(resp) =
                                                        serde_json::from_str::<JsonRpcResponse>(
                                                            data,
                                                        )
                                                    {
                                                        McpClient::handle_response(resp, &pending)
                                                            .await;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Some(Err(e)) => {
                                        tracing::warn!("SSE stream error: {}", e);
                                        break; // Break inner loop to reconnect
                                    }
                                    None => {
                                        tracing::warn!("SSE stream ended");
                                        break; // Break inner loop to reconnect
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to connect to SSE endpoint: {}", e);
                    }
                }

                // Delay before retry
                tokio::time::sleep(retry_delay).await;
                retry_delay = std::cmp::min(retry_delay * 2, max_delay);
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
            .headers(self.headers.clone())
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

        if response.status().is_success() {
            if let Some(ct) = response.headers().get("content-type") {
                if ct.to_str().unwrap_or("").contains("application/json") {
                    if let Ok(resp) = response.json::<JsonRpcResponse>().await {
                        McpClient::handle_response(resp, &self.pending).await;
                    }
                }
            }
        }
        Ok(())
    }

    fn is_connected(&self) -> bool {
        let guard = self.sse_handle.lock().unwrap();
        guard.is_some() && !self.disconnected_flag.load(Ordering::Relaxed)
    }

    fn stop(&self) {
        // Signal shutdown to the SSE loop
        self.shutdown_flag.store(true, Ordering::Relaxed);
        self.disconnected_flag.store(true, Ordering::Relaxed);
        
        // Abort the current SSE task if it exists
        let mut handle_guard = self.sse_handle.lock().unwrap();
        if let Some(handle) = handle_guard.take() {
            handle.abort();
        }
        
        // Fail all pending requests (async block to handle the await)
        let pending = self.pending.clone();
        tokio::spawn(async move {
            let mut pending_guard = pending.lock().await;
            for (_, sender) in pending_guard.drain() {
                let _ = sender.send(Err(anyhow::anyhow!("Transport disconnected")));
            }
        });
    }
}

pub struct McpClient {
    transport: Arc<dyn Transport>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value>>>>>,
}

impl McpClient {
    pub async fn new(config: &McpTransport) -> Result<Self> {
        let pending = Arc::new(Mutex::new(
            HashMap::<String, oneshot::Sender<Result<Value>>>::new(),
        ));
        let pending_clone = pending.clone();

        let transport: Arc<dyn Transport> = match config {
            McpTransport::Stdio { command, args, env } => {
                let mut cmd = Command::new(command);
                cmd.args(args);
                cmd.envs(env);
                cmd.stdin(Stdio::piped());
                cmd.stdout(Stdio::piped());
                cmd.stderr(Stdio::piped()); // Capture stderr

                let mut child = cmd
                    .spawn()
                    .context(format!("Failed to spawn command: {}", command))?;

                let mut stdin = child.stdin.take().context("Failed to open stdin")?;
                let stdout = child.stdout.take().context("Failed to open stdout")?;
                let stderr = child.stderr.take().context("Failed to open stderr")?;

                let (tx, mut rx) = tokio::sync::mpsc::channel::<JsonRpcRequest>(32);
                let status = Arc::new(Mutex::new(TransportStatus::Connected));
                let stderr_log = Arc::new(Mutex::new(Vec::new()));

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

                // Stderr Loop
                let stderr_log_clone = stderr_log.clone();
                tokio::spawn(async move {
                    let reader = BufReader::new(stderr);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let mut log = stderr_log_clone.lock().await;
                        if log.len() >= 100 {
                            log.remove(0);
                        } // Keep last 100 lines
                        log.push(line);
                    }
                });

                // Exit Monitor
                let status_clone = status.clone();
                tokio::spawn(async move {
                    let _ = child.wait().await;
                    let mut s = status_clone.lock().await;
                    *s = TransportStatus::Disconnected("Process exited".to_string());
                });

                Arc::new(StdioTransport {
                    tx,
                    status,
                    stderr_log,
                })
            }
            McpTransport::Sse { url, headers } => {
                let client = HttpClient::new();
                let headers = build_header_map(headers)?;
                let session_id = Arc::new(Mutex::new(None));
                let sse_handle = Arc::new(std::sync::Mutex::new(None));
                let shutdown_flag = Arc::new(AtomicBool::new(false));
                let disconnected_flag = Arc::new(AtomicBool::new(false));

                Arc::new(SseTransport {
                    url: url.clone(),
                    client,
                    headers,
                    session_id,
                    sse_handle,
                    pending: pending_clone,
                    shutdown_flag,
                    disconnected_flag,
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

        struct CancelGuard {
            id: String,
            transport: Arc<dyn Transport>,
            completed: bool,
        }
        impl Drop for CancelGuard {
            fn drop(&mut self) {
                if !self.completed {
                    let id = self.id.clone();
                    let t = self.transport.clone();
                    tokio::spawn(async move {
                        let req = JsonRpcRequest {
                            jsonrpc: "2.0".to_string(),
                            id: None,
                            method: "notifications/cancelled".to_string(),
                            params: Some(json!({"requestId": id})),
                        };
                        let _ = t.send(req).await;
                    });
                }
            }
        }

        let mut guard = CancelGuard {
            id: id_val.clone(),
            transport: self.transport.clone(),
            completed: false,
        };

        match tokio::time::timeout(timeout, resp_rx).await {
            Ok(r) => {
                guard.completed = true;
                r.context("Response channel closed")?
            }
            Err(_) => {
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

    pub fn stop(&self) {
        self.transport.stop();
    }
}
