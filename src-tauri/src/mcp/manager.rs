use crate::mcp::client::McpClient;
use crate::mcp::config::{McpServerConfig, Settings};
use anyhow::Result;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct McpManager {
    clients: RwLock<HashMap<String, Arc<McpClient>>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
        }
    }

    pub async fn shutdown(&self) {
        let mut clients = self.clients.write().await;
        clients.clear();
    }

    pub async fn initialize(&self, settings: Settings) -> Result<()> {
        let mut clients = self.clients.write().await;

        for (name, config) in settings.mcp_servers {
            if clients.contains_key(&name) {
                continue;
            }

            println!("Starting MCP server: {}", name);
            if let Err(e) = self
                .start_server_internal(&mut clients, &name, &config)
                .await
            {
                eprintln!("Failed to start MCP server {}: {}", name, e);
            }
        }
        Ok(())
    }

    async fn start_server_internal(
        &self,
        clients: &mut HashMap<String, Arc<McpClient>>,
        name: &str,
        config: &McpServerConfig,
    ) -> Result<()> {
        // Create Client
        let client = McpClient::new(&config.transport).await?;

        // Perform Handshake
        let init_params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "roots": {
                    "listChanged": true
                },
                "sampling": {}
            },
            "clientInfo": {
                "name": "Nebula",
                "version": "0.1.0"
            }
        });

        match client.request("initialize", Some(init_params)).await {
            Ok(resp) => {
                println!("Server {} initialized: {:?}", name, resp);
                if let Err(e) = client.notify("notifications/initialized", None).await {
                    eprintln!("Failed to send initialized notification to {}: {}", name, e);
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to initialize handshake: {}", e));
            }
        }

        clients.insert(name.to_string(), Arc::new(client));
        Ok(())
    }

    pub async fn restart_server(&self, name: String, config: McpServerConfig) -> Result<()> {
        let mut clients = self.clients.write().await;
        // Remove existing
        clients.remove(&name);

        println!("Restarting MCP server: {}", name);
        self.start_server_internal(&mut clients, &name, &config)
            .await
    }

    pub async fn get_client(&self, name: &str) -> Option<Arc<McpClient>> {
        self.clients.read().await.get(name).cloned()
    }

    pub async fn list_servers(&self) -> Vec<String> {
        self.clients
            .read()
            .await
            .iter()
            //    .filter(|(_, client)| client.is_connected()) // Show all for now, or maybe only connected
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub async fn get_all_tools(&self) -> Vec<crate::llm::provider::ToolDefinition> {
        let clients = self.clients.read().await;
        let mut all_tools = Vec::new();

        for (name, client) in clients.iter() {
            if !client.is_connected() {
                continue;
            }
            // Mcp lists tools via tools/list
            if let Ok(resp) = client.request("tools/list", None).await {
                // Parse resp
                if let Some(tools) = resp.get("tools").and_then(|t| t.as_array()) {
                    for tool in tools {
                        let t_name = tool["name"].as_str().unwrap_or("unknown");
                        let t_desc = tool["description"].as_str().unwrap_or("").to_string();
                        let t_schema = tool["inputSchema"].clone();

                        // Namesmace tools: server__toolname
                        let unique_name = format!("{}__{}", name, t_name);

                        all_tools.push(crate::llm::provider::ToolDefinition {
                            name: unique_name,
                            description: t_desc,
                            input_schema: t_schema,
                        });
                    }
                }
            }
        }
        all_tools
    }

    pub async fn call_tool(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let parts: Vec<&str> = name.splitn(2, "__").collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!("Invalid tool name format"));
        }
        let server_name = parts[0];
        let tool_name = parts[1];

        if let Some(client) = self.get_client(server_name).await {
            let params = json!({
                "name": tool_name,
                "arguments": args
            });
            return client.request("tools/call", Some(params)).await;
        }
        Err(anyhow::anyhow!("Server not found"))
    }
}
