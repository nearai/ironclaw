//! Real Docker-backed [`SandboxTransport`].
//!
//! Speaks NDJSON to a `sandbox_daemon` process running inside a per-project
//! container via `docker exec -i`. The container itself is created and
//! managed by [`super::lifecycle`]; this module just owns the exec stream
//! and the request/response correlation.
//!
//! # v1 concurrency model
//!
//! Calls are serialized through a single `tokio::Mutex`. The daemon also
//! processes one request at a time, so adding host-side parallelism would
//! gain nothing without a multiplexed daemon. The plan documents this as a
//! future optimization.
//!
//! # IPC failures
//!
//! Anything that breaks the exec stream (container crash, daemon exit,
//! Docker socket dropping, JSON-parse errors, timeouts) is mapped to
//! [`MountError::Backend`] so the bridge surfaces it as `code=sandbox_error`
//! rather than as a normal tool error.

use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use bytes::Bytes;
use futures::StreamExt;
use ironclaw_engine::MountError;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use super::protocol::{Request, Response};
use super::transport::SandboxTransport;

/// Default per-call timeout. Long enough for `cargo build` and friends.
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(600);

/// A Docker exec session into one running container, with serialized
/// dispatch.
pub struct DockerTransport {
    docker: Docker,
    container_id: String,
    /// Holds the exec session state. Lazily created on the first call so
    /// `DockerTransport::new` doesn't need to be async.
    session: Mutex<Option<ExecSession>>,
    call_timeout: Duration,
}

impl std::fmt::Debug for DockerTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockerTransport")
            .field("container_id", &self.container_id)
            .field("call_timeout", &self.call_timeout)
            .finish()
    }
}

/// One live `docker exec -i sandbox_daemon` session.
struct ExecSession {
    /// Writer half of the exec stdin (what the host sends to the daemon).
    /// `Box::pin(...)` adds the `Unpin` bound that bollard's exec input
    /// stream doesn't carry directly.
    input: Pin<Box<dyn tokio::io::AsyncWrite + Send>>,
    /// Buffered reader over the exec stdout (NDJSON responses).
    output: BufReader<StreamReader>,
}

impl DockerTransport {
    pub fn new(docker: Docker, container_id: impl Into<String>) -> Self {
        Self {
            docker,
            container_id: container_id.into(),
            session: Mutex::new(None),
            call_timeout: DEFAULT_CALL_TIMEOUT,
        }
    }

    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    pub fn container_id(&self) -> &str {
        &self.container_id
    }

    /// Build a fresh exec session against the daemon binary inside the container.
    async fn open_session(&self) -> Result<ExecSession, MountError> {
        let exec = self
            .docker
            .create_exec(
                &self.container_id,
                CreateExecOptions {
                    cmd: Some(vec!["/usr/local/bin/sandbox_daemon"]),
                    attach_stdin: Some(true),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    tty: Some(false),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| MountError::Backend {
                reason: format!("create_exec failed: {e}"),
            })?;

        let started = self
            .docker
            .start_exec(
                &exec.id,
                Some(StartExecOptions {
                    detach: false,
                    tty: false,
                    output_capacity: None,
                }),
            )
            .await
            .map_err(|e| MountError::Backend {
                reason: format!("start_exec failed: {e}"),
            })?;

        match started {
            StartExecResults::Attached { input, output } => Ok(ExecSession {
                input,
                output: BufReader::new(StreamReader::new(output.boxed())),
            }),
            StartExecResults::Detached => Err(MountError::Backend {
                reason: "start_exec returned Detached but we asked for attached".into(),
            }),
        }
    }

    async fn ensure_session<'a>(
        &self,
        guard: &'a mut tokio::sync::MutexGuard<'_, Option<ExecSession>>,
    ) -> Result<&'a mut ExecSession, MountError> {
        if guard.is_none() {
            **guard = Some(self.open_session().await?);
        }
        guard.as_mut().ok_or_else(|| MountError::Backend {
            reason: "session disappeared immediately after creation".into(),
        })
    }
}

