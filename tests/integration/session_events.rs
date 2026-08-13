//! Whole-path evidence for the session event transport (2026-08-13 design,
//! Phase 0): a REAL turn through the production workflow, streamed over a
//! REAL WebSocket connection to the session route mounted from the real
//! `webui_v2` router, ending with the exact durable finalized assistant
//! reply the HTTP timeline serves. Also proves two logical subscriptions on
//! one socket deliver their own threads independently.
//!
//! The single fake is the scripted model at the vendor-SDK seam
//! (`tests/integration/CLAUDE.md`); the caller extension is injected the way
//! the bearer middleware would after consuming a single-use socket ticket
//! (the ticket protocol itself is pinned at the webui crate tier in
//! `auth_route_contract.rs`).

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use ironclaw_assistant::RebornServices;
use ironclaw_event_log::InMemoryDurableEventLog;
use ironclaw_turns::{ReplyTargetBindingRef, TurnEventProjectionSource};
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::webui_mount::{get_json, mount_webui_v2_router, webui_caller_for};
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const FINAL_REPLY: &str = "the precise durable answer for the session socket";

fn session_services(h: &RebornIntegrationHarness) -> Arc<RebornServices> {
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let reply_target_binding_ref =
        ReplyTargetBindingRef::new("session-socket-test").expect("valid reply target binding ref");
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

type SessionSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect_session_socket(addr: std::net::SocketAddr) -> SessionSocket {
    let url = format!("ws://{addr}/api/webchat/v2/session/websocket");
    let (socket, response) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(url),
    )
    .await
    .expect("session socket connects within 5s")
    .expect("session socket upgrades");
    assert_eq!(response.status().as_u16(), 101);
    socket
}

async fn subscribe(socket: &mut SessionSocket, subscription_id: &str, thread_id: &str) {
    let frame = serde_json::json!({
        "type": "subscribe",
        "subscription_id": subscription_id,
        "selector": {"kind": "thread", "thread_id": thread_id},
        "after_cursor": null,
    });
    socket
        .send(WsMessage::Text(frame.to_string().into()))
        .await
        .expect("subscribe frame sends");
}

async fn next_frame(socket: &mut SessionSocket, deadline: std::time::Instant) -> Value {
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, socket.next()).await {
            Ok(Some(Ok(WsMessage::Text(text)))) => {
                return serde_json::from_str(&text).expect("session frame parses");
            }
            Ok(Some(Ok(WsMessage::Close(_)))) | Ok(None) => panic!("session socket closed early"),
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(error))) => panic!("session socket error: {error}"),
            Err(_) => panic!("no session frame before deadline"),
        }
    }
}

/// Collect `event` frames for `subscription_id` until one carries the
/// finalized transcript row, returning `(finalized_body, all_texts)` where
/// `all_texts` is every text body observed for the run in delivery order.
async fn collect_until_finalized(
    socket: &mut SessionSocket,
    subscription_id: &str,
    deadline: std::time::Instant,
) -> (String, Vec<String>) {
    let mut texts = Vec::new();
    loop {
        let frame = next_frame(socket, deadline).await;
        assert_eq!(frame["schema"], "webui.session_event.v1");
        if frame["type"] != "event" || frame["subscription_id"] != subscription_id {
            continue;
        }
        let event = &frame["event"];
        // The browser event body never carries extension-delivery routing
        // metadata — that envelope stops at the product boundary.
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

/// The core Phase 0 proof: a turn submitted through the production workflow
/// streams over one session-socket subscription and ends with the exact
/// finalized assistant reply that the durable HTTP timeline serves.
#[tokio::test]
async fn session_socket_streams_the_exact_durable_final_reply() {
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

    // The thread materializes on first admission; complete the turn through
    // the production workflow, then subscribe. The session subscription
    // replays the durable projection from the origin — the reconnect path
    // every browser exercises — and must end with the exact finalized reply.
    h.submit_turn("answer over the session socket")
        .await
        .expect("turn completes");

    let mut socket = connect_session_socket(addr).await;
    subscribe(&mut socket, "chat-active", &thread_id).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let admitted = next_frame(&mut socket, deadline).await;
    assert_eq!(admitted["type"], "subscribed", "admission: {admitted}");
    assert_eq!(admitted["subscription_id"], "chat-active");

    let (finalized, texts) = collect_until_finalized(&mut socket, "chat-active", deadline).await;
    assert_eq!(
        finalized, FINAL_REPLY,
        "the finalized stream row must carry the exact assistant reply",
    );
    assert!(
        texts.iter().all(|body| FINAL_REPLY.starts_with(body)),
        "every streamed text body must be a prefix of the final reply \
         (cumulative, never reordered fragments): {texts:?}",
    );

    // The durable transcript is the source of truth: the HTTP timeline's
    // finalized assistant message must match the streamed reply exactly.
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
        "session socket and durable transcript must agree byte-for-byte",
    );

    let _ = socket.close(None).await;
    serve_handle.abort();
}

/// Two logical subscriptions on one physical socket deliver their own
/// threads independently: each receives its own finalized reply, and
/// neither ever receives a frame tagged for the other.
#[tokio::test]
async fn one_session_socket_carries_two_threads_independently() {
    let group = RebornIntegrationGroup::builtin_tools()
        .await
        .expect("group builds");
    let thread_a = group
        .thread("conv-session-socket-a")
        .script([RebornScriptedReply::text("alpha thread final reply")])
        .build()
        .await
        .expect("thread a builds");
    let thread_b = group
        .thread("conv-session-socket-b")
        .script([RebornScriptedReply::text("beta thread final reply")])
        .build()
        .await
        .expect("thread b builds");

    // One event stream over the shared runtime serves both threads; each
    // subscription's scope comes from its own selector authorization.
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

    let mut socket = connect_session_socket(addr).await;
    let id_a = thread_a.binding.thread_id.as_str().to_string();
    let id_b = thread_b.binding.thread_id.as_str().to_string();
    subscribe(&mut socket, "sub-a", &id_a).await;
    subscribe(&mut socket, "sub-b", &id_b).await;

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut admitted = 0;
    while admitted < 2 {
        let frame = next_frame(&mut socket, deadline).await;
        assert_ne!(
            frame["type"], "subscription_error",
            "both selectors must authorize: {frame}"
        );
        if frame["type"] == "subscribed" {
            admitted += 1;
        }
    }

    // Drain frames until each subscription produced its own finalized reply.
    let mut final_a: Option<String> = None;
    let mut final_b: Option<String> = None;
    while final_a.is_none() || final_b.is_none() {
        let frame = next_frame(&mut socket, deadline).await;
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
    assert_eq!(final_a.as_deref(), Some("alpha thread final reply"));
    assert_eq!(final_b.as_deref(), Some("beta thread final reply"));

    let _ = socket.close(None).await;
    serve_handle.abort();
}
