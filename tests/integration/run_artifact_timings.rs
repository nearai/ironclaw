//! Timing evidence in the user-downloadable run artifact, driven through the
//! real product surface (the real `webui_v2` router over a real
//! `RebornServices`, mirroring `webui_v2_product_api.rs`'s pattern).
//!
//! SCOPE NOTE: the integration harness wires the prompt diagnostic sink (so
//! model-call/inference timings are observable) but has NO tool diagnostic
//! sink (`staged_capability_io_for_test` /
//! `staged_capability_io_with_observer_for_test` in
//! `crates/app/ironclaw_composition/src/runtime/capability_host.rs` hardcode
//! `tool_diagnostic_sink: None`, unlike production's `capability_wiring` in
//! `runtime.rs`). Tool-execution timings (`iterations[].tool_calls`,
//! `totals.tool_calls`, per-tool durations) are therefore NOT observable here
//! and are deliberately not asserted below — see the plan's Task 6 report for
//! the follow-up. The brief's third test
//! (`exported_timings_carry_no_tool_arguments_or_results`) is deliberately
//! omitted: with no tool sink there are no tool entries at all, so it would
//! pass vacuously. The no-payload-leak guarantee is already pinned
//! non-vacuously at crate tier by
//! `no_bounded_payload_text_reaches_the_projection` in
//! `crates/product/ironclaw_assistant/src/reborn_services/run_artifact/timings.rs`.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use ironclaw_assistant::{RebornRunArtifact, RebornServices};
use ironclaw_product_contracts::surface::ProductSurface;
use ironclaw_webui::webui_v2::{
    DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2Capabilities, WebUiV2State, webui_v2_router,
};
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::webui_mount::{get_json, webui_caller_for};

/// The run/thread artifact export routes are mounted only when the
/// deployment opts in (`WebUiV2State::with_regression_artifact_export_enabled`,
/// off by default in production). `reborn_support::webui_mount::mount_webui_v2_router`
/// deliberately leaves that flag off for its other callers, so this test
/// builds its own router with the flag on — mirroring
/// `crates/product/ironclaw_webui/tests/webui_v2_handlers_contract.rs::artifact_router_with`.
fn artifact_export_router(
    services: Arc<dyn ProductSurface>,
    caller: ironclaw_product_contracts::surface::ProductSurfaceCaller,
) -> Router {
    webui_v2_router(
        WebUiV2State::new(services, DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER)
            .with_regression_artifact_export_enabled(true),
    )
    .layer(axum::Extension(caller))
    .layer(axum::Extension(WebUiV2Capabilities::default()))
}

#[tokio::test]
async fn exported_run_artifact_carries_per_iteration_timings() {
    // Three model iterations (two tool-call turns, one final reply). Tool
    // execution timing is unobservable in this harness (see module doc), so
    // this drives the run purely to exercise the model-call/inference timing
    // path — the one lane the harness's diagnostic sink actually captures.
    let h = RebornIntegrationHarness::builder("thread-timings")
        .with_builtin_http_tools()
        .with_model_call_delay_for_test(Duration::from_millis(5))
        .script([
            RebornScriptedReply::tool_call(
                "builtin.http",
                serde_json::json!({"url": "https://example.com/a"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.http",
                serde_json::json!({"url": "https://example.com/b"}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness");
    let run_id = h.submit_turn("check the build").await.expect("run");

    let services = RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone())
        .with_diagnostic_store(h.diagnostic_store());
    let router = artifact_export_router(Arc::new(services), webui_caller_for(&h.binding));

    let (status, body) = get_json(
        router,
        &format!(
            "/api/webchat/v2/threads/{}/runs/{}/artifact",
            h.binding.thread_id.as_str(),
            run_id
        ),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "artifact body: {body}");
    let artifact: RebornRunArtifact = serde_json::from_value(body).expect("artifact deserializes");

    assert!(artifact.timings.available);
    assert!(!artifact.timings.complete);
    assert_eq!(artifact.timings.iterations.len(), 3);
    assert_eq!(artifact.timings.totals.iterations, 3);
    assert!(artifact.timings.totals.inference_ms.known_total > 0);
    assert_eq!(artifact.timings.totals.inference_ms.unavailable_samples, 0);
    assert!(artifact.timings.totals.wall_clock_ms.is_some());
    assert!(
        artifact.timings.iterations[0].inference_ms.is_some(),
        "model-call latency arrives through the prompt diagnostic sink"
    );
}

#[tokio::test]
async fn exported_run_artifact_keeps_timestamps_when_timings_were_evicted() {
    let h = RebornIntegrationHarness::builder("thread-evicted")
        .script([RebornScriptedReply::text("hello")])
        .build()
        .await
        .expect("harness");
    let run_id = h.submit_turn("hello").await.expect("run");

    // No `with_diagnostic_store`: services fall back to their own empty store,
    // which is exactly the post-restart / post-eviction case a user hits when
    // filing a bug report the next day.
    let services = RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone());
    let router = artifact_export_router(Arc::new(services), webui_caller_for(&h.binding));

    let (status, body) = get_json(
        router,
        &format!(
            "/api/webchat/v2/threads/{}/runs/{}/artifact",
            h.binding.thread_id.as_str(),
            run_id
        ),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "artifact body: {body}");
    let artifact: RebornRunArtifact =
        serde_json::from_value(body).expect("artifact export must still succeed");

    assert!(!artifact.timings.available);
    assert_eq!(
        artifact.timings.unavailable_reason.as_deref(),
        Some("run_not_resident")
    );
    assert!(
        artifact
            .messages
            .iter()
            .any(|message| message.created_at.is_some()),
        "the durable timestamp floor must survive an absent diagnostic store"
    );
    assert!(artifact.timings.totals.wall_clock_ms.is_some());
}
