//! Factory for creating MCP clients from server configuration.
//!
//! Encapsulates the transport dispatch logic (stdio, Unix socket, HTTP)
//! so that callers don't need to match on `EffectiveTransport` themselves.

use std::sync::Arc;

use crate::secrets::SecretsStore;
use crate::tools::mcp::config::{EffectiveTransport, McpServerConfig};
use crate::tools::mcp::{McpClient, McpProcessManager, McpSessionManager, McpTransport};

/// Error returned when MCP client creation fails.
#[derive(Debug, thiserror::Error)]
pub enum McpFactoryError {
    #[error("Failed to spawn stdio MCP server '{name}': {reason}")]
    StdioSpawn { name: String, reason: String },
    #[error("Failed to connect to Unix MCP server '{name}': {reason}")]
    UnixConnect { name: String, reason: String },
    #[error("Unix socket transport is not supported on this platform (server '{name}')")]
    UnixNotSupported { name: String },
}

/// Create an `McpClient` from a server configuration, dispatching on the
/// effective transport type.
pub async fn create_client_from_config(
    server: McpServerConfig,
    session_manager: &Arc<McpSessionManager>,
    process_manager: &Arc<McpProcessManager>,
    secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
    user_id: &str,
) -> Result<McpClient, McpFactoryError> {
    let server_name = server.name.clone();

    match server.effective_transport() {
        EffectiveTransport::Stdio { command, args, env } => {
            let transport = process_manager
                .spawn_stdio(&server_name, command, args.to_vec(), env.clone())
                .await
                .map_err(|e| McpFactoryError::StdioSpawn {
                    name: server_name.clone(),
                    reason: e.to_string(),
                })?;

            Ok(McpClient::new_with_transport(
                &server_name,
                transport as Arc<dyn McpTransport>,
                None,
                secrets,
                user_id,
                Some(server),
            ))
        }
        #[cfg(unix)]
        EffectiveTransport::Unix { socket_path } => {
            let transport = crate::tools::mcp::unix_transport::UnixMcpTransport::connect(
                &server_name,
                socket_path,
            )
            .await
            .map_err(|e| McpFactoryError::UnixConnect {
                name: server_name.clone(),
                reason: e.to_string(),
            })?;

            Ok(McpClient::new_with_transport(
                &server_name,
                Arc::new(transport) as Arc<dyn McpTransport>,
                None,
                secrets,
                user_id,
                Some(server),
            ))
        }
        #[cfg(not(unix))]
        EffectiveTransport::Unix { .. } => {
            Err(McpFactoryError::UnixNotSupported { name: server_name })
        }
        EffectiveTransport::Http => {
            if let Some(ref secrets) = secrets {
                let has_tokens =
                    crate::tools::mcp::is_authenticated(&server, secrets, user_id).await;

                if has_tokens || server.requires_auth() {
                    Ok(McpClient::new_authenticated(
                        server,
                        Arc::clone(session_manager),
                        Arc::clone(secrets),
                        user_id,
                    ))
                } else {
                    Ok(McpClient::new_with_config(server)
                        .with_session_manager(Arc::clone(session_manager)))
                }
            } else {
                Ok(McpClient::new_with_config(server)
                    .with_session_manager(Arc::clone(session_manager)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::mcp::config::McpServerConfig;

    /// Regression: non-OAuth HTTP clients must have a session manager so
    /// Streamable HTTP servers receive `Mcp-Session-Id` headers.
    #[tokio::test]
    async fn test_factory_non_oauth_http_has_session_manager() {
        let server = McpServerConfig::new("test-server", "http://localhost:9999");
        let sm = Arc::new(McpSessionManager::new());
        let pm = Arc::new(McpProcessManager::new());

        // No secrets → non-OAuth path
        let client = create_client_from_config(server, &sm, &pm, None, "test-user")
            .await
            .expect("factory should succeed for HTTP config");

        assert!(
            client.has_session_manager(),
            "Non-OAuth HTTP client must have session manager attached"
        );
    }
}
