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

/// Composite key for a stdio MCP child process: the activating user
/// plus the server name. Both fields participate in `Hash` / `Eq` so
/// two users activating the same server name each get — and keep —
/// their own child process instead of one silently overwriting the
/// other's transport handle.
///
/// Stdio MCP servers receive credentials via their spawn `env` map, so
/// sharing a single child across users would leak one tenant's
/// credentials to the other's dispatches. Per-user children are
/// required; the process manager must track them independently.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct McpProcessKey {
    pub user_id: String,
    pub server_name: String,
}

impl McpProcessKey {
    pub fn new(user_id: &str, server_name: &str) -> Self {
        Self {
            user_id: user_id.to_string(),
            server_name: server_name.to_string(),
        }
    }
}

/// Manages stdio MCP server processes.
///
/// Handles spawning, tracking, and shutdown of child processes. Keyed
/// by `(user_id, server_name)` so that multiple tenants activating
/// the same server name end up with distinct, independently tracked
/// child processes — see `McpProcessKey` for the rationale.
pub struct McpProcessManager {
    transports: RwLock<HashMap<McpProcessKey, Arc<StdioMcpTransport>>>,
    configs: RwLock<HashMap<McpProcessKey, StdioSpawnConfig>>,
}

impl McpProcessManager {
    pub fn new() -> Self {
        Self {
            transports: RwLock::new(HashMap::new()),
            configs: RwLock::new(HashMap::new()),
        }
    }

    /// Spawn a new stdio MCP server process for `(user_id,
    /// server_name)`. If an entry already exists for the same pair
    /// (same-user re-activation), the existing process is shut down
    /// first so the replacement doesn't leave an orphan. Other users'
    /// processes on the same `server_name` are untouched.
    pub async fn spawn_stdio(
        &self,
        user_id: &str,
        name: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Result<Arc<StdioMcpTransport>, ToolError> {
        let name = name.into();
        let command = command.into();
        let key = McpProcessKey::new(user_id, &name);

        // Same-user re-activation: shut the previous child down before
        // the new one takes its slot so the old process doesn't become
        // an orphan.
        //
        // CRITICAL: the write-guard from `transports.write().await` is
        // dropped at the end of the enclosing `let` statement (inside
        // this block), BEFORE we `.await` the shutdown. Holding the
        // guard across the await would block every other caller of
        // the process manager for the duration of `shutdown()` (which
        // can take seconds if the child is wedged) and risk a
        // deadlock if any shutdown path ever re-enters the manager.
        let previous = {
            let mut map = self.transports.write().await;
            map.remove(&key)
        };
        if let Some(old_transport) = previous
            && let Err(e) = old_transport.shutdown().await
        {
            tracing::warn!(
                user_id = %user_id,
                server = %name,
                error = %e,
                "Failed to shut down previous stdio MCP child before replacement"
            );
        }

        // Store config for potential restart
        self.configs.write().await.insert(
            key.clone(),
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
            .insert(key, Arc::clone(&transport));

        Ok(transport)
    }

    /// Get a transport by `(user_id, server_name)`.
    pub async fn get(&self, user_id: &str, name: &str) -> Option<Arc<StdioMcpTransport>> {
        self.transports
            .read()
            .await
            .get(&McpProcessKey::new(user_id, name))
            .cloned()
    }

    /// Shut down all managed transports.
    pub async fn shutdown_all(&self) {
        let transports: Vec<(McpProcessKey, Arc<StdioMcpTransport>)> = {
            let mut map = self.transports.write().await;
            map.drain().collect()
        };

        for (key, transport) in transports {
            if let Err(e) = transport.shutdown().await {
                tracing::warn!(
                    user_id = %key.user_id,
                    server = %key.server_name,
                    error = %e,
                    "Failed to shut down MCP stdio server",
                );
            }
        }
    }

    /// Shut down the transport for `(user_id, server_name)`.
    pub async fn shutdown(&self, user_id: &str, name: &str) -> Result<(), ToolError> {
        let key = McpProcessKey::new(user_id, name);
        let transport = self.transports.write().await.remove(&key);

        if let Some(transport) = transport {
            transport.shutdown().await?;
        }

        self.configs.write().await.remove(&key);
        Ok(())
    }

    /// Attempt to restart a crashed transport for `(user_id,
    /// server_name)` with exponential backoff.
    ///
    /// Tries up to 5 times with delays of 1s, 2s, 4s, 8s, 16s (total: 31s max wait).
    pub async fn try_restart(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Arc<StdioMcpTransport>, ToolError> {
        let key = McpProcessKey::new(user_id, name);
        let config = self
            .configs
            .read()
            .await
            .get(&key)
            .cloned()
            .ok_or_else(|| {
                ToolError::ExternalService(format!(
                    "No spawn config for MCP server '{}' (user {}), cannot restart",
                    name, user_id
                ))
            })?;

        // Shut down and remove old transport to avoid orphaning a
        // wedged process. The write-guard is scoped to the inner
        // block so it's released BEFORE awaiting `shutdown()` — see
        // the matching rationale in `spawn_stdio`.
        let previous = {
            let mut map = self.transports.write().await;
            map.remove(&key)
        };
        if let Some(old_transport) = previous {
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
                        .insert(key.clone(), Arc::clone(&transport));
                    tracing::info!(
                        user_id = %user_id,
                        server = %name,
                        "MCP stdio server restarted after {} attempt(s)",
                        attempt + 1
                    );
                    return Ok(transport);
                }
                Err(e) => {
                    tracing::warn!(
                        user_id = %user_id,
                        server = %name,
                        "Restart attempt {}/{} failed: {}",
                        attempt + 1,
                        max_retries,
                        e
                    );
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            ToolError::ExternalService(format!(
                "Failed to restart MCP server '{}' (user {}) after {} attempts",
                name, user_id, max_retries
            ))
        }))
    }

    /// Get `(user_id, server_name)` pairs of all managed transports.
    pub async fn managed_servers(&self) -> Vec<McpProcessKey> {
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

    #[test]
    fn test_process_key_partitions_by_user_and_server() {
        let k1 = McpProcessKey::new("user-a", "stdio_server");
        let k2 = McpProcessKey::new("user-b", "stdio_server");
        let k3 = McpProcessKey::new("user-a", "other_server");
        let k1_dup = McpProcessKey::new("user-a", "stdio_server");

        assert_ne!(k1, k2, "different users on same server must not collide");
        assert_ne!(k1, k3, "same user on different servers must not collide");
        assert_eq!(k1, k1_dup, "same (user, server) must be equal");
    }
}
