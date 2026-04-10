//! Factory for creating MCP clients from server configuration.
//!
//! Encapsulates the transport dispatch logic (stdio, Unix socket, HTTP)
//! so that callers don't need to match on `EffectiveTransport` themselves.

use std::sync::Arc;

use crate::secrets::SecretsStore;
use crate::tools::mcp::config::{EffectiveTransport, McpServerConfig};
use crate::tools::mcp::http_transport::HttpMcpTransport;
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
    #[error("Invalid configuration for MCP server '{name}': {reason}")]
    InvalidConfig { name: String, reason: String },
    #[error("{0}")]
    Transport(#[from] crate::tools::tool::ToolError),
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
            // Authenticated (OAuth) path: tokens exist or server requires auth.
            if let Some(ref secrets) = secrets {
                let has_tokens =
                    crate::tools::mcp::is_authenticated(&server, secrets, user_id).await;

                if has_tokens || server.requires_auth() {
                    return Ok(McpClient::new_authenticated(
                        server,
                        Arc::clone(session_manager),
                        Arc::clone(secrets),
                        user_id,
                    )?);
                }
            }

            // Non-OAuth HTTP: wire the session manager into the *transport* so
            // it captures `Mcp-Session-Id` from responses. Passing it only to
            // the client (via `with_session_manager`) is not enough — the
            // transport must know about it to read/write the header.
            let transport = Arc::new(
                HttpMcpTransport::new(server.url.clone(), server.name.clone())?
                    .with_session_manager(Arc::clone(session_manager)),
            );
            Ok(McpClient::new_with_transport(
                server.name.clone(),
                transport,
                Some(Arc::clone(session_manager)),
                secrets,
                user_id,
                Some(server),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use crate::secrets::{CreateSecretParams, InMemorySecretsStore, SecretsCrypto, SecretsStore};
    use crate::testing::credentials::TEST_CRYPTO_KEY;
    use crate::tools::mcp::OAuthConfig;
    use crate::tools::mcp::client::McpClientConstructor;

    fn empty_secrets_store() -> Arc<dyn SecretsStore + Send + Sync> {
        let key = secrecy::SecretString::from(TEST_CRYPTO_KEY.to_string());
        let crypto = Arc::new(SecretsCrypto::new(key).expect("test crypto"));
        Arc::new(InMemorySecretsStore::new(crypto))
    }

    /// Regression for nearai/ironclaw#1948 — caller-level coverage.
    ///
    /// `requires_auth()` had a unit test for the custom-Authorization-header
    /// case, but nothing exercised the *caller* of that predicate. This test
    /// drives `create_client_from_config` (the caller) and asserts the
    /// factory takes the non-auth construction path when the user has set
    /// their own Authorization header on a remote https MCP server.
    ///
    /// Without this caller-level coverage, a future change that bypassed
    /// `requires_auth()` (e.g. by inlining the localhost check, or by
    /// reordering the `has_tokens || requires_auth()` short-circuit) could
    /// re-introduce the bug while leaving the predicate unit tests green.
    ///
    /// See `.claude/rules/testing.md` ("Test Through the Caller, Not Just
    /// the Helper") for the rule and the bug history.
    #[tokio::test]
    async fn factory_takes_non_auth_path_when_authorization_header_set() {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            "Bearer sk-user-supplied".to_string(),
        );
        let server = McpServerConfig::new("authheader-1948", "https://api.example.com")
            .with_headers(headers);

        let secrets = empty_secrets_store();
        let session_manager = Arc::new(McpSessionManager::new());
        let process_manager = Arc::new(McpProcessManager::new());

        let client = create_client_from_config(
            server,
            &session_manager,
            &process_manager,
            Some(secrets),
            "test-user",
        )
        .await
        .expect("factory should succeed for https config with custom Authorization header");

        assert_eq!(
            client.constructor_kind(),
            McpClientConstructor::WithTransport,
            "factory must take the non-auth construction path when the user has \
             supplied an Authorization header — taking the OAuth/auth path triggers \
             unexpected DCR/refresh side effects (nearai/ironclaw#1948)"
        );
    }

    /// Regression for nearai/ironclaw#1948 — case-insensitive variant.
    ///
    /// HTTP header names are case-insensitive (RFC 9110). The factory must
    /// honor a custom `AUTHORIZATION` header even when OAuth metadata is
    /// pre-configured (the strongest signal that the user owns the
    /// credential path).
    #[tokio::test]
    async fn factory_takes_non_auth_path_with_uppercase_authorization_and_oauth_config() {
        let mut headers = HashMap::new();
        headers.insert(
            "AUTHORIZATION".to_string(),
            "Bearer sk-user-supplied".to_string(),
        );
        let server = McpServerConfig::new("authheader-1948-upper", "https://api.example.com")
            .with_headers(headers)
            .with_oauth(OAuthConfig::new("client-id"));

        let secrets = empty_secrets_store();
        let session_manager = Arc::new(McpSessionManager::new());
        let process_manager = Arc::new(McpProcessManager::new());

        let client = create_client_from_config(
            server,
            &session_manager,
            &process_manager,
            Some(secrets),
            "test-user",
        )
        .await
        .expect("factory should succeed even with both OAuth config and Authorization header");

        assert_eq!(
            client.constructor_kind(),
            McpClientConstructor::WithTransport,
            "an explicit Authorization header must win over pre-configured OAuth \
             metadata (nearai/ironclaw#1948)"
        );
    }

    /// Negative control for the same regression.
    ///
    /// Without a custom Authorization header, a remote https MCP server
    /// *should* take the auth construction path. If this test ever stops
    /// seeing `Authenticated`, the factory's call site has stopped honoring
    /// `requires_auth()` at all and the positive tests above are not
    /// actually proving anything.
    #[tokio::test]
    async fn factory_takes_auth_path_for_remote_https_without_authorization_header() {
        let server = McpServerConfig::new("noheader-1948", "https://api.example.com");

        let secrets = empty_secrets_store();
        let session_manager = Arc::new(McpSessionManager::new());
        let process_manager = Arc::new(McpProcessManager::new());

        let client = create_client_from_config(
            server,
            &session_manager,
            &process_manager,
            Some(secrets),
            "test-user",
        )
        .await
        .expect("factory should succeed for plain remote https config");

        assert_eq!(
            client.constructor_kind(),
            McpClientConstructor::Authenticated,
            "without an explicit Authorization header, the factory must enter \
             the auth construction path (otherwise the positive tests above \
             prove nothing — see nearai/ironclaw#1948)"
        );
    }

    /// Negative control: with stored OAuth tokens, the factory must take
    /// the auth path even when `requires_auth()` returns false. This pins
    /// the `has_tokens || requires_auth()` short-circuit so a refactor
    /// that drops the `has_tokens` clause does not silently break sessions
    /// that have already authenticated.
    #[tokio::test]
    async fn factory_takes_auth_path_when_tokens_already_stored() {
        // Localhost https — `requires_auth()` returns false, so without
        // the `has_tokens` short-circuit the factory would take the
        // non-auth path and the user would lose access to their stored
        // OAuth token.
        let server = McpServerConfig::new("stored-token-1948", "https://localhost:8443");

        let secrets = empty_secrets_store();
        secrets
            .create(
                "test-user",
                CreateSecretParams::new("mcp_stored-token-1948_access_token", "stored-oauth-token"),
            )
            .await
            .expect("seed token");

        let session_manager = Arc::new(McpSessionManager::new());
        let process_manager = Arc::new(McpProcessManager::new());

        let client = create_client_from_config(
            server,
            &session_manager,
            &process_manager,
            Some(secrets),
            "test-user",
        )
        .await
        .expect("factory should succeed for localhost https with stored token");

        assert_eq!(
            client.constructor_kind(),
            McpClientConstructor::Authenticated,
            "stored OAuth tokens must keep routing through the auth path even \
             when requires_auth() returns false; otherwise refresh stops working"
        );
    }

    #[tokio::test]
    async fn test_factory_non_oauth_http_has_session_manager() {
        let server = McpServerConfig::new("test-server", "http://localhost:9999");
        let session_manager = Arc::new(McpSessionManager::new());
        let process_manager = Arc::new(McpProcessManager::new());

        let client = create_client_from_config(
            server,
            &session_manager,
            &process_manager,
            None,
            "test-user",
        )
        .await
        .expect("factory should succeed for HTTP config");

        assert!(
            client.has_session_manager(),
            "non-OAuth HTTP clients must carry a session manager"
        );
    }

    /// Regression test: the factory must wire the session manager into the
    /// *transport*, not just the client. Otherwise the transport never
    /// captures `Mcp-Session-Id` from responses and subsequent requests
    /// lack the header, causing the server to reject them.
    #[tokio::test]
    async fn test_factory_non_oauth_http_transport_captures_session_id() {
        use axum::http::header::HeaderName;
        use axum::{Router, http::StatusCode, response::IntoResponse, routing::post};
        use tokio::net::TcpListener;

        const SESSION_ID: &str = "test-session-abc123";

        async fn session_echo() -> impl IntoResponse {
            let body = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {}
            })
            .to_string();
            (
                StatusCode::OK,
                [(
                    HeaderName::from_static("mcp-session-id"),
                    SESSION_ID.to_string(),
                )],
                body,
            )
        }

        let app = Router::new().route("/", post(session_echo));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://127.0.0.1:{}", addr.port());

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let server = McpServerConfig::new("session-test", &url);
        let session_manager = Arc::new(McpSessionManager::new());
        let process_manager = Arc::new(McpProcessManager::new());

        let client = create_client_from_config(
            server,
            &session_manager,
            &process_manager,
            None,
            "test-user",
        )
        .await
        .expect("factory should succeed for HTTP config");

        // Pre-create a session entry so that update_session_id has something to update.
        // In production, the MCP initialize handshake calls get_or_create before responses arrive.
        session_manager.get_or_create("session-test", &url).await;

        // Send a request through the client's transport to trigger session capture.
        use crate::tools::mcp::protocol::McpRequest;
        let request = McpRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(1),
            method: "test".to_string(),
            params: Some(serde_json::json!({})),
        };
        let headers = std::collections::HashMap::new();
        client
            .transport()
            .send(&request, &headers)
            .await
            .expect("request should succeed");

        // Verify the session manager captured the session ID from the response.
        let captured = session_manager.get_session_id("session-test").await;
        assert_eq!(
            captured.as_deref(),
            Some(SESSION_ID),
            "transport must capture Mcp-Session-Id into session manager"
        );
    }
}
