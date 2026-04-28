//! Model Context Protocol (MCP) integration.
//!
//! MCP allows the agent to connect to external tool servers that provide
//! additional capabilities through a standardized protocol.
//!
//! Supports both local (unauthenticated) and hosted (OAuth-authenticated) servers.
//! Transport options include HTTP (Streamable HTTP / SSE), stdio (subprocess),
//! and Unix domain sockets.
//!
//! ## Usage
//!
//! ```ignore
//! // Simple client (no auth)
//! let client = McpClient::new("http://localhost:8080");
//!
//! // Authenticated client (for hosted servers)
//! let client = McpClient::new_authenticated(
//!     config,
//!     session_manager,
//!     secrets,
//!     "user_id",
//! );
//!
//! // List and register tools
//! let tools = client.create_tools().await?;
//! for tool in tools {
//!     registry.register(tool);
//! }
//! ```

pub mod auth;
mod client;
pub(crate) mod client_store;
pub mod config;
pub mod factory;
pub(crate) mod http_transport;
pub(crate) mod process;
mod protocol;
pub mod session;
pub(crate) mod stdio_transport;
pub(crate) mod transport;
#[cfg(unix)]
pub(crate) mod unix_transport;

pub use auth::{is_authenticated, refresh_access_token};
pub use client::McpClient;
pub(crate) use client::mcp_tool_id;
pub(crate) use client_store::{McpClientStore, surface_signature};
pub use config::{McpServerConfig, McpServersFile, OAuthConfig};
pub use factory::{McpFactoryError, create_client_from_config};
pub use process::McpProcessManager;
pub use protocol::{InitializeResult, McpRequest, McpResponse, McpTool};
pub use session::McpSessionManager;
pub use transport::McpTransport;

/// Normalize a configured MCP server name to the runtime form used for
/// client-store keys and tool prefixes.
///
/// Persisted configs may still contain legacy hyphenated names, but MCP tool
/// wrappers and LLM-facing identifiers use underscore-only prefixes so model
/// tool calls and registry lookups agree.
pub(crate) fn normalize_server_name(name: &str) -> String {
    name.replace('-', "_")
}

fn contains_ascii_word(message: &str, word: &str) -> bool {
    message
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|candidate| candidate.eq_ignore_ascii_case(word))
}

pub(crate) fn is_auth_error_message(message: &str) -> bool {
    contains_ascii_word(message, "401")
        || contains_ascii_word(message, "unauthorized")
        || contains_ascii_word(message, "authentication")
        || (contains_ascii_word(message, "400")
            && (contains_ascii_word(message, "authorization")
                || contains_ascii_word(message, "authenticate")))
}

pub(crate) fn has_http_unauthorized_status(message: &str) -> bool {
    contains_ascii_word(message, "401")
}

#[cfg(test)]
mod tests {
    use super::{has_http_unauthorized_status, is_auth_error_message, normalize_server_name};

    #[test]
    fn test_is_auth_error_message_matches_whole_words() {
        assert!(is_auth_error_message("401 Unauthorized"));
        assert!(is_auth_error_message(
            "MCP error: Unauthorized (code -32001)"
        ));
        assert!(is_auth_error_message("request requires authentication."));
        assert!(is_auth_error_message(
            "400: Authorization header is badly formatted"
        ));
        assert!(is_auth_error_message(
            "400: please authenticate before retrying"
        ));
        assert!(!is_auth_error_message("localhost:4010 did not respond"));
        assert!(!is_auth_error_message("code 4001 authorization_cache_hit"));
        assert!(!is_auth_error_message("reauthentication required"));
        assert!(!is_auth_error_message("authorizations are cached"));
    }

    #[test]
    fn normalize_server_name_folds_hyphens_to_underscores() {
        assert_eq!(normalize_server_name("my-mcp-server"), "my_mcp_server");
        assert_eq!(normalize_server_name("nearai"), "nearai");
    }

    #[test]
    fn has_http_unauthorized_status_matches_401_only() {
        assert!(has_http_unauthorized_status(
            "[nearai] MCP server returned status: 401 Unauthorized"
        ));
        assert!(!has_http_unauthorized_status(
            "HTTP status 403. Update credential."
        ));
        assert!(!has_http_unauthorized_status("MCP error: Unauthorized"));
        assert!(!has_http_unauthorized_status(
            "localhost:4010 did not respond"
        ));
    }
}
