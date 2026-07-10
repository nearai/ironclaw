//! Tool-calling turn: proves the §3.7 two-tier egress design end-to-end —
//! scripted `builtin.http` call → real `RuntimeHttpEgress` → recording egress
//! (Tier-2) → finalized reply. The existing cases use the scripted `TraceLlm`
//! seam; the LocalDev result-reference regression below uses the public runtime
//! builder with a model-boundary gateway so the real capability wiring runs.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
    HostManagedToolResultContent,
};
use ironclaw_reborn_composition::{
    RebornCompositionProfile, RebornRuntimeIdentity, RebornRuntimeInput, build_reborn_runtime,
    local_runtime_build_input,
};
use ironclaw_turns::run_profile::{
    LoopCapabilityPort, ProviderToolCall, RegisterProviderToolCallRequest,
};
use serde_json::json;

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;

const LARGE_ECHO_MESSAGE: &str = "PAYLOAD0123456789ABCDEF_";
const LARGE_ECHO_TAIL: &str = "UNREPLAYED_RAW_TOOL_RESULT_TAIL";

fn large_echo_message() -> String {
    format!("{}{}", LARGE_ECHO_MESSAGE.repeat(100), LARGE_ECHO_TAIL)
}

#[derive(Debug, Default)]
struct LargeEchoResultReadGateway {
    requests: Mutex<Vec<HostManagedModelRequest>>,
    calls: Mutex<usize>,
}

impl LargeEchoResultReadGateway {
    fn captured_requests(&self) -> Vec<HostManagedModelRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

fn model_gateway_error(error: impl std::fmt::Display) -> HostManagedModelError {
    HostManagedModelError::safe(
        HostManagedModelErrorKind::InvalidRequest,
        format!("capability interaction failed: {error}"),
    )
}

#[async_trait]
impl HostManagedModelGateway for LargeEchoResultReadGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests.lock().expect("request lock").push(request);
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "expected capability-aware model path",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let call_index = {
            let mut calls = self.calls.lock().expect("call lock");
            let index = *calls;
            *calls += 1;
            index
        };
        self.requests
            .lock()
            .expect("request lock")
            .push(request.clone());
        if call_index == 1 {
            let tool_result = request
                .messages
                .iter()
                .find(|message| message.role == HostManagedModelMessageRole::ToolResult)
                .expect("second model call should include the echo result");
            let result_ref = match tool_result.tool_result_content.as_ref() {
                Some(HostManagedToolResultContent::Reference { envelope }) => {
                    envelope.result_ref.clone()
                }
                other => panic!("expected an echo result reference, got {other:?}"),
            };
            let result_read_tool = capabilities
                .tool_definitions()
                .map_err(model_gateway_error)?
                .into_iter()
                .find(|definition| definition.capability_id.as_str() == "builtin.result_read")
                .expect("builtin.result_read must be visible through LocalDev wiring");
            let candidate = capabilities.register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
                provider_id: "test-provider".to_string(), provider_model_id: "test-model".to_string(),
                turn_id: Some("provider-turn-2".to_string()), id: "call-2".to_string(), name: result_read_tool.name,
                arguments: json!({"result_ref": result_ref, "offset": 0, "max_bytes": 2048}),
                response_reasoning: None, reasoning: None, signature: None,
            })).await.map_err(model_gateway_error)?;
            return Ok(HostManagedModelResponse::capability_calls(
                vec![candidate],
                "",
            ));
        }
        if call_index == 2 {
            return Ok(HostManagedModelResponse::assistant_reply(
                "bounded result read",
            ));
        }
        let echo_tool = capabilities
            .tool_definitions()
            .map_err(model_gateway_error)?
            .into_iter()
            .find(|definition| {
                definition.capability_id == CapabilityId::new("builtin.echo").expect("echo id")
            })
            .expect("builtin.echo must be visible through LocalDev wiring");
        let candidate = capabilities
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                turn_id: Some("provider-turn-1".to_string()),
                id: "call-1".to_string(),
                name: echo_tool.name,
                arguments: json!({"message": large_echo_message()}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }))
            .await
            .map_err(model_gateway_error)?;
        Ok(HostManagedModelResponse::capability_calls(
            vec![candidate],
            "",
        ))
    }
}

#[tokio::test]
async fn runs_http_tool_call_through_recorded_egress() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("fetched"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch items").await.expect("turn completes");
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("Tier-2 egress captured");
    h.assert_reply_contains("fetched")
        .await
        .expect("final reply finalized");
}

const HTTP_TOOL_URL: &str = "https://api.example.test/v1/items";

