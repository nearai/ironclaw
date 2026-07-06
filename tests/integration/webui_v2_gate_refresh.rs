//! W5-WEBUI-API-2: a browser refresh mid-gate must let the user rediscover
//! and resolve a pending approval gate. Mounts the real `webui_v2` router
//! over a hand-built `RebornServices` facade (mirrors `webui_v2_product_api.rs`)
//! wired with the harness's own turn-state-converged
//! `ApprovalInteractionService` (`local_dev_approval_interaction_service_with_turn_state_for_test`,
//! the same seam `RebornIntegrationGroupBuilder::with_real_gate_dispatch_services`
//! wires into `DefaultProductWorkflow`) and the production event-stream recipe
//! `sse_activity_stream_replay_and_reconnect` already pins.
//!
//! "Refresh" is simulated the same way that precedent does: a fresh
//! `stream_events` drain with `after_cursor: None` — the SSE handler is a
//! polling wrapper over the same drain (W5-WEBUI-SPIKE), so this is
//! behaviorally equivalent to a browser opening a brand new `EventSource`
//! after a cold reload, without the fragility of reading a chunked HTTP body
//! through `tower::ServiceExt::oneshot`.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::Arc;

use axum::http::StatusCode;
use ironclaw_events::InMemoryDurableEventLog;
use ironclaw_product_adapters::ProductOutboundPayload;
use ironclaw_product_workflow::{RebornServices, RebornServicesApi, RebornStreamEventsRequest};
use ironclaw_turns::{ReplyTargetBindingRef, TurnEventProjectionSource, TurnStatus};
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::webui_mount::{mount_webui_v2_router, post_json, webui_caller_for};

#[tokio::test]
async fn approval_gate_rediscovered_and_resolved_after_refresh() {
    let group = RebornIntegrationGroup::live_approvals()
        .await
        .expect("live-approvals group builds");
    let h = group
        .thread("conv-webui-api2-approval-refresh")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                serde_json::json!({"path": "/workspace/api2_refresh_approved.txt", "content": "API2_REFRESH_PAYLOAD"}),
            ),
            RebornScriptedReply::text("file written after the post-refresh approval"),
        ])
        .build()
        .await
        .expect("thread builds");

    let (run_id, gate_ref) = h
        .submit_turn_until_blocked("write the api2 refresh file")
        .await
        .expect("blocks on a real approval gate");

    // Wire the REAL approval interaction service over the group's own shared
    // turn-state store — same test-support seam
    // `with_real_gate_dispatch_services` uses for `DefaultProductWorkflow`,
    // applied here directly to a webui-level `RebornServices` instead.
    let capability_harness = group
        .capability_harness()
        .expect("live_approvals always uses a HostRuntime capability backend");
    let reborn_services = capability_harness
        .reborn_services_for_test()
        .expect("live_approvals harness is built via new_with_options");
    let approval_interactions = reborn_services
        .local_dev_approval_interaction_service_with_turn_state_for_test(
            h.coordinator.clone(),
            h.turn_store.clone(),
        )
        .expect("local-dev capability policy is valid")
        .expect("harness has a local-dev runtime");

    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let reply_target_binding_ref =
        ReplyTargetBindingRef::new("webui-api2-test").expect("valid reply target binding ref");
    let turn_event_source: Arc<dyn TurnEventProjectionSource> = h.turn_store.clone();
    let event_stream = ironclaw_reborn_composition::test_support::build_webui_event_stream_for_test(
        event_log,
        turn_event_source,
        h.coordinator.clone(),
        reply_target_binding_ref,
    );
    let services: Arc<dyn RebornServicesApi> = Arc::new(
        RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone())
            .with_event_stream(event_stream)
            .with_approval_interactions(approval_interactions),
    );
    let caller = webui_caller_for(&h.binding);
    let thread_id = h.binding.thread_id.as_str().to_string();

    // --- simulate a cold browser refresh: fresh drain, after_cursor: None ---
    let replayed = services
        .stream_events(
            caller.clone(),
            RebornStreamEventsRequest {
                thread_id: thread_id.clone(),
                after_cursor: None,
            },
        )
        .await
        .expect("post-refresh drain succeeds");
    let gate_prompt = replayed
        .events
        .iter()
        .find_map(|envelope| match &envelope.payload {
            ProductOutboundPayload::GatePrompt(view) if view.gate_ref == gate_ref.as_str() => {
                Some(view)
            }
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "expected the replayed cold-refresh drain to surface a GatePrompt for {gate_ref:?}: {:?}",
                replayed.events
            )
        });
    assert_eq!(
        gate_prompt.turn_run_id, run_id,
        "replayed gate prompt must be for the actual blocked run"
    );

    // --- resolve via the REAL route, not a direct-resume test shortcut ---
    let (status, body) = post_json(
        mount_webui_v2_router(services.clone(), caller),
        &format!(
            "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/gates/{}/resolve",
            gate_ref.as_str()
        ),
        serde_json::json!({
            "client_action_id": "webui-api2-approve-after-refresh",
            "resolution": "approved",
            "always": false,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "resolve_gate response body: {body}");

    h.wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("run completes after the real resolve_gate route resumes it");
    h.assert_workspace_file_contains("api2_refresh_approved.txt", "API2_REFRESH_PAYLOAD")
        .await
        .expect("the approved write actually re-dispatched and persisted");
}
