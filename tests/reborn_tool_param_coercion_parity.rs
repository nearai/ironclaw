#[allow(dead_code)]
#[path = "support/reborn_parity_qa/mod.rs"]
mod parity_qa_support;
#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod reborn_support;
// Required by parity_qa_support::model_replay through crate::support::trace_llm.
mod support;

use ironclaw_host_api::{
    action::{NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern},
    ids::CapabilityId,
};
use ironclaw_host_runtime::{GLOB_CAPABILITY_ID, HTTP_CAPABILITY_ID};
use ironclaw_loop_host::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_turns::TurnStatus;
use parity_qa_support::{
    binary_e2e::RebornBinaryE2EHarness,
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
    // The requested 4 KiB remains the model-visible shaping budget. The
    // transport receives the 10 MiB artifact-capture ceiling so a larger body
    // can spill durably instead of being destroyed at the egress boundary.
    assert_eq!(request.response_body_limit, Some(10 * 1024 * 1024));
    assert!(
        request
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("x-coercion") && value == "ok"),
        "stringified headers should be coerced before HTTP dispatch: {:?}",
        request.headers
    );
    assert!(
        request
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("content-type")
                && value == "application/json"),
        "JSON body coercion should trigger the default content-type header: {:?}",
        request.headers
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

#[test]
fn reborn_provider_tool_scalar_arguments_are_schema_coerced_before_file_dispatch() {
    run_async_test_with_stack(
        "reborn_provider_tool_scalar_arguments_are_schema_coerced_before_file_dispatch",
        reborn_provider_tool_scalar_arguments_are_schema_coerced_before_file_dispatch_impl,
    );
}

async fn reborn_provider_tool_scalar_arguments_are_schema_coerced_before_file_dispatch_impl() {
    let glob = CapabilityId::new(GLOB_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![
                // `hidden` is a boolean schema field: the stringified "true"
                // must be coerced before the pinned coding glob engine reads it,
                // otherwise the dotfile is filtered out (as_bool("true") is
                // None -> default false) and the observation below fails.
                RebornScriptedProviderToolCall::new(
                    glob.clone(),
                    "call_glob_with_stringified_hidden",
                    serde_json::json!({
                        "path": "/workspace/coercion/*",
                        "hidden": "true",
                    }),
                ),
                // `limit` is a number schema field: the stringified "1" must
                // be coerced to 1, otherwise the engine's as_u64 returns None
                // and the DEFAULT_LIMIT (200) shows all three entries.
                RebornScriptedProviderToolCall::new(
                    glob.clone(),
                    "call_glob_with_stringified_limit",
                    serde_json::json!({
                        "path": "/workspace/coercion/*.txt",
                        "limit": "1",
                    }),
                ),
            ],
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
    // Seed the workspace fixtures directly (matching the trace-suite fixture
    // pattern) so the run scripts only the two glob calls whose scalar
    // coercion is under test — no parallel write legs.
    seed_coercion_workspace(&harness);
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
    assert_eq!(invocations[0].capability_id, glob);
    assert_eq!(invocations[1].capability_id, glob);

    let results = harness.capability_results();
    assert_eq!(results.len(), 2);
    let hidden_output = results[0].output["output"].as_str().expect("glob output");
    assert!(
        hidden_output.contains(".secret.txt"),
        "stringified boolean `hidden` must be coerced to true so the dotfile \
         is included, got: {hidden_output}"
    );
    assert!(
        hidden_output.contains("lines.txt") && hidden_output.contains("notes.txt"),
        "the glob must still list the visible entries, got: {hidden_output}"
    );
    let limited_output = results[1].output["output"].as_str().expect("glob output");
    let txt_rows = limited_output
        .lines()
        .filter(|line| line.contains(".txt"))
        .count();
    assert_eq!(
        txt_rows, 1,
        "stringified number `limit` must be coerced to 1 so only one entry \
         is shown, got: {limited_output}"
    );

    let model_requests = harness.model_requests();
    assert_eq!(model_requests.len(), 2);
    assert_eq!(tool_result_count(&model_requests[1]), 2);

    harness.assert_model_exhausted();
    harness.shutdown().await;
}

fn seed_coercion_workspace(harness: &RebornBinaryE2EHarness) {
    let coercion_dir = harness
        .host_workspace_file_path("coercion")
        .expect("coercion directory path");
    std::fs::create_dir_all(&coercion_dir).expect("create coercion directory");
    std::fs::write(
        coercion_dir.join("lines.txt"),
        "alpha\nbeta\ngamma\ndelta\n",
    )
    .expect("write lines fixture");
    std::fs::write(coercion_dir.join("notes.txt"), "note\n").expect("write notes fixture");
    std::fs::write(coercion_dir.join(".secret.txt"), "hidden\n").expect("write dotfile fixture");
}

fn tool_result_count(request: &ironclaw_loop_host::HostManagedModelRequest) -> usize {
    request
        .messages
        .iter()
        .filter(|message| message.role == HostManagedModelMessageRole::ToolResult)
        .count()
}

/// Runs the async test body on a dedicated 16 MiB-stack thread, mirroring
/// `tests/integration/reborn_integration_coding_registration.rs`'s
/// `run_async_test_with_stack`
/// (and the QA smoke suite): the file-profile harness build's async
/// state-machine chain exceeds the default 2 MiB libtest thread stack.
fn run_async_test_with_stack<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio test runtime")
                .block_on(test());
        })
        .expect("spawn stack-sized test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
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
