mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use ironclaw_reborn_tui::client::ApiClient;
use ironclaw_reborn_tui::client::events::subscribe;
use support::{MockServer, SseScript, SseScriptEvent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn keep_alive_frame(cursor: &str) -> serde_json::Value {
    serde_json::json!({"cursor": cursor, "type": "keep_alive"})
}

#[tokio::test]
async fn happy_stream_yields_frames_in_order() {
    let server = MockServer::start().await;
    server.queue_sse(SseScript {
        events: vec![
            SseScriptEvent {
                event: "keep_alive".to_string(),
                id: Some("\"cursor:1\"".to_string()),
                data: keep_alive_frame("cursor:1"),
            },
            SseScriptEvent {
                event: "keep_alive".to_string(),
                id: Some("\"cursor:2\"".to_string()),
                data: keep_alive_frame("cursor:2"),
            },
        ],
    });
    // Second connection attempt: nothing left queued -> ends immediately,
    // which the client will treat as a reconnect-eligible clean end. The
    // test only asserts the first two frames, then drops the stream.
    server.queue_sse(SseScript::default());

    let client = server.client();
    let mut stream = std::pin::pin!(subscribe(&client, "thread-1", None));

    let first = stream.next().await.expect("first frame").expect("ok");
    assert_eq!(first.cursor().as_str(), "cursor:1");
    let second = stream.next().await.expect("second frame").expect("ok");
    assert_eq!(second.cursor().as_str(), "cursor:2");
}

#[tokio::test]
async fn reconnect_sends_last_event_id_verbatim_after_a_drop() {
    let server = MockServer::start().await;
    server.queue_sse(SseScript {
        events: vec![SseScriptEvent {
            event: "keep_alive".to_string(),
            id: Some("\"cursor:1\"".to_string()),
            data: keep_alive_frame("cursor:1"),
        }],
    });
    server.queue_sse(SseScript {
        events: vec![SseScriptEvent {
            event: "keep_alive".to_string(),
            id: Some("\"cursor:2\"".to_string()),
            data: keep_alive_frame("cursor:2"),
        }],
    });

    let client = server.client();
    let mut stream = std::pin::pin!(subscribe(&client, "thread-1", None));

    let first = stream.next().await.expect("first frame").expect("ok");
    assert_eq!(first.cursor().as_str(), "cursor:1");
    let second = stream
        .next()
        .await
        .expect("second frame after reconnect")
        .expect("ok");
    assert_eq!(second.cursor().as_str(), "cursor:2");

    let requests = server.requests();
    let event_requests: Vec<_> = requests
        .iter()
        .filter(|r| r.path.ends_with("/events"))
        .collect();
    assert_eq!(event_requests.len(), 2);
    assert_eq!(event_requests[0].last_event_id, None);
    assert_eq!(
        event_requests[1].last_event_id.as_deref(),
        Some("\"cursor:1\"")
    );
}

#[tokio::test]
async fn budget_exhaustion_yields_reconnect_budget_exhausted_after_three_empty_reconnects() {
    let server = MockServer::start().await;
    // Every connection ends immediately with zero events -> the client
    // reconnects up to 3 times, then must give up.
    for _ in 0..4 {
        server.queue_sse(SseScript::default());
    }

    let client = server.client();
    let mut stream = std::pin::pin!(subscribe(&client, "thread-1", None));
    let started = tokio::time::Instant::now();

    let error = stream
        .next()
        .await
        .expect("terminal item")
        .expect_err("expected error");
    assert!(matches!(
        error,
        ironclaw_reborn_tui::client::ClientError::ReconnectBudgetExhausted {
            attempts: 3,
            window_secs: 60
        }
    ));
    assert!(
        stream.next().await.is_none(),
        "stream must end after budget exhaustion"
    );
    assert!(
        tokio::time::Instant::now().duration_since(started) >= Duration::from_secs(7),
        "the three reconnects must be paced at 1s, 2s, and 4s"
    );

    let event_requests = server
        .requests()
        .into_iter()
        .filter(|r| r.path.ends_with("/events"))
        .count();
    // 1 initial connect + 3 reconnects = 4 total attempts.
    assert_eq!(event_requests, 4);
}

/// Raw chunked-HTTP server for parser-boundary regressions. Each entry is
/// written as its own HTTP chunk with a scheduler yield between chunks, so
/// reqwest exposes realistic transport boundaries rather than one String
/// body assembled by the axum fixture.
async fn spawn_chunked_sse_server(chunks: Vec<Vec<u8>>) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind chunked SSE server");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut request = Vec::new();
        let mut buf = [0_u8; 1024];
        loop {
            let Ok(read) = socket.read(&mut buf).await else {
                return;
            };
            if read == 0 {
                return;
            }
            request.extend_from_slice(&buf[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }

        let header = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n";
        if socket.write_all(header).await.is_err() {
            return;
        }
        for chunk in chunks {
            let prefix = format!("{:x}\r\n", chunk.len());
            if socket.write_all(prefix.as_bytes()).await.is_err()
                || socket.write_all(&chunk).await.is_err()
                || socket.write_all(b"\r\n").await.is_err()
            {
                return;
            }
            tokio::task::yield_now().await;
        }
        let _ = socket.write_all(b"0\r\n\r\n").await;
    });
    addr
}

