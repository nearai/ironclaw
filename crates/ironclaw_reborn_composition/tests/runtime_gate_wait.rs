//! RED regression test for the `wait_for_terminal` gate-park bug.
//!
//! `send_user_message` internally calls `wait_for_terminal`, which polls
//! `TurnRunState::status.is_terminal()` in a loop until `poll_settings.max_total`.
//! A run that parks on a capability approval gate transitions to
//! `TurnStatus::BlockedApproval` — a non-terminal, non-running state.
//! `is_terminal()` returns `false` for it, so the old `wait_for_terminal`
//! keeps polling until the full `max_total` budget (60 s here) elapses,
//! then cancels the run and returns `Err(RebornRuntimeError::RunTimeout)`.
//!
//! The fix makes `send_user_message` (or `wait_for_terminal`) return
//! promptly with `Ok(AssistantReply { status: BlockedApproval, .. })` the
//! first time it sees the run is parked on a user-resolvable gate.
//!
//! This test is intentionally RED on the unpatched runtime:
//! - On OLD code: `tokio::time::timeout(5s, send_user_message(...))` fires
//!   because `wait_for_terminal` would sit for 60 s before cancelling.
//! - On FIXED code: `send_user_message` returns within a few hundred ms
//!   with `Ok(reply)` where `reply.status == BlockedApproval`.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_runtime::SHELL_CAPABILITY_ID;
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_reborn_composition::{
    PollSettings, RebornBuildInput, RebornRuntimeError, RebornRuntimeIdentity, RebornRuntimeInput,
    build_reborn_runtime,
};
use ironclaw_turns::{
    TurnStatus,
    run_profile::{LoopCapabilityPort, ProviderToolCall, RegisterProviderToolCallRequest},
};

/// A model gateway that, on its first call, registers a `builtin.shell`
/// capability tool call and returns it to the loop — triggering the
/// `AskDestructive` approval check which parks the run as `BlockedApproval`.
///
/// The gateway never reaches a second call because the run parks on the gate
/// before returning to the model.
#[derive(Debug, Default)]
struct ShellApprovalGateway {
    calls: StdMutex<usize>,
}

fn model_capability_error(
    e: ironclaw_turns::run_profile::AgentLoopHostError,
) -> HostManagedModelError {
    HostManagedModelError::safe(HostManagedModelErrorKind::Unavailable, e.safe_summary)
}

#[async_trait]
impl HostManagedModelGateway for ShellApprovalGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        // This runtime uses the capability-aware path; the plain stream
        // path should never be called. Fail loudly if it is.
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "expected capability-aware model path",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        _request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let call_index = {
            let mut calls = self.calls.lock().expect("shell gateway lock poisoned");
            let idx = *calls;
            *calls += 1;
            idx
        };

        if call_index > 0 {
            // The run should have parked on the approval gate before a
            // second model call; reaching here means the gate wasn't
            // triggered correctly. Return a plain reply so the run
            // completes rather than looping forever.
            return Ok(HostManagedModelResponse::assistant_reply(
                "unexpected second model call — approval gate not triggered",
            ));
        }

        // Find `builtin.shell` in the visible tool surface.
        let shell_id = ironclaw_host_api::CapabilityId::new(SHELL_CAPABILITY_ID)
            .expect("valid shell capability id");
        let shell_tool = capabilities
            .tool_definitions()
            .map_err(model_capability_error)?
            .into_iter()
            .find(|def| def.capability_id == shell_id)
            .expect("builtin.shell must be visible with LocalHost process backend");

        // Register a provider tool call for `builtin.shell`. With
        // `ApprovalPolicy::AskDestructive` the capability host returns
        // `ApprovalRequired`, which the agent loop converts to a
        // `BlockedApproval` run state.
        let candidate = capabilities
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                turn_id: Some("provider-turn-shell-gate".to_string()),
                id: "call-shell-gate".to_string(),
                name: shell_tool.name,
                arguments: serde_json::json!({ "command": "echo gate-test" }),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }))
            .await
            .map_err(model_capability_error)?;

        Ok(HostManagedModelResponse::capability_calls(
            vec![candidate],
            "",
        ))
    }
}

/// The effective runtime policy for a local-dev single-user session with
/// `AskDestructive` approval — identical to the shape used in `runtime.rs`
/// internal tests, reproduced here to keep this external test self-contained.
fn local_dev_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

