#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use ironclaw_host_api::CapabilityId;
use ironclaw_host_api::{NetworkPolicy, NetworkScheme, NetworkTargetPattern};
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, HTTP_CAPABILITY_ID, JSON_CAPABILITY_ID, READ_FILE_CAPABILITY_ID,
    TIME_CAPABILITY_ID,
};
use ironclaw_loop_support::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_turns::{TurnStatus, run_profile::LoopHostMilestoneKind};
use reborn_support::{
    harness::{HarnessWaitConfig, RebornBinaryE2EHarness, assert_milestone_order},
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

const PATCHED_CONTENT: &str = "alpha\npatched\nomega\n";

#[tokio::test]
async fn reborn_trace_core_builtin_tools_parity() {
    let time = CapabilityId::new(TIME_CAPABILITY_ID).expect("valid capability id");
    let json = CapabilityId::new(JSON_CAPABILITY_ID).expect("valid capability id");
    let http = CapabilityId::new(HTTP_CAPABILITY_ID).expect("valid capability id");
    let read_file = CapabilityId::new(READ_FILE_CAPABILITY_ID).expect("valid capability id");
    let apply_patch = CapabilityId::new(APPLY_PATCH_CAPABILITY_ID).expect("valid capability id");
    let live_http = LiveHttpServer::start().await;
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![
                RebornScriptedProviderToolCall::new(
                    time.clone(),
                    "call_time_diff",
                    serde_json::json!({
                        "operation": "diff",
                        "input": "2026-05-12T13:00:00Z",
                        "timestamp2": "2026-05-12T15:30:00Z",
                    }),
                ),
                RebornScriptedProviderToolCall::new(
                    json.clone(),
                    "call_json_query",
                    serde_json::json!({
                        "operation": "query",
                        "data": {"items": [{"name": "alpha"}]},
                        "path": "items[0].name",
                    }),
                ),
                RebornScriptedProviderToolCall::new(
                    http.clone(),
                    "call_http_get",
                    serde_json::json!({
                        "url": live_http.url("/v1/items"),
                        "headers": {"x-request-id": "reborn-core-builtins"},
                        "timeout_ms": 2500,
                    }),
                ),
            ],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                read_file.clone(),
                "call_read_patch_target",
                serde_json::json!({"path": "/workspace/patch-target.txt"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                apply_patch.clone(),
                "call_apply_patch",
                serde_json::json!({
                    "path": "/workspace/patch-target.txt",
                    "old_string": "needs-patch",
                    "new_string": "patched",
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("core builtins trace complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness =
        RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities_live_http_egress(
            "room-trace-core-builtins",
            model_gateway,
            live_http_network_policy(live_http.port),
        )
        .await
        .expect("harness");
    seed_workspace(&harness);
    harness.start();

    let submitted = harness
        .submit_text("event-trace-core-builtins", "exercise core builtins")
        .await
        .expect("submit text");
    harness
        .wait_for_status_with_config(
            submitted.run_id,
            TurnStatus::Completed,
            HarnessWaitConfig {
                timeout: Duration::from_secs(15),
                poll_interval: Duration::from_millis(10),
            },
        )
        .await
        .expect("completed run");
    harness
        .assert_final_reply("core builtins trace complete")
        .await
        .expect("final reply");

    let patched_path = harness
        .host_workspace_file_path("patch-target.txt")
        .expect("patch target path");
    assert_eq!(
        std::fs::read_to_string(patched_path).expect("patched file"),
        PATCHED_CONTENT
    );

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 5);
    assert_eq!(invocations[0].capability_id, time);
    assert_eq!(invocations[1].capability_id, json);
    assert_eq!(invocations[2].capability_id, http);
    assert_eq!(invocations[3].capability_id, read_file);
    assert_eq!(invocations[4].capability_id, apply_patch);

    let seen_requests = live_http.requests();
    assert_eq!(seen_requests.len(), 1);
    assert_eq!(seen_requests[0].path, "/v1/items");
    assert_eq!(
        seen_requests[0].request_id.as_deref(),
        Some("reborn-core-builtins")
    );

    let results = harness.capability_results();
    assert_eq!(results[2].capability_id, http);
    assert_eq!(results[2].output["status"], serde_json::json!(200));
    assert_eq!(
        results[2].output["body_text"],
        serde_json::json!(r#"{"accepted":true,"source":"live-loopback"}"#)
    );

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(tool_result_count(&requests[1]), 3);
    assert_eq!(tool_result_count(&requests[2]), 4);
    assert_eq!(tool_result_count(&requests[3]), 5);
    assert!(
        requests[3].messages.iter().any(|message| message.role
            == HostManagedModelMessageRole::ToolResult
            && message.content.contains("result:")),
        "apply_patch result ref should be visible before final reply"
    );

    assert_milestone_order(
        &harness.milestones(),
        |kind| matches!(kind, LoopHostMilestoneKind::CapabilityBatchCompleted { .. }),
        |kind| matches!(kind, LoopHostMilestoneKind::AssistantReplyFinalized { .. }),
    );

    tokio::task::yield_now().await;
    harness.shutdown().await;
}

fn seed_workspace(harness: &RebornBinaryE2EHarness) {
    std::fs::write(
        harness
            .host_workspace_file_path("patch-target.txt")
            .expect("patch target path"),
        "alpha\nneeds-patch\nomega\n",
    )
    .expect("write patch target");
}

fn tool_result_count(request: &ironclaw_loop_support::HostManagedModelRequest) -> usize {
    request
        .messages
        .iter()
        .filter(|message| message.role == HostManagedModelMessageRole::ToolResult)
        .count()
}

struct LiveHttpServer {
    port: u16,
    requests: Arc<Mutex<Vec<LiveHttpRequest>>>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveHttpRequest {
    path: String,
    request_id: Option<String>,
}

impl LiveHttpServer {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind live HTTP test server");
        let port = listener.local_addr().expect("local addr").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = LiveHttpState {
            requests: Arc::clone(&requests),
        };
        let app = Router::new()
            .route("/v1/items", get(live_items))
            .with_state(state);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            port,
            requests,
            task,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn requests(&self) -> Vec<LiveHttpRequest> {
        self.requests
            .lock()
            .expect("live HTTP request log lock poisoned")
            .clone()
    }
}

impl Drop for LiveHttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone)]
struct LiveHttpState {
    requests: Arc<Mutex<Vec<LiveHttpRequest>>>,
}

async fn live_items(State(state): State<LiveHttpState>, headers: HeaderMap) -> impl IntoResponse {
    state
        .requests
        .lock()
        .expect("live HTTP request log lock poisoned")
        .push(LiveHttpRequest {
            path: "/v1/items".to_string(),
            request_id: headers
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .map(ToString::to_string),
        });

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({"accepted": true, "source": "live-loopback"})),
    )
        .into_response()
}

fn live_http_network_policy(port: u16) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Http),
            host_pattern: "127.0.0.1".to_string(),
            port: Some(port),
        }],
        deny_private_ip_ranges: false,
        max_egress_bytes: Some(10_000),
    }
}