async fn read_http_request(socket: &mut tokio::net::TcpStream) -> Option<Vec<u8>> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = socket.read(&mut buffer).await.ok()?;
        if read == 0 {
            return None;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return Some(request);
        }
    }
}

async fn spawn_open_malformed_sse_server(
    attempts: usize,
) -> (std::net::SocketAddr, Arc<Mutex<Vec<Vec<u8>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind malformed SSE server");
    let addr = listener.local_addr().expect("malformed SSE server addr");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    tokio::spawn(async move {
        for _ in 0..attempts {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let captured = captured.clone();
            tokio::spawn(async move {
                let Some(request) = read_http_request(&mut socket).await else {
                    return;
                };
                captured
                    .lock()
                    .expect("lock captured requests")
                    .push(request);

                let block = b"id: malformed-cursor\ndata: {not-json}\n\n";
                let header = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n";
                let prefix = format!("{:x}\r\n", block.len());
                if socket.write_all(header).await.is_err()
                    || socket.write_all(prefix.as_bytes()).await.is_err()
                    || socket.write_all(block).await.is_err()
                    || socket.write_all(b"\r\n").await.is_err()
                {
                    return;
                }

                std::future::pending::<()>().await;
            });
        }
    });
    (addr, requests)
}

async fn spawn_truncated_error_sse_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind truncated SSE error server");
    let addr = listener
        .local_addr()
        .expect("truncated SSE error server addr");
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        if read_http_request(&mut socket).await.is_none() {
            return;
        }
        let _ = socket
            .write_all(
                b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 64\r\nconnection: close\r\n\r\npartial body",
            )
            .await;
    });
    addr
}

async fn spawn_unauthorized_sse_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind unauthorized SSE server");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut buf = [0_u8; 1024];
        let _ = socket.read(&mut buf).await;
        let _ = socket
            .write_all(b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\n\r\n")
            .await;
    });
    addr
}

#[tokio::test]
async fn unauthorized_sse_response_is_terminal_without_reconnect() {
    let addr = spawn_unauthorized_sse_server().await;
    let client = ApiClient::new(format!("http://{addr}"), "bad-token".to_string());
    let mut stream = std::pin::pin!(subscribe(&client, "thread-1", None));

    let error = stream
        .next()
        .await
        .expect("terminal unauthorized item")
        .expect_err("401 must be classified as unauthorized");
    assert!(matches!(
        error,
        ironclaw_reborn_tui::client::ClientError::Unauthorized
    ));
    assert!(stream.next().await.is_none(), "401 must not reconnect");
}

