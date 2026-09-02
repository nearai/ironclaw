//! The page's session event stream over the real stack: a bearer-authenticated
//! `POST /api/webchat/v2/session/events` whose body names the subscription set
//! answers `text/event-stream`, admits each thread subscription, and streams
//! the exact durable finalized reply the HTTP timeline serves. Two logical
//! subscriptions on one stream deliver their own threads independently.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use ironclaw_assistant::RebornServices;
use ironclaw_event_log::InMemoryDurableEventLog;
use ironclaw_turns::{ReplyTargetBindingRef, TurnEventProjectionSource};
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::webui_mount::{get_json, mount_webui_v2_router, webui_caller_for};
use serde_json::Value;

const FINAL_REPLY: &str = "the precise durable answer for the session stream";

fn session_services(h: &RebornIntegrationHarness) -> Arc<RebornServices> {
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let reply_target_binding_ref =
        ReplyTargetBindingRef::new("session-stream-test").expect("valid reply target binding ref");
    let turn_event_source: Arc<dyn TurnEventProjectionSource> = h.turn_event_projection_for_test();
    let event_stream =
        ironclaw_composition::test_support::build_product_event_stream_with_thread_service_for_test(
            event_log,
            turn_event_source,
            h.coordinator.clone(),
            reply_target_binding_ref,
            h.thread_harness.service.clone(),
        );
    Arc::new(
        RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone())
            .with_event_stream(event_stream),
    )
}

async fn serve_router(router: axum::Router) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local_addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (addr, handle)
}

/// One open session stream: the response body as a line-oriented SSE reader.
struct SessionStream {
    response: reqwest::Response,
    buffered: Vec<u8>,
    data_lines: Vec<String>,
}

async fn open_session_stream(
    addr: std::net::SocketAddr,
    subscriptions: Vec<(&str, &str)>,
) -> SessionStream {
    let body = serde_json::json!({
        "subscriptions": subscriptions
            .into_iter()
            .map(|(id, thread_id)| serde_json::json!({
                "subscription_id": id,
                "selector": {"kind": "thread", "thread_id": thread_id},
                "after_cursor": null,
            }))
            .collect::<Vec<_>>(),
    });
    let response = tokio::time::timeout(
        Duration::from_secs(15),
        reqwest::Client::new()
            .post(format!("http://{addr}/api/webchat/v2/session/events"))
            .header("accept", "text/event-stream")
            .json(&body)
            .send(),
    )
    .await
    .expect("session stream opens within 15s")
    .expect("session stream request");
    assert_eq!(response.status().as_u16(), 200);
    assert!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream")),
        "the session stream answers as text/event-stream",
    );
    SessionStream {
        response,
        buffered: Vec::new(),
        data_lines: Vec::new(),
    }
}

impl SessionStream {
    /// The next session frame (`data:` payload of one SSE event).
    async fn next_frame(&mut self, deadline: std::time::Instant) -> Value {
        loop {
            while let Some(newline) = self.buffered.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.buffered.drain(..=newline).collect();
                let line = String::from_utf8_lossy(&line)
                    .trim_end_matches(['\r', '\n'])
                    .to_string();
                if line.is_empty() {
                    if !self.data_lines.is_empty() {
                        let payload = self.data_lines.join("\n");
                        self.data_lines.clear();
                        return serde_json::from_str(&payload).expect("session frame parses");
                    }
                    continue;
                }
                if let Some(data) = line.strip_prefix("data:") {
                    self.data_lines.push(data.trim_start().to_string());
                }
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            match tokio::time::timeout(remaining, self.response.chunk()).await {
                Ok(Ok(Some(chunk))) => self.buffered.extend_from_slice(&chunk),
                Ok(Ok(None)) => panic!("session stream closed early"),
                Ok(Err(error)) => panic!("session stream error: {error}"),
                Err(_) => panic!("no session frame before deadline"),
            }
        }
    }
}

async fn collect_until_finalized(
    stream: &mut SessionStream,
    subscription_id: &str,
    deadline: std::time::Instant,
) -> (String, Vec<String>) {
    let mut texts = Vec::new();
    loop {
        let frame = stream.next_frame(deadline).await;
        assert_eq!(frame["schema"], "webui.session_event.v1");
        if frame["type"] != "event" || frame["subscription_id"] != subscription_id {
            continue;
        }
        let event = &frame["event"];
        for forbidden in [
            "adapter_id",
            "installation_id",
            "target",
            "delivery_attempt_id",
        ] {
            assert!(
                event.get(forbidden).is_none(),
                "browser frame must not leak `{forbidden}`: {event}"
            );
        }
        let Some(items) = event["state"]["items"].as_array() else {
            continue;
        };
        for item in items {
            let Some(text) = item.get("text") else {
                continue;
            };
            let body = text["body"].as_str().unwrap_or_default().to_string();
            texts.push(body.clone());
            if text["finalized"] == true {
                return (body, texts);
            }
        }
    }
}

