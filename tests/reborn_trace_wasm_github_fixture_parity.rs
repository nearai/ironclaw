#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_host_api::CapabilityId;
use ironclaw_loop_support::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_turns::TurnStatus;
use reborn_support::{
    harness::RebornBinaryE2EHarness,
    model_replay::{RebornModelReplayStep, RebornTraceReplayModelGateway},
};

#[tokio::test]
async fn reborn_trace_advertises_github_v2_wasm_capabilities() {
    let expected_capabilities = vec![
        CapabilityId::new("github.search_issues").expect("valid capability id"),
        CapabilityId::new("github.get_issue").expect("valid capability id"),
        CapabilityId::new("github.comment_issue").expect("valid capability id"),
    ];
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::AssertProviderToolsThenResponse {
            capability_ids: expected_capabilities,
            response: HostManagedModelResponse::assistant_reply("github wasm trace complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_github_issue_capabilities(
        "room-trace-github-wasm",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-trace-github-wasm", "show GitHub issue tools")
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("github wasm trace complete")
        .await
        .expect("final reply");

    assert_eq!(
        harness.capability_invocations(),
        Vec::new(),
        "advertisement trace must not call live GitHub or execute the WASM module"
    );
    let requests = harness.model_requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.role == HostManagedModelMessageRole::User
                && message.content.contains("show GitHub issue tools")),
        "trace should exercise the real inbound user-to-model path"
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}
