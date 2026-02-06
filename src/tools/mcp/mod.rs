//! Model Context Protocol (MCP) integration.
//!
//! MCP allows the agent to connect to external tool servers that provide
//! additional capabilities through a standardized protocol.
//!
//! Supports both local (unauthenticated) and hosted (OAuth-authenticated) servers.
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
pub mod config;
mod protocol;
pub mod session;

pub use auth::{is_authenticated, refresh_access_token};
pub use client::McpClient;
pub use config::{McpServerConfig, McpServersFile, OAuthConfig};
pub use protocol::{InitializeResult, McpRequest, McpResponse, McpTool};
pub use session::McpSessionManager;