#[tokio::test]
async fn session_stream_streams_the_exact_durable_final_reply() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text(FINAL_REPLY)])
        .build()
        .await
        .expect("harness builds");
    let services = session_services(&h);
    let caller = webui_caller_for(&h.binding);
    let thread_id = h.binding.thread_id.as_str().to_string();
    let router = mount_webui_v2_router(services, caller);
    let (addr, serve_handle) = serve_router(router.clone()).await;

    h.submit_turn("answer over the session stream")
        .await
        .expect("turn completes");

    let mut stream = open_session_stream(addr, vec![("chat-active", &thread_id)]).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let admitted = stream.next_frame(deadline).await;
    assert_eq!(admitted["type"], "subscribed", "admission: {admitted}");
    assert_eq!(admitted["subscription_id"], "chat-active");

    let (finalized, texts) = collect_until_finalized(&mut stream, "chat-active", deadline).await;
    assert_eq!(
        finalized, FINAL_REPLY,
        "the finalized stream row must carry the exact assistant reply",
    );
    assert!(
        texts.iter().all(|body| FINAL_REPLY.starts_with(body)),
        "every streamed text body must be a prefix of the final reply \
         (cumulative, never reordered fragments): {texts:?}",
    );

    let (status, body) = get_json(
        router,
        &format!("/api/webchat/v2/threads/{thread_id}/timeline"),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "timeline: {body}");
    let durable = body["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .find(|message| message["kind"] == "assistant" && message["status"] == "finalized")
        .unwrap_or_else(|| panic!("no finalized assistant message: {body}"));
    assert_eq!(
        durable["content"].as_str(),
        Some(FINAL_REPLY),
        "session stream and durable transcript must agree byte-for-byte",
    );

    drop(stream);
    serve_handle.abort();
}

#[tokio::test]
async fn one_session_stream_carries_two_threads_independently() {
    let group = RebornIntegrationGroup::builtin_tools()
        .await
        .expect("group builds");
    let thread_a = group
        .thread("conv-session-stream-a")
        .script([RebornScriptedReply::text("alpha thread final reply")])
        .build()
        .await
        .expect("thread a builds");
    let thread_b = group
        .thread("conv-session-stream-b")
        .script([RebornScriptedReply::text("beta thread final reply")])
        .build()
        .await
        .expect("thread b builds");

    let services = session_services(&thread_a);
    let caller = webui_caller_for(&thread_a.binding);
    let router = mount_webui_v2_router(services, caller);
    let (addr, serve_handle) = serve_router(router).await;

    thread_a
        .submit_turn("go alpha")
        .await
        .expect("thread a completes");
    thread_b
        .submit_turn("go beta")
        .await
        .expect("thread b completes");

    let id_a = thread_a.binding.thread_id.as_str().to_string();
    let id_b = thread_b.binding.thread_id.as_str().to_string();
    let mut stream = open_session_stream(addr, vec![("sub-a", &id_a), ("sub-b", &id_b)]).await;

    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut final_a: Option<String> = None;
    let mut final_b: Option<String> = None;
    let mut admitted = 0;
    while final_a.is_none() || final_b.is_none() {
        let frame = stream.next_frame(deadline).await;
        assert_ne!(
            frame["type"], "subscription_error",
            "both selectors must authorize: {frame}"
        );
        if frame["type"] == "subscribed" {
            admitted += 1;
        }
        if frame["type"] != "event" {
            continue;
        }
        let subscription = frame["subscription_id"].as_str().unwrap_or_default();
        let thread_in_frame = frame["event"]["state"]["thread_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if !thread_in_frame.is_empty() {
            match subscription {
                "sub-a" => assert_eq!(
                    thread_in_frame, id_a,
                    "subscription A must only carry thread A"
                ),
                "sub-b" => assert_eq!(
                    thread_in_frame, id_b,
                    "subscription B must only carry thread B"
                ),
                other => panic!("unexpected subscription id {other}"),
            }
        }
        let Some(items) = frame["event"]["state"]["items"].as_array() else {
            continue;
        };
        for item in items {
            let Some(text) = item.get("text") else {
                continue;
            };
            if text["finalized"] != true {
                continue;
            }
            let body = text["body"].as_str().unwrap_or_default().to_string();
            match subscription {
                "sub-a" => final_a = Some(body),
                "sub-b" => final_b = Some(body),
                _ => {}
            }
        }
    }
    assert_eq!(
        admitted, 2,
        "both subscriptions must be explicitly admitted before their replay completes"
    );
    assert_eq!(final_a.as_deref(), Some("alpha thread final reply"));
    assert_eq!(final_b.as_deref(), Some("beta thread final reply"));

    drop(stream);
    serve_handle.abort();
}
