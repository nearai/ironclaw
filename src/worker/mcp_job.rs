//! Background-job descriptor for running a single MCP tool call as a durable
//! IronClaw job (Phase 2 of the MCP background-jobs design).
//!
//! The job "mode" lives in `JobContext.metadata` (a `serde_json::Value`) rather
//! than the orchestrator `JobMode` enum, which is not on this dispatch path.

use serde::{Deserialize, Serialize};

/// Everything needed to run one MCP tool call as a background job and inject
/// its result back into the originating agent thread on completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpJobSpec {
    /// MCP server name (e.g. "msbsandbox").
    pub server: String,
    /// Unprefixed MCP tool name (e.g. "run_python").
    pub tool: String,
    /// Tool call parameters.
    pub params: serde_json::Value,
    /// Owning user.
    pub user_id: String,
    /// Originating channel, used to inject the result on completion.
    pub channel: String,
    /// Originating thread, if any.
    pub thread_id: Option<String>,
}

impl McpJobSpec {
    /// The metadata blob persisted on the job's `JobContext`. Identity fields
    /// (`user_id` / `channel` / `thread_id`) live on the context itself, not
    /// here, so they are intentionally omitted.
    pub fn to_metadata(&self) -> serde_json::Value {
        serde_json::json!({
            "mode": "mcp_tool",
            "server": self.server,
            "tool": self.tool,
            "params": self.params,
        })
    }

    /// Rebuild a spec from a persisted metadata blob plus the context-supplied
    /// identity fields. Returns `None` unless `mode == "mcp_tool"` and the
    /// required `server` / `tool` fields are present.
    pub fn from_metadata(
        meta: &serde_json::Value,
        user_id: &str,
        channel: &str,
        thread_id: Option<String>,
    ) -> Option<Self> {
        if meta.get("mode").and_then(|m| m.as_str()) != Some("mcp_tool") {
            return None;
        }
        let server = meta.get("server").and_then(|v| v.as_str())?.to_string();
        let tool = meta.get("tool").and_then(|v| v.as_str())?.to_string();
        let params = meta
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        Some(Self {
            server,
            tool,
            params,
            user_id: user_id.to_string(),
            channel: channel.to_string(),
            thread_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_roundtrip() {
        let spec = McpJobSpec {
            server: "msbsandbox".into(),
            tool: "run_python".into(),
            params: serde_json::json!({"code":"print(1)"}),
            user_id: "u1".into(),
            channel: "gateway".into(),
            thread_id: Some("t1".into()),
        };
        let meta = spec.to_metadata();
        assert_eq!(meta["mode"], "mcp_tool");
        let back = McpJobSpec::from_metadata(&meta, "u1", "gateway", Some("t1".into())).unwrap();
        assert_eq!(back.server, "msbsandbox");
        assert_eq!(back.tool, "run_python");
        assert_eq!(back.params["code"], "print(1)");
        assert!(
            McpJobSpec::from_metadata(&serde_json::json!({"mode":"other"}), "u", "c", None)
                .is_none()
        );
    }

    #[test]
    fn from_metadata_requires_server_and_tool() {
        // mode is right but required fields missing → None (no panic).
        let meta = serde_json::json!({"mode":"mcp_tool","server":"s"});
        assert!(McpJobSpec::from_metadata(&meta, "u", "c", None).is_none());
    }
}
