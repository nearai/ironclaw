//! `ironclaw repl` — connect to a running Ironclaw daemon via Unix socket.
//!
//! The daemon must already be running (e.g. via `systemctl start ironclaw`)
//! and listening on its Unix socket (default: `~/.ironclaw/ironclaw.sock`).

use std::path::PathBuf;

use clap::Parser;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;

/// Protocol messages — must stay in sync with
/// `crate::channels::unix_socket_repl::ReplMessage`.
#[derive(Debug, Serialize, Deserialize)]
enum ReplMessage {
    Connect {
        version: String,
        client_info: Option<String>,
    },
    Message {
        content: String,
        session_id: Option<String>,
    },
    Response {
        content: String,
        session_id: Option<String>,
        is_complete: bool,
    },
    Disconnect {
        session_id: Option<String>,
        reason: Option<String>,
    },
    Ping,
    Pong,
}

/// Connect to a running Ironclaw service via its Unix socket REPL.
///
/// The Ironclaw daemon must be running (e.g. `ironclaw run` or via systemd).
/// This command does not start a new instance.
#[derive(Parser, Debug)]
pub struct ReplCommand {
    /// Path to the Unix socket.
    ///
    /// Defaults to `~/.ironclaw/ironclaw.sock` — the socket the daemon creates
    /// at startup when not running in CLI-only mode.
    #[arg(long)]
    pub socket: Option<PathBuf>,
}

impl ReplCommand {
    pub async fn run(&self) -> anyhow::Result<()> {
        let socket_path = self
            .socket
            .clone()
            .unwrap_or_else(|| crate::bootstrap::ironclaw_base_dir().join("ironclaw.sock"));

        let stream = UnixStream::connect(&socket_path).await.map_err(|e| {
            anyhow::anyhow!(
                "Failed to connect to Ironclaw REPL at {}: {}.\n\
                 Make sure the daemon is running (`ironclaw run` or `systemctl start ironclaw`).",
                socket_path.display(),
                e
            )
        })?;

        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);

        // ── Handshake ────────────────────────────────────────────────
        send_message(
            &mut write_half,
            &ReplMessage::Connect {
                version: env!("CARGO_PKG_VERSION").to_string(),
                client_info: None,
            },
        )
        .await?;

        match read_message(&mut reader).await? {
            ReplMessage::Response { content, .. } => println!("{content}"),
            other => eprintln!("unexpected welcome: {other:?}"),
        }

        println!("Type 'exit' or 'quit' to disconnect.");
        println!("---");

        // ── Main loop ────────────────────────────────────────────────
        let mut stdin = BufReader::new(tokio::io::stdin());
        let mut line = String::new();

        loop {
            {
                use std::io::Write as _;
                print!("> ");
                std::io::stdout().flush()?;
            }

            line.clear();
            let n = stdin.read_line(&mut line).await?;
            if n == 0 {
                // Ctrl-D / EOF
                break;
            }

            let input = line.trim();

            if input.eq_ignore_ascii_case("exit") || input.eq_ignore_ascii_case("quit") {
                send_message(
                    &mut write_half,
                    &ReplMessage::Disconnect {
                        session_id: None,
                        reason: Some("client requested disconnect".to_string()),
                    },
                )
                .await?;
                break;
            }

            if input.is_empty() {
                continue;
            }

            send_message(
                &mut write_half,
                &ReplMessage::Message {
                    content: input.to_string(),
                    session_id: None,
                },
            )
            .await?;

            match read_message(&mut reader).await {
                Ok(ReplMessage::Response { content, .. }) => println!("{content}"),
                Ok(ReplMessage::Ping) => {
                    send_message(&mut write_half, &ReplMessage::Pong).await?;
                }
                Ok(other) => {
                    eprintln!("unexpected server message: {other:?}");
                }
                Err(e) => {
                    eprintln!("Connection lost: {e}");
                    break;
                }
            }
        }

        Ok(())
    }
}

async fn send_message(write: &mut OwnedWriteHalf, msg: &ReplMessage) -> anyhow::Result<()> {
    let mut line = serde_json::to_string(msg)?;
    line.push('\n');
    write.write_all(line.as_bytes()).await?;
    Ok(())
}

async fn read_message(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
) -> anyhow::Result<ReplMessage> {
    let mut buf = String::new();
    reader.read_line(&mut buf).await?;
    if buf.is_empty() {
        anyhow::bail!("server disconnected");
    }
    let msg = serde_json::from_str(&buf)?;
    Ok(msg)
}
