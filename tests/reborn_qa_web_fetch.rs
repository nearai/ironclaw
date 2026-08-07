//! QA use-case coverage for web/HTTP fetch flows:
//!
//! - "check if api.github.com returns a 200 status" → the agent reports
//!   the endpoint's current HTTP status.
//! - "summarize the latest release from https://github.com/nearai/ironclaw"
//!   → summary of the most recent release.
//! - "search Hacker News for any recent posts mentioning 'IronClaw' or
//!   'NEAR AI'" → the agent reports matching posts.
//! - "summarize the latest BTC news" → the agent reports a summary built
//!   from the fetched articles (UC1, the content half of the Telegram
//!   daily-news digest; the Telegram inbound half is
//!   `reborn_qa_channel_delivery`).
//! - "list my repos in GitHub" → the agent reports the caller's repos
//!   (UC4). Distinct from the release-summary case above: that one reads a
//!   single release document, this one reads the authenticated repo list.
//! - "check my recent emails and add any from a near.ai address to my
//!   Google Sheet called ABC" → the near.ai inbound is appended as a sheet
//!   row (UC6). The only case here whose second leg is a WRITE, so it is
//!   also the coverage for a mutating provider call reaching the wire with
//!   the row payload intact.
//!
//! External endpoints are replaced by a live loopback HTTP server so the
//! real `builtin.http` capability, network policy, and egress path are
//! exercised deterministically. Gmail/Sheets/GitHub are modeled the same
//! way their QA siblings model Drive (`reborn_qa_doc_grounding`): as real
//! HTTP round trips against the loopback server, so the capability,
//! policy, and egress path are real even though the vendor is not.

#[allow(dead_code)]
#[path = "support/reborn_parity_qa/mod.rs"]
mod parity_qa_support;
#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod reborn_support;
mod support;

