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
    ApprovalRequestId, Blocked, CapabilityDisplayOutputPreview, CapabilityId, ExtensionId,
    FailureKind, ProcessId, Resolution, ResourceEstimate, RuntimeCredentialAuthRequirement,
    RuntimeKind, VendorId,
};
use ironclaw_host_runtime::{
    CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, HostRuntime, HostRuntimeError,
    HostRuntimeHealth, HostRuntimeStatus, RuntimeApprovalGate, RuntimeApprovalResume,
    RuntimeAuthGate, RuntimeAuthResume, RuntimeBlockedReason, RuntimeCapabilityCompleted,
    RuntimeCapabilityFailure, RuntimeCapabilityOutcome, RuntimeCapabilityUnknown, RuntimeGateId,
    RuntimeInvocation, RuntimeProcessHandle, RuntimeResourceGate, RuntimeStatusRequest,
    VisibleCapabilitySurface,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityApprovalResume, CapabilityAuthResume,
    CapabilityFailureKind, CapabilityInputRef, CapabilityResumeToken, LoopCapabilityPort,
    LoopHostMilestoneSink, LoopRequestBatch, LoopRunContext, RegisterProviderToolCallRequest,
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

    assert!(matches!(&outcome, Resolution::Done(o) if o.verdict.is_success()));
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
    assert!(matches!(&outcome, Resolution::Done(o) if o.verdict.is_success()));
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
async fn runtime_completed_display_preview_is_forwarded_to_result_writer() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let result_writer = Arc::new(RecordingResultWriter::default());
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
                    capability_id: capability_id.clone(),
                    output: serde_json::json!({"ok": true}),
                    display_preview: Some(CapabilityDisplayOutputPreview {
                        output_summary: Some("Edited 1 file: +1/-1".to_string()),
                        output_preview: "--- a/file\n+++ b/file\n-old\n+new\n".to_string(),
                        output_kind: "unified_diff".to_string(),
                        subtitle: Some("/workspace/file".to_string()),
                        truncated: false,
                    }),
                    usage: ResourceUsage::default(),
                },
            )))],
        )),
        result_writer.clone(),
        milestone_sink,
        "thread-runtime-capability-display-preview",
    )
    .await;

    let outcome = invoke_visible_runtime_capability(&port)
        .await
        .expect("runtime outcome maps to loop outcome");

    assert!(matches!(&outcome, Resolution::Done(o) if o.verdict.is_success()));
    let previews = result_writer.display_previews();
    let preview = previews
        .into_iter()
        .next()
        .flatten()
        .expect("display preview forwarded");
    assert_eq!(preview.output_kind, "unified_diff");
    assert!(preview.output_preview.contains("+new"));
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

    assert!(matches!(&outcome, Resolution::Done(o) if o.verdict.is_success()));
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
async fn runtime_capability_batch_returns_runtime_unavailable_as_failed_outcome() {
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
            vec![Err(HostRuntimeError::Unavailable {
                reason: "runtime unavailable".to_string(),
            })],
        )),
        Arc::new(RecordingResultWriter::default()),
        milestone_sink.clone(),
        "thread-runtime-capability-batch-runtime-unavailable",
    )
    .await;
    let invocation = visible_runtime_invocation(&port).await;

    let batch = port
        .invoke_capability_batch(LoopRequestBatch {
            invocations: vec![invocation],
            stop_on_first_suspension: false,
        })
        .await
        .expect("runtime unavailability should be returned as a capability failure");

    assert!(!batch.stopped_on_suspension);
    assert_eq!(batch.resolutions.len(), 1);
    assert!(matches!(
        &batch.resolutions[0],
        Resolution::Done(o)
            if o.verdict.error_kind() == Some(&FailureKind::Unavailable)
                && o.summary.as_str() == "runtime unavailable"
    ));
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 2);
    assert!(matches!(
        &milestones[1].kind,
        ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed {
            activity_id: _,
            capability_id: actual,
            provider: Some(provider),
            runtime: Some(RuntimeKind::FirstParty),
            reason_kind,
            ..
        } if actual == &capability_id
            && provider == &provider_id
            && reason_kind == &CapabilityFailureKind::Unavailable
    ));
}

#[tokio::test]
async fn runtime_capability_batch_continues_after_runtime_failure_outcome() {
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
            vec![
                Err(HostRuntimeError::Unavailable {
                    reason: "runtime unavailable".to_string(),
                }),
                Ok(RuntimeCapabilityOutcome::Completed(Box::new(
                    RuntimeCapabilityCompleted {
                        capability_id: capability_id.clone(),
                        output: serde_json::json!({"ok": true}),
                        display_preview: None,
                        usage: ResourceUsage::default(),
                    },
                ))),
            ],
        )),
        Arc::new(RecordingResultWriter::default()),
        milestone_sink.clone(),
        "thread-runtime-capability-batch-continues-after-runtime-failure",
    )
    .await;
    port.visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let mut second_call = provider_tool_call();
    second_call.id = "call_2".to_string();
    let first = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_tool_call()))
        .await
        .expect("first provider tool call registers");
    let second = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(second_call))
        .await
        .expect("second provider tool call registers");

    let batch = port
        .invoke_capability_batch(LoopRequestBatch {
            invocations: vec![
                LoopRequest {
                    activity_id: first.activity_id,
                    surface_version: first.surface_version,
                    capability_id: first.capability_id,
                    input_ref: first.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                },
                LoopRequest {
                    activity_id: second.activity_id,
                    surface_version: second.surface_version,
                    capability_id: second.capability_id,
                    input_ref: second.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                },
            ],
            stop_on_first_suspension: false,
        })
        .await
        .expect("runtime failure should not abort the remaining batch");

    assert!(!batch.stopped_on_suspension);
    assert_eq!(batch.resolutions.len(), 2);
    assert!(matches!(
        &batch.resolutions[0],
        Resolution::Done(o)
            if o.verdict.error_kind() == Some(&FailureKind::Unavailable)
                && o.summary.as_str() == "runtime unavailable"
    ));
    assert!(matches!(
        &batch.resolutions[1],
        Resolution::Done(o) if o.verdict.is_success()
    ));
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 4);
    assert!(matches!(
        &milestones[1].kind,
        ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed {
            capability_id: actual,
            provider: Some(provider),
            runtime: Some(RuntimeKind::FirstParty),
            reason_kind,
            ..
        } if actual == &capability_id
            && provider == &provider_id
            && reason_kind == &CapabilityFailureKind::Unavailable
    ));
    assert!(matches!(
        &milestones[3].kind,
        ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityCompleted {
            capability_id: actual,
            provider,
            runtime: RuntimeKind::FirstParty,
            ..
        } if actual == &capability_id && provider == &provider_id
    ));
}

