//! Model Context Protocol (MCP) integration.
//!
//! MCP allows the agent to connect to external tool servers that provide
//! additional capabilities through a standardized protocol.

mod client;
mod protocol;

pub use client::McpClient;
pub use protocol::{McpRequest, McpResponse, McpTool};
