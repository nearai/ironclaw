mod support;

use futures::StreamExt;
use ironclaw_reborn_tui::client::events::subscribe;
use support::{MockServer, SseScript, SseScriptEvent};

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
