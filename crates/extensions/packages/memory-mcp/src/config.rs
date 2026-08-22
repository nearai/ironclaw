//! Which tools this provider calls, and what it calls them with.
//!
//! The MCP server URL, credentials, and TLS belong to the
//! [`crate::McpMemoryTransport`] implementation, not here. This struct carries
//! only the vendor-shaped naming the provider needs to build a tool call.
//!
//! Tool NAMES are configuration rather than constants because that is the whole
//! point of this crate: a second memory system that speaks the same lane
//! semantics under different tool names binds by editing config, not by adding a
//! Rust crate. The defaults are the names Mnesis Core publishes in its
//! `manifests/mcp-tools.json`, since it is the first implementer.

use serde::{Deserialize, Serialize};

/// Default long-term retrieval tool (`read_long_term` lane).
pub const DEFAULT_SEARCH_TOOL: &str = "memory_search";
/// Default interaction-recording tool (`record_interaction` lane).
pub const DEFAULT_RECORD_TOOL: &str = "memory_add_session";

/// Tool naming for one MCP-backed memory provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpMemoryConfig {
    /// Tool called for the long-term retrieval lane.
    #[serde(default = "default_search_tool")]
    pub search_tool: String,
    /// Tool called for the interaction-recording lane. `None` declares that this
    /// provider does not record; the lane then reports `recorded: false` rather
    /// than calling anything. Keep it consistent with the `[memory].lifecycle`
    /// hooks the manifest declares — an undeclared hook is never called at all.
    #[serde(default = "default_record_tool")]
    pub record_tool: Option<String>,
}

fn default_search_tool() -> String {
    DEFAULT_SEARCH_TOOL.to_string()
}

fn default_record_tool() -> Option<String> {
    Some(DEFAULT_RECORD_TOOL.to_string())
}

impl Default for McpMemoryConfig {
    fn default() -> Self {
        Self {
            search_tool: default_search_tool(),
            record_tool: default_record_tool(),
        }
    }
}

impl McpMemoryConfig {
    /// Config with the default (Mnesis-shaped) tool names.
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the long-term retrieval tool name.
    pub fn with_search_tool(mut self, tool: impl Into<String>) -> Self {
        self.search_tool = tool.into();
        self
    }

    /// Override the interaction-recording tool name.
    pub fn with_record_tool(mut self, tool: impl Into<String>) -> Self {
        self.record_tool = Some(tool.into());
        self
    }

    /// Declare that this provider records nothing.
    pub fn without_record_tool(mut self) -> Self {
        self.record_tool = None;
        self
    }
}
