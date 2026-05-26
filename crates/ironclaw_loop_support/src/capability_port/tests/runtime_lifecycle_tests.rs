use super::*;
use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use ironclaw_host_api::{
    ApprovalRequestId, CapabilityId, ExtensionId, ProcessId, ResourceEstimate, RuntimeKind,
};
use ironclaw_host_runtime::{
    CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, HostRuntime, HostRuntimeError,
    HostRuntimeHealth, HostRuntimeStatus, RuntimeApprovalGate, RuntimeAuthGate,
    RuntimeBlockedReason, RuntimeCapabilityFailure, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest, RuntimeCapabilityUnknown,
    RuntimeGateId, RuntimeProcessHandle, RuntimeResourceGate, RuntimeStatusRequest,
    VisibleCapabilitySurface,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityFailureKind, CapabilityOutcome,
    LoopCapabilityPort, LoopHostMilestoneSink,
};

#[tokio::test]
async fn runtime_capability_invocation_emits_dispatch_lifecycle_milestones() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let milestone_sink =
        Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
    let port = runtime_capability_port(
        &capability_id,
        &provider_id,
        Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id.clone(),
            provider_id.clone(),
        )])),
        Arc::new(RecordingResultWriter::default()),
        milestone_sink.clone(),
        "thread-runtime-capability-milestones",
    )
    .await;

    let outcome = invoke_visible_runtime_capability(&port)
        .await
        .expect("capability invocation succeeds");

    assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
    let milestones = milestone_sink.milestones();
    assert!(matches!(
        &milestones[0].kind,
        ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityInvoked {
            activity_id: _,
            capability_id: actual
        } if actual == &capability_id
    ));
    assert!(matches!(
        &milestones[1].kind,
        ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityCompleted {
            activity_id: _,
            capability_id: actual,
            provider,
            runtime: RuntimeKind::FirstParty,
            output_bytes
        } if actual == &capability_id
            && provider == &provider_id
            && *output_bytes == RECORDING_OUTPUT_BYTES
    ));
}

#[tokio::test]
async fn runtime_capability_emits_completion_after_result_write_retry_succeeds() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let milestone_sink =
        Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
    let result_writer = Arc::new(FailOnceResultWriter::default());
    let port = runtime_capability_port(
        &capability_id,
        &provider_id,
        Arc::new(RecordingHostRuntime::new(vec![visible_capability(
            capability_id.clone(),
            provider_id.clone(),
        )])),
        result_writer.clone(),
        milestone_sink.clone(),
        "thread-runtime-capability-milestone-retry",
    )
    .await;
    let invocation = visible_runtime_invocation(&port).await;

    let first_error = port
        .invoke_capability(invocation.clone())
        .await
        .expect_err("first result write fails");
    assert_eq!(
        first_error.kind,
        AgentLoopHostErrorKind::TranscriptWriteFailed
    );
    assert_eq!(milestone_sink.milestones().len(), 1);

    let outcome = port
        .invoke_capability(invocation)
        .await
        .expect("cached runtime outcome writes on retry");
    assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
    assert_eq!(result_writer.attempts(), 2);
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 2);
    assert!(matches!(
        &milestones[1].kind,
        ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityCompleted {
            activity_id: _,
            capability_id: actual,
            provider,
            runtime: RuntimeKind::FirstParty,
            output_bytes
        } if actual == &capability_id
            && provider == &provider_id
            && *output_bytes == RECORDING_OUTPUT_BYTES
    ));
}

