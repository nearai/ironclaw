//! `ironclaw repl` - connect to a running Ironclaw daemon via Unix socket.
//!
//! The daemon must already be running (for example via `systemctl start ironclaw`)
//! and listening on its Unix socket (default: `/tmp/ironclaw.sock`).

use std::path::PathBuf;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tokio::sync::{oneshot, watch, Mutex as AsyncMutex};

/// Protocol messages. Keep this in sync with the server.
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

#[derive(Default)]
struct SharedState {
    active_session_id: Option<String>,
    awaiting_response: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket_path = resolve_socket_path()?;

    println!("IronClaw Unix Socket Client v2");
    println!("Connecting to: {}", socket_path.display());

    let stream = UnixStream::connect(&socket_path).await.map_err(|e| {
        anyhow::anyhow!(
            "Failed to connect to Ironclaw REPL at {}: {}.\n\
             Make sure the daemon is running (`ironclaw run` or `systemctl start ironclaw`).",
            socket_path.display(),
            e
        )
    })?;

    println!("Connected!");

    let (read_half, write_half) = stream.into_split();
    let writer = Arc::new(AsyncMutex::new(write_half));
    let state = Arc::new(Mutex::new(SharedState::default()));
    let (ready_tx, ready_rx) = oneshot::channel();
    let (closed_tx, mut closed_rx) = watch::channel(false);

    send_message(
        &writer,
        &ReplMessage::Connect {
            version: env!("CARGO_PKG_VERSION").to_string(),
            client_info: Some("unix-socket-client-v2".to_string()),
        },
    )
    .await?;

    let reader_handle = tokio::spawn(read_loop(
        BufReader::new(read_half),
        Arc::clone(&writer),
        Arc::clone(&state),
        ready_tx,
        closed_tx,
    ));

    let _ = ready_rx.await;
    if *closed_rx.borrow() {
        let _ = reader_handle.await;
        println!("Goodbye!");
        return Ok(());
    }

    println!("Type 'exit' or 'quit' to disconnect.");
    println!("---");
    print_prompt()?;

    let mut input_rx = spawn_stdin_reader();

    loop {
        tokio::select! {
            maybe_line = input_rx.recv() => {
                let Some(line) = maybe_line else {
                    break;
                };

                let input = line.trim();
                if input.is_empty() {
                    if !is_awaiting_response(&state) {
                        print_prompt()?;
                    }
                    continue;
                }

                if input.eq_ignore_ascii_case("exit") || input.eq_ignore_ascii_case("quit") {
                    let current_session_id = current_session_id(&state);
                    send_message(
                        &writer,
                        &ReplMessage::Disconnect {
                            session_id: current_session_id,
                            reason: Some("client requested disconnect".to_string()),
                        },
                    )
                    .await?;
                    break;
                }

                if input.eq_ignore_ascii_case("/reset") {
                    fail_pending_request(&state);
                    eprintln!("Cleared pending response state.");
                    print_prompt()?;
                    continue;
                }

                if is_awaiting_response(&state) {
                    eprintln!("Still waiting for the previous response; please wait.");
                    continue;
                }

                begin_request(&state);
                let current_session_id = current_session_id(&state);
                send_message(
                    &writer,
                    &ReplMessage::Message {
                        content: input.to_string(),
                        session_id: current_session_id,
                    },
                )
                .await?;
            }
            changed = closed_rx.changed() => {
                if changed.is_ok() && *closed_rx.borrow() {
                    break;
                }
            }
        }
    }

    reader_handle.abort();
    println!("Goodbye!");
    Ok(())
}

fn resolve_socket_path() -> anyhow::Result<PathBuf> {
    let mut args = std::env::args().skip(1);
    let mut positional_path = None;

    while let Some(arg) = args.next() {
        if arg == "--socket" {
            let value = args
                .next()
                .context("`--socket` requires a path, for example `--socket /tmp/ironclaw.sock`")?;
            return Ok(PathBuf::from(value));
        }

        if let Some(value) = arg.strip_prefix("--socket=") {
            return Ok(PathBuf::from(value));
        }

        if arg == "-h" || arg == "--help" {
            println!("Usage: unix-socket-client-v2 [--socket PATH] [PATH]");
            println!("Environment: IRONCLAW_SOCKET=/tmp/ironclaw.sock");
            std::process::exit(0);
        }

        if arg.starts_with('-') {
            anyhow::bail!("unknown argument: {arg}");
        }

        if positional_path.is_some() {
            anyhow::bail!("unexpected extra argument: {arg}");
        }

        positional_path = Some(PathBuf::from(arg));
    }

    if let Some(path) = positional_path {
        return Ok(path);
    }

    if let Some(path) = std::env::var_os("IRONCLAW_SOCKET") {
        return Ok(PathBuf::from(path));
    }

    Ok(PathBuf::from("/tmp/ironclaw.sock"))
}

