mod support;

use ironclaw_product_workflow::WebUiGateResolution;
use support::{MockServer, ScriptedResponse};

#[tokio::test]
async fn send_message_posts_content_only_body() {
    let server = MockServer::start().await;
    server.queue(
        "POST /api/webchat/v2/threads/thread-1/messages",
        ScriptedResponse::ok(serde_json::json!({
            "outcome": "submitted",
            "thread_id": "thread-1",
            "accepted_message_ref": "msg-1",
            "turn_id": "turn-1",
            "run_id": "run-1",
            "status": "queued",
            "resolved_run_profile_id": "default",
            "resolved_run_profile_version": 1,
            "event_cursor": 1
        })),
    );

    let client = server.client();
    client
        .send_message("thread-1", "hello")
        .await
        .expect("send message");

    let body = server.requests()[0].body.clone().expect("body");
    assert_eq!(body["content"], "hello");
}

#[tokio::test]
async fn resolve_gate_approved_sends_resolution_and_always() {
    let server = MockServer::start().await;
    server.queue(
        "POST /api/webchat/v2/threads/thread-1/runs/run-1/gates/gate-1/resolve",
        ScriptedResponse::ok(serde_json::json!({"outcome": "resumed", "run_id": "run-1"})),
    );

    let client = server.client();
    client
        .resolve_gate(
            "thread-1",
            "run-1",
            "gate-1",
            WebUiGateResolution::Approved { always: true },
        )
        .await
        .expect("resolve gate");

    let body = server.requests()[0].body.clone().expect("body");
    assert_eq!(body["resolution"], "approved");
    assert_eq!(body["always"], true);
    assert!(body.get("credential_ref").is_none());
}

#[tokio::test]
async fn resolve_gate_declined_sends_denied_resolution() {
    let server = MockServer::start().await;
    server.queue(
        "POST /api/webchat/v2/threads/thread-1/runs/run-1/gates/gate-0/resolve",
        ScriptedResponse::ok(serde_json::json!({"outcome": "resumed", "run_id": "run-1"})),
    );

    let client = server.client();
    client
        .resolve_gate("thread-1", "run-1", "gate-0", WebUiGateResolution::Declined)
        .await
        .expect("resolve gate");

    let body = server.requests()[0].body.clone().expect("body");
    // Wire value must be "denied" (not "declined") — parse_gate_resolution
    // (webui_inbound.rs) only accepts "denied"/"cancelled" for this variant.
    assert_eq!(body["resolution"], "denied");
    assert!(body.get("always").is_none());
}

#[tokio::test]
async fn resolve_gate_credential_provided_sends_credential_ref() {
    let server = MockServer::start().await;
    server.queue(
        "POST /api/webchat/v2/threads/thread-1/runs/run-1/gates/gate-2/resolve",
        ScriptedResponse::ok(serde_json::json!({"outcome": "resumed", "run_id": "run-1"})),
    );

    let client = server.client();
    client
        .resolve_gate(
            "thread-1",
            "run-1",
            "gate-2",
            WebUiGateResolution::CredentialProvided {
                credential_ref: "cred-ref-1".to_string(),
            },
        )
        .await
        .expect("resolve gate");

    let body = server.requests()[0].body.clone().expect("body");
    assert_eq!(body["resolution"], "credential_provided");
    assert_eq!(body["credential_ref"], "cred-ref-1");
    assert!(body.get("always").is_none());
}