#[tokio::test]
async fn runtime_capability_failed_and_unknown_outcomes_emit_failure_milestones() {
    let cases = [
        (
            RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure::new(
                CapabilityId::new("demo.echo").expect("valid capability id"),
                RuntimeFailureKind::InvalidInput,
                Some("invalid input".to_string()),
            )),
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

        assert!(matches!(
            &outcome,
            Resolution::Done(o) if o.verdict.error_kind().is_some()
        ));
        let milestones = milestone_sink.milestones();
        assert_eq!(milestones.len(), 2);
        assert!(matches!(
            &milestones[1].kind,
            ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed {
                activity_id: _,
                capability_id: actual,
                provider: Some(provider),
                runtime: Some(RuntimeKind::FirstParty),
                reason_kind,
                ..
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
                    display_preview: None,
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
            credential_requirements: Vec::new(),
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
async fn runtime_auth_gate_forwards_credential_requirements() {
    let capability_id = CapabilityId::new("demo.echo").expect("capability id");
    let provider_id = ExtensionId::new("demo").expect("provider id");
    let requirement = RuntimeCredentialAuthRequirement {
        provider: VendorId::new("github").unwrap(),
        setup: Default::default(),
        requester_extension: provider_id.clone(),
        provider_scopes: Vec::new(),
    };
    let port = runtime_capability_port(
        &capability_id,
        &provider_id,
        Arc::new(QueuedHostRuntime::new(
            vec![visible_capability(
                capability_id.clone(),
                provider_id.clone(),
            )],
            vec![Ok(RuntimeCapabilityOutcome::AuthRequired(
                RuntimeAuthGate {
                    gate_id: RuntimeGateId::new(),
                    capability_id: capability_id.clone(),
                    reason: RuntimeBlockedReason::AuthRequired,
                    required_secrets: Vec::new(),
                    credential_requirements: vec![requirement.clone()],
                },
            ))],
        )),
        Arc::new(RecordingResultWriter::default()),
        Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default()),
        "thread-runtime-auth-requirement",
    )
    .await;

    let outcome = invoke_visible_runtime_capability(&port)
        .await
        .expect("auth gate is a suspension outcome");

    // §5.3 Stage 2: the auth gate's `credential_requirements` moved off the
    // loop-facing channel to `GateRecord::Auth`; the `Resolution` now carries only
    // the auth gate identity. This port wires no readable gate-record store, so the
    // requirements are unrecoverable from the returned value here.
    // Flip consequence (§5.2.9, confirmed): auth `credential_requirements` no
    // longer ride the loop-facing channel — they persist on `GateRecord::Auth`,
    // and the runner re-reads them at the blocked exit. That end-to-end
    // persist→read→`TurnRunRecord.credential_requirements` path is covered by the
    // production-composition integration test
    // `reborn_integration_auth_gate::runtime_401_after_injection_populates_provider_credential_requirement`,
    // so this port-tier test asserts only that the outcome is an auth block.
    assert!(
        matches!(&outcome, Resolution::Blocked(Blocked::Auth(_))),
        "auth gate must surface as an auth block, got {outcome:?}"
    );
}

#[tokio::test]
async fn auth_resume_uses_replay_input_without_resolving_stale_input_ref() {
    let capability_id = CapabilityId::new("gmail.list_messages").expect("valid capability id");
    let provider_id = ExtensionId::new("gmail").expect("valid provider id");
    let runtime = Arc::new(
        QueuedHostRuntime::new(
            vec![visible_capability(
                capability_id.clone(),
                provider_id.clone(),
            )],
            vec![Ok(RuntimeCapabilityOutcome::AuthRequired(
                RuntimeAuthGate {
                    gate_id: RuntimeGateId::new(),
                    capability_id: capability_id.clone(),
                    reason: RuntimeBlockedReason::AuthRequired,
                    required_secrets: Vec::new(),
                    credential_requirements: Vec::new(),
                },
            ))],
        )
        .with_auth_resume_outcomes(vec![Ok(RuntimeCapabilityOutcome::Completed(
            Box::new(RuntimeCapabilityCompleted {
                capability_id: capability_id.clone(),
                output: serde_json::json!({"auth_resumed": true}),
                display_preview: None,
                usage: ResourceUsage::default(),
            }),
        ))]),
    );
    let resolver = Arc::new(OneShotInputResolver::new(serde_json::json!({
        "query": "is:unread",
        "max_results": 10
    })));
    let mut context = execution_context("thread-auth-resume-replay");
    let run_context = loop_run_context(&context).await;
    let input_ref = CapabilityInputRef::new(format!("input:{}:gmail-list", run_context.run_id))
        .expect("valid input ref");
    let loop_driver_extension =
        loop_driver_execution_extension_id(&run_context).expect("valid extension id");
    context.grants.grants.push(dispatch_capability_grant(
        &capability_id,
        &loop_driver_extension,
    ));
    // The fresh dispatch raises an auth gate and the host persists the replay
    // payload here; the resume reconstitutes {input, estimate} from it host-side
    // (§5.3 Stage 2a-i) rather than re-resolving the possibly-stale input ref.
    let replay_store = Arc::new(RecordingReplayPayloadStore::default());
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
            provider_id,
            dispatch_trust_decision(),
        )])),
        resolver.clone(),
        Arc::new(RecordingResultWriter::default()),
        Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default()),
    )
    .with_replay_payload_store(replay_store.clone())
    .port_for_run_context(run_context);
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");

    let activity_id = ironclaw_turns::CapabilityActivityId::new();
    let auth_blocked = port
        .invoke_capability(LoopRequest {
            activity_id,
            surface_version: surface.version.clone(),
            capability_id: capability_id.clone(),
            input_ref: input_ref.clone(),
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("first dispatch reaches auth gate");
    // §5.3 Stage 2: the auth gate surfaces as `Resolution::Blocked(Blocked::Auth)`;
    // the loop reconstructs `CapabilityAuthResume` from the surviving resume token on
    // the gate waypoint (mirrors `auth_resume_from_gate` in the executor).
    let Resolution::Blocked(blocked @ Blocked::Auth(_)) = &auth_blocked else {
        panic!("auth gate must carry replay metadata, got {auth_blocked:?}");
    };
    let auth_resume = CapabilityAuthResume {
        resume_token: Some(
            CapabilityResumeToken::new(
                blocked
                    .resume_token()
                    .expect("auth gate must carry a resume token")
                    .as_str(),
            )
            .expect("valid resume token"),
        ),
        disposition: None,
        prior_approval: None,
    };
    assert_eq!(
        resolver.resolve_count(),
        1,
        "first dispatch resolves the staged input once"
    );

    let auth_resumed = port
        .invoke_capability(LoopRequest {
            activity_id,
            surface_version: surface.version,
            capability_id,
            input_ref,
            approval_resume: None,
            auth_resume: Some(auth_resume),
        })
        .await
        .expect("auth resume must use replay input instead of resolving input_ref again");
    assert!(
        matches!(&auth_resumed, Resolution::Done(o) if o.verdict.is_success()),
        "auth resume must dispatch and complete, got {auth_resumed:?}"
    );
    assert_eq!(
        resolver.resolve_count(),
        1,
        "auth resume must not resolve the stale input ref"
    );
    let requests = runtime.auth_resume_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].3,
        serde_json::json!({"query": "is:unread", "max_results": 10}),
        "auth resume runtime request must receive the original input"
    );
}

