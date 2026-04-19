//! Unix Socket REPL Server Channel.
//!
//! Provides a REPL interface over a Unix domain socket so external clients
//! (e.g. `ironclaw repl`) can connect to a running Ironclaw daemon without
//! starting a second instance.
//!
//! # Protocol
//!
//! Newline-delimited JSON using the [`ReplMessage`] enum.  Each party sends
//! one complete JSON object per line.  The handshake is:
//!
//! 1. Client → Server: `Connect { version, client_info }`
//! 2. Server → Client: `Response { content: "welcome …", is_complete: true }`
//! 3. Client → Server: `Message { content }` (repeated)
//! 4. Server → Client: `Response { content }` (repeated)
//! 5. Client → Server: `Disconnect` (or EOF)

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, warn};

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse};
use crate::error::ChannelError;

/// Wire protocol for Unix Socket REPL communication.
#[derive(Debug, Serialize, Deserialize)]
pub enum ReplMessage {
    /// Client → Server: initial handshake.
    Connect {
        version: String,
        client_info: Option<String>,
    },
    /// Client → Server: user input.
    Message {
        content: String,
        session_id: Option<String>,
    },
    /// Server → Client: agent response or welcome message.
    Response {
        content: String,
        session_id: Option<String>,
        is_complete: bool,
    },
    /// Client → Server: graceful disconnect.
    Disconnect {
        session_id: Option<String>,
        reason: Option<String>,
    },
    /// Keepalive ping (either direction).
    Ping,
    /// Keepalive pong (either direction).
    Pong,
}

/// Session registry: session_id → socket write half.
///
/// Only the write half is stored here so the per-connection read loop (which
/// owns the read half) and the channel's [`Channel::respond`] (which looks up
/// the write half) never conflict.
type SessionMap = Arc<RwLock<HashMap<String, OwnedWriteHalf>>>;

/// Unix Socket REPL channel.
///
/// Listens on a Unix domain socket path and allows multiple simultaneous REPL
/// clients to talk to the running daemon.
pub struct UnixSocketReplChannel {
    socket_path: PathBuf,
    max_connections: usize,
    /// Owner user ID forwarded to the agent for persistence scoping.
    owner_id: String,
    sessions: SessionMap,
}

impl UnixSocketReplChannel {
    /// Create a new channel that listens on `socket_path`.
    ///
    /// `max_connections` caps simultaneous clients (default: 10).
    /// `owner_id` should match the configured instance owner.
    pub fn new(
        socket_path: impl Into<PathBuf>,
        max_connections: usize,
        owner_id: impl Into<String>,
    ) -> Self {
        Self {
            socket_path: socket_path.into(),
            max_connections,
            owner_id: owner_id.into(),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Serialize and write a [`ReplMessage`] as a newline-terminated JSON line.
    async fn write_message(write: &mut OwnedWriteHalf, msg: &ReplMessage) -> Result<(), ChannelError> {
        let mut line = serde_json::to_string(msg)
            .map_err(|e| ChannelError::InvalidMessage(e.to_string()))?;
        line.push('\n');
        write
            .write_all(line.as_bytes())
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "unix_socket_repl".to_string(),
                reason: e.to_string(),
            })
    }