use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, Uri, header},
    response::IntoResponse,
    routing::{get, post},
};
use ironclaw_host_api::ids::CapabilityId;
use ironclaw_host_runtime::HTTP_CAPABILITY_ID;
use ironclaw_loop_host::HostManagedModelResponse;
use ironclaw_turns::TurnStatus;
use parity_qa_support::binary_e2e::RebornBinaryE2EHarness;
use parity_qa_support::model_replay::{
    RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
};
use parity_qa_support::network::{
    LiveLoopbackHttpServer, LiveLoopbackHttpState, loopback_http_policy,
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

/// UC1 (Daily news digest), content half: "summarize the latest BTC news".
/// The Telegram inbound/outbound half is
/// `reborn_qa_channel_delivery::reborn_qa_telegram_dm_btc_news_request_gets_reply_in_same_thread`
/// — the installation-scoped harness that binds a channel thread cannot
/// also mount the real host runtime, so the two halves are pinned
/// separately at this tier.
#[tokio::test]
async fn reborn_qa_btc_news_summary_from_web_search() {
    const REPLY: &str = "Latest BTC news: spot ETF inflows hit a monthly high, and a core dev proposal to soften the fee market is under review.";

    let http = CapabilityId::new(HTTP_CAPABILITY_ID).expect("valid capability id");
    let server =
        LiveLoopbackHttpServer::start(Router::new().route("/v1/news", get(btc_news))).await;
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                http.clone(),
                "call_search_btc_news",
                serde_json::json!({
                    "url": server.url("/v1/news?query=bitcoin"),
                    "timeout_ms": 2500,
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(REPLY),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness =
        RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities_live_http_egress(
            "room-qa-btc-news",
            model_gateway,
            loopback_http_policy(server.port()),
        )
        .await
        .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-qa-btc-news", "summarize the latest BTC news")
        .await
        .expect("submit BTC news request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(REPLY)
        .await
        .expect("BTC news summary reply");

    let results = harness.capability_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].capability_id, http);
    assert_eq!(results[0].output["status"], serde_json::json!(200));
    let body = results[0].output["body_text"]
        .as_str()
        .expect("news body text");
    assert!(
        body.contains("spot ETF inflows") && body.contains("fee market"),
        "both fetched articles should reach the model as a tool result, got {body}"
    );

    assert_eq!(
        server.requests(),
        vec!["/v1/news?query=bitcoin".to_string()]
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

/// UC4 (Competitor release tracker): "list my repos in GitHub". The sibling
/// `reborn_qa_latest_release_summary_from_github_api` covers the release
/// document; this covers the account-scoped repo listing, which is the ask
/// the QA script actually types and the only case here that asserts a
/// multi-item collection survives to the model.
#[tokio::test]
async fn reborn_qa_github_repo_list_reports_caller_repos() {
    const REPLY: &str =
        "You have 3 repos: nearai/ironclaw, nearai/holonear, and nearai/near-api-js.";

    let http = CapabilityId::new(HTTP_CAPABILITY_ID).expect("valid capability id");
    let server =
        LiveLoopbackHttpServer::start(Router::new().route("/user/repos", get(user_repos))).await;
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                http.clone(),
                "call_list_repos",
                serde_json::json!({
                    "url": server.url("/user/repos?per_page=100"),
                    "timeout_ms": 2500,
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(REPLY),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness =
        RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities_live_http_egress(
            "room-qa-github-repo-list",
            model_gateway,
            loopback_http_policy(server.port()),
        )
        .await
        .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-qa-github-repo-list", "list my repos in GitHub")
        .await
        .expect("submit repo list request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(REPLY)
        .await
        .expect("repo list reply");

    let results = harness.capability_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].capability_id, http);
    let body = results[0].output["body_text"]
        .as_str()
        .expect("repo list body text");
    for repo in ["nearai/ironclaw", "nearai/holonear", "nearai/near-api-js"] {
        assert!(
            body.contains(repo),
            "every listed repo should reach the model as a tool result, missing {repo} in {body}"
        );
    }

    assert_eq!(
        server.requests(),
        vec!["/user/repos?per_page=100".to_string()]
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

/// UC6 (CRM inbound tracker): "check my recent emails and add any from a
/// near.ai address to my Google Sheet called ABC". Two real HTTP legs in
/// one turn — a read of the inbox and a WRITE that appends the row — so
/// this is the QA case that proves a mutating provider call carries its
/// payload across the egress boundary, not just that a read reaches the
/// model. The recurring form of the same ask is
/// `reborn_qa_routines::reborn_qa_routine_created_for_crm_inbox_sweep_every_30_minutes`.
#[tokio::test]
async fn reborn_qa_near_ai_inbound_email_is_appended_to_sheet() {
    const REPLY: &str =
        "Added 1 new near.ai inbound (dana at near.ai — partnership intro) to the ABC sheet.";

    let http = CapabilityId::new(HTTP_CAPABILITY_ID).expect("valid capability id");
    let server = LiveLoopbackHttpServer::start(
        Router::new()
            .route("/gmail/v1/users/me/messages", get(recent_messages))
            .route(
                "/sheets/v4/spreadsheets/ABC/values/append",
                post(append_row),
            ),
    )
    .await;
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                http.clone(),
                "call_read_inbox",
                serde_json::json!({
                    "url": server.url("/gmail/v1/users/me/messages?q=newer_than%3A1d"),
                    "timeout_ms": 2500,
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                http.clone(),
                "call_append_sheet_row",
                serde_json::json!({
                    "url": server.url("/sheets/v4/spreadsheets/ABC/values/append"),
                    "method": "post",
                    "body": r#"{"values":[["dana@near.ai","Partnership intro"]]}"#,
                    "timeout_ms": 2500,
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(REPLY),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness =
        RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities_live_http_egress(
            "room-qa-crm-inbox-sweep",
            model_gateway,
            loopback_http_policy(server.port()),
        )
        .await
        .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-qa-crm-inbox-sweep",
            "check my recent emails and add any from a near.ai address to my Google Sheet called ABC",
        )
        .await
        .expect("submit CRM sweep request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(REPLY)
        .await
        .expect("CRM sweep reply");

    let results = harness.capability_results();
    assert_eq!(results.len(), 2, "one inbox read plus one sheet append");
    let inbox_body = results[0].output["body_text"]
        .as_str()
        .expect("inbox body text");
    assert!(
        inbox_body.contains("dana@near.ai"),
        "the near.ai inbound should reach the model, got {inbox_body}"
    );
    assert!(
        inbox_body.contains("marco@example.com"),
        "the non-near.ai inbound should also reach the model so the filtering \
         decision is the model's, not the fixture's — got {inbox_body}"
    );
    assert_eq!(results[1].output["status"], serde_json::json!(200));

    // The append must have carried the row payload across the egress
    // boundary; asserting only on the reply would pass with an empty body.
    let appended = server.request_bodies();
    assert_eq!(
        appended.len(),
        1,
        "exactly one sheet append should cross the wire, got {appended:?}"
    );
    assert!(
        appended[0].contains("dana@near.ai") && appended[0].contains("Partnership intro"),
        "the appended row payload should reach the sheet endpoint, got {}",
        appended[0]
    );
    assert!(
        !appended[0].contains("marco@example.com"),
        "the non-near.ai inbound must not be appended, got {}",
        appended[0]
    );

    assert_eq!(
        server.requests(),
        vec![
            "/gmail/v1/users/me/messages?q=newer_than%3A1d".to_string(),
            "/sheets/v4/spreadsheets/ABC/values/append".to_string(),
        ]
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

async fn btc_news(State(state): State<LiveLoopbackHttpState>, uri: Uri) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "articles": [
                {
                    "title": "Spot ETF inflows hit a monthly high",
                    "summary": "Institutional spot ETF inflows reached their highest level this month.",
                },
                {
                    "title": "Core dev proposal to soften the fee market",
                    "summary": "A proposal to soften the fee market is under review by core developers.",
                },
            ],
        })),
    )
        .into_response()
}

async fn user_repos(State(state): State<LiveLoopbackHttpState>, uri: Uri) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!([
            {"full_name": "nearai/ironclaw", "private": false},
            {"full_name": "nearai/holonear", "private": true},
            {"full_name": "nearai/near-api-js", "private": false},
        ])),
    )
        .into_response()
}

/// Two inbounds, only one of them from a near.ai address — the filtering
/// decision has to be the model's, so the fixture must offer a negative.
async fn recent_messages(
    State(state): State<LiveLoopbackHttpState>,
    uri: Uri,
) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "messages": [
                {"from": "dana@near.ai", "subject": "Partnership intro"},
                {"from": "marco@example.com", "subject": "Conference invite"},
            ],
        })),
    )
        .into_response()
}

async fn append_row(
    State(state): State<LiveLoopbackHttpState>,
    uri: Uri,
    body: String,
) -> impl IntoResponse {
    state.record_with_body(&uri, &body);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({"updates": {"updatedRows": 1}})),
    )
        .into_response()
}