#[tokio::test]
async fn local_dev_large_echo_uses_bounded_result_reference_and_result_read() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(LargeEchoResultReadGateway::default());
    let input = local_runtime_build_input(
        RebornCompositionProfile::LocalDev,
        "large-echo-result-owner",
        root.path().join("local-dev"),
    )
    .expect("local-dev input builds");
    let runtime = build_reborn_runtime(
        RebornRuntimeInput::from_services(input)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: "large-echo-result-tenant".to_string(),
                agent_id: "large-echo-result-agent".to_string(),
                source_binding_id: "large-echo-result-source".to_string(),
                reply_target_binding_id: "large-echo-result-reply".to_string(),
            })
            .with_poll_settings(ironclaw_reborn_composition::PollSettings {
                interval: Duration::from_millis(10),
                max_total: Duration::from_secs(10),
            })
            .with_model_gateway_override(gateway.clone()),
    )
    .await
    .expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    runtime
        .enable_global_auto_approve_for_test(&conversation)
        .await;
    let reply = tokio::time::timeout(
        Duration::from_secs(15),
        runtime.send_user_message(&conversation, "echo a large result"),
    )
    .await
    .expect("runtime send finishes")
    .expect("runtime send succeeds");
    assert!(reply.is_successful_final_reply(), "reply: {reply:?}");
    runtime.shutdown().await.expect("runtime shutdown");

    let requests = gateway.captured_requests();
    assert_eq!(
        requests.len(),
        3,
        "echo, result_read, and final model calls"
    );
    let echo_result = requests[1]
        .messages
        .iter()
        .find(|message| {
            message.role == HostManagedModelMessageRole::ToolResult
                && message
                    .tool_result_provider_call
                    .as_ref()
                    .is_some_and(|call| call.capability_id.as_str() == "builtin.echo")
        })
        .expect("second model request includes the echo result");
    let echo_envelope = match echo_result.tool_result_content.as_ref() {
        Some(HostManagedToolResultContent::Reference { envelope }) => envelope,
        other => panic!("expected bounded echo result reference, got {other:?}"),
    };
    assert!(echo_result.content.contains("result_reference"));
    assert!(echo_result.content.contains(&echo_envelope.result_ref));
    assert!(!echo_result.content.contains(LARGE_ECHO_TAIL));
    assert!(echo_result.content.len() <= 4096);
    let echo_result_ref = echo_envelope.result_ref.clone();

    let result_read_result = requests[2]
        .messages
        .iter()
        .rev()
        .find(|message| {
            message.role == HostManagedModelMessageRole::ToolResult
                && message
                    .tool_result_provider_call
                    .as_ref()
                    .is_some_and(|call| call.capability_id.as_str() == "builtin.result_read")
        })
        .expect("third model request includes result_read output");
    let result_read_call = result_read_result
        .tool_result_provider_call
        .as_ref()
        .expect("result_read provider call metadata");
    assert_eq!(result_read_call.arguments["result_ref"], echo_result_ref);
    assert!(result_read_result.content.contains(LARGE_ECHO_MESSAGE));
    assert!(!result_read_result.content.contains(LARGE_ECHO_TAIL));
    let observation: serde_json::Value =
        serde_json::from_str(&result_read_result.content).expect("result_read envelope");
    assert_eq!(
        observation["model_observation"]["detail"]["next_offset"],
        2048
    );
}

/// Guards against vacuous pass: with no scripted tool call, both
/// `assert_tool_invoked` and `assert_egress_request_matching` must return `Err`.
#[tokio::test]
async fn assertions_fail_when_tool_did_not_run() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("no tool")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("just talk").await.expect("turn completes");
    assert!(h.assert_tool_invoked("builtin.http").await.is_err());
    assert!(
        h.assert_egress_request_matching("api.example.test")
            .await
            .is_err()
    );
}

/// Proves the assertions discriminate when the invocation + egress lists are
/// NON-empty: a real `builtin.http` call runs, but assertions for a *different*
/// capability/host must still return `Err` (the "present but no match" branch).
#[tokio::test]
async fn assertions_fail_when_tool_present_but_requested_tool_or_url_does_not_match() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch items").await.expect("turn completes");
    // Prove capture lists are NON-empty first, so the checks below exercise the
    // mismatch branch, not the empty-list branch.
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran before mismatch assertions");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("http egress captured before mismatch assertions");
    // Non-empty invocation list — wrong capability id must fail.
    assert!(
        h.assert_tool_invoked("some.other.capability")
            .await
            .is_err()
    );
    // Non-empty egress list — non-matching host substring must fail.
    assert!(
        h.assert_egress_request_matching("nonmatching.host.test")
            .await
            .is_err()
    );
}