#[tokio::test]
async fn runtime_capability_terminal_milestone_failure_is_retryable_without_rewriting_result() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let runtime = Arc::new(RecordingHostRuntime::new(vec![visible_capability(
        capability_id.clone(),
        provider_id.clone(),
    )]));
    let result_writer = Arc::new(RecordingResultWriter::default());
    let milestone_sink = Arc::new(FailOnceTerminalMilestoneSink::default());
    let port = runtime_capability_port(
        &capability_id,
        &provider_id,
        runtime.clone(),
        result_writer.clone(),
        milestone_sink.clone(),
        "thread-runtime-capability-milestone-fail-retry",
    )
    .await;
    let invocation = visible_runtime_invocation(&port).await;

    let first_error = port
        .invoke_capability(invocation.clone())
        .await
        .expect_err("terminal milestone publish fails first");
    assert_eq!(first_error.kind, AgentLoopHostErrorKind::Unavailable);
    assert_eq!(runtime.take_requests().len(), 1);
    assert_eq!(result_writer.records().len(), 1);

    let outcome = port
        .invoke_capability(invocation)
        .await
        .expect("pending terminal milestone publishes on retry");

    assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
    assert_eq!(runtime.take_requests().len(), 1);
    assert_eq!(result_writer.records().len(), 1);
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 2);
    assert!(matches!(
        &milestones[1].kind,
        ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityCompleted {
            activity_id: _,
            capability_id: actual,
            provider,
            runtime: RuntimeKind::FirstParty,
            output_bytes
        } if actual == &capability_id
            && provider == &provider_id
            && *output_bytes == RECORDING_OUTPUT_BYTES
    ));
}

#[tokio::test]
async fn runtime_capability_failed_and_unknown_outcomes_emit_failure_milestones() {
    let cases = [
        (
            RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
                capability_id: CapabilityId::new("demo.echo").expect("valid capability id"),
                kind: RuntimeFailureKind::InvalidInput,
                message: Some("invalid input".to_string()),
            }),
            CapabilityFailureKind::InvalidInput,
        ),
        (
            RuntimeCapabilityOutcome::Unknown(RuntimeCapabilityUnknown {
                capability_id: CapabilityId::new("demo.echo").expect("valid capability id"),
                kind: "custom_failure".to_string(),
                message: Some("custom failure".to_string()),
            }),
            capability_failure_kind("custom_failure").expect("valid custom failure kind"),
        ),
    ];

    for (outcome, expected_kind) in cases {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let milestone_sink =
            Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
        let port = runtime_capability_port(
            &capability_id,
            &provider_id,
            Arc::new(QueuedHostRuntime::new(
                vec![visible_capability(
                    capability_id.clone(),
                    provider_id.clone(),
                )],
                vec![Ok(outcome)],
            )),
            Arc::new(RecordingResultWriter::default()),
            milestone_sink.clone(),
            "thread-runtime-capability-failure-milestone",
        )
        .await;

        let outcome = invoke_visible_runtime_capability(&port)
            .await
            .expect("runtime failure outcome maps to loop outcome");

        assert!(matches!(outcome, CapabilityOutcome::Failed(_)));
        let milestones = milestone_sink.milestones();
        assert_eq!(milestones.len(), 2);
        assert!(matches!(
            &milestones[1].kind,
            ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed {
                activity_id: _,
                capability_id: actual,
                provider: Some(provider),
                runtime: Some(RuntimeKind::FirstParty),
                reason_kind
            } if actual == &capability_id && provider == &provider_id && reason_kind == &expected_kind
        ));
    }
}

#[tokio::test]
async fn runtime_capability_mismatched_outcome_does_not_emit_terminal_milestone() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let other_capability_id = CapabilityId::new("demo.other").expect("valid capability id");
    let milestone_sink =
        Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
    let port = runtime_capability_port(
        &capability_id,
        &provider_id,
        Arc::new(QueuedHostRuntime::new(
            vec![visible_capability(
                capability_id.clone(),
                provider_id.clone(),
            )],
            vec![Ok(RuntimeCapabilityOutcome::Completed(Box::new(
                RuntimeCapabilityCompleted {
                    capability_id: other_capability_id,
                    output: serde_json::json!({"ok": true}),
                    usage: ResourceUsage::default(),
                },
            )))],
        )),
        Arc::new(RecordingResultWriter::default()),
        milestone_sink.clone(),
        "thread-runtime-capability-mismatched-outcome",
    )
    .await;

    let error = invoke_visible_runtime_capability(&port)
        .await
        .expect_err("mismatched runtime outcome is rejected");

    assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 1);
    assert!(matches!(
        &milestones[0].kind,
        ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityInvoked {
            activity_id: _,
            capability_id: actual
        } if actual == &capability_id
    ));
}

