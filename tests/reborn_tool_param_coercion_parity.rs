#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
// Required by reborn_support::model_replay through crate::support::trace_llm.
mod support;

use ironclaw_host_api::{
    CapabilityId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
};
use ironclaw_host_runtime::{
    HTTP_CAPABILITY_ID, READ_FILE_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID,
};
use ironclaw_loop_support::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_turns::TurnStatus;
use reborn_support::{
    harness::RebornBinaryE2EHarness,
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

#[tokio::test]
async fn reborn_provider_tool_arguments_are_schema_coerced_before_http_dispatch() {
    let http = CapabilityId::new(HTTP_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                http.clone(),
                "call_http_with_stringified_params",
                serde_json::json!({
                    "url": "https://api.example.test/v1/coercion",
                    "method": "post",
                    "headers": "[{\"name\":\"x-coercion\",\"value\":\"ok\"}]",
                    "body": "{\"ok\":true}",
                    "timeout_ms": "2500",
                    "response_body_limit": "4096"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("coercion trace complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness =
        RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities_network_policy(
            "room-tool-param-coercion",
            model_gateway,
            http_network_policy(),
        )
        .await
        .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-tool-param-coercion",
            "exercise provider tool parameter coercion",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("coercion trace complete")
        .await
        .expect("final reply");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].capability_id, http);

    let http_requests = harness.runtime_http_requests();
    assert_eq!(http_requests.len(), 1);
    let request = &http_requests[0];
    assert_eq!(request.method, NetworkMethod::Post);
    assert_eq!(request.url.as_str(), "https://api.example.test/v1/coercion");
    assert_eq!(request.timeout_ms, Some(2500));
    assert_eq!(request.response_body_limit, Some(4096));
    assert!(
        request
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("x-coercion") && value == "ok"),
        "stringified headers should be coerced before HTTP dispatch: {:?}",
        &request.headers
    );
    assert!(
        request
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("content-type")
                && value == "application/json"),
        "JSON body coercion should trigger the default content-type header: {:?}",
        &request.headers
    );
    assert_eq!(request.body.as_slice(), br#"{"ok":true}"#);

    let results = harness.capability_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].capability_id, http);
    assert_eq!(results[0].output["status"], serde_json::json!(200));

    let model_requests = harness.model_requests();
    assert_eq!(model_requests.len(), 2);
    assert_eq!(tool_result_count(&model_requests[1]), 1);

    harness.assert_model_exhausted();
    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_provider_tool_scalar_arguments_are_schema_coerced_before_file_dispatch() {
    let write_file = CapabilityId::new(WRITE_FILE_CAPABILITY_ID).expect("valid capability id");
    let read_file = CapabilityId::new(READ_FILE_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                write_file.clone(),
                "call_write_file_for_scalar_coercion",
                serde_json::json!({
                    "path": "/workspace/coercion/lines.txt",
                    "content": "alpha\nbeta\ngamma\ndelta\n",
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                read_file.clone(),
                "call_read_file_with_stringified_range",
                serde_json::json!({
                    "path": "/workspace/coercion/lines.txt",
                    "offset": "2",
                    "limit": "2",
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("file coercion trace complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_file_capabilities(
        "room-file-tool-param-coercion",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-file-tool-param-coercion",
            "exercise scalar provider tool parameter coercion",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("file coercion trace complete")
        .await
        .expect("final reply");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0].capability_id, write_file);
    assert_eq!(invocations[1].capability_id, read_file);

    let results = harness.capability_results();
    assert_eq!(results.len(), 2);
    assert_eq!(results[1].capability_id, read_file);
    assert_eq!(results[1].output["lines_shown"], serde_json::json!(2));
    assert_eq!(results[1].output["total_lines"], serde_json::json!(4));
    let content = results[1].output["content"]
        .as_str()
        .expect("read_file content");
    assert!(content.contains("2│ beta"), "unexpected content: {content}");
    assert!(
        content.contains("3│ gamma"),
        "unexpected content: {content}"
    );
    assert!(
        !content.contains("1│ alpha"),
        "unexpected content: {content}"
    );
    assert!(
        !content.contains("4│ delta"),
        "unexpected content: {content}"
    );

    let model_requests = harness.model_requests();
    assert_eq!(model_requests.len(), 3);
    assert_eq!(tool_result_count(&model_requests[1]), 1);
    assert_eq!(tool_result_count(&model_requests[2]), 2);

    harness.assert_model_exhausted();
    harness.shutdown().await;
}

fn tool_result_count(request: &ironclaw_loop_support::HostManagedModelRequest) -> usize {
    request
        .messages
        .iter()
        .filter(|message| message.role == HostManagedModelMessageRole::ToolResult)
        .count()
}

fn http_network_policy() -> NetworkPolicy {
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
