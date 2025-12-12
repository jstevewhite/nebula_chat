use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, oneshot};
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

pub struct McpClient {
    tx: tokio::sync::mpsc::Sender<JsonRpcRequest>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value>>>>>,
}

impl McpClient {
    pub async fn new(cmd: &str, args: &[String], env: &HashMap<String, String>) -> Result<Self> {
        let mut command = Command::new(cmd);
        command.args(args);
        command.envs(env);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::inherit()); 

        let mut child = command.spawn().context(format!("Failed to spawn command: {}", cmd))?;

        let mut stdin = child.stdin.take().context("Failed to open stdin")?;
        let stdout = child.stdout.take().context("Failed to open stdout")?;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<JsonRpcRequest>(32);
        let pending = Arc::new(Mutex::new(HashMap::<String, oneshot::Sender<Result<Value>>>::new()));
        let pending_clone = pending.clone();

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

        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // Log line for debug?
                if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(&line) {
                     if let Some(id) = resp.id {
                        let id_str = match id {
                            Value::String(s) => s,
                            Value::Number(n) => n.to_string(),
                            _ => id.to_string(), 
                        };
                        
                        let mut map = pending_clone.lock().await;
                        if let Some(sender) = map.remove(&id_str) {
                            if let Some(err) = resp.error {
                                let _ = sender.send(Err(anyhow::anyhow!("RPC Error {}: {}", err.code, err.message)));
                            } else {
                                let _ = sender.send(Ok(resp.result.unwrap_or(Value::Null)));
                            }
                        }
                     }
                }
            }
        });

        Ok(Self { tx, pending })
    }

    pub async fn request(&self, method: &str, params: Option<Value>) -> Result<Value> {
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
            map.insert(id_val, resp_tx);
        }

        if self.tx.send(req).await.is_err() {
            // Remove from map if send failed
            // Actually hard to remove efficiently without key here, but it's fine (leak small if sending fails which means client closed)
            return Err(anyhow::anyhow!("Client closed"));
        }

        resp_rx.await.context("Response channel closed")?
    }

    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<()> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: method.to_string(),
            params,
        };
        self.tx.send(req).await.context("Client closed")?;
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        !self.tx.is_closed()
    }
}