#[tokio::test]
async fn malformed_frame_reconnects_to_budget_without_advancing_cursor() {
    let (addr, requests) = spawn_open_malformed_sse_server(4).await;
    let client = ApiClient::new(format!("http://{addr}"), "test-token".to_string());
    let mut stream = std::pin::pin!(subscribe(&client, "thread-1", None));

    tokio::time::timeout(Duration::from_secs(12), async {
        for _ in 0..4 {
            let error = stream
                .next()
                .await
                .expect("malformed frame item")
                .expect_err("malformed JSON must fail the connection");
            assert!(matches!(
                error,
                ironclaw_reborn_tui::client::ClientError::StreamParse(_)
            ));
        }

        let terminal = stream
            .next()
            .await
            .expect("terminal reconnect item")
            .expect_err("reconnect budget must be exhausted");
        assert!(matches!(
            terminal,
            ironclaw_reborn_tui::client::ClientError::ReconnectBudgetExhausted {
                attempts: 3,
                window_secs: 60
            }
        ));
        assert!(stream.next().await.is_none(), "stream must terminate");
    })
    .await
    .expect("malformed-frame reconnects must remain bounded");

    let requests = requests.lock().expect("lock captured requests");
    assert_eq!(requests.len(), 4, "initial connection plus three retries");
    assert!(requests.iter().all(|request| {
        !String::from_utf8_lossy(request)
            .to_ascii_lowercase()
            .contains("last-event-id:")
    }));
}

#[tokio::test]
async fn truncated_sse_error_body_propagates_transport_failure() {
    let addr = spawn_truncated_error_sse_server().await;
    let client = ApiClient::new(format!("http://{addr}"), "test-token".to_string());
    let mut stream = std::pin::pin!(subscribe(&client, "thread-1", None));

    let error = stream
        .next()
        .await
        .expect("transport failure item")
        .expect_err("truncated error body must not become a server error");
    assert!(matches!(
        error,
        ironclaw_reborn_tui::client::ClientError::Transport(_)
    ));
}

#[tokio::test]
async fn split_multibyte_utf8_across_http_chunks_is_decoded_losslessly() {
    let cursor = "cursor:café";
    let data = serde_json::to_string(&keep_alive_frame(cursor)).expect("serialize frame");
    let block = format!("event: keep_alive\nid: \"{cursor}\"\ndata: {data}\n\n");
    let bytes = block.into_bytes();
    let split = bytes
        .windows("é".len())
        .position(|window| window == "é".as_bytes())
        .expect("unicode marker in block")
        + 1;
    let addr =
        spawn_chunked_sse_server(vec![bytes[..split].to_vec(), bytes[split..].to_vec()]).await;
    let client = ApiClient::new(format!("http://{addr}"), "test-token".to_string());
    let mut stream = std::pin::pin!(subscribe(&client, "thread-1", None));

    let frame = stream
        .next()
        .await
        .expect("stream item")
        .expect("valid split UTF-8 frame");
    assert_eq!(frame.cursor().as_str(), cursor);
}

#[tokio::test]
async fn oversized_incomplete_sse_line_is_rejected() {
    let oversized = vec![b'x'; 64 * 1024 + 1];
    let addr = spawn_chunked_sse_server(vec![oversized]).await;
    let client = ApiClient::new(format!("http://{addr}"), "test-token".to_string());
    let mut stream = std::pin::pin!(subscribe(&client, "thread-1", None));

    let error = stream
        .next()
        .await
        .expect("stream item")
        .expect_err("oversized line must fail closed");
    assert!(matches!(
        error,
        ironclaw_reborn_tui::client::ClientError::StreamProtocol(_)
    ));
    assert!(
        stream.next().await.is_none(),
        "protocol failure is terminal"
    );
}

#[tokio::test]
async fn oversized_sse_event_is_rejected_before_json_decode() {
    let data_line = format!("data: {}\n", "x".repeat(64 * 1024 - 16));
    let mut block = Vec::new();
    for _ in 0..17 {
        block.extend_from_slice(data_line.as_bytes());
    }
    block.push(b'\n');
    let addr = spawn_chunked_sse_server(vec![block]).await;
    let client = ApiClient::new(format!("http://{addr}"), "test-token".to_string());
    let mut stream = std::pin::pin!(subscribe(&client, "thread-1", None));

    let error = stream
        .next()
        .await
        .expect("stream item")
        .expect_err("oversized event must fail closed");
    assert!(matches!(
        error,
        ironclaw_reborn_tui::client::ClientError::StreamProtocol(_)
    ));
}