#[tokio::test]
async fn runtime_capability_suspension_outcomes_do_not_emit_terminal_lifecycle_milestones() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let cases = [
        RuntimeCapabilityOutcome::ApprovalRequired(RuntimeApprovalGate {
            approval_request_id: ApprovalRequestId::new(),
            capability_id: capability_id.clone(),
            reason: RuntimeBlockedReason::ApprovalRequired,
        }),
        RuntimeCapabilityOutcome::AuthRequired(RuntimeAuthGate {
            gate_id: RuntimeGateId::new(),
            capability_id: capability_id.clone(),
            reason: RuntimeBlockedReason::AuthRequired,
            required_secrets: Vec::new(),
        }),
        RuntimeCapabilityOutcome::ResourceBlocked(RuntimeResourceGate {
            gate_id: RuntimeGateId::new(),
            capability_id: capability_id.clone(),
            reason: RuntimeBlockedReason::ResourceLimit,
            estimate: ResourceEstimate::default(),
        }),
        RuntimeCapabilityOutcome::SpawnedProcess(RuntimeProcessHandle {
            process_id: ProcessId::new(),
            capability_id: capability_id.clone(),
        }),
    ];

    for outcome in cases {
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let milestone_sink =
            Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
        let port = runtime_capability_port(
            &capability_id,
            &provider_id,
            Arc::new(QueuedHostRuntime::new(
                vec![visible_capability(
                    capability_id.clone(),
                    provider_id.clone(),
                )],
                vec![Ok(outcome)],
            )),
            Arc::new(RecordingResultWriter::default()),
            milestone_sink.clone(),
            "thread-runtime-capability-suspension-milestone",
        )
        .await;

        invoke_visible_runtime_capability(&port)
            .await
            .expect("suspension outcome maps to loop outcome");

        let milestones = milestone_sink.milestones();
        assert_eq!(milestones.len(), 1);
        assert!(matches!(
            &milestones[0].kind,
            ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityInvoked {
                activity_id: _,
                capability_id: actual
            } if actual == &capability_id
        ));
    }
}

#[tokio::test]
async fn runtime_capability_unknown_outcome_with_invalid_kind_does_not_emit_failure_milestone() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let milestone_sink =
        Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
    let port = runtime_capability_port(
        &capability_id,
        &provider_id,
        Arc::new(QueuedHostRuntime::new(
            vec![visible_capability(
                capability_id.clone(),
                provider_id.clone(),
            )],
            vec![Ok(RuntimeCapabilityOutcome::Unknown(
                RuntimeCapabilityUnknown {
                    capability_id: capability_id.clone(),
                    kind: "invalid kind with spaces".to_string(),
                    message: Some("bad kind".to_string()),
                },
            ))],
        )),
        Arc::new(RecordingResultWriter::default()),
        milestone_sink.clone(),
        "thread-runtime-capability-invalid-unknown-kind",
    )
    .await;

    let error = invoke_visible_runtime_capability(&port)
        .await
        .expect_err("invalid unknown kind is rejected");

    assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 1);
    assert!(matches!(
        &milestones[0].kind,
        ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityInvoked {
            activity_id: _,
            capability_id: actual
        } if actual == &capability_id
    ));
}

#[tokio::test]
async fn runtime_capability_host_error_emits_failure_milestone() {
    let cases = [
        (
            HostRuntimeError::invalid_request("bad request"),
            AgentLoopHostErrorKind::InvalidInvocation,
        ),
        (
            HostRuntimeError::unavailable("runtime unavailable"),
            AgentLoopHostErrorKind::Unavailable,
        ),
    ];

    for (runtime_error, expected_error_kind) in cases {
        let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
        let provider_id = ExtensionId::new("demo").expect("valid provider id");
        let milestone_sink =
            Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
        let port = runtime_capability_port(
            &capability_id,
            &provider_id,
            Arc::new(QueuedHostRuntime::new(
                vec![visible_capability(
                    capability_id.clone(),
                    provider_id.clone(),
                )],
                vec![Err(runtime_error)],
            )),
            Arc::new(RecordingResultWriter::default()),
            milestone_sink.clone(),
            "thread-runtime-capability-host-error-milestone",
        )
        .await;

        let error = invoke_visible_runtime_capability(&port)
            .await
            .expect_err("host runtime error propagates");

        assert_eq!(error.kind, expected_error_kind);
        let milestones = milestone_sink.milestones();
        assert_eq!(milestones.len(), 2);
        assert!(matches!(
            &milestones[1].kind,
            ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed {
                activity_id: _,
                capability_id: actual,
                provider: Some(provider),
                runtime: Some(RuntimeKind::FirstParty),
                reason_kind
            } if actual == &capability_id
                && provider == &provider_id
                && reason_kind.as_str() == expected_error_kind.as_str()
        ));
    }
}

