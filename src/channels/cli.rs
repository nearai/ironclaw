//! CLI/stdin channel for interactive terminal usage.

use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse};
use crate::error::ChannelError;

/// CLI channel for interactive terminal input.
pub struct CliChannel {
    running: Arc<AtomicBool>,
}

impl CliChannel {
    /// Create a new CLI channel.
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Default for CliChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Channel for CliChannel {
    fn name(&self) -> &str {
        "cli"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();

        let (tx, rx) = mpsc::channel(32);

        // Spawn a blocking task to read from stdin
        tokio::task::spawn_blocking(move || {
            let stdin = io::stdin();
            let reader = stdin.lock();

            // Print prompt
            print_prompt();

            for line in reader.lines() {
                if !running.load(Ordering::SeqCst) {
                    break;
                }

                match line {
                    Ok(content) => {
                        let content = content.trim();
                        if content.is_empty() {
                            print_prompt();
                            continue;
                        }

                        // Handle exit commands
                        if content == "exit" || content == "quit" || content == "/quit" {
                            running.store(false, Ordering::SeqCst);
                            break;
                        }

                        let msg = IncomingMessage::new("cli", "local-user", content);

                        if tx.blocking_send(msg).is_err() {
                            // Channel closed, stop reading
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Error reading stdin: {}", e);
                        break;
                    }
                }
            }

            tracing::debug!("CLI input loop ended");
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Print response to stdout
        println!("\n{}\n", response.content);
        print_prompt();
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        // CLI is always healthy if we're running
        if self.running.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: "cli".to_string(),
            })
        }
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}

fn print_prompt() {
    print!("agent> ");
    let _ = io::stdout().flush();
}
