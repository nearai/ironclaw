#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_host_api::{CapabilityId, NetworkPolicy, NetworkScheme, NetworkTargetPattern};
use ironclaw_host_runtime::HTTP_CAPABILITY_ID;
use ironclaw_loop_support::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_turns::TurnStatus;
use reborn_support::{
    harness::RebornBinaryE2EHarness,
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

#[tokio::test]
async fn reborn_http_network_scope_isolation_parity() {
    let http = CapabilityId::new(HTTP_CAPABILITY_ID).expect("valid capability id");
    let allowed_gateway = http_gateway(
        http.clone(),
        "call_http_allowed",
        "allowed network policy reply",
    );
    let denied_gateway = http_gateway(
        http.clone(),
        "call_http_denied",
        "denied network policy reply",
    );

    let mut allowed =
        RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities_network_policy(
            "room-http-network-allowed",
            allowed_gateway,
            allow_api_example_policy(),
        )
        .await
        .expect("allowed harness");
    let mut denied =
        RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities_network_policy(
            "room-http-network-denied",
            denied_gateway,
            deny_all_network_policy(),
        )
        .await
        .expect("denied harness");

    allowed.start();
    denied.start();

    let allowed_turn = allowed
        .submit_text("event-http-network-allowed", "allowed http request")
        .await
        .expect("submit allowed turn");
    allowed
        .wait_for_submitted_status(&allowed_turn, TurnStatus::Completed)
        .await
        .expect("allowed run completed");

    let denied_turn = denied
        .submit_text("event-http-network-denied", "denied http request")
        .await
        .expect("submit denied turn");
    denied
        .wait_for_submitted_status(&denied_turn, TurnStatus::Completed)
        .await
        .expect("denied run completed after surfacing scoped network policy rejection");

    assert!(
        allowed
            .capability_invocations()
            .iter()
            .any(|invocation| invocation.capability_id == http),
        "allowed scope should invoke HTTP capability"
    );
    assert!(
        denied
            .capability_invocations()
            .iter()
            .any(|invocation| invocation.capability_id == http),
        "denied scope should attempt HTTP capability before policy rejection"
    );

    let allowed_results = capability_result_text(&allowed);
    assert!(
        allowed_results.contains("accepted") && !allowed_results.contains("denied"),
        "allowed scope should persist mocked HTTP response only: {allowed_results}"
    );

    let denied_results = tool_result_text(&denied);
    assert!(
        denied_results.contains("capability failed with network"),
        "denied scope should surface sanitized network policy failure to the model: {denied_results}"
    );
    assert!(
        !denied_results.contains("accepted"),
        "denied scope must not inherit allowed scope HTTP response: {denied_results}"
    );
    assert!(
        denied.capability_results().is_empty(),
        "denied scope must not persist a successful HTTP capability result"
    );

    allowed.assert_model_exhausted();
    allowed.shutdown().await;
    denied.shutdown().await;
}

fn http_gateway(
    http: CapabilityId,
    call_id: &'static str,
    final_reply: &'static str,
) -> RebornTraceReplayModelGateway {
    RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                http,
                call_id,
                serde_json::json!({
                    "url": "https://api.example.test/v1/items",
                    "headers": {"x-request-id": call_id},
                    "timeout_ms": 2500,
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(final_reply),
            expected_tool_results: Vec::new(),
        },
    ])
}

fn tool_result_text(harness: &RebornBinaryE2EHarness) -> String {
    harness
        .model_requests()
        .iter()
        .flat_map(|request| request.messages.iter())
        .filter(|message| message.role == HostManagedModelMessageRole::ToolResult)
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn capability_result_text(harness: &RebornBinaryE2EHarness) -> String {
    harness
        .capability_results()
        .iter()
        .map(|result| result.output.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn allow_api_example_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
}

fn deny_all_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: Vec::new(),
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
}
