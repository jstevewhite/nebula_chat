use crate::mcp::client::McpClient;
use crate::mcp::config::{McpServerConfig, Settings};
use anyhow::Result;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct McpManager {
    clients: RwLock<HashMap<String, Arc<McpClient>>>,
    starting: RwLock<HashSet<String>>,
    tool_cache: RwLock<HashMap<String, Vec<crate::llm::provider::ToolDefinition>>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
            starting: RwLock::new(HashSet::new()),
            tool_cache: RwLock::new(HashMap::new()),
        }
    }

    pub async fn shutdown(&self) {
        // Stop all clients cleanly before clearing
        let client_names: Vec<String> = {
            let clients = self.clients.read().await;
            clients.keys().cloned().collect()
        };
        
        for name in client_names {
            if let Some(client) = self.get_client(&name).await {
                client.stop();
            }
        }

        let mut clients = self.clients.write().await;
        clients.clear();
        let mut starting = self.starting.write().await;
        starting.clear();
        let mut tool_cache = self.tool_cache.write().await;
        tool_cache.clear();
    }

    fn init_params() -> serde_json::Value {
        json!({
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
        })
    }

    async fn try_mark_starting(&self, name: &str) -> bool {
        // Lock ordering: starting -> clients
        let mut starting = self.starting.write().await;
        if starting.contains(name) {
            return false;
        }
        let clients = self.clients.read().await;
        if clients.contains_key(name) {
            return false;
        }
        starting.insert(name.to_string());
        true
    }

    async fn clear_starting(&self, name: &str) {
        let mut starting = self.starting.write().await;
        starting.remove(name);
    }

    async fn start_client(&self, name: &str, config: &McpServerConfig) -> Result<McpClient> {
        // Create Client
        let client = McpClient::new(&config.transport).await?;

        // Perform Handshake
        let init_params = Self::init_params();

        tracing::info!("Sending initialize request to {}", name);
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

        Ok(client)
    }

    pub async fn initialize(&self, settings: Settings) -> Result<()> {
        for (name, config) in settings.mcp_servers {
            if !self.try_mark_starting(&name).await {
                continue;
            }

            println!("Starting MCP server: {}", name);

            let start_result = self.start_client(&name, &config).await;
            self.clear_starting(&name).await;

            match start_result {
                Ok(client) => {
                    let mut clients = self.clients.write().await;
                    clients.insert(name.clone(), Arc::new(client));
                    // Check if we need to clear cache? New server, no cache yet.
                }
                Err(e) => {
                    eprintln!("Failed to start MCP server {}: {}", name, e);
                }
            }
        }

        Ok(())
    }

    pub async fn restart_server(&self, name: String, config: McpServerConfig) -> Result<()> {
        // Prevent concurrent restarts/starts of the same server.
        {
            let mut starting = self.starting.write().await;
            if starting.contains(&name) {
                return Err(anyhow::anyhow!("Server '{}' is already starting", name));
            }
            starting.insert(name.clone());
        }

        // Stop existing client cleanly before removing
        if let Some(client) = self.get_client(&name).await {
            client.stop();
        }

        // Remove existing client (short lock)
        {
            let mut clients = self.clients.write().await;
            clients.remove(&name);
            // Invalidate cache
            let mut cache = self.tool_cache.write().await;
            cache.remove(&name);
        }

        println!("Restarting MCP server: {}", name);

        let start_result = self.start_client(&name, &config).await;
        self.clear_starting(&name).await;

        match start_result {
            Ok(client) => {
                let mut clients = self.clients.write().await;
                clients.insert(name, Arc::new(client));
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub async fn get_client(&self, name: &str) -> Option<Arc<McpClient>> {
        self.clients.read().await.get(name).cloned()
    }

    pub async fn list_servers(&self) -> Vec<String> {
        self.clients
            .read()
            .await
            .iter()
            .filter(|(_, client)| client.is_connected())
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub async fn remove_server(&self, name: &str) {
        // Stop the client cleanly before removing
        if let Some(client) = self.get_client(name).await {
            client.stop();
        }

        {
            let mut clients = self.clients.write().await;
            clients.remove(name);
        }
        {
            let mut starting = self.starting.write().await;
            starting.remove(name);
        }
        {
            let mut cache = self.tool_cache.write().await;
            cache.remove(name);
        }
    }

    pub async fn get_all_tools(&self) -> Vec<crate::llm::provider::ToolDefinition> {
        // Snapshots keys to avoid holding lock while iterating/fetching
        let client_map: HashMap<String, Arc<McpClient>> = self.clients.read().await.clone();
        let mut all_tools = Vec::new();

        for (name, client) in client_map {
            if !client.is_connected() {
                continue;
            }

            // Check Cache
            {
                let cache = self.tool_cache.read().await;
                if let Some(tools) = cache.get(&name) {
                    all_tools.extend(tools.clone());
                    continue;
                }
            }

            // Fetch
            if let Ok(resp) = client.request("tools/list", None).await {
                if let Some(tools) = resp.get("tools").and_then(|t| t.as_array()) {
                    let mut server_tools = Vec::new();
                    for tool in tools {
                        let t_name = tool["name"].as_str().unwrap_or("unknown");
                        let t_desc = tool["description"].as_str().unwrap_or("").to_string();
                        let t_schema = tool["inputSchema"].clone();

                        let unique_name = format!("{}__{}", name, t_name);
                        server_tools.push(crate::llm::provider::ToolDefinition {
                            name: unique_name,
                            description: t_desc,
                            input_schema: t_schema,
                        });
                    }

                    // Update cache
                    let mut cache = self.tool_cache.write().await;
                    cache.insert(name.clone(), server_tools.clone());

                    all_tools.extend(server_tools);
                }
            }
        }
        all_tools
    }

    pub async fn get_server_for_tool(&self, tool_name: &str) -> Option<String> {
        let parts: Vec<&str> = tool_name.splitn(2, "__").collect();
        if parts.len() == 2 {
            Some(parts[0].to_string())
        } else {
            None
        }
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
