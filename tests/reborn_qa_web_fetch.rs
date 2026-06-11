//! QA use-case coverage for web/HTTP fetch flows:
//!
//! - "check if api.github.com returns a 200 status" → the agent reports
//!   the endpoint's current HTTP status.
//! - "summarize the latest release from https://github.com/nearai/ironclaw"
//!   → summary of the most recent release.
//! - "search Hacker News for any recent posts mentioning 'IronClaw' or
//!   'NEAR AI'" → the agent reports matching posts.
//!
//! External endpoints are replaced by a live loopback HTTP server so the
//! real `builtin.http` capability, network policy, and egress path are
//! exercised deterministically.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, Uri, header},
    response::IntoResponse,
    routing::get,
};
use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::HTTP_CAPABILITY_ID;
use ironclaw_loop_support::HostManagedModelResponse;
use ironclaw_turns::TurnStatus;
use reborn_support::{
    harness::RebornBinaryE2EHarness,
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
    network::{LiveLoopbackHttpServer, LiveLoopbackHttpState, loopback_http_policy},
};

#[tokio::test]
async fn reborn_qa_endpoint_status_check_reports_http_200() {
    let http = CapabilityId::new(HTTP_CAPABILITY_ID).expect("valid capability id");
    let server =
        LiveLoopbackHttpServer::start(Router::new().route("/status", get(status_ok))).await;
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                http.clone(),
                "call_status_check",
                serde_json::json!({
                    "url": server.url("/status"),
                    "timeout_ms": 2500,
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "The endpoint returned HTTP 200 OK",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness =
        RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities_live_http_egress(
            "room-qa-endpoint-status",
            model_gateway,
            loopback_http_policy(server.port()),
        )
        .await
        .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-qa-endpoint-status",
            "check if api.github.com returns a 200 status",
        )
        .await
        .expect("submit status check request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("The endpoint returned HTTP 200 OK")
        .await
        .expect("status reply");

    let results = harness.capability_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].capability_id, http);
    assert_eq!(results[0].output["status"], serde_json::json!(200));

    assert_eq!(server.requests(), vec!["/status".to_string()]);
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_qa_latest_release_summary_from_github_api() {
    let http = CapabilityId::new(HTTP_CAPABILITY_ID).expect("valid capability id");
    let server = LiveLoopbackHttpServer::start(Router::new().route(
        "/repos/nearai/ironclaw/releases/latest",
        get(latest_release),
    ))
    .await;
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                http.clone(),
                "call_fetch_latest_release",
                serde_json::json!({
                    "url": server.url("/repos/nearai/ironclaw/releases/latest"),
                    "timeout_ms": 2500,
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "Latest release v0.9.0: adds Reborn WebUI operator observability routes",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness =
        RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities_live_http_egress(
            "room-qa-release-summary",
            model_gateway,
            loopback_http_policy(server.port()),
        )
        .await
        .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-qa-release-summary",
            "summarize the latest release from https://github.com/nearai/ironclaw",
        )
        .await
        .expect("submit release summary request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(
            "Latest release v0.9.0: adds Reborn WebUI operator observability routes",
        )
        .await
        .expect("release summary reply");

    let results = harness.capability_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].capability_id, http);
    assert_eq!(results[0].output["status"], serde_json::json!(200));
    let body = results[0].output["body_text"]
        .as_str()
        .expect("release body text");
    assert!(
        body.contains("v0.9.0"),
        "release payload should reach the model as a tool result, got {body}"
    );

    assert_eq!(
        server.requests(),
        vec!["/repos/nearai/ironclaw/releases/latest".to_string()]
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_qa_hacker_news_keyword_search_reports_matches() {
    let http = CapabilityId::new(HTTP_CAPABILITY_ID).expect("valid capability id");
    let server =
        LiveLoopbackHttpServer::start(Router::new().route("/api/v1/search", get(hn_search))).await;
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                http.clone(),
                "call_search_hn",
                serde_json::json!({
                    "url": server.url("/api/v1/search?query=IronClaw%20OR%20%22NEAR%20AI%22"),
                    "timeout_ms": 2500,
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "Found 2 matching Hacker News posts: 'IronClaw secure personal agents' and 'NEAR AI ships cloud API'",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness =
        RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities_live_http_egress(
            "room-qa-hn-search",
            model_gateway,
            loopback_http_policy(server.port()),
        )
        .await
        .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-qa-hn-search",
            "search Hacker News for any recent posts mentioning 'IronClaw' or 'NEAR AI'",
        )
        .await
        .expect("submit HN search request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(
            "Found 2 matching Hacker News posts: 'IronClaw secure personal agents' and 'NEAR AI ships cloud API'",
        )
        .await
        .expect("HN search reply");

    let results = harness.capability_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].capability_id, http);
    let body = results[0].output["body_text"]
        .as_str()
        .expect("search body text");
    assert!(
        body.contains("IronClaw") && body.contains("NEAR AI"),
        "search payload should include both keyword matches, got {body}"
    );

    assert_eq!(
        server.requests(),
        vec!["/api/v1/search?query=IronClaw%20OR%20%22NEAR%20AI%22".to_string()]
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

async fn status_ok(State(state): State<LiveLoopbackHttpState>, uri: Uri) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({"status": "ok"})),
    )
        .into_response()
}

async fn latest_release(State(state): State<LiveLoopbackHttpState>, uri: Uri) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "tag_name": "v0.9.0",
            "name": "IronClaw v0.9.0",
            "body": "Adds Reborn WebUI operator observability routes",
        })),
    )
        .into_response()
}

async fn hn_search(State(state): State<LiveLoopbackHttpState>, uri: Uri) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "hits": [
                {"title": "IronClaw secure personal agents", "points": 128},
                {"title": "NEAR AI ships cloud API", "points": 96},
            ],
        })),
    )
        .into_response()
}
