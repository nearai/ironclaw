//! NDJSON wire protocol shared with the `sandbox_daemon` binary.
//!
//! Both sides — the host's `ContainerizedFilesystemBackend` and the in-container
//! `sandbox_daemon` — serialize/deserialize the same Serde types so the
//! protocol stays in lockstep. The daemon copy lives in
//! `src/bin/sandbox_daemon.rs`; if you change anything here, change it there
//! too. (A future cleanup can move both into a shared crate — see Phase 7.)

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Tool names the host may dispatch to the daemon. The daemon also accepts
/// the v1 aliases (`read_file`, `write_file`) but the host always speaks the
/// v2 names so the wire format is unambiguous.
pub const SUPPORTED_TOOLS: &[&str] = &[
    "file_read",
    "file_write",
    "list_dir",
    "apply_patch",
    "shell",
];

/// One JSON-RPC request line sent to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Correlation id. Required for `execute_tool`; optional for `health` /
    /// `shutdown` but the host always sets it.
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Request {
    /// Build an `execute_tool` request for `tool_name` with the given input.
    pub fn execute_tool(id: impl Into<String>, tool_name: &str, input: Value) -> Self {
        Self {
            id: id.into(),
            method: "execute_tool".into(),
            params: serde_json::json!({ "name": tool_name, "input": input }),
        }
    }

    /// Build a `health` request.
    pub fn health(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            method: "health".into(),
            params: Value::Null,
        }
    }

    /// Build a `shutdown` request.
    pub fn shutdown(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            method: "shutdown".into(),
            params: Value::Null,
        }
    }
}

/// One JSON-RPC response line returned by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

/// Error envelope. The `code` is one of:
///
/// - `tool_error` — the tool ran but reported a normal failure (NotFound,
///   non-zero exit, etc.). Surface to the LLM.
/// - `invalid_params` — the request was malformed (missing field, wrong
///   type). Bug in the host or the LLM-supplied params.
/// - `parse_error` — the daemon could not parse the JSON line at all.
/// - `unknown_method` — the host sent a method the daemon doesn't know.
/// - `sandbox_error` / `backend` — infrastructure failure on the daemon side.
/// - `rate_limited` — tool returned a rate-limit error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execute_tool_round_trip() {
        let req = Request::execute_tool("abc", "file_read", serde_json::json!({"path": "/x"}));
        let json = serde_json::to_string(&req).unwrap();
        let parsed: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "abc");
        assert_eq!(parsed.method, "execute_tool");
        assert_eq!(parsed.params["name"], "file_read");
        assert_eq!(parsed.params["input"]["path"], "/x");
    }

    #[test]
    fn response_with_error_round_trips() {
        let resp = Response {
            id: Some("1".into()),
            result: None,
            error: Some(RpcError {
                code: "tool_error".into(),
                message: "boom".into(),
                details: Value::Null,
            }),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: Response = serde_json::from_str(&json).unwrap();
        assert!(parsed.result.is_none());
        assert_eq!(parsed.error.unwrap().code, "tool_error");
    }
}
