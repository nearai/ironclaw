//! MCP stdio process manager.
//!
//! Manages the lifecycle of MCP servers running as child processes.
//! Handles spawning, shutdown, and crash recovery with exponential backoff.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use crate::tools::mcp::stdio_transport::StdioMcpTransport;
use crate::tools::mcp::transport::McpTransport;
use crate::tools::tool::ToolError;

/// Configuration for spawning a stdio MCP server.
#[derive(Debug, Clone)]
pub struct StdioSpawnConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// Manages stdio MCP server processes.
///
/// Handles spawning, tracking, and shutdown of child processes.
pub struct McpProcessManager {
    transports: RwLock<HashMap<String, Arc<StdioMcpTransport>>>,
    configs: RwLock<HashMap<String, StdioSpawnConfig>>,
}

impl McpProcessManager {
    pub fn new() -> Self {
        Self {
            transports: RwLock::new(HashMap::new()),
            configs: RwLock::new(HashMap::new()),
        }
    }

    /// Spawn a new stdio MCP server process.
    pub async fn spawn_stdio(
        &self,
        name: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Result<Arc<StdioMcpTransport>, ToolError> {
        let name = name.into();
        let command = command.into();

        // Store config for potential restart
        self.configs.write().await.insert(
            name.clone(),
            StdioSpawnConfig {
                command: command.clone(),
                args: args.clone(),
                env: env.clone(),
            },
        );

        let transport = Arc::new(StdioMcpTransport::spawn(&name, &command, args, env).await?);

        self.transports
            .write()
            .await
            .insert(name, Arc::clone(&transport));

        Ok(transport)
    }

    /// Get a transport by server name.
    pub async fn get(&self, name: &str) -> Option<Arc<StdioMcpTransport>> {
        self.transports.read().await.get(name).cloned()
    }

    /// Shut down all managed transports.
    pub async fn shutdown_all(&self) {
        let transports: Vec<(String, Arc<StdioMcpTransport>)> = {
            let mut map = self.transports.write().await;
            map.drain().collect()
        };

        for (name, transport) in transports {
            if let Err(e) = transport.shutdown().await {
                tracing::warn!("Failed to shut down MCP stdio server '{}': {}", name, e);
            }
        }
    }

    /// Shut down a specific transport by name.
    pub async fn shutdown(&self, name: &str) -> Result<(), ToolError> {
        let transport = self.transports.write().await.remove(name);

        if let Some(transport) = transport {
            transport.shutdown().await?;
        }

        self.configs.write().await.remove(name);
        Ok(())
    }

    /// Attempt to restart a crashed transport with exponential backoff.
    ///
    /// Tries up to 5 times with delays of 1s, 2s, 4s, 8s, 16s (total: 31s max wait).
    pub async fn try_restart(&self, name: &str) -> Result<Arc<StdioMcpTransport>, ToolError> {
        let config = self
            .configs
            .read()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| {
                ToolError::ExternalService(format!(
                    "No spawn config for MCP server '{}', cannot restart",
                    name
                ))
            })?;

        // Shut down and remove old transport to avoid orphaning a wedged process.
        if let Some(old_transport) = self.transports.write().await.remove(name) {
            let _ = old_transport.shutdown().await;
        }

        let max_retries = 5;
        let mut last_err = None;

        for attempt in 0..max_retries {
            let delay = Duration::from_secs(1 << attempt);
            tokio::time::sleep(delay).await;

            match StdioMcpTransport::spawn(
                name,
                &config.command,
                config.args.clone(),
                config.env.clone(),
            )
            .await
            {
                Ok(transport) => {
                    let transport = Arc::new(transport);
                    self.transports
                        .write()
                        .await
                        .insert(name.to_string(), Arc::clone(&transport));
                    tracing::info!(
                        "MCP stdio server '{}' restarted after {} attempt(s)",
                        name,
                        attempt + 1
                    );
                    return Ok(transport);
                }
                Err(e) => {
                    tracing::warn!(
                        "Restart attempt {}/{} for MCP server '{}' failed: {}",
                        attempt + 1,
                        max_retries,
                        name,
                        e
                    );
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            ToolError::ExternalService(format!(
                "Failed to restart MCP server '{}' after {} attempts",
                name, max_retries
            ))
        }))
    }

    /// Get names of all managed transports.
    pub async fn managed_servers(&self) -> Vec<String> {
        self.transports.read().await.keys().cloned().collect()
    }
}

impl Default for McpProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_empty_manager() {
        let _manager = McpProcessManager::new();
    }

    #[tokio::test]
    async fn test_managed_servers_returns_empty_list_initially() {
        let manager = McpProcessManager::new();
        let servers = manager.managed_servers().await;
        assert!(servers.is_empty());
    }

    #[tokio::test]
    async fn test_shutdown_all_on_empty_manager_does_not_panic() {
        let manager = McpProcessManager::new();
        manager.shutdown_all().await;
    }
}
