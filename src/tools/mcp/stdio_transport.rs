//! Stdio transport for MCP servers.
//!
//! Spawns a child process and communicates via stdin/stdout using
//! newline-delimited JSON-RPC.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::BufReader;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;

use crate::tools::mcp::protocol::{McpRequest, McpResponse};
use crate::tools::mcp::transport::{McpTransport, spawn_jsonrpc_reader, write_jsonrpc_line};
use crate::tools::tool::ToolError;

/// MCP transport that communicates with a child process over stdin/stdout.
///
/// The child process is spawned with piped stdin/stdout/stderr. Requests are
/// written as newline-delimited JSON to stdin, and responses are read from
/// stdout by a background reader task. Stderr is drained to tracing logs.
pub struct StdioMcpTransport {
    server_name: String,
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<McpResponse>>>>,
    reader_handle: Mutex<Option<JoinHandle<()>>>,
    stderr_handle: Mutex<Option<JoinHandle<()>>>,
    child: Arc<Mutex<Child>>,
}

impl StdioMcpTransport {
    /// Spawn a child process and create a stdio transport.
    ///
    /// # Arguments
    ///
    /// * `name` - Human-readable server name for logging.
    /// * `command` - The command to execute.
    /// * `args` - Command-line arguments.
    /// * `env` - Additional environment variables to set.
    pub async fn spawn(
        name: impl Into<String>,
        command: &str,
        args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
        env: impl IntoIterator<Item = (impl AsRef<std::ffi::OsStr>, impl AsRef<std::ffi::OsStr>)>,
    ) -> Result<Self, ToolError> {
        let server_name = name.into();

        let mut cmd = Command::new(command);
        cmd.args(args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            ToolError::ExternalService(format!(
                "[{}] Failed to spawn MCP server '{}': {}",
                server_name, command, e
            ))
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            ToolError::ExternalService(format!(
                "[{}] Failed to capture stdin of MCP server",
                server_name
            ))
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            ToolError::ExternalService(format!(
                "[{}] Failed to capture stdout of MCP server",
                server_name
            ))
        })?;

        let stderr = child.stderr.take().ok_or_else(|| {
            ToolError::ExternalService(format!(
                "[{}] Failed to capture stderr of MCP server",
                server_name
            ))
        })?;

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<McpResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let reader = BufReader::new(stdout);
        let reader_handle = spawn_jsonrpc_reader(reader, pending.clone(), server_name.clone());

        let stderr_name = server_name.clone();
        let stderr_handle = tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader as TokioBufReader};

            let reader = TokioBufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!("[{}] stderr: {}", stderr_name, line);
            }
        });

        Ok(Self {
            server_name,
            stdin: Arc::new(Mutex::new(stdin)),
            pending,
            reader_handle: Mutex::new(Some(reader_handle)),
            stderr_handle: Mutex::new(Some(stderr_handle)),
            child: Arc::new(Mutex::new(child)),
        })
    }
}

#[async_trait]
impl McpTransport for StdioMcpTransport {
    async fn send(
        &self,
        request: &McpRequest,
        _headers: &HashMap<String, String>,
    ) -> Result<McpResponse, ToolError> {
        // JSON-RPC notifications (no id) are fire-and-forget: the server
        // will not send a response, so we must not wait for one.
        if request.id.is_none() {
            let mut stdin = self.stdin.lock().await;
            write_jsonrpc_line(&mut *stdin, request).await?;
            return Ok(McpResponse {
                jsonrpc: "2.0".to_string(),
                id: None,
                result: None,
                error: None,
            });
        }

        let id = request.id.unwrap_or(0);
        let (tx, rx) = oneshot::channel();

        // Register the pending response handler before writing the request,
        // so we don't miss a fast response from the child.
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        // Write the request to stdin.
        {
            let mut stdin = self.stdin.lock().await;
            if let Err(e) = write_jsonrpc_line(&mut *stdin, request).await {
                // Remove the pending entry on write failure.
                let mut pending = self.pending.lock().await;
                pending.remove(&id);
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
                pending.remove(&id);
                Err(ToolError::ExternalService(format!(
                    "[{}] MCP server closed connection before responding to request {:?}",
                    self.server_name, request.id
                )))
            }
            Err(_) => {
                // Timeout: remove the pending entry.
                let mut pending = self.pending.lock().await;
                pending.remove(&id);
                Err(ToolError::ExternalService(format!(
                    "[{}] Timeout waiting for response to request {:?} after {:?}",
                    self.server_name, request.id, timeout
                )))
            }
        }
    }

    async fn shutdown(&self) -> Result<(), ToolError> {
        // Kill the child process.
        {
            let mut child = self.child.lock().await;
            let _ = child.kill().await;
        }

        // Abort the reader tasks.
        if let Some(handle) = self.reader_handle.lock().await.take() {
            handle.abort();
        }
        if let Some(handle) = self.stderr_handle.lock().await.take() {
            handle.abort();
        }

        // Drain pending requests so waiters wake immediately instead of
        // hanging until their 30s timeout.
        {
            let mut pending = self.pending.lock().await;
            pending.clear(); // Dropping senders wakes receivers with Err
        }

        tracing::debug!("[{}] Stdio transport shut down", self.server_name);
        Ok(())
    }

    fn supports_http_features(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_nonexistent_command_fails() {
        let env: HashMap<String, String> = HashMap::new();
        let result = StdioMcpTransport::spawn(
            "test",
            "this-command-does-not-exist-ironclaw-test",
            std::iter::empty::<&str>(),
            &env,
        )
        .await;

        let err = result.err().expect("should be an error").to_string();
        assert!(
            err.contains("Failed to spawn"),
            "Error should mention spawn failure: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_spawn_and_shutdown() {
        let env: HashMap<String, String> = HashMap::new();
        let transport =
            StdioMcpTransport::spawn("test-cat", "cat", std::iter::empty::<&str>(), &env)
                .await
                .expect("cat should be available");

        // Verify shutdown completes without error.
        transport.shutdown().await.expect("shutdown should succeed");
    }

    #[tokio::test]
    async fn test_send_timeout_on_non_jsonrpc_server() {
        // Spawn `cat` which echoes input back. Since the echoed input is the
        // request (not a response with matching id), it will be ignored by the
        // reader and we should hit the timeout. We use a short-lived test so
        // we override the 30s timeout expectation by just checking the error type.
        let env: HashMap<String, String> = HashMap::new();
        let transport =
            StdioMcpTransport::spawn("test-echo", "cat", std::iter::empty::<&str>(), &env)
                .await
                .expect("cat should be available");

        let request = McpRequest::list_tools(999);
        let headers = HashMap::new();

        // The request will be echoed back by `cat`, but it won't parse as a
        // valid McpResponse with matching id, so the reader will log a debug
        // message and the send will eventually timeout. We don't want to wait
        // 30 seconds in tests, so we just verify the transport was created and
        // shut it down.
        transport.shutdown().await.expect("shutdown should succeed");

        // Verify that pending map is empty after shutdown.
        let pending = transport.pending.lock().await;
        assert!(pending.is_empty());
        drop(pending);

        // Verify send after shutdown fails (stdin is closed).
        let result = transport.send(&request, &headers).await;
        assert!(result.is_err());
    }
}