#[tokio::test]
async fn one_read_cannot_queue_an_unbounded_number_of_sse_frames() {
    let mut chunk = Vec::new();
    for index in 0..65 {
        let data = serde_json::to_string(&keep_alive_frame(&format!("cursor:{index}")))
            .expect("serialize frame");
        chunk.extend_from_slice(format!("data: {data}\n\n").as_bytes());
    }
    let addr = spawn_chunked_sse_server(vec![chunk]).await;
    let client = ApiClient::new(format!("http://{addr}"), "test-token".to_string());
    let mut stream = std::pin::pin!(subscribe(&client, "thread-1", None));

    let mut frames = 0;
    loop {
        match stream.next().await.expect("bounded stream item") {
            Ok(_) => frames += 1,
            Err(ironclaw_reborn_tui::client::ClientError::StreamProtocol(_)) => break,
            Err(other) => panic!("unexpected stream error: {other}"),
        }
    }
    assert!(
        frames < 64,
        "one read queued {frames} frames before failing"
    );
    assert!(
        stream.next().await.is_none(),
        "protocol failure is terminal"
    );
}

/// Hand-rolled, `support::MockServer`-independent raw TCP + chunked-HTTP
/// server: `support::MockServer`'s `sse_response` (axum, `Json`/string body)
/// always closes the connection once its scripted events are written, which
/// is exactly what masked the mid-connection-buffering bug this test exists
/// to catch (see `client/events.rs`'s module doc). This server instead
/// writes two SSE blocks as two separate chunked-encoding chunks and then
/// deliberately never sends the terminating `0\r\n\r\n` chunk or closes the
/// socket — mirroring how the real `ironclaw-reborn serve` holds an SSE
/// response open for its full `SSE_MAX_LIFETIME` (~5 minutes). The
/// connection is only actually torn down when the test's `#[tokio::test]`
/// runtime drops the spawned task at the end of the test function.
async fn spawn_open_ended_sse_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind open-ended sse server");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };

        // Drain the request line/headers up to the blank line that ends an
        // HTTP request. Best-effort: this server only ever serves one
        // request, so it doesn't need to parse the request at all beyond
        // knowing where it ends.
        let mut buf = [0u8; 1024];
        loop {
            match socket.read(&mut buf).await {
                Ok(0) => return,
                Ok(n) if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") => break,
                Ok(_) => continue,
                Err(_) => return,
            }
        }

        let header = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n";
        if socket.write_all(header.as_bytes()).await.is_err() {
            return;
        }

        for cursor in ["cursor:open:1", "cursor:open:2"] {
            let data = serde_json::to_string(&keep_alive_frame(cursor)).expect("serialize");
            let block = format!("event: keep_alive\nid: \"{cursor}\"\ndata: {data}\n\n");
            let chunk = format!("{:x}\r\n{block}\r\n", block.len());
            if socket.write_all(chunk.as_bytes()).await.is_err() {
                return;
            }
        }

        // Park forever (until the test's runtime drops this task) instead
        // of writing the terminating chunk or closing the socket.
        let (_tx, rx) = tokio::sync::oneshot::channel::<()>();
        let _: Result<(), _> = rx.await;
    });
    addr
}

#[tokio::test]
async fn sse_frames_yield_promptly_even_when_the_connection_stays_open() {
    let addr = spawn_open_ended_sse_server().await;
    let client = ApiClient::new(format!("http://{addr}"), "test-token".to_string());
    let mut stream = std::pin::pin!(subscribe(&client, "thread-1", None));

    let first = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("first frame must arrive without waiting for the connection to close")
        .expect("stream item")
        .expect("ok");
    assert_eq!(first.cursor().as_str(), "cursor:open:1");

    let second = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("second frame must also arrive promptly")
        .expect("stream item")
        .expect("ok");
    assert_eq!(second.cursor().as_str(), "cursor:open:2");
}