#[tokio::test]
async fn denied_auth_resume_terminalizes_through_runtime_without_dispatch() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let runtime = Arc::new(
        QueuedHostRuntime::new(Vec::new(), Vec::new()).with_auth_decline_outcomes(vec![
            Err(HostRuntimeError::unavailable(
                "run-state filesystem unavailable",
            )),
            Ok(RuntimeCapabilityOutcome::Failed(
                RuntimeCapabilityFailure::new(
                    capability_id.clone(),
                    RuntimeFailureKind::GateDeclined,
                    Some("auth gate denied by user".to_string()),
                ),
            )),
        ]),
    );
    let result_writer = Arc::new(RecordingResultWriter::default());
    let milestone_sink =
        Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
    let port = runtime_capability_port(
        &capability_id,
        &provider_id,
        runtime.clone(),
        result_writer.clone(),
        milestone_sink.clone(),
        "thread-auth-decline",
    )
    .await;
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("current surface without the removed capability");
    let activity_id = ironclaw_turns::CapabilityActivityId::new();
    let expected_invocation_id = InvocationId::from_uuid(activity_id.as_uuid());
    let invocation = LoopRequest {
        activity_id,
        surface_version: surface.version,
        capability_id: capability_id.clone(),
        input_ref: CapabilityInputRef::new("input:removed-capability-denial")
            .expect("valid input ref"),
        approval_resume: None,
        auth_resume: Some(CapabilityAuthResume::denied()),
    };

    let unavailable = port
        .invoke_capability(invocation.clone())
        .await
        .expect_err("transient decline failure must remain retryable");
    assert!(matches!(
        unavailable.kind,
        AgentLoopHostErrorKind::Unavailable
    ));
    assert_eq!(runtime.auth_decline_requests().len(), 1);
    assert!(
        result_writer.failure_previews().is_empty(),
        "transient decline failure must not stage a terminal result"
    );
    assert!(
        !milestone_sink.milestones().iter().any(|milestone| matches!(
            &milestone.kind,
            ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed { .. }
        )),
        "transient decline failure must not record a terminal milestone"
    );

    let outcome = port
        .invoke_capability(invocation)
        .await
        .expect("exact denied auth resume remains retryable");

    assert!(matches!(
        &outcome,
        Resolution::Done(done)
            if done.verdict.error_kind() == Some(&FailureKind::GateDeclined)
    ));
    let requests = runtime.auth_decline_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].0.invocation_id, expected_invocation_id);
    assert_eq!(
        requests[1].0.resource_scope.invocation_id,
        expected_invocation_id
    );
    assert_eq!(requests[1].1, capability_id);
    let failure_previews = result_writer.failure_previews();
    assert_eq!(failure_previews.len(), 1);
    assert_eq!(failure_previews[0].0, expected_invocation_id);
    assert_eq!(failure_previews[0].1, capability_id);
    assert!(failure_previews[0].2.contains("denied"));
    assert!(milestone_sink.milestones().iter().any(|milestone| matches!(
        &milestone.kind,
        ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed {
            activity_id,
            capability_id: failed_capability_id,
            reason_kind: CapabilityFailureKind::GateDeclined,
            ..
        } if *activity_id == ironclaw_turns::CapabilityActivityId::from_uuid(
            expected_invocation_id.as_uuid()
        ) && failed_capability_id == &capability_id
    )));

    let replay = LoopRequest {
        activity_id,
        surface_version: ironclaw_turns::run_profile::CapabilitySurfaceVersion::new(
            "surface-after-removal",
        )
        .expect("valid surface version"),
        capability_id: capability_id.clone(),
        input_ref: CapabilityInputRef::new("input:changed-after-removal").expect("valid input ref"),
        approval_resume: None,
        auth_resume: Some(CapabilityAuthResume::denied()),
    };
    let replay_outcome = port
        .invoke_capability(replay)
        .await
        .expect("denial replay remains terminal after surface and input drift");
    assert!(matches!(
        &replay_outcome,
        Resolution::Done(done)
            if done.verdict.error_kind() == Some(&FailureKind::GateDeclined)
    ));
    assert_eq!(
        runtime.auth_decline_requests().len(),
        2,
        "denial replay must reuse the terminal result without calling runtime again"
    );
    assert_eq!(
        result_writer.failure_previews().len(),
        1,
        "denial replay must not restage the terminal failure preview"
    );
}