/// Regression for the `wait_for_terminal` gate-park kill bug.
///
/// A run that parks on a capability approval gate reaches
/// `TurnStatus::BlockedApproval` — non-terminal. The old `wait_for_terminal`
/// only checks `is_terminal()`, so it keeps polling for the full
/// `poll_settings.max_total` (60 s here) before cancelling the run and
/// returning `Err(RebornRuntimeError::RunTimeout)`.
///
/// On OLD code this test TIMES OUT: the 5-second outer `tokio::time::timeout`
/// fires because `wait_for_terminal` blocks for the full 60-second budget.
///
/// On FIXED code `send_user_message` returns promptly (within a few hundred
/// ms) with `Ok(AssistantReply { status: BlockedApproval, .. })`.
#[tokio::test]
async fn gate_parked_run_is_surfaced_not_killed_on_send() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(ShellApprovalGateway::default());
    let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();

    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev("gate-wait-owner", root.path().to_path_buf())
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "gate-wait-tenant".to_string(),
        agent_id: "gate-wait-agent".to_string(),
        source_binding_id: "gate-wait-source".to_string(),
        reply_target_binding_id: "gate-wait-reply".to_string(),
    })
    // Large max_total: on the OLD wait_for_terminal, the send would block
    // for the full 60 s window (polling BlockedApproval) before cancelling.
    // The 5-second outer guard below makes this unmistakably RED.
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(60),
    })
    .with_model_gateway_override(gateway_for_runtime);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");

    // The send must return PROMPTLY — far shorter than the 60 s max_total.
    // On OLD code the 5-second timeout fires here (RED).
    // On FIXED code send_user_message sees BlockedApproval and returns immediately (GREEN).
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        runtime.send_user_message(&conversation, "run a shell command"),
    )
    .await
    .expect("send_user_message must return promptly, not block until max_total then cancel");

    let reply = result
        .expect("a run parked on an approval gate must surface as Ok(reply), not Err(RunTimeout)");

    assert_eq!(
        reply.status,
        TurnStatus::BlockedApproval,
        "a run parked on a capability approval gate must surface as BlockedApproval; \
         got {:?} — if this is RunTimeout the old wait_for_terminal bug is active",
        reply.status,
    );

    runtime.shutdown().await.expect("shutdown");
}

/// A model gateway that never returns — it awaits a oneshot receiver that is
/// never signaled, so the run stays `Running` indefinitely.
#[derive(Debug)]
struct HangingModelGateway {
    rx: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl HangingModelGateway {
    fn new() -> Self {
        let (_tx, rx) = tokio::sync::oneshot::channel();
        // Drop the sender immediately is NOT what we want (that resolves the
        // receiver with an error). Leak the sender so the receiver never
        // resolves, keeping the model call pending for the test's lifetime.
        std::mem::forget(_tx);
        Self {
            rx: tokio::sync::Mutex::new(Some(rx)),
        }
    }
}

#[async_trait]
impl HostManagedModelGateway for HangingModelGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        if let Some(rx) = self.rx.lock().await.take() {
            let _ = rx.await;
        }
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            "hanging gateway never resolves",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        _request: HostManagedModelRequest,
        _capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        if let Some(rx) = self.rx.lock().await.take() {
            let _ = rx.await;
        }
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            "hanging gateway never resolves",
        ))
    }
}

/// Guards the safety timeout: a genuinely `Running` run that never reaches a
/// terminal or parked state MUST still be cancelled at `poll_settings.max_total`
/// and surface `Err(RebornRuntimeError::RunTimeout)`. This pins that the
/// `wait_class()` fix did NOT disable the timeout for non-parked runs — a
/// regression where `Running` was misclassified as parked would make this hang.
#[tokio::test]
async fn genuinely_running_run_still_times_out_and_cancels() {
    let root = tempfile::tempdir().unwrap();
    let gateway: Arc<dyn HostManagedModelGateway> = Arc::new(HangingModelGateway::new());

    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev("timeout-owner", root.path().to_path_buf())
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "timeout-tenant".to_string(),
        agent_id: "timeout-agent".to_string(),
        source_binding_id: "timeout-source".to_string(),
        reply_target_binding_id: "timeout-reply".to_string(),
    })
    // Short max_total so the timeout fires quickly; the run never advances.
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_millis(300),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        runtime.send_user_message(&conversation, "hang forever"),
    )
    .await
    .expect("send must return after the poll budget, not hang past the outer guard");

    assert!(
        matches!(result, Err(RebornRuntimeError::RunTimeout { .. })),
        "a genuinely running run must still time out and cancel; got {result:?}"
    );

    runtime.shutdown().await.expect("shutdown");
}