/// Proves the multi-segment `builtin.http.save` capability id (`.`→`__`
/// encoding to `builtin__http__save` at the provider seam) resolves end-to-end,
/// writing to the `/workspace` mount `core_builtin_tools` provides read-write.
#[tokio::test]
async fn runs_http_save_tool_call_through_recorded_egress() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.http.save",
                json!({"url": HTTP_TOOL_URL, "save_to": "/workspace/response.json"}),
            ),
            RebornScriptedReply::text("saved"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch and save")
        .await
        .expect("turn completes");
    h.assert_tool_invoked("builtin.http.save")
        .await
        .expect("http.save tool ran");
    // The save path must reach the real `RuntimeHttpEgress`.
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("http.save egress captured");
    h.assert_reply_contains("saved")
        .await
        .expect("final reply finalized");
}

/// The globally-disabled `builtin.spawn_subagent` capability (configured
/// through `DefaultPlannedRuntimeConfig::disabled_capability_ids`, applied as
/// the OUTERMOST `PerSurfaceCapabilityDenyDecorator` in
/// `build_default_planned_runtime_inner` — see that function's doc comments)
/// must never reach the model-facing tool list, whichever port would
/// otherwise have surfaced it: the flavor-aware `SubagentSpawnCapabilityDecorator`
/// (always wired, independent of any harness extension registry) or the
/// host-runtime first-party manifest stub (`builtin_first_party_package()` in
/// `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs`, included in
/// `core_builtin_tools()`'s registry unconditionally).
///
/// Non-vacuity: confirmed by direct inspection that `core_builtin_tools()`'s
/// capability port surfaces `builtin__spawn_subagent` when the deny decorator
/// is bypassed (i.e. `spawn_decorator` runs before the outermost deny filter
/// in composition order) — so this assertion is pinning a real strip, not
/// asserting absence from an already-empty surface. `builtin__http` is
/// asserted present as the non-vacuity control for THIS test's own capture.
#[tokio::test]
async fn disabled_spawn_subagent_capability_is_stripped_from_model_surface() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("hello").await.expect("turn completes");

    let captured = h.scripted_llm.captured_tool_definitions();
    let names: Vec<&str> = captured
        .iter()
        .flatten()
        .map(|def| def.name.as_str())
        .collect();

    // Neither encoding of the disabled capability id may appear in what the
    // model was shown (provider-seam `__` encoding, or the raw dotted id).
    assert!(
        !names.contains(&"builtin__spawn_subagent"),
        "disabled capability's provider seam name must not be advertised: {names:?}"
    );
    assert!(
        !names.contains(&"builtin.spawn_subagent"),
        "disabled capability's raw dotted id must not be advertised: {names:?}"
    );
    // Control: a real capability IS present, so the absence asserts above are
    // not vacuously true against an empty surface.
    assert!(
        names.contains(&"builtin__http"),
        "control tool builtin__http must be present: {names:?}"
    );
}

/// A model that calls the disabled `builtin.spawn_subagent` anyway is rejected
/// at the gateway (`CapabilitySurfaceDenyFilter`, before
/// `register_provider_tool_call` ever stages an invocation) — the whole
/// provider response fails with `InvalidOutput` → `Unavailable`, reaching a
/// terminal `TurnStatus::Failed`/`"model_unavailable"` after exactly one
/// scripted turn. No `ToolResultReference` is persisted; `assert_tool_invoked`
/// returning `Err` proves the capability was never dispatched.
#[tokio::test]
async fn disabled_spawn_subagent_capability_call_anyway_fails_the_run() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([RebornScriptedReply::tool_call(
            "builtin.spawn_subagent",
            json!({"goal": "test"}),
        )])
        .build()
        .await
        .expect("harness builds");

    let run_id = h
        .submit_turn_async("spawn a subagent")
        .await
        .expect("turn submitted");
    let state = h
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Failed)
        .await
        .expect("run reaches Failed after the disabled capability is rejected at the gateway");
    let failure = state
        .failure
        .as_ref()
        .expect("a Failed run must carry a failure detail");
    assert_eq!(
        failure.category(),
        "model_unavailable",
        "expected the Unavailable fidelity category (InvalidOutput -> Unavailable), got {failure:?}"
    );

    // No side effect: the capability was rejected before dispatch, so it was
    // never invoked.
    assert!(
        h.assert_tool_invoked("builtin.spawn_subagent")
            .await
            .is_err(),
        "disabled capability must never be dispatched, even when the model calls it anyway"
    );
}