#[tokio::test]
async fn denied_auth_resume_rejects_mismatched_optional_token_before_runtime() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let runtime = Arc::new(QueuedHostRuntime::new(Vec::new(), Vec::new()));
    let port = runtime_capability_port(
        &capability_id,
        &provider_id,
        runtime.clone(),
        Arc::new(RecordingResultWriter::default()),
        Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default()),
        "thread-auth-decline-token-mismatch",
    )
    .await;
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("current empty surface");
    let activity_id = ironclaw_turns::CapabilityActivityId::new();
    let error = port
        .invoke_capability(LoopRequest {
            activity_id,
            surface_version: surface.version,
            capability_id,
            input_ref: CapabilityInputRef::new("input:malformed-auth-denial")
                .expect("valid input ref"),
            approval_resume: None,
            auth_resume: Some(CapabilityAuthResume {
                resume_token: Some(resume_token_for_different_activity(activity_id)),
                disposition: Some(ironclaw_turns::GateResumeDisposition::Denied),
                prior_approval: None,
            }),
        })
        .await
        .expect_err("mismatched denied resume token must fail closed");

    assert!(matches!(
        error.kind,
        AgentLoopHostErrorKind::InvalidInvocation
    ));
    assert!(
        runtime.auth_decline_requests().is_empty(),
        "malformed denied resume must fail before runtime"
    );
}

#[tokio::test]
async fn host_runtime_default_auth_decline_fails_closed_as_unavailable() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let error = NoopHostRuntime
        .decline_auth_capability((
            execution_context("thread-default-auth-decline"),
            capability_id,
        ))
        .await
        .expect_err("runtime without durable decline support must fail closed");

    assert!(matches!(error, HostRuntimeError::Unavailable { .. }));
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
async fn runtime_capability_unavailable_returns_failed_outcome_and_emits_failure_milestone() {
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
            vec![Err(HostRuntimeError::unavailable("runtime unavailable"))],
        )),
        Arc::new(RecordingResultWriter::default()),
        milestone_sink.clone(),
        "thread-runtime-capability-unavailable-milestone",
    )
    .await;

    let outcome = invoke_visible_runtime_capability(&port)
        .await
        .expect("host runtime unavailability should become a capability failure");

    assert!(matches!(
        &outcome,
        Resolution::Done(o)
            if o.verdict.error_kind() == Some(&FailureKind::Unavailable)
    ));
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 2);
    assert!(matches!(
        &milestones[1].kind,
        ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed {
            activity_id: _,
            capability_id: actual,
            provider: Some(provider),
            runtime: Some(RuntimeKind::FirstParty),
            reason_kind,
            ..
        } if actual == &capability_id
            && provider == &provider_id
            && reason_kind == &CapabilityFailureKind::Unavailable
    ));
}

#[tokio::test]
async fn runtime_capability_invalid_request_preserves_host_error_and_emits_failure_milestone() {
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
            vec![Err(HostRuntimeError::invalid_request("bad request"))],
        )),
        Arc::new(RecordingResultWriter::default()),
        milestone_sink.clone(),
        "thread-runtime-capability-invalid-request-milestone",
    )
    .await;

    let error = invoke_visible_runtime_capability(&port)
        .await
        .expect_err("host runtime invalid request should remain a host error");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 2);
    assert!(matches!(
        &milestones[1].kind,
        ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed {
            activity_id: _,
            capability_id: actual,
            provider: Some(provider),
            runtime: Some(RuntimeKind::FirstParty),
            reason_kind,
            ..
        } if actual == &capability_id
            && provider == &provider_id
            && reason_kind.as_str() == AgentLoopHostErrorKind::InvalidInvocation.as_str()
    ));
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

async fn visible_runtime_invocation(port: &HostRuntimeLoopCapabilityPort) -> LoopRequest {
    port.visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_tool_call()))
        .await
        .expect("provider tool call registers");
    LoopRequest {
        activity_id: candidate.activity_id,
        surface_version: candidate.surface_version,
        capability_id: candidate.capability_id,
        input_ref: candidate.input_ref,
        approval_resume: None,
        auth_resume: None,
    }
}

async fn invoke_visible_runtime_capability(
    port: &HostRuntimeLoopCapabilityPort,
) -> Result<Resolution, AgentLoopHostError> {
    port.invoke_capability(visible_runtime_invocation(port).await)
        .await
}

