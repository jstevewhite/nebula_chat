use crate::mcp::client::McpClient;
use crate::mcp::config::{McpServerConfig, Settings};
use anyhow::Result;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

/// A tool advertised by an MCP server, paired with the original (un-sanitized)
/// tool name. The outward-facing `def.name` is namespaced and may be rewritten
/// to satisfy provider tool-name rules (see `sanitize_tool_name`), so we keep
/// the server's real tool name here to route `tools/call` back correctly.
#[derive(Clone)]
struct ToolEntry {
    def: crate::llm::provider::ToolDefinition,
    original_name: String,
}

/// Anthropic (custom tools), Amazon Bedrock, and the OpenAI tool schema all
/// require tool names to match `^[a-zA-Z0-9_-]{1,128}$`. MCP servers can
/// advertise names containing `.`, `:`, `/`, spaces, etc., and the
/// `<server>__<tool>` prefix can push the combined name past 128 chars — either
/// gets the whole request rejected with a 400. Replace any disallowed
/// character with `_` and clamp the length. Routing back to the real server +
/// tool goes through the tool cache (`ToolEntry`), so rewriting the
/// outward-facing name is safe.
fn sanitize_tool_name(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // All retained chars are single-byte ASCII, so truncating by byte length is
    // also a valid char boundary.
    if out.len() > 128 {
        out.truncate(128);
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

pub struct McpManager {
    clients: RwLock<HashMap<String, Arc<McpClient>>>,
    starting: RwLock<HashSet<String>>,
    tool_cache: RwLock<HashMap<String, Vec<ToolEntry>>>,
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
        tracing::info!("Creating MCP client for {}: {:?}", name, config.transport);
        let client = McpClient::new(&config.transport).await?;
        tracing::info!("MCP client created successfully for {}", name);

        // Perform Handshake
        let init_params = Self::init_params();

        tracing::info!("Sending initialize request to {}", name);
        match client.request("initialize", Some(init_params)).await {
            Ok(resp) => {
                tracing::info!("Server {} initialized: {:?}", name, resp);
                println!("Server {} initialized: {:?}", name, resp);
                if let Err(e) = client.notify("notifications/initialized", None).await {
                    tracing::error!("Failed to send initialized notification to {}: {}", name, e);
                    eprintln!("Failed to send initialized notification to {}: {}", name, e);
                }
            }
            Err(e) => {
                tracing::error!("Failed to initialize handshake for {}: {}", name, e);
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
                    all_tools.extend(tools.iter().map(|e| e.def.clone()));
                    continue;
                }
            }

            // Fetch
            if let Ok(resp) = client.request("tools/list", None).await {
                if let Some(tools) = resp.get("tools").and_then(|t| t.as_array()) {
                    let mut server_tools = Vec::new();
                    // Sanitizing can collapse distinct raw names onto the same
                    // string (e.g. `read.file` and `read:file`); track emitted
                    // names within this server and disambiguate so neither tool
                    // silently shadows the other.
                    let mut seen: HashSet<String> = HashSet::new();
                    for tool in tools {
                        let t_name = tool["name"].as_str().unwrap_or("unknown");
                        let t_desc = tool["description"].as_str().unwrap_or("").to_string();
                        let t_schema = tool["inputSchema"].clone();

                        let mut unique_name =
                            sanitize_tool_name(&format!("{}__{}", name, t_name));
                        if !seen.insert(unique_name.clone()) {
                            let mut suffix = 2;
                            loop {
                                let candidate = format!("{}_{}", unique_name, suffix);
                                if seen.insert(candidate.clone()) {
                                    unique_name = candidate;
                                    break;
                                }
                                suffix += 1;
                            }
                        }
                        server_tools.push(ToolEntry {
                            def: crate::llm::provider::ToolDefinition {
                                name: unique_name,
                                description: t_desc,
                                input_schema: t_schema,
                            },
                            original_name: t_name.to_string(),
                        });
                    }

                    // Update cache
                    let mut cache = self.tool_cache.write().await;
                    cache.insert(name.clone(), server_tools.clone());

                    all_tools.extend(server_tools.iter().map(|e| e.def.clone()));
                }
            }
        }
        all_tools
    }

    pub async fn get_server_for_tool(&self, tool_name: &str) -> Option<String> {
        // Preferred: resolve via the cache, since the outward-facing name may
        // have been sanitized and no longer split cleanly on "__".
        {
            let cache = self.tool_cache.read().await;
            for (server, entries) in cache.iter() {
                if entries.iter().any(|e| e.def.name == tool_name) {
                    return Some(server.clone());
                }
            }
        }
        // Fallback: legacy "<server>__<tool>" split for names not in the cache.
        let parts: Vec<&str> = tool_name.splitn(2, "__").collect();
        if parts.len() == 2 {
            Some(parts[0].to_string())
        } else {
            // Tools are advertised as "<server>__<tool>"; a bare name means we
            // can't route it. Log so misconfigured tools don't silently vanish
            // from the routing layer.
            tracing::warn!(
                "get_server_for_tool: cannot route tool '{}' — expected '<server>__<tool>' format",
                tool_name
            );
            None
        }
    }

    pub async fn call_tool(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        // Recover the real server + the server's original tool name from the
        // cache. The `name` the model produced is what we advertised, which may
        // be a sanitized alias whose "__" split no longer matches reality.
        let route = {
            let cache = self.tool_cache.read().await;
            cache.iter().find_map(|(server, entries)| {
                entries
                    .iter()
                    .find(|e| e.def.name == name)
                    .map(|e| (server.clone(), e.original_name.clone()))
            })
        };

        let (server_name, tool_name) = match route {
            Some(route) => route,
            None => {
                // Fallback: legacy split for names not present in the cache.
                let parts: Vec<&str> = name.splitn(2, "__").collect();
                if parts.len() != 2 {
                    return Err(anyhow::anyhow!("Invalid tool name format"));
                }
                (parts[0].to_string(), parts[1].to_string())
            }
        };

        if let Some(client) = self.get_client(&server_name).await {
            let params = json!({
                "name": tool_name,
                "arguments": args
            });
            return client.request("tools/call", Some(params)).await;
        }
        Err(anyhow::anyhow!("Server not found"))
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_tool_name;

    fn is_api_safe(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 128
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }

    #[test]
    fn leaves_already_valid_names_untouched() {
        for name in ["filesystem__read_file", "github__create-issue", "a", "A1_-z"] {
            assert_eq!(sanitize_tool_name(name), name);
            assert!(is_api_safe(name));
        }
    }

    #[test]
    fn replaces_disallowed_characters() {
        // Dots, colons, slashes, and spaces are the common MCP offenders that
        // trip Anthropic/Bedrock's `^[a-zA-Z0-9_-]{1,128}$` validation.
        assert_eq!(sanitize_tool_name("server__read.file"), "server__read_file");
        assert_eq!(sanitize_tool_name("srv__ns:tool"), "srv__ns_tool");
        assert_eq!(sanitize_tool_name("srv__a/b c"), "srv__a_b_c");
        assert!(is_api_safe(&sanitize_tool_name("srv__weird@name!")));
    }

    #[test]
    fn clamps_overlong_names_to_128() {
        let long = format!("server__{}", "x".repeat(200));
        let out = sanitize_tool_name(&long);
        assert_eq!(out.len(), 128);
        assert!(is_api_safe(&out));
    }

    #[test]
    fn never_produces_empty_name() {
        assert!(!sanitize_tool_name("").is_empty());
        assert!(is_api_safe(&sanitize_tool_name("")));
    }
}
