//! Shared MCP transport trait and JSON-RPC framing helpers.
//!
//! Provides the [`McpTransport`] trait that all MCP transports implement,
//! plus `write_jsonrpc_line` and `spawn_jsonrpc_reader` for newline-delimited
//! JSON-RPC over byte streams (used by stdio and unix socket transports).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;

use crate::tools::mcp::protocol::{McpRequest, McpResponse};
use crate::tools::tool::ToolError;

/// Trait for sending JSON-RPC requests to an MCP server and receiving responses.
///
/// Implementations handle the underlying transport (HTTP, stdio, unix socket, etc.).
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a request and wait for the corresponding response.
    ///
    /// `headers` are used by HTTP-based transports (e.g., `Mcp-Session-Id`);
    /// stream-based transports may ignore them.
    async fn send(
        &self,
        request: &McpRequest,
        headers: &HashMap<String, String>,
    ) -> Result<McpResponse, ToolError>;

    /// Shut down the transport, releasing any resources (child processes, connections).
    async fn shutdown(&self) -> Result<(), ToolError>;

    /// Whether this transport supports HTTP-specific features like session headers.
    fn supports_http_features(&self) -> bool {
        false
    }
}

/// Serialize an [`McpRequest`] as a single JSON line and write it to `writer`.
///
/// The line is terminated with `\n` and the writer is flushed.
pub async fn write_jsonrpc_line(
    writer: &mut (impl AsyncWrite + Unpin),
    request: &McpRequest,
) -> Result<(), ToolError> {
    let json = serde_json::to_string(request).map_err(|e| {
        ToolError::ExternalService(format!("Failed to serialize JSON-RPC request: {e}"))
    })?;

    writer.write_all(json.as_bytes()).await.map_err(|e| {
        ToolError::ExternalService(format!("Failed to write JSON-RPC request: {e}"))
    })?;

    writer
        .write_all(b"\n")
        .await
        .map_err(|e| ToolError::ExternalService(format!("Failed to write newline: {e}")))?;

    writer
        .flush()
        .await
        .map_err(|e| ToolError::ExternalService(format!("Failed to flush JSON-RPC writer: {e}")))?;

    Ok(())
}

/// Spawn a background task that reads newline-delimited JSON-RPC responses from
/// `reader` and dispatches them to the matching pending sender in `pending`.
///
/// Each line is parsed as an [`McpResponse`]. If the response has an `id` that
/// matches a pending request, the corresponding [`oneshot::Sender`] is resolved.
/// Parse failures are logged at debug level and skipped.
pub fn spawn_jsonrpc_reader<R: AsyncBufRead + Unpin + Send + 'static>(
    reader: R,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<McpResponse>>>>,
    server_name: String,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let response = match serde_json::from_str::<McpResponse>(&line) {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::debug!(
                        "[{}] Failed to parse JSON-RPC response: {} — line: {}",
                        server_name,
                        e,
                        line
                    );
                    continue;
                }
            };

            let id = response.id;
            let mut map = pending.lock().await;
            if let Some(tx) = map.remove(&id) {
                // Ignore send error — the receiver may have been dropped (timeout).
                let _ = tx.send(response);
            } else {
                tracing::debug!(
                    "[{}] Received response for unknown request id {}",
                    server_name,
                    id
                );
            }
        }

        tracing::debug!("[{}] JSON-RPC reader finished", server_name);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_write_jsonrpc_line_serializes_and_flushes() {
        let request = McpRequest {
            jsonrpc: "2.0".into(),
            id: 1,
            method: "test/method".into(),
            params: None,
        };

        let mut buf = Vec::new();
        write_jsonrpc_line(&mut buf, &request)
            .await
            .expect("write should succeed");

        let written = String::from_utf8(buf).expect("should be valid UTF-8");
        assert!(written.ends_with('\n'));

        let parsed: serde_json::Value =
            serde_json::from_str(written.trim()).expect("should be valid JSON");
        assert_eq!(parsed["id"], 1);
        assert_eq!(parsed["method"], "test/method");
    }

    #[tokio::test]
    async fn test_spawn_jsonrpc_reader_dispatches_response() {
        let response = McpResponse {
            jsonrpc: "2.0".into(),
            id: 42,
            result: Some(serde_json::json!({"tools": []})),
            error: None,
        };
        let line = format!("{}\n", serde_json::to_string(&response).unwrap());

        let reader = std::io::Cursor::new(line.into_bytes());
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<McpResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let (tx, rx) = oneshot::channel();
        {
            let mut map = pending.lock().await;
            map.insert(42, tx);
        }

        let handle = spawn_jsonrpc_reader(reader, pending.clone(), "test".into());

        let resp = rx.await.expect("should receive response");
        assert_eq!(resp.id, 42);
        assert!(resp.result.is_some());

        handle.await.expect("reader task should finish");
    }

    #[tokio::test]
    async fn test_spawn_jsonrpc_reader_skips_invalid_lines() {
        let input = "this is not json\n{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":null}\n";
        let reader = std::io::Cursor::new(input.as_bytes().to_vec());
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<McpResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let (tx, rx) = oneshot::channel();
        {
            let mut map = pending.lock().await;
            map.insert(7, tx);
        }

        let handle = spawn_jsonrpc_reader(reader, pending.clone(), "test".into());

        let resp = rx
            .await
            .expect("should receive response despite earlier invalid line");
        assert_eq!(resp.id, 7);

        handle.await.expect("reader task should finish");
    }
}