#[tokio::test]
async fn approval_resume_metadata_invokes_runtime_resume_with_original_invocation() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let approval_request_id = ApprovalRequestId::new();
    let runtime = Arc::new(ApprovalResumeRecordingRuntime::new(
        visible_capability(capability_id.clone(), provider_id.clone()),
        approval_request_id,
    ));
    let mut context = execution_context("thread-approval-resume");
    let run_context = loop_run_context(&context).await;
    let loop_driver_extension =
        loop_driver_execution_extension_id(&run_context).expect("valid extension id");
    context.grants.grants.push(dispatch_capability_grant(
        &capability_id,
        &loop_driver_extension,
    ));
    // Wire the host-private replay-payload store: raw input/estimate no longer
    // ride the loop-facing outcome (§5.3 Stage 2a-i), so the host persists them
    // at the fresh gate raise and reconstitutes them on resume.
    let replay_store = Arc::new(RecordingReplayPayloadStore::default());
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
            provider_id,
            dispatch_trust_decision(),
        )])),
        Arc::new(InputRefEchoResolver),
        Arc::new(RecordingResultWriter::default()),
        Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default()),
    )
    .with_replay_payload_store(replay_store.clone())
    .port_for_run_context(run_context);

    let first_invocation = visible_runtime_invocation(&port).await;
    let first = port
        .invoke_capability(first_invocation.clone())
        .await
        .expect("first invocation returns approval gate");
    // §5.3 Stage 2: the approval gate surfaces as `Resolution::Blocked(Blocked::Approval)`
    // carrying only the resume token; the loop rebuilds `CapabilityApprovalResume` from
    // it (mirrors `approval_resume_from_gate` in the executor). The `correlation_id` left
    // the channel — the host persists it in the replay payload at the fresh gate raise and
    // reconstitutes it on resume, so we recover it from that same store below.
    let Resolution::Blocked(blocked @ Blocked::Approval(_)) = &first else {
        panic!("approval gate must carry resume metadata, got {first:?}");
    };
    let resume_token = CapabilityResumeToken::new(
        blocked
            .resume_token()
            .expect("approval gate carries a resume token")
            .as_str(),
    )
    .expect("valid resume token");

    // The fresh gate raise must have persisted the host-private replay payload
    // keyed by the invocation id encoded in the resume token — the loop-facing
    // outcome deliberately no longer carries raw input/estimate.
    let raised_invocation_id = ironclaw_host_api::InvocationId::parse(resume_token.as_str())
        .expect("resume token carries original invocation id");
    let persisted = replay_store
        .get(raised_invocation_id)
        .expect("fresh approval-gate raise must persist a replay payload");
    assert_eq!(persisted.input, serde_json::json!({ "message": "hello" }));
    assert_eq!(persisted.estimate, ResourceEstimate::default());
    assert_eq!(persisted.input_ref, first_invocation.input_ref);

    let resume = CapabilityApprovalResume {
        approval_request_id,
        resume_token,
        correlation_id: persisted.correlation_id,
        input_ref: first_invocation.input_ref.clone(),
    };

    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let resumed = port
        .invoke_capability(LoopRequest {
            activity_id: first_invocation.activity_id,
            surface_version: surface.version,
            capability_id: capability_id.clone(),
            input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new(
                "input:approval-resume-replayed-call",
            )
            .expect("valid input ref"),
            approval_resume: Some(resume.clone()),
            auth_resume: None,
        })
        .await
        .expect("approval resume dispatch succeeds");

    assert!(matches!(&resumed, Resolution::Done(o) if o.verdict.is_success()));
    assert_eq!(runtime.invoke_count(), 1);
    let resume_requests = runtime.resume_requests();
    assert_eq!(resume_requests.len(), 1);
    assert_eq!(resume_requests[0].1, approval_request_id);
    let resume_invocation_id = ironclaw_host_api::InvocationId::parse(resume.resume_token.as_str())
        .expect("resume token carries original invocation id");
    assert_eq!(resume_requests[0].0.invocation_id, resume_invocation_id);
    assert_eq!(
        resume_requests[0].0.resource_scope.invocation_id,
        resume_invocation_id
    );
    assert_eq!(resume_requests[0].0.correlation_id, resume.correlation_id);
    assert_eq!(resume.input_ref, first_invocation.input_ref);
    assert_eq!(
        resume_requests[0].4,
        serde_json::json!({ "message": "hello" })
    );
}