async fn runtime_capability_port(
    capability_id: &CapabilityId,
    provider_id: &ExtensionId,
    runtime: Arc<dyn HostRuntime>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    thread_id: &str,
) -> HostRuntimeLoopCapabilityPort {
    let mut context = execution_context(thread_id);
    let run_context = loop_run_context(&context).await;
    let loop_driver_extension =
        loop_driver_execution_extension_id(&run_context).expect("valid extension id");
    context.grants.grants.push(dispatch_capability_grant(
        capability_id,
        &loop_driver_extension,
    ));
    HostRuntimeLoopCapabilityPortFactory::new(
        runtime,
        visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
            provider_id.clone(),
            dispatch_trust_decision(),
        )])),
        dummy_input_resolver(),
        result_writer,
        milestone_sink,
    )
    .port_for_run_context(run_context)
}

async fn visible_runtime_invocation(port: &HostRuntimeLoopCapabilityPort) -> CapabilityInvocation {
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let candidate = port
        .register_provider_tool_call(provider_tool_call())
        .await
        .expect("provider tool call registers");
    CapabilityInvocation {
        surface_version: surface.version,
        capability_id: candidate.capability_id,
        input_ref: candidate.input_ref,
    }
}

async fn invoke_visible_runtime_capability(
    port: &HostRuntimeLoopCapabilityPort,
) -> Result<CapabilityOutcome, AgentLoopHostError> {
    port.invoke_capability(visible_runtime_invocation(port).await)
        .await
}

struct QueuedHostRuntime {
    capabilities: Vec<VisibleCapability>,
    outcomes: Mutex<VecDeque<Result<RuntimeCapabilityOutcome, HostRuntimeError>>>,
}

impl QueuedHostRuntime {
    fn new(
        capabilities: Vec<VisibleCapability>,
        outcomes: Vec<Result<RuntimeCapabilityOutcome, HostRuntimeError>>,
    ) -> Self {
        Self {
            capabilities,
            outcomes: Mutex::new(VecDeque::from(outcomes)),
        }
    }
}

#[async_trait]
impl HostRuntime for QueuedHostRuntime {
    async fn invoke_capability(
        &self,
        _request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.outcomes
            .lock()
            .expect("outcomes lock")
            .pop_front()
            .expect("queued host runtime outcome")
    }

    async fn resume_capability(
        &self,
        _request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        unreachable!("queued host runtime should not resume")
    }

    async fn visible_capabilities(
        &self,
        _request: ironclaw_host_runtime::VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
        Ok(VisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new("surface-v1").expect("valid version"),
            capabilities: self.capabilities.clone(),
        })
    }

    async fn cancel_work(
        &self,
        _request: CancelRuntimeWorkRequest,
    ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
        unreachable!("queued host runtime should not cancel work")
    }

    async fn runtime_status(
        &self,
        _request: RuntimeStatusRequest,
    ) -> Result<HostRuntimeStatus, HostRuntimeError> {
        unreachable!("queued host runtime should not report status")
    }

    async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
        unreachable!("queued host runtime should not report health")
    }
}

#[derive(Default)]
struct FailOnceTerminalMilestoneSink {
    failures: AtomicUsize,
    milestones: Mutex<Vec<ironclaw_turns::run_profile::LoopHostMilestone>>,
}

impl FailOnceTerminalMilestoneSink {
    fn milestones(&self) -> Vec<ironclaw_turns::run_profile::LoopHostMilestone> {
        self.milestones.lock().expect("milestones lock").clone()
    }
}

#[async_trait]
impl LoopHostMilestoneSink for FailOnceTerminalMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: ironclaw_turns::run_profile::LoopHostMilestone,
    ) -> Result<(), AgentLoopHostError> {
        let is_terminal = matches!(
            &milestone.kind,
            ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityCompleted { .. }
                | ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed { .. }
        );
        if is_terminal && self.failures.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "terminal milestone sink unavailable",
            ));
        }
        self.milestones
            .lock()
            .expect("milestones lock")
            .push(milestone);
        Ok(())
    }
}