    /// Handle one client connection from accept to disconnect.
    async fn handle_connection(
        read: OwnedReadHalf,
        mut write: OwnedWriteHalf,
        message_tx: mpsc::Sender<IncomingMessage>,
        sessions: SessionMap,
        max_connections: usize,
        owner_id: String,
    ) {
        let session_id = uuid::Uuid::new_v4().simple().to_string();

        // ── Handshake ────────────────────────────────────────────────
        let mut reader = BufReader::new(read);
        let mut buf = String::new();
        match reader.read_line(&mut buf).await {
            Ok(0) => return, // EOF before handshake
            Ok(_) => {}
            Err(e) => {
                warn!(%e, "unix socket REPL: handshake read error");
                return;
            }
        }

        match serde_json::from_str::<ReplMessage>(&buf) {
            Ok(ReplMessage::Connect { version, .. }) => {
                info!(%version, %session_id, "unix socket REPL client connected");
            }
            Ok(other) => {
                warn!(?other, "unix socket REPL: expected Connect, got other message");
                return;
            }
            Err(e) => {
                warn!(%e, "unix socket REPL: handshake parse error");
                return;
            }
        }

        // ── Register session ─────────────────────────────────────────
        {
            let mut map = sessions.write().await;
            if map.len() >= max_connections {
                warn!(%max_connections, "unix socket REPL: max connections reached");
                // Send a friendly error before dropping
                let _ = Self::write_message(
                    &mut write,
                    &ReplMessage::Response {
                        content: format!(
                            "Server busy: maximum connections ({max_connections}) reached."
                        ),
                        session_id: Some(session_id.clone()),
                        is_complete: true,
                    },
                )
                .await;
                return;
            }

            // Send welcome before inserting so we own `write` exclusively here
            let welcome = ReplMessage::Response {
                content: format!("Connected to Ironclaw REPL. Session: {session_id}"),
                session_id: Some(session_id.clone()),
                is_complete: true,
            };
            if let Err(e) = Self::write_message(&mut write, &welcome).await {
                warn!(%e, "unix socket REPL: failed to send welcome");
                return;
            }

            map.insert(session_id.clone(), write);
        }

        // ── Message loop ─────────────────────────────────────────────
        loop {
            buf.clear();
            match reader.read_line(&mut buf).await {
                Ok(0) => {
                    debug!(%session_id, "unix socket REPL client disconnected (EOF)");
                    break;
                }
                Ok(_) => {
                    match serde_json::from_str::<ReplMessage>(&buf) {
                        Ok(ReplMessage::Message { content, .. }) => {
                            let msg = IncomingMessage::new(
                                "unix_socket_repl",
                                &owner_id,
                                content,
                            )
                            .with_sender_id(session_id.clone())
                            .with_metadata(
                                serde_json::json!({ "unix_session_id": &session_id }),
                            );
                            if message_tx.send(msg).await.is_err() {
                                break;
                            }
                        }
                        Ok(ReplMessage::Ping) => {
                            let mut map = sessions.write().await;
                            if let Some(w) = map.get_mut(&session_id) {
                                let _ = Self::write_message(w, &ReplMessage::Pong).await;
                            }
                        }
                        Ok(ReplMessage::Disconnect { .. }) => {
                            debug!(%session_id, "unix socket REPL client requested disconnect");
                            break;
                        }
                        Ok(other) => {
                            warn!(?other, "unix socket REPL: unexpected message type");
                        }
                        Err(e) => {
                            warn!(%e, "unix socket REPL: message parse error");
                        }
                    }
                }
                Err(e) => {
                    error!(%e, %session_id, "unix socket REPL: read error");
                    break;
                }
            }
        }

        sessions.write().await.remove(&session_id);
        info!(%session_id, "unix socket REPL session ended");
    }
}

#[async_trait]
impl Channel for UnixSocketReplChannel {
    fn name(&self) -> &str {
        "unix_socket_repl"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        // Clean up a stale socket file from a previous run.
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path).map_err(|e| ChannelError::StartupFailed {
                name: "unix_socket_repl".to_string(),
                reason: format!("failed to remove stale socket: {e}"),
            })?;
        }
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ChannelError::StartupFailed {
                name: "unix_socket_repl".to_string(),
                reason: format!("failed to create socket directory: {e}"),
            })?;
        }

        let listener =
            UnixListener::bind(&self.socket_path).map_err(|e| ChannelError::StartupFailed {
                name: "unix_socket_repl".to_string(),
                reason: format!("failed to bind {}: {e}", self.socket_path.display()),
            })?;

        info!(path = ?self.socket_path, "unix socket REPL server listening");

        let (msg_tx, msg_rx) = mpsc::channel(64);
        let sessions = Arc::clone(&self.sessions);
        let max_connections = self.max_connections;
        let owner_id = self.owner_id.clone();

        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _addr)) => {
                        let (read, write) = stream.into_split();
                        let tx = msg_tx.clone();
                        let sessions = Arc::clone(&sessions);
                        let owner_id = owner_id.clone();
                        tokio::spawn(async move {
                            Self::handle_connection(
                                read,
                                write,
                                tx,
                                sessions,
                                max_connections,
                                owner_id,
                            )
                            .await;
                        });
                    }
                    Err(e) => {
                        error!(%e, "unix socket REPL: accept error");
                    }
                }
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(msg_rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let session_id = msg
            .metadata
            .get("unix_session_id")
            .and_then(|v| v.as_str())
            .unwrap_or(msg.sender_id.as_str());

        let mut sessions = self.sessions.write().await;
        if let Some(write) = sessions.get_mut(session_id) {
            Self::write_message(
                write,
                &ReplMessage::Response {
                    content: response.content,
                    session_id: Some(session_id.to_string()),
                    is_complete: true,
                },
            )
            .await?;
        } else {
            debug!(%session_id, "unix socket REPL: session not found for response (client may have disconnected)");
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        if self.socket_path.exists() {
            Ok(())
        } else {
            Err(ChannelError::StartupFailed {
                name: "unix_socket_repl".to_string(),
                reason: format!("socket file missing: {}", self.socket_path.display()),
            })
        }
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path).map_err(|e| ChannelError::StartupFailed {
                name: "unix_socket_repl".to_string(),
                reason: format!("failed to remove socket on shutdown: {e}"),
            })?;
        }
        info!("unix socket REPL server stopped");
        Ok(())
    }
}