#[tokio::test]
async fn auth_resume_after_approval_reuses_original_invocation_identity() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let approval_request_id = ApprovalRequestId::new();
    // The approval resume returns an auth gate, modeling a capability that
    // needs a credential after its approval was granted.
    let runtime = Arc::new(ApprovalResumeRecordingRuntime::new_with_resume_outcomes(
        visible_capability(capability_id.clone(), provider_id.clone()),
        approval_request_id,
        vec![Ok(RuntimeCapabilityOutcome::AuthRequired(
            RuntimeAuthGate {
                gate_id: RuntimeGateId::new(),
                capability_id: capability_id.clone(),
                reason: RuntimeBlockedReason::AuthRequired,
                required_secrets: Vec::new(),
                credential_requirements: Vec::new(),
            },
        ))],
    ));
    let mut context = execution_context("thread-auth-resume-identity");
    let run_context = loop_run_context(&context).await;
    let loop_driver_extension =
        loop_driver_execution_extension_id(&run_context).expect("valid extension id");
    context.grants.grants.push(dispatch_capability_grant(
        &capability_id,
        &loop_driver_extension,
    ));
    // Wire the host-private replay-payload store: raw input/estimate no longer
    // ride the loop-facing outcome (§5.3 Stage 2a-i), so the host persists them
    // at the fresh gate raise and reconstitutes them on resume.
    let replay_store = Arc::new(RecordingReplayPayloadStore::default());
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
            provider_id,
            dispatch_trust_decision(),
        )])),
        Arc::new(InputRefEchoResolver),
        Arc::new(RecordingResultWriter::default()),
        Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default()),
    )
    .with_replay_payload_store(replay_store.clone())
    .port_for_run_context(run_context);

    let first_invocation = visible_runtime_invocation(&port).await;
    let first = port
        .invoke_capability(first_invocation.clone())
        .await
        .expect("first invocation returns approval gate");
    // §5.3 Stage 2: rebuild `CapabilityApprovalResume` from the approval gate's
    // surviving resume token; the `correlation_id` is recovered from the replay
    // payload the host persisted at the fresh gate raise (it left the channel).
    let Resolution::Blocked(blocked @ Blocked::Approval(_)) = &first else {
        panic!("approval gate must carry resume metadata, got {first:?}");
    };
    let resume_token = CapabilityResumeToken::new(
        blocked
            .resume_token()
            .expect("approval gate carries a resume token")
            .as_str(),
    )
    .expect("valid resume token");
    let raised_invocation_id = ironclaw_host_api::InvocationId::parse(resume_token.as_str())
        .expect("resume token carries original invocation id");
    let persisted = replay_store
        .get(raised_invocation_id)
        .expect("fresh approval-gate raise must persist a replay payload");
    let resume = CapabilityApprovalResume {
        approval_request_id,
        resume_token,
        correlation_id: persisted.correlation_id,
        input_ref: first_invocation.input_ref.clone(),
    };

    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let auth_blocked = port
        .invoke_capability(LoopRequest {
            activity_id: first_invocation.activity_id,
            surface_version: surface.version.clone(),
            capability_id: capability_id.clone(),
            input_ref: first_invocation.input_ref.clone(),
            approval_resume: Some(resume.clone()),
            auth_resume: None,
        })
        .await
        .expect("approval resume dispatch reaches the auth gate");
    assert!(
        matches!(&auth_blocked, Resolution::Blocked(Blocked::Auth(_))),
        "approval resume must surface the credential gate, got {auth_blocked:?}"
    );

    // Re-dispatch after the auth gate the way the executor does: carrying the
    // ORIGINAL invocation identity and the already-granted approval.
    // Use a stable correlation_id so we can assert it arrives at the runtime.
    let original_correlation_id = resume.correlation_id;
    let auth_resumed = port
        .invoke_capability(LoopRequest {
            activity_id: first_invocation.activity_id,
            surface_version: surface.version,
            capability_id: capability_id.clone(),
            input_ref: first_invocation.input_ref.clone(),
            approval_resume: None,
            auth_resume: Some(CapabilityAuthResume {
                resume_token: Some(resume.resume_token.clone()),
                disposition: None,
                // Carry the prior approval so the port restores the original
                // correlation identifier onto the invocation context.
                prior_approval: Some(ironclaw_turns::run_profile::AuthResumeApprovalIdentity {
                    approval_request_id,
                    correlation_id: original_correlation_id,
                }),
            }),
        })
        .await
        .expect("auth resume dispatch succeeds");
    assert!(
        matches!(&auth_resumed, Resolution::Done(o) if o.verdict.is_success()),
        "auth resume must dispatch and complete, got {auth_resumed:?}"
    );

    // Auth re-dispatch must reuse the original invocation identifier so that
    // fingerprinted approval leases (scoped to the original invocation) remain matchable.
    let original_invocation_id =
        ironclaw_host_api::InvocationId::parse(resume.resume_token.as_str())
            .expect("resume token carries original invocation id");
    let auth_resume_requests = runtime.auth_resume_requests();
    assert_eq!(auth_resume_requests.len(), 1);
    assert_eq!(
        auth_resume_requests[0].0.invocation_id, original_invocation_id,
        "auth re-dispatch must reuse the original invocation id"
    );
    assert_eq!(
        auth_resume_requests[0].0.resource_scope.invocation_id, original_invocation_id,
        "lease matching scope must carry the original invocation id"
    );
    assert_eq!(
        auth_resume_requests[0].4,
        Some(approval_request_id),
        "the granted approval must travel with the auth re-dispatch"
    );
    // Original correlation identifier must be restored onto the invocation context.
    assert_eq!(
        auth_resume_requests[0].0.correlation_id, original_correlation_id,
        "auth re-dispatch must restore the original correlation_id"
    );
    assert_eq!(runtime.invoke_count(), 1, "no fresh first-call invocation");
    assert_eq!(runtime.resume_requests().len(), 1);
}