#[async_trait]
impl SandboxTransport for DockerTransport {
    async fn dispatch(&self, request: Request) -> Result<Response, MountError> {
        let mut guard = self.session.lock().await;

        // Ensure the session is open. If a previous call broke the stream,
        // re-open lazily.
        let outcome = async {
            let session = self.ensure_session(&mut guard).await?;
            tokio::time::timeout(self.call_timeout, dispatch_one(session, &request))
                .await
                .map_err(|_| MountError::Backend {
                    reason: format!(
                        "sandbox call '{}' timed out after {}s",
                        request.method,
                        self.call_timeout.as_secs()
                    ),
                })?
        }
        .await;

        if let Err(MountError::Backend { ref reason }) = outcome {
            warn!(
                container = %self.container_id,
                method = %request.method,
                error = %reason,
                "sandbox transport error; dropping exec session — next call will reconnect"
            );
            *guard = None;
        }

        outcome
    }
}

async fn dispatch_one(
    session: &mut ExecSession,
    request: &Request,
) -> Result<Response, MountError> {
    use tokio::io::AsyncBufReadExt;

    let mut bytes = serde_json::to_vec(request).map_err(|e| MountError::Backend {
        reason: format!("serialize request: {e}"),
    })?;
    bytes.push(b'\n');
    session
        .input
        .write_all(&bytes)
        .await
        .map_err(|e| MountError::Backend {
            reason: format!("write to daemon stdin: {e}"),
        })?;
    session
        .input
        .flush()
        .await
        .map_err(|e| MountError::Backend {
            reason: format!("flush daemon stdin: {e}"),
        })?;

    let mut line = String::new();
    let read = session
        .output
        .read_line(&mut line)
        .await
        .map_err(|e| MountError::Backend {
            reason: format!("read from daemon stdout: {e}"),
        })?;
    if read == 0 {
        return Err(MountError::Backend {
            reason: "daemon closed stdout (EOF)".into(),
        });
    }
    debug!(line = %line.trim_end(), "sandbox_daemon → host");
    serde_json::from_str(line.trim_end()).map_err(|e| MountError::Backend {
        reason: format!("parse daemon response: {e}; raw: {line}"),
    })
}

/// Wraps the bollard `LogOutput` byte stream into something `tokio::io::AsyncRead`
/// can consume so we can use `BufReader::read_line` on the stdout side.
struct StreamReader {
    stream: Pin<
        Box<
            dyn futures::Stream<
                    Item = Result<bollard::container::LogOutput, bollard::errors::Error>,
                > + Send,
        >,
    >,
    buffer: Bytes,
}

impl StreamReader {
    fn new(
        stream: Pin<
            Box<
                dyn futures::Stream<
                        Item = Result<bollard::container::LogOutput, bollard::errors::Error>,
                    > + Send,
            >,
        >,
    ) -> Self {
        Self {
            stream,
            buffer: Bytes::new(),
        }
    }
}

impl tokio::io::AsyncRead for StreamReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        use std::task::Poll;
        loop {
            if !self.buffer.is_empty() {
                let n = std::cmp::min(self.buffer.len(), buf.remaining());
                let chunk = self.buffer.split_to(n);
                buf.put_slice(&chunk);
                return Poll::Ready(Ok(()));
            }
            match self.stream.as_mut().poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Err(std::io::Error::other(format!("docker stream: {e}"))));
                }
                Poll::Ready(Some(Ok(log))) => {
                    use bollard::container::LogOutput;
                    self.buffer = match log {
                        LogOutput::StdOut { message } => message,
                        LogOutput::StdErr { message } => {
                            let text = String::from_utf8_lossy(&message);
                            debug!(stderr = %text.trim_end(), "sandbox_daemon stderr");
                            continue;
                        }
                        LogOutput::Console { message } => message,
                        LogOutput::StdIn { .. } => continue,
                    };
                }
            }
        }
    }
}
