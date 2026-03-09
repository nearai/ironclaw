//! Unix domain socket transport for MCP servers.
//!
//! Connects to an existing Unix socket and communicates using
//! newline-delimited JSON-RPC.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;

use crate::tools::mcp::protocol::{McpRequest, McpResponse};
use crate::tools::mcp::transport::{McpTransport, spawn_jsonrpc_reader, write_jsonrpc_line};
use crate::tools::tool::ToolError;

/// MCP transport that communicates over a Unix domain socket.
///
/// Connects to an existing Unix socket at the given path. Requests are
/// written as newline-delimited JSON to the write half, and responses are
/// read from the read half by a background reader task.
pub struct UnixMcpTransport {
    socket_path: PathBuf,
    server_name: String,
    writer: Arc<Mutex<tokio::io::WriteHalf<UnixStream>>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<McpResponse>>>>,
    reader_handle: Mutex<Option<JoinHandle<()>>>,
}

impl UnixMcpTransport {
    /// Connect to an existing Unix domain socket and create a transport.
    ///
    /// # Arguments
    ///
    /// * `name` - Human-readable server name for logging.
    /// * `socket_path` - Path to the Unix domain socket.
    pub async fn connect(
        name: impl Into<String>,
        socket_path: impl AsRef<Path>,
    ) -> Result<Self, ToolError> {
        let server_name = name.into();
        let socket_path = socket_path.as_ref().to_path_buf();

        let stream = UnixStream::connect(&socket_path).await.map_err(|e| {
            ToolError::ExternalService(format!(
                "[{}] Failed to connect to Unix socket '{}': {}",
                server_name,
                socket_path.display(),
                e
            ))
        })?;

        let (read_half, write_half) = tokio::io::split(stream);

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<McpResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let reader = BufReader::new(read_half);
        let reader_handle = spawn_jsonrpc_reader(reader, pending.clone(), server_name.clone());

        Ok(Self {
            socket_path,
            server_name,
            writer: Arc::new(Mutex::new(write_half)),
            pending,
            reader_handle: Mutex::new(Some(reader_handle)),
        })
    }

    /// Get the path to the Unix domain socket.
    #[cfg(test)]
    pub(crate) fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Get the server name.
    #[cfg(test)]
    pub(crate) fn server_name(&self) -> &str {
        &self.server_name
    }
}

#[async_trait]
impl McpTransport for UnixMcpTransport {
    async fn send(
        &self,
        request: &McpRequest,
        _headers: &HashMap<String, String>,
    ) -> Result<McpResponse, ToolError> {
        let (tx, rx) = oneshot::channel();

        // Register the pending response handler before writing the request,
        // so we don't miss a fast response from the server.
        {
            let mut pending = self.pending.lock().await;
            pending.insert(request.id.unwrap_or(0), tx);
        }

        // Write the request to the socket.
        {
            let mut writer = self.writer.lock().await;
            if let Err(e) = write_jsonrpc_line(&mut *writer, request).await {
                // Remove the pending entry on write failure.
                let mut pending = self.pending.lock().await;
                pending.remove(&request.id.unwrap_or(0));
                return Err(e);
            }
        }

        // Wait for the response with a timeout.
        let timeout = Duration::from_secs(30);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => {
                // Sender was dropped (reader task ended). Clean up pending entry.
                let mut pending = self.pending.lock().await;
                pending.remove(&request.id.unwrap_or(0));
                Err(ToolError::ExternalService(format!(
                    "[{}] MCP server closed connection before responding to request {:?}",
                    self.server_name, request.id
                )))
            }
            Err(_) => {
                // Timeout: remove the pending entry.
                let mut pending = self.pending.lock().await;
                pending.remove(&request.id.unwrap_or(0));
                Err(ToolError::ExternalService(format!(
                    "[{}] Timeout waiting for response to request {:?} after {:?}",
                    self.server_name, request.id, timeout
                )))
            }
        }
    }

    async fn shutdown(&self) -> Result<(), ToolError> {
        // Abort the reader task.
        if let Some(handle) = self.reader_handle.lock().await.take() {
            handle.abort();
        }

        // Drain pending requests so waiters wake immediately instead of
        // hanging until their 30s timeout.
        {
            let mut pending = self.pending.lock().await;
            pending.clear(); // Dropping senders wakes receivers with Err
        }

        tracing::debug!(
            "[{}] Unix transport shut down (socket: {})",
            self.server_name,
            self.socket_path.display()
        );
        Ok(())
    }

    fn supports_http_features(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn test_connect_nonexistent_socket_fails() {
        let tmp_dir = tempfile::tempdir().expect("create temp dir");
        let socket_path = tmp_dir.path().join("nonexistent.sock");

        let result = UnixMcpTransport::connect("test", &socket_path).await;

        let err = result.err().expect("should be an error").to_string();
        assert!(
            err.contains("Failed to connect"),
            "Error should mention connection failure: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_round_trip_via_unix_socket() {
        // Create a temporary directory for the socket.
        let tmp_dir = tempfile::tempdir().expect("create temp dir");
        let socket_path = tmp_dir.path().join("test.sock");

        // Bind a listener on the socket.
        let listener = UnixListener::bind(&socket_path).expect("bind listener");

        // Spawn an echo handler that reads one JSON-RPC request and writes
        // back a valid McpResponse with the same id.
        let handler = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept connection");
            let (read_half, mut write_half) = tokio::io::split(stream);
            let mut reader = TokioBufReader::new(read_half);
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .await
                .expect("read request line");

            // Parse the request to extract the id.
            let req: McpRequest = serde_json::from_str(&line).expect("parse request");

            // Build a valid response.
            let response = McpResponse {
                jsonrpc: "2.0".to_string(),
                id: req.id,
                result: Some(serde_json::json!({"tools": []})),
                error: None,
            };

            let mut resp_bytes = serde_json::to_vec(&response).expect("serialize response");
            resp_bytes.push(b'\n');
            write_half
                .write_all(&resp_bytes)
                .await
                .expect("write response");
            write_half.flush().await.expect("flush");
        });

        // Connect to the socket via our transport.
        let transport = UnixMcpTransport::connect("test-uds", &socket_path)
            .await
            .expect("connect should succeed");

        assert_eq!(transport.socket_path(), socket_path.as_path());
        assert_eq!(transport.server_name(), "test-uds");

        // Send a list_tools request and verify the round-trip.
        let request = McpRequest::list_tools(42);
        let headers = HashMap::new();
        let response = transport.send(&request, &headers).await.expect("send");

        assert_eq!(response.id, Some(42));
        assert!(response.result.is_some());
        assert!(response.error.is_none());

        // Clean up.
        transport.shutdown().await.expect("shutdown");
        handler.await.expect("handler task");
    }

    #[tokio::test]
    async fn test_shutdown_is_idempotent() {
        let tmp_dir = tempfile::tempdir().expect("create temp dir");
        let socket_path = tmp_dir.path().join("idle.sock");

        let listener = UnixListener::bind(&socket_path).expect("bind listener");

        // Accept in the background so the connect succeeds.
        let _handler = tokio::spawn(async move {
            let _stream = listener.accept().await;
        });

        let transport = UnixMcpTransport::connect("test-idle", &socket_path)
            .await
            .expect("connect");

        // Calling shutdown twice should not panic or error.
        transport.shutdown().await.expect("first shutdown");
        transport.shutdown().await.expect("second shutdown");
    }
}