#[tokio::test]
async fn approval_resume_host_error_returns_failed_outcome_and_emits_failure_milestone() {
    let capability_id = CapabilityId::new("demo.echo").expect("valid capability id");
    let provider_id = ExtensionId::new("demo").expect("valid provider id");
    let approval_request_id = ApprovalRequestId::new();
    let runtime = Arc::new(ApprovalResumeRecordingRuntime::new_with_resume_outcomes(
        visible_capability(capability_id.clone(), provider_id.clone()),
        approval_request_id,
        vec![Err(HostRuntimeError::unavailable("runtime unavailable"))],
    ));
    let milestone_sink =
        Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
    let mut context = execution_context("thread-approval-resume-host-error");
    let run_context = loop_run_context(&context).await;
    let loop_driver_extension =
        loop_driver_execution_extension_id(&run_context).expect("valid extension id");
    context.grants.grants.push(dispatch_capability_grant(
        &capability_id,
        &loop_driver_extension,
    ));
    let port = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request(context).with_provider_trust(std::collections::BTreeMap::from([(
            provider_id.clone(),
            dispatch_trust_decision(),
        )])),
        Arc::new(InputRefEchoResolver),
        Arc::new(RecordingResultWriter::default()),
        milestone_sink.clone(),
    )
    // Fresh raise persists the replay payload; the approval resume reconstitutes
    // it host-side before re-dispatch (§5.3 Stage 2a-i).
    .with_replay_payload_store(Arc::new(RecordingReplayPayloadStore::default()))
    .port_for_run_context(run_context);

    let first_invocation = visible_runtime_invocation(&port).await;
    let first = port
        .invoke_capability(first_invocation.clone())
        .await
        .expect("first invocation returns approval gate");
    // §5.3 Stage 2: rebuild `CapabilityApprovalResume` from the approval gate's
    // surviving resume token. The correlation_id/input_ref are advisory on resume
    // (the host reconstitutes them from the persisted replay payload keyed by the
    // token's invocation id), so a fresh correlation id is fine here.
    let Resolution::Blocked(blocked @ Blocked::Approval(_)) = &first else {
        panic!("approval gate must carry resume metadata, got {first:?}");
    };
    let resume = CapabilityApprovalResume {
        approval_request_id,
        resume_token: CapabilityResumeToken::new(
            blocked
                .resume_token()
                .expect("approval gate carries a resume token")
                .as_str(),
        )
        .expect("valid resume token"),
        correlation_id: ironclaw_host_api::CorrelationId::new(),
        input_ref: first_invocation.input_ref.clone(),
    };

    let surface = port
        .visible_capabilities(VisibleCapabilityRequest {})
        .await
        .expect("visible capabilities load");
    let resumed = port
        .invoke_capability(LoopRequest {
            activity_id: first_invocation.activity_id,
            surface_version: surface.version,
            capability_id: capability_id.clone(),
            input_ref: CapabilityInputRef::new("input:approval-resume-host-error")
                .expect("valid input ref"),
            approval_resume: Some(resume),
            auth_resume: None,
        })
        .await
        .expect("approval resume host error should become a capability failure");

    assert!(matches!(
        &resumed,
        Resolution::Done(o)
            if o.verdict.error_kind() == Some(&FailureKind::Unavailable)
                && o.summary.as_str() == "runtime unavailable"
    ));
    assert_eq!(runtime.invoke_count(), 1);
    assert_eq!(runtime.resume_requests().len(), 1);
    let milestones = milestone_sink.milestones();
    assert_eq!(milestones.len(), 3);
    assert!(matches!(
        &milestones[2].kind,
        ironclaw_turns::run_profile::LoopHostMilestoneKind::CapabilityFailed {
            capability_id: actual,
            provider: Some(provider),
            runtime: Some(RuntimeKind::FirstParty),
            reason_kind,
            ..
        } if actual == &capability_id
            && provider == &provider_id
            && reason_kind == &CapabilityFailureKind::Unavailable
    ));
}

struct InputRefEchoResolver;

#[async_trait]
impl LoopCapabilityInputResolver for InputRefEchoResolver {
    async fn resolve_capability_input(
        &self,
        _run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        Ok(serde_json::json!({ "input_ref": input_ref.as_str() }))
    }
}

struct OneShotInputResolver {
    payload: serde_json::Value,
    resolve_count: AtomicUsize,
}

impl OneShotInputResolver {
    fn new(payload: serde_json::Value) -> Self {
        Self {
            payload,
            resolve_count: AtomicUsize::new(0),
        }
    }

    fn resolve_count(&self) -> usize {
        self.resolve_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LoopCapabilityInputResolver for OneShotInputResolver {
    async fn resolve_capability_input(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        if self.resolve_count.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(self.payload.clone())
        } else {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "capability input ref is not scoped to this loop run",
            ))
        }
    }
}

struct ApprovalResumeRecordingRuntime {
    capability: VisibleCapability,
    approval_request_id: ApprovalRequestId,
    invoke_count: AtomicUsize,
    resume_requests: Mutex<Vec<RuntimeApprovalResume>>,
    resume_outcomes: Mutex<VecDeque<Result<RuntimeCapabilityOutcome, HostRuntimeError>>>,
    auth_resume_requests: Mutex<Vec<RuntimeAuthResume>>,
}

impl ApprovalResumeRecordingRuntime {
    fn new(capability: VisibleCapability, approval_request_id: ApprovalRequestId) -> Self {
        Self::new_with_resume_outcomes(capability, approval_request_id, Vec::new())
    }

    fn new_with_resume_outcomes(
        capability: VisibleCapability,
        approval_request_id: ApprovalRequestId,
        resume_outcomes: Vec<Result<RuntimeCapabilityOutcome, HostRuntimeError>>,
    ) -> Self {
        Self {
            capability,
            approval_request_id,
            invoke_count: AtomicUsize::new(0),
            resume_requests: Mutex::new(Vec::new()),
            resume_outcomes: Mutex::new(VecDeque::from(resume_outcomes)),
            auth_resume_requests: Mutex::new(Vec::new()),
        }
    }

    fn invoke_count(&self) -> usize {
        self.invoke_count.load(Ordering::SeqCst)
    }

    fn resume_requests(&self) -> Vec<RuntimeApprovalResume> {
        self.resume_requests
            .lock()
            .expect("resume requests lock")
            .clone()
    }

    fn auth_resume_requests(&self) -> Vec<RuntimeAuthResume> {
        self.auth_resume_requests
            .lock()
            .expect("auth resume requests lock")
            .clone()
    }
}

