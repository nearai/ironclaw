//! Client-tool denial through the production coordinator and PlannedDriver.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_turns::{ExternalToolSpec, TurnStatus};
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

#[tokio::test]
async fn external_tool_gate_denied_resume_does_not_redispatch_parked_call() {
    let external_tool = ExternalToolSpec::new(
        "client_weather",
        "Look up weather in the caller",
        json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"],
            "additionalProperties": false
        }),
    )
    .expect("valid external-tool spec");
    let group = RebornIntegrationGroup::builder()
        .with_external_tools_for_test(vec![external_tool])
        .builtin_tools()
        .await
        .expect("external-tool group builds");
    let harness = group
        .thread("conv-external-tool-denied-resume")
        .script([
            RebornScriptedReply::tool_call("client_weather", json!({ "city": "Paris" })),
            RebornScriptedReply::tool_call("builtin.time", json!({})),
            RebornScriptedReply::text("the client tool was cancelled; the host tool still ran"),
        ])
        .build()
        .await
        .expect("thread builds");

    let run_id = harness
        .submit_turn_async("check the weather, then report the current time")
        .await
        .expect("turn accepted");

    let blocked = harness
        .wait_for_status(run_id, TurnStatus::BlockedExternalTool)
        .await
        .expect("client tool parks the real run");
    let gate_ref = blocked
        .gate_ref
        .expect("blocked external-tool run carries a gate ref");
    harness
        .deny_external_tool_gate(run_id, &gate_ref)
        .await
        .expect("external-tool gate denied through coordinator");
    harness
        .wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("denied parked call is not re-dispatched into another external-tool block");

    harness
        .assert_capability_result_count("external_tool.client_weather", 0)
        .await
        .expect("denied client tool never fabricates an execution result");
    harness
        .assert_tool_error_summary_contains("external tool gate cancelled by client")
        .await
        .expect("denial is persisted as a model-visible tool outcome");
    harness
        .assert_tool_invocation_count("builtin.time", 1)
        .await
        .expect("an unrelated capability still dispatches after denial");
}