async fn read_loop(
    mut reader: BufReader<OwnedReadHalf>,
    writer: Arc<AsyncMutex<OwnedWriteHalf>>,
    state: Arc<Mutex<SharedState>>,
    ready_tx: oneshot::Sender<()>,
    closed_tx: watch::Sender<bool>,
) -> anyhow::Result<()> {
    let mut response_buffers: HashMap<Option<String>, String> = HashMap::new();
    let mut ready_tx = Some(ready_tx);

    loop {
        match read_message(&mut reader).await {
            Ok(ReplMessage::Response {
                content,
                session_id: incoming_session_id,
                is_complete,
            }) => {
                let response = response_buffers
                    .entry(incoming_session_id.clone())
                    .or_default();
                response.push_str(&content);

                if is_complete {
                    let response = response_buffers
                        .remove(&incoming_session_id)
                        .unwrap_or_default();
                    complete_response(
                        &state,
                        incoming_session_id.clone(),
                        ready_tx.is_some(),
                        &response,
                    );

                    print_response(&response)?;

                    if let Some(ready_tx) = ready_tx.take() {
                        let _ = ready_tx.send(());
                    } else if !is_awaiting_response(&state) {
                        print_prompt()?;
                    }
                }
            }
            Ok(ReplMessage::Ping) => {
                send_message(&writer, &ReplMessage::Pong).await?;
            }
            Ok(ReplMessage::Disconnect {
                session_id: incoming_session_id,
                reason,
            }) => {
                mark_disconnected(&state, incoming_session_id.clone());
                let response = response_buffers
                    .remove(&incoming_session_id)
                    .unwrap_or_default();
                if !response.is_empty() {
                    print_response(&response)?;
                }
                if let Some(reason) = reason {
                    eprintln!("Server disconnected: {reason}");
                } else {
                    eprintln!("Server disconnected");
                }
                let _ = closed_tx.send(true);
                if let Some(ready_tx) = ready_tx.take() {
                    let _ = ready_tx.send(());
                }
                return Ok(());
            }
            Ok(other) => {
                eprintln!("unexpected server message: {other:?}");
            }
            Err(e) => {
                fail_pending_request(&state);
                for (_, response) in response_buffers.drain() {
                    if response.is_empty() {
                        continue;
                    }
                    print_response(&response)?;
                }
                eprintln!("Connection lost: {e}");
                let _ = closed_tx.send(true);
                if let Some(ready_tx) = ready_tx.take() {
                    let _ = ready_tx.send(());
                }
                return Ok(());
            }
        }
    }
}

fn current_session_id(state: &Arc<Mutex<SharedState>>) -> Option<String> {
    state
        .lock()
        .ok()
        .and_then(|guard| guard.active_session_id.clone())
}

fn begin_request(state: &Arc<Mutex<SharedState>>) {
    if let Ok(mut guard) = state.lock() {
        guard.awaiting_response = true;
    }
}

fn is_awaiting_response(state: &Arc<Mutex<SharedState>>) -> bool {
    state
        .lock()
        .map(|guard| guard.awaiting_response)
        .unwrap_or(false)
}

fn complete_response(
    state: &Arc<Mutex<SharedState>>,
    incoming_session_id: Option<String>,
    is_initial_response: bool,
    response: &str,
) {
    if let Ok(mut guard) = state.lock() {
        if is_initial_response {
            if let Some(incoming_session_id) = incoming_session_id {
                guard.active_session_id = Some(incoming_session_id);
            }
            return;
        }

        if is_background_job_update(response) {
            return;
        }

        if guard.awaiting_response {
            if let Some(incoming_session_id) = incoming_session_id {
                guard.active_session_id = Some(incoming_session_id);
            }
            guard.awaiting_response = false;
            return;
        }
    }
}

fn fail_pending_request(state: &Arc<Mutex<SharedState>>) {
    if let Ok(mut guard) = state.lock() {
        guard.awaiting_response = false;
    }
}

fn is_background_job_update(response: &str) -> bool {
    response.trim_start().starts_with("[Job ")
}

fn mark_disconnected(state: &Arc<Mutex<SharedState>>, incoming_session_id: Option<String>) {
    if let Ok(mut guard) = state.lock() {
        if let Some(incoming_session_id) = incoming_session_id {
            if guard.active_session_id.is_none()
                || guard.active_session_id.as_ref() == Some(&incoming_session_id)
            {
                guard.active_session_id = Some(incoming_session_id);
            }
        }
        guard.awaiting_response = false;
    }
}

fn spawn_stdin_reader() -> tokio::sync::mpsc::UnboundedReceiver<String> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    std::thread::spawn(move || {
        use std::io::BufRead as _;

        let stdin = std::io::stdin();
        let mut handle = stdin.lock();

        loop {
            let mut line = String::new();
            match handle.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    rx
}

fn print_prompt() -> anyhow::Result<()> {
    use std::io::Write as _;

    print!("> ");
    std::io::stdout().flush()?;
    Ok(())
}

fn print_response(response: &str) -> anyhow::Result<()> {
    if response.is_empty() {
        return Ok(());
    }

    if response.ends_with('\n') {
        print!("{response}");
    } else {
        println!("{response}");
    }
    std::io::Write::flush(&mut std::io::stdout())?;
    Ok(())
}

async fn send_message(
    write: &Arc<AsyncMutex<OwnedWriteHalf>>,
    msg: &ReplMessage,
) -> anyhow::Result<()> {
    let mut line = serde_json::to_string(msg)?;
    line.push('\n');
    let mut write = write.lock().await;
    write.write_all(line.as_bytes()).await?;
    Ok(())
}

async fn read_message(reader: &mut BufReader<OwnedReadHalf>) -> anyhow::Result<ReplMessage> {
    let mut buf = String::new();
    reader.read_line(&mut buf).await?;
    if buf.is_empty() {
        anyhow::bail!("server disconnected");
    }
    let msg = serde_json::from_str(&buf)?;
    Ok(msg)
}