#[async_trait]
impl HostRuntime for ApprovalResumeRecordingRuntime {
    async fn invoke_capability(
        &self,
        request: RuntimeInvocation,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.invoke_count.fetch_add(1, Ordering::SeqCst);
        Ok(RuntimeCapabilityOutcome::ApprovalRequired(
            RuntimeApprovalGate {
                approval_request_id: self.approval_request_id,
                capability_id: request.1,
                reason: RuntimeBlockedReason::ApprovalRequired,
            },
        ))
    }

    async fn resume_capability(
        &self,
        request: RuntimeApprovalResume,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.resume_requests
            .lock()
            .expect("resume requests lock")
            .push(request.clone());
        if let Some(outcome) = self
            .resume_outcomes
            .lock()
            .expect("resume outcomes lock")
            .pop_front()
        {
            return outcome;
        }
        Ok(RuntimeCapabilityOutcome::Completed(Box::new(
            RuntimeCapabilityCompleted {
                capability_id: request.2,
                output: serde_json::json!({"resumed": true}),
                display_preview: None,
                usage: ResourceUsage::default(),
            },
        )))
    }

    async fn auth_resume_capability(
        &self,
        request: RuntimeAuthResume,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.auth_resume_requests
            .lock()
            .expect("auth resume requests lock")
            .push(request.clone());
        Ok(RuntimeCapabilityOutcome::Completed(Box::new(
            RuntimeCapabilityCompleted {
                capability_id: request.1,
                output: serde_json::json!({"auth_resumed": true}),
                display_preview: None,
                usage: ResourceUsage::default(),
            },
        )))
    }

    async fn visible_capabilities(
        &self,
        _request: ironclaw_host_runtime::VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
        Ok(VisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new("surface-v1").expect("valid version"),
            capabilities: vec![self.capability.clone()],
        })
    }

    async fn cancel_work(
        &self,
        _request: CancelRuntimeWorkRequest,
    ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
        unreachable!("approval resume recording runtime should not cancel work")
    }

    async fn runtime_status(
        &self,
        _request: RuntimeStatusRequest,
    ) -> Result<HostRuntimeStatus, HostRuntimeError> {
        unreachable!("approval resume recording runtime should not report status")
    }

    async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
        unreachable!("approval resume recording runtime should not report health")
    }
}

struct QueuedHostRuntime {
    capabilities: Vec<VisibleCapability>,
    outcomes: Mutex<VecDeque<Result<RuntimeCapabilityOutcome, HostRuntimeError>>>,
    auth_resume_outcomes: Mutex<VecDeque<Result<RuntimeCapabilityOutcome, HostRuntimeError>>>,
    auth_resume_requests: Mutex<Vec<RuntimeAuthResume>>,
    auth_decline_outcomes: Mutex<VecDeque<Result<RuntimeCapabilityOutcome, HostRuntimeError>>>,
    auth_decline_requests: Mutex<Vec<ironclaw_host_runtime::RuntimeAuthDecline>>,
}

impl QueuedHostRuntime {
    fn new(
        capabilities: Vec<VisibleCapability>,
        outcomes: Vec<Result<RuntimeCapabilityOutcome, HostRuntimeError>>,
    ) -> Self {
        Self {
            capabilities,
            outcomes: Mutex::new(VecDeque::from(outcomes)),
            auth_resume_outcomes: Mutex::new(VecDeque::new()),
            auth_resume_requests: Mutex::new(Vec::new()),
            auth_decline_outcomes: Mutex::new(VecDeque::new()),
            auth_decline_requests: Mutex::new(Vec::new()),
        }
    }

    fn with_auth_resume_outcomes(
        self,
        outcomes: Vec<Result<RuntimeCapabilityOutcome, HostRuntimeError>>,
    ) -> Self {
        *self
            .auth_resume_outcomes
            .lock()
            .expect("auth resume outcomes lock") = VecDeque::from(outcomes);
        self
    }

    fn auth_resume_requests(&self) -> Vec<RuntimeAuthResume> {
        self.auth_resume_requests
            .lock()
            .expect("auth resume requests lock")
            .clone()
    }

    fn with_auth_decline_outcomes(
        self,
        outcomes: Vec<Result<RuntimeCapabilityOutcome, HostRuntimeError>>,
    ) -> Self {
        *self
            .auth_decline_outcomes
            .lock()
            .expect("auth decline outcomes lock") = VecDeque::from(outcomes);
        self
    }

    fn auth_decline_requests(&self) -> Vec<ironclaw_host_runtime::RuntimeAuthDecline> {
        self.auth_decline_requests
            .lock()
            .expect("auth decline requests lock")
            .clone()
    }
}

#[async_trait]
impl HostRuntime for QueuedHostRuntime {
    async fn invoke_capability(
        &self,
        _request: RuntimeInvocation,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.outcomes
            .lock()
            .expect("outcomes lock")
            .pop_front()
            .expect("queued host runtime outcome")
    }

    async fn resume_capability(
        &self,
        _request: RuntimeApprovalResume,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        unreachable!("queued host runtime should not resume")
    }

    async fn auth_resume_capability(
        &self,
        request: RuntimeAuthResume,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.auth_resume_requests
            .lock()
            .expect("auth resume requests lock")
            .push(request);
        self.auth_resume_outcomes
            .lock()
            .expect("auth resume outcomes lock")
            .pop_front()
            .expect("queued host runtime auth resume outcome")
    }

    async fn decline_auth_capability(
        &self,
        request: ironclaw_host_runtime::RuntimeAuthDecline,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.auth_decline_requests
            .lock()
            .expect("auth decline requests lock")
            .push(request);
        self.auth_decline_outcomes
            .lock()
            .expect("auth decline outcomes lock")
            .pop_front()
            .expect("queued host runtime auth decline outcome")
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

// arch-exempt: large_file, pre-existing large file minimally touched for the §5.3 Stage 2a-i replay-payload move (field/store wiring + tests), plan #6175
