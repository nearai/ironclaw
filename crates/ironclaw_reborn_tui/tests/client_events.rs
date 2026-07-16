mod support;

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

    let event_requests = server
        .requests()
        .into_iter()
        .filter(|r| r.path.ends_with("/events"))
        .count();
    // 1 initial connect + 3 reconnects = 4 total attempts.
    assert_eq!(event_requests, 4);
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
