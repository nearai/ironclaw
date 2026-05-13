//! Contract tests for the product workflow facade.

use std::sync::Arc;

use chrono::{Duration, Utc};
use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{
    AdapterInstallationId, ApprovalDecision, ApprovalResolutionPayload, AuthRequirement,
    AuthResolutionPayload, AuthResolutionResult, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, InboundCommandPayload, LinkedThreadActionPayload,
    MissionActionPayload as AdapterMissionActionPayload,
    MissionFireSuppressionReason as AdapterMissionFireSuppressionReason, ParsedProductInbound,
    ProductAdapterError, ProductAdapterId, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload, ProductRejectionDisposition, ProductRejectionKind, ProductTriggerReason,
    ProductWorkflow, ProductWorkflowRejectionKind, ProjectionSubscriptionPayload,
    ProtocolAuthEvidence, SystemActionPayload, TrustedInboundContext, UserMessagePayload,
};
use ironclaw_product_workflow::{
    ActionDispatchKind, ActionFingerprintKey, AuthRequestRef, DefaultProductWorkflow,
    FakeApprovalInteractionService, FakeAuthInteractionService, FakeIdempotencyLedger,
    FakeInboundTurnService, FakeLinkedThreadActionService, FakeMissionService,
    FakeProductCommandRouter, FakeProjectionSubscriptionAuthority, FakeSystemActionService,
    IdempotencyDecision, IdempotencyLedger, InboundTurnOutcome, LinkedThreadActionId,
    MissionFireRejectionReason,
    MissionFireSuppressionReason as WorkflowMissionFireSuppressionReason, ProductCommandName,
    ProductWorkflowError, ResolvedBinding, SourceBindingKey,
};
use ironclaw_turns::{AcceptedMessageRef, LoopGateRef, TurnError, TurnRunId};
use uuid::Uuid;

fn sample_envelope_with_payload(
    event_suffix: &str,
    payload: ProductInboundPayload,
) -> ProductInboundEnvelope {
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Secret".into(),
        },
        "install_alpha",
    );
    let context = TrustedInboundContext::from_verified_evidence(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        Utc::now(),
        &evidence,
    )
    .expect("verified");

    let parsed = ParsedProductInbound::new(
        ExternalEventId::new(format!("evt:{event_suffix}")).expect("valid"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("valid"),
        payload,
    )
    .expect("parsed");

    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
}

fn sample_envelope(event_suffix: &str) -> ProductInboundEnvelope {
    sample_envelope_with_payload(
        event_suffix,
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("valid"),
        ),
    )
}

fn sample_noop_envelope(event_suffix: &str) -> ProductInboundEnvelope {
    sample_envelope_with_payload(event_suffix, ProductInboundPayload::NoOp)
}

fn sample_linked_thread_action_envelope(
    event_suffix: &str,
    payload: LinkedThreadActionPayload,
) -> ProductInboundEnvelope {
    sample_envelope_with_payload(
        event_suffix,
        ProductInboundPayload::LinkedThreadAction(payload),
    )
}

fn sample_system_action_envelope(
    event_suffix: &str,
    payload: SystemActionPayload,
) -> ProductInboundEnvelope {
    sample_envelope_with_payload(event_suffix, ProductInboundPayload::SystemAction(payload))
}

fn sample_approval_envelope(event_suffix: &str, gate_ref: &str) -> ProductInboundEnvelope {
    sample_envelope_with_payload(
        event_suffix,
        ProductInboundPayload::ApprovalResolution(
            ApprovalResolutionPayload::new(gate_ref, ApprovalDecision::ApproveOnce)
                .expect("valid approval payload"),
        ),
    )
}

fn sample_auth_envelope(event_suffix: &str, auth_request_ref: &str) -> ProductInboundEnvelope {
    sample_envelope_with_payload(
        event_suffix,
        ProductInboundPayload::AuthResolution(
            AuthResolutionPayload::new(
                auth_request_ref,
                AuthResolutionResult::CredentialProvided {
                    credential_ref: "cred_abc".into(),
                },
            )
            .expect("valid auth payload"),
        ),
    )
}

fn sample_mission_action_envelope(
    event_suffix: &str,
    intent: &str,
    mission_id_hint: Option<&str>,
) -> ProductInboundEnvelope {
    sample_envelope_with_payload(
        event_suffix,
        ProductInboundPayload::MissionAction(
            AdapterMissionActionPayload::new(
                intent,
                mission_id_hint.map(String::from),
                Some("{\"trigger\":\"manual\"}".to_string()),
            )
            .expect("valid mission action payload"),
        ),
    )
}

fn sample_user_message_envelope_with_text(
    event_suffix: &str,
    text: &str,
) -> ProductInboundEnvelope {
    sample_envelope_with_payload(
        event_suffix,
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(text, vec![], ProductTriggerReason::DirectChat)
                .expect("valid user message payload"),
        ),
    )
}

fn sample_command_envelope(
    event_suffix: &str,
    command: &str,
    arguments: &str,
) -> ProductInboundEnvelope {
    sample_envelope_with_payload(
        event_suffix,
        ProductInboundPayload::Command(
            InboundCommandPayload::new(command, arguments, ProductTriggerReason::BotCommand)
                .expect("valid command payload"),
        ),
    )
}

fn sample_subscription_envelope(event_suffix: &str) -> ProductInboundEnvelope {
    sample_envelope_with_payload(
        event_suffix,
        ProductInboundPayload::SubscriptionRequest(
            ProjectionSubscriptionPayload::new(None, None).expect("valid subscription payload"),
        ),
    )
}

#[test]
fn action_fingerprint_retains_typed_identifiers() {
    let adapter_id = ProductAdapterId::new("test_adapter").expect("valid");
    let installation_id = AdapterInstallationId::new("install_alpha").expect("valid");
    let source_binding_key = SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
        .expect("valid source binding key");
    let external_event_id = ExternalEventId::new("evt:typed").expect("valid");

    let fingerprint = ActionFingerprintKey::new(
        adapter_id.clone(),
        installation_id.clone(),
        source_binding_key.clone(),
        external_event_id.clone(),
    );

    assert_eq!(fingerprint.adapter_id, adapter_id);
    assert_eq!(fingerprint.installation_id, installation_id);
    assert_eq!(fingerprint.source_binding_key, source_binding_key);
    assert_eq!(fingerprint.external_event_id, external_event_id);
}

#[test]
fn turn_submission_error_maps_to_stable_product_category() {
    let err: ProductAdapterError = ProductWorkflowError::TurnSubmissionFailed {
        error: TurnError::Unauthorized,
    }
    .into();

    match err {
        ProductAdapterError::WorkflowRejected {
            kind,
            status_code,
            retryable,
            ..
        } => {
            assert_eq!(kind, ProductWorkflowRejectionKind::Unauthorized);
            assert_eq!(status_code, 403);
            assert!(!retryable);
        }
        other => panic!("expected typed workflow rejection, got {other:?}"),
    }
}

#[test]
fn action_dispatch_kind_retains_typed_payload_refs() {
    let command_payload = ProductInboundPayload::Command(
        InboundCommandPayload::new("help", "", ProductTriggerReason::BotCommand).expect("valid"),
    );
    assert_eq!(
        ActionDispatchKind::try_from_payload(&command_payload).expect("command kind"),
        ActionDispatchKind::Command {
            command: ProductCommandName::new("help").expect("valid command")
        }
    );

    let gate_ref = LoopGateRef::new("gate:approval-1").expect("valid gate ref");
    let approval_payload = ProductInboundPayload::ApprovalResolution(
        ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::ApproveOnce)
            .expect("valid"),
    );
    assert_eq!(
        ActionDispatchKind::try_from_payload(&approval_payload).expect("approval kind"),
        ActionDispatchKind::ApprovalResolution { gate_ref }
    );

    let auth_payload = ProductInboundPayload::AuthResolution(
        AuthResolutionPayload::new("auth-request-1", AuthResolutionResult::Denied).expect("valid"),
    );
    assert_eq!(
        ActionDispatchKind::try_from_payload(&auth_payload).expect("auth kind"),
        ActionDispatchKind::AuthResolution {
            auth_request_ref: AuthRequestRef::new("auth-request-1").expect("valid auth ref")
        }
    );

    let linked_payload = ProductInboundPayload::LinkedThreadAction(
        LinkedThreadActionPayload::new("open-thread", None, None).expect("valid"),
    );
    assert_eq!(
        ActionDispatchKind::try_from_payload(&linked_payload).expect("linked kind"),
        ActionDispatchKind::LinkedThreadAction {
            action_id: LinkedThreadActionId::new("open-thread").expect("valid action id")
        }
    );
}

fn fake_binding() -> ResolvedBinding {
    ResolvedBinding {
        tenant_id: TenantId::new("tenant:fake").expect("valid tenant"),
        user_id: UserId::new("user:fake").expect("valid user"),
        thread_id: ThreadId::new("thread:fake").expect("valid thread"),
        agent_id: Some(AgentId::new("agent:fake").expect("valid agent")),
        project_id: None,
    }
}

fn build_workflow() -> (
    DefaultProductWorkflow,
    Arc<FakeInboundTurnService>,
    Arc<FakeIdempotencyLedger>,
) {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone());
    (workflow, inbound, ledger)
}

#[tokio::test]
async fn user_message_dispatches_through_inbound_turn_service() {
    let (workflow, inbound, ledger) = build_workflow();
    let envelope = sample_envelope("1");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    assert_eq!(inbound.accepted_count(), 1);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn noop_returns_noop_ack() {
    let (workflow, inbound, ledger) = build_workflow();
    let envelope = sample_noop_envelope("noop1");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::NoOp));
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn duplicate_envelope_replays_prior_outcome() {
    let (workflow, inbound, _ledger) = build_workflow();

    // First submission.
    let envelope = sample_envelope("dup1");
    let first_ack = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("first accept");
    assert!(matches!(first_ack, ProductInboundAck::Accepted { .. }));
    assert_eq!(inbound.accepted_count(), 1);

    // Second submission of same envelope.
    let second_ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("second accept");
    assert!(matches!(second_ack, ProductInboundAck::Duplicate { .. }));
    // InboundTurnService should NOT be called a second time.
    assert_eq!(inbound.accepted_count(), 1);
}

#[tokio::test]
async fn settled_user_message_records_actual_submitted_run_id() {
    let (workflow, _inbound, ledger) = build_workflow();
    let envelope = sample_envelope("run-id");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");
    let ProductInboundAck::Accepted {
        submitted_run_id, ..
    } = ack
    else {
        panic!("expected accepted ack");
    };
    let actions = ledger.settled_actions();
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0].dispatch_kind,
        Some(ActionDispatchKind::UserMessageTurn {
            run_id: submitted_run_id
        })
    );
}

#[tokio::test]
async fn retryable_dispatch_failure_releases_fingerprint_for_recovery() {
    let (workflow, inbound, ledger) = build_workflow();
    inbound.force_failure(ProductWorkflowError::Transient {
        reason: "turn coordinator unavailable".into(),
    });

    let envelope = sample_envelope("transient-released");
    let first_err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("first attempt should be retryable");
    assert!(first_err.is_retryable());
    assert_eq!(inbound.attempt_count(), 1);
    assert_eq!(ledger.in_flight_count(), 0);

    let second_err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("released fingerprint should retry dispatch");
    assert!(second_err.is_retryable());
    assert_eq!(inbound.attempt_count(), 2);
    assert_eq!(ledger.settled_count(), 0);
}

#[tokio::test]
async fn deferred_busy_is_not_settled_and_retry_can_submit_same_message() {
    let (workflow, inbound, ledger) = build_workflow();
    let accepted_message_ref = AcceptedMessageRef::new("msg:busy-retry").expect("valid msg ref");
    let busy_run = TurnRunId::new();
    inbound.program_outcome(InboundTurnOutcome::DeferredBusy {
        accepted_message_ref: accepted_message_ref.clone(),
        active_run_id: busy_run,
        binding: fake_binding(),
    });

    let envelope = sample_envelope("busy-retry");
    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("busy ack");
    assert!(matches!(first, ProductInboundAck::DeferredBusy { .. }));
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);

    let submitted_run_id = TurnRunId::new();
    inbound.program_outcome(InboundTurnOutcome::Submitted {
        accepted_message_ref: accepted_message_ref.clone(),
        submitted_run_id,
        binding: fake_binding(),
    });
    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect("retry submit");
    assert!(matches!(
        second,
        ProductInboundAck::Accepted {
            submitted_run_id: run_id,
            ..
        } if run_id == submitted_run_id
    ));
    assert_eq!(inbound.attempt_count(), 2);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn fake_ledger_expiration_reclaims_in_flight_fingerprint() {
    let ledger = FakeIdempotencyLedger::new();
    let received_at = Utc::now();
    let fingerprint = ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;").expect("valid"),
        ExternalEventId::new("evt:lease").expect("valid"),
    );

    let first = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("reserve");
    assert!(matches!(first, IdempotencyDecision::New(_)));
    let duplicate = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect_err("fresh in-flight action blocks duplicate dispatch");
    assert!(matches!(duplicate, ProductWorkflowError::Transient { .. }));

    assert_eq!(
        ledger.expire_in_flight_before(received_at + Duration::seconds(1)),
        1
    );
    let reclaimed = ledger
        .begin_or_replay(fingerprint, received_at)
        .await
        .expect("expired fingerprint can be reclaimed");
    assert!(matches!(reclaimed, IdempotencyDecision::New(_)));
}

#[tokio::test]
async fn permanent_turn_submission_failure_settles_terminal_rejection() {
    let (workflow, inbound, ledger) = build_workflow();
    inbound.force_failure(ProductWorkflowError::TurnSubmissionFailed {
        error: TurnError::Unauthorized,
    });

    let envelope = sample_envelope("terminal-turn-error");
    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("unauthorized turn rejection should surface error");
    assert!(!err.is_retryable());
    assert_eq!(ledger.settled_count(), 1);

    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("terminal rejection should replay duplicate ack");
    let ProductInboundAck::Duplicate { prior } = replay else {
        panic!("expected duplicate replay")
    };
    let ProductInboundAck::Rejected(rejection) = *prior else {
        panic!("expected rejected prior outcome")
    };
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
}

#[tokio::test]
async fn retryable_turn_submission_failure_releases_for_retry() {
    let (workflow, inbound, ledger) = build_workflow();
    inbound.force_failure(ProductWorkflowError::TurnSubmissionFailed {
        error: TurnError::Unavailable {
            reason: "turn store unavailable".into(),
        },
    });

    let envelope = sample_envelope("retryable-turn-error");
    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("unavailable turn rejection should surface retryable error");
    assert!(first.is_retryable());
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("released retryable turn rejection should dispatch again");
    assert!(second.is_retryable());
    assert_eq!(inbound.attempt_count(), 2);
}

#[tokio::test]
async fn settle_failure_does_not_return_success_ack() {
    let (workflow, inbound, ledger) = build_workflow();
    ledger.force_settle_failure(ironclaw_product_workflow::ProductWorkflowError::Transient {
        reason: "settle timeout".into(),
    });

    let envelope = sample_envelope("settle-fail");
    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("settle failure should fail request");
    assert!(err.is_retryable());
    assert_eq!(inbound.accepted_count(), 1);
    assert_eq!(ledger.settled_count(), 0);
}

#[tokio::test]
async fn unsupported_action_is_settled_as_terminal_rejection() {
    let (workflow, _inbound, ledger) = build_workflow();
    let envelope = sample_noop_envelope("unsupported-base");
    let context = TrustedInboundContext::from_verified_evidence(
        envelope.adapter_id().clone(),
        envelope.installation_id().clone(),
        Utc::now(),
        &ProtocolAuthEvidence::test_verified(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Secret".into(),
            },
            "install_alpha",
        ),
    )
    .expect("verified");
    let parsed = ParsedProductInbound::new(
        ExternalEventId::new("evt:unsupported").expect("valid"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("valid"),
        ProductInboundPayload::AuthResolution(
            ironclaw_product_adapters::AuthResolutionPayload::new(
                "auth:1",
                ironclaw_product_adapters::AuthResolutionResult::Denied,
            )
            .expect("valid"),
        ),
    )
    .expect("parsed");
    let unsupported =
        ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope");

    let err = workflow
        .accept_inbound(unsupported.clone())
        .await
        .expect_err("unsupported should error");
    assert!(!err.is_retryable());
    assert_eq!(ledger.settled_count(), 1);

    let replay = workflow
        .accept_inbound(unsupported)
        .await
        .expect("duplicate replay");
    assert!(matches!(replay, ProductInboundAck::Duplicate { .. }));
}

#[tokio::test]
async fn ledger_transient_failure_surfaces_retryable_error() {
    let (workflow, _inbound, ledger) = build_workflow();
    ledger.force_failure(ironclaw_product_workflow::ProductWorkflowError::Transient {
        reason: "db timeout".into(),
    });

    let envelope = sample_envelope("fail1");
    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("should fail");
    assert!(err.is_retryable());
}

// ---------------------------------------------------------------------------
// ProductCommandRouter dispatch arm
// ---------------------------------------------------------------------------

#[tokio::test]
async fn command_payload_dispatches_through_product_command_router() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let router = Arc::new(FakeProductCommandRouter::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_command_router(router.clone());

    let envelope = sample_command_envelope("cmd-routed", "status", "verbose");
    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("command should route through router");

    match ack {
        ProductInboundAck::CommandRouted { command } => {
            assert_eq!(command.as_str(), "status");
        }
        other => panic!("expected CommandRouted ack, got {other:?}"),
    }

    let routed = router.routed();
    assert_eq!(routed.len(), 1);
    assert_eq!(routed[0].0.as_str(), "status");
    assert_eq!(routed[0].1, "verbose");
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn unknown_command_settles_terminal_rejection() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let router = Arc::new(FakeProductCommandRouter::new());
    router.program_unknown();
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_command_router(router.clone());

    let envelope = sample_command_envelope("cmd-unknown", "mystery", "");
    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("unknown command surfaces as rejection ack, not Err");

    match ack {
        ProductInboundAck::Rejected(rejection) => {
            assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
            assert_eq!(
                rejection.disposition,
                ProductRejectionDisposition::Permanent
            );
        }
        other => panic!("expected Rejected ack, got {other:?}"),
    }

    // Ledger should record this as a settled terminal rejection, not an
    // in-flight or released action.
    assert_eq!(ledger.settled_count(), 1);
    assert_eq!(ledger.in_flight_count(), 0);
    let actions = ledger.settled_actions();
    assert_eq!(actions.len(), 1);
    let outcome = actions[0].outcome.clone().expect("settled outcome");
    assert!(matches!(outcome, ProductInboundAck::Rejected(_)));
}

#[tokio::test]
async fn command_without_configured_router_settles_command_routing_unavailable() {
    // No `.with_command_router(...)` — exercises the gate.
    let (workflow, _inbound, ledger) = build_workflow();
    let envelope = sample_command_envelope("cmd-no-router", "help", "");

    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("command without configured router should error");
    assert!(!err.is_retryable());

    // The error should be redacted at the boundary but the variant identifies
    // it as an internal adapter error projected from
    // CommandRoutingUnavailable.
    match err {
        ProductAdapterError::Internal { .. } => {}
        other => panic!("expected Internal error variant, got {other:?}"),
    }

    // CommandRoutingUnavailable maps to a terminal rejection ack, so the
    // ledger must settle (not release) the action.
    assert_eq!(ledger.settled_count(), 1);
    assert_eq!(ledger.in_flight_count(), 0);
    let actions = ledger.settled_actions();
    assert_eq!(actions.len(), 1);
    let outcome = actions[0].outcome.clone().expect("settled outcome");
    match outcome {
        ProductInboundAck::Rejected(rejection) => {
            assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
            assert_eq!(
                rejection.disposition,
                ProductRejectionDisposition::Permanent
            );
        }
        other => panic!("expected Rejected outcome, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// ProjectionSubscriptionAuthority + resolve_projection_subscription
// ---------------------------------------------------------------------------

#[tokio::test]
async fn projection_subscription_uses_authority_and_creates_no_ledger_row() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let authority = Arc::new(FakeProjectionSubscriptionAuthority::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_projection_authority(authority.clone());

    let envelope = sample_subscription_envelope("sub-1");
    let request = workflow
        .resolve_projection_subscription(envelope)
        .await
        .expect("authority should authorize the subscription");

    // The deterministic default response builds a non-empty actor and scope
    // from the envelope identifiers.
    assert!(
        !request.actor.user_id.as_str().is_empty(),
        "actor user_id must not be empty"
    );
    assert!(
        !request.scope.tenant_id.as_str().is_empty(),
        "scope tenant_id must not be empty"
    );
    assert!(
        !request.scope.thread_id.as_str().is_empty(),
        "scope thread_id must not be empty"
    );

    // AC #14 of #3280: projection-read path must NOT take a ledger lock.
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    // The authority must have been consulted exactly once.
    assert_eq!(authority.authorization_count(), 1);
}

#[tokio::test]
async fn subscription_request_via_accept_inbound_is_rejected_without_ledger_row() {
    // Even with an authority wired, routing a SubscriptionRequest through
    // accept_inbound is a usage error and must be rejected at the gate
    // before any ledger touch.
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let authority = Arc::new(FakeProjectionSubscriptionAuthority::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_projection_authority(authority.clone());

    let envelope = sample_subscription_envelope("sub-gate");
    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("subscription_request via accept_inbound is unsupported");

    // The gate maps UnsupportedActionKind to a redacted ProductAdapterError.
    match err {
        ProductAdapterError::Internal { .. } => {}
        other => panic!("expected Internal error variant, got {other:?}"),
    }

    // AC #14: the gate must reject before any ledger lock is taken.
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    // And the authority must NOT have been consulted on the gate path.
    assert_eq!(authority.authorization_count(), 0);
}

#[tokio::test]
async fn projection_subscription_without_authority_returns_unsupported() {
    // Build the workflow WITHOUT `.with_projection_authority(...)`.
    let (workflow, _inbound, ledger) = build_workflow();
    let envelope = sample_subscription_envelope("sub-no-auth");

    let err = workflow
        .resolve_projection_subscription(envelope)
        .await
        .expect_err("missing authority should error");

    match err {
        ProductAdapterError::Internal { .. } => {}
        other => panic!("expected Internal error variant, got {other:?}"),
    }

    // resolve_projection_subscription is a read-only path; failure must
    // still leave the ledger untouched.
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
}

// ---------------------------------------------------------------------------
// LinkedThreadActionService dispatch arm
// ---------------------------------------------------------------------------

#[tokio::test]
async fn linked_thread_action_dispatches_through_linked_thread_service() {
    let (workflow, _inbound, _ledger) = build_workflow();
    let linked = Arc::new(FakeLinkedThreadActionService::new());
    let workflow = workflow.with_linked_thread_service(linked.clone());

    let payload = LinkedThreadActionPayload::new(
        "open-thread",
        Some("data".to_string()),
        Some("msg:123".to_string()),
    )
    .expect("valid linked thread action payload");
    let envelope = sample_linked_thread_action_envelope("lta-1", payload);

    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("linked thread action ack");
    let ProductInboundAck::LinkedThreadActionRouted { action_id } = ack else {
        panic!("expected LinkedThreadActionRouted ack");
    };
    assert_eq!(action_id.as_str(), "open-thread");

    let actions = linked.actions();
    assert_eq!(actions.len(), 1);
    let (recorded_id, recorded_data, recorded_reply_target) = &actions[0];
    assert_eq!(recorded_id.as_str(), "open-thread");
    assert_eq!(recorded_data.as_deref(), Some("data"));
    assert_eq!(recorded_reply_target.as_deref(), Some("msg:123"));
}

#[tokio::test]
async fn linked_thread_action_passes_data_and_reply_target_through() {
    let (workflow, _inbound, _ledger) = build_workflow();
    let linked = Arc::new(FakeLinkedThreadActionService::new());
    let workflow = workflow.with_linked_thread_service(linked.clone());

    let json_data = "{\"json\":1}".to_string();
    let payload = LinkedThreadActionPayload::new("share", Some(json_data.clone()), None)
        .expect("valid linked thread action payload");
    let envelope = sample_linked_thread_action_envelope("lta-passthrough", payload);

    let _ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("linked thread action ack");

    let actions = linked.actions();
    assert_eq!(actions.len(), 1);
    let (recorded_id, recorded_data, recorded_reply_target) = &actions[0];
    assert_eq!(recorded_id.as_str(), "share");
    assert_eq!(recorded_data.as_deref(), Some(json_data.as_str()));
    assert!(recorded_reply_target.is_none());
}

#[tokio::test]
async fn linked_thread_action_without_configured_service_settles_unsupported() {
    let (workflow, _inbound, ledger) = build_workflow();
    let payload = LinkedThreadActionPayload::new("open-thread", None, None)
        .expect("valid linked thread action payload");
    let envelope = sample_linked_thread_action_envelope("lta-unconfigured", payload);

    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("unsupported should error");
    assert!(!err.is_retryable());
    assert_eq!(ledger.settled_count(), 1);

    // The terminal ack is captured by the ledger; replaying surfaces it as
    // Duplicate { prior: Rejected }. The redacted reason cannot be inspected
    // externally, but the rejection's permanent disposition and PolicyDenied
    // kind prove the linked-thread-action unsupported path settled — the
    // workflow's `UnsupportedActionKind { kind: "linked_thread_action" }`
    // error is the only producer of this terminal ack on this envelope.
    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("duplicate replay");
    let ProductInboundAck::Duplicate { prior } = replay else {
        panic!("expected duplicate replay")
    };
    let ProductInboundAck::Rejected(rejection) = *prior else {
        panic!("expected rejected prior outcome")
    };
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
    assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
}

#[tokio::test]
async fn linked_thread_action_settles_idempotency_ledger() {
    let (workflow, _inbound, ledger) = build_workflow();
    let linked = Arc::new(FakeLinkedThreadActionService::new());
    let workflow = workflow.with_linked_thread_service(linked.clone());

    let payload = LinkedThreadActionPayload::new("open-thread", Some("d".to_string()), None)
        .expect("valid linked thread action payload");
    let envelope = sample_linked_thread_action_envelope("lta-idem", payload);

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("first dispatch");
    assert!(matches!(
        first,
        ProductInboundAck::LinkedThreadActionRouted { .. }
    ));
    assert_eq!(ledger.settled_count(), 1);
    assert_eq!(linked.action_count(), 1);

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect("duplicate replay");
    let ProductInboundAck::Duplicate { prior } = second else {
        panic!("expected duplicate replay")
    };
    assert!(matches!(
        *prior,
        ProductInboundAck::LinkedThreadActionRouted { .. }
    ));
    // The downstream service must NOT be invoked a second time.
    assert_eq!(linked.action_count(), 1);
}

// ---------------------------------------------------------------------------
// SystemActionService dispatch arm
// ---------------------------------------------------------------------------

#[tokio::test]
async fn system_action_dispatches_through_system_action_service() {
    let (workflow, _inbound, _ledger) = build_workflow();
    let system = Arc::new(FakeSystemActionService::new());
    let workflow = workflow.with_system_action_service(system.clone());

    let payload = SystemActionPayload::new(
        "scheduler:cron_1",
        "heartbeat_tick",
        Some("thread:abc".to_string()),
        Some("payload data".to_string()),
    )
    .expect("valid system action payload");
    let envelope = sample_system_action_envelope("sa-1", payload);

    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("system action ack");
    assert!(matches!(ack, ProductInboundAck::NoOp));

    let actions = system.actions();
    assert_eq!(actions.len(), 1);
    let (actor, kind, scope, data) = &actions[0];
    assert_eq!(actor, "scheduler:cron_1");
    assert_eq!(kind, "heartbeat_tick");
    assert_eq!(scope.as_deref(), Some("thread:abc"));
    assert_eq!(data.as_deref(), Some("payload data"));
}

#[tokio::test]
async fn system_action_requires_accountable_actor_and_kind() {
    // Workflow-level confirmation of AC #15 of #3280: there is no
    // `is_internal` bypass. A SystemActionPayload missing an actor or kind
    // cannot even be constructed, so the unsupported-without-accountable-actor
    // shape is unrepresentable at the wire boundary before dispatch is reached.
    assert!(SystemActionPayload::new("", "", None, None).is_err());
    assert!(SystemActionPayload::new("", "heartbeat_tick", None, None).is_err());
    assert!(SystemActionPayload::new("scheduler:cron_1", "", None, None).is_err());

    // A well-formed payload (actor + kind both present) reaches dispatch.
    let (workflow, _inbound, _ledger) = build_workflow();
    let system = Arc::new(FakeSystemActionService::new());
    let workflow = workflow.with_system_action_service(system.clone());

    let payload = SystemActionPayload::new("scheduler:cron_1", "heartbeat_tick", None, None)
        .expect("valid system action payload");
    let envelope = sample_system_action_envelope("sa-required", payload);

    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("system action ack");
    assert!(matches!(ack, ProductInboundAck::NoOp));
    assert_eq!(system.action_count(), 1);
}

#[tokio::test]
async fn system_action_without_configured_service_settles_unsupported() {
    let (workflow, _inbound, ledger) = build_workflow();
    let payload = SystemActionPayload::new("scheduler:cron_1", "heartbeat_tick", None, None)
        .expect("valid system action payload");
    let envelope = sample_system_action_envelope("sa-unconfigured", payload);

    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("unsupported should error");
    assert!(!err.is_retryable());
    assert_eq!(ledger.settled_count(), 1);

    // Replay surfaces the prior Rejected ack; the redacted reason cannot be
    // inspected externally, but the rejection's permanent disposition and
    // PolicyDenied category prove the system-action unsupported path settled
    // — the workflow's `UnsupportedActionKind { kind: "system_action" }` error
    // is the only producer of this terminal ack on this envelope.
    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("duplicate replay");
    let ProductInboundAck::Duplicate { prior } = replay else {
        panic!("expected duplicate replay")
    };
    let ProductInboundAck::Rejected(rejection) = *prior else {
        panic!("expected rejected prior outcome")
    };
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
    assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
}

#[tokio::test]
async fn system_action_settles_idempotency_ledger() {
    let (workflow, _inbound, ledger) = build_workflow();
    let system = Arc::new(FakeSystemActionService::new());
    let workflow = workflow.with_system_action_service(system.clone());

    let payload = SystemActionPayload::new("scheduler:cron_1", "heartbeat_tick", None, None)
        .expect("valid system action payload");
    let envelope = sample_system_action_envelope("sa-idem", payload);

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("first dispatch");
    assert!(matches!(first, ProductInboundAck::NoOp));
    assert_eq!(ledger.settled_count(), 1);
    assert_eq!(system.action_count(), 1);

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect("duplicate replay");
    let ProductInboundAck::Duplicate { prior } = second else {
        panic!("expected duplicate replay")
    };
    assert!(matches!(*prior, ProductInboundAck::NoOp));
    // The downstream service must NOT be invoked a second time.
    assert_eq!(system.action_count(), 1);
}

// ===========================================================================
// MissionService dispatch arm contract tests
//
// Covers AC #12 of issue #3280: only explicit `MissionActionPayload`
// envelopes reach the MissionService. The v1 bug shape — see
// `src/bridge/router.rs::fire_event_missions_for_message` — pattern-matched
// inbound user-message text against active `MissionCadence::OnEvent`
// regexes and side-fired missions in parallel to the turn. Reborn MUST NOT
// reproduce that path; the regression guard is
// `ordinary_user_message_text_never_reaches_mission_service` below.
// ===========================================================================

#[tokio::test]
async fn mission_action_submitted_returns_mission_submitted_ack() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let mission = Arc::new(FakeMissionService::new());
    mission.program_submitted();

    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_mission_service(mission.clone());

    let envelope = sample_mission_action_envelope("mission-submit", "fire", Some("mission_xyz"));
    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("mission submit ack");

    match ack {
        ProductInboundAck::MissionSubmitted {
            mission_fire_ref,
            submitted_run_id,
        } => {
            assert_ne!(
                mission_fire_ref.as_uuid(),
                Uuid::nil(),
                "mission_fire_ref must be a fresh UUID"
            );
            // TurnRunId has no nil sentinel; assert it parses to a
            // non-empty debug string so we know the workflow actually
            // plumbed the value through rather than substituting a
            // default.
            assert!(
                !format!("{submitted_run_id:?}").is_empty(),
                "submitted_run_id must be populated"
            );
        }
        other => panic!("expected MissionSubmitted, got {other:?}"),
    }

    assert_eq!(mission.fire_count(), 1);
    let fires = mission.fires();
    assert_eq!(fires.len(), 1);
    assert_eq!(fires[0].mission_intent, "fire");
    assert_eq!(fires[0].mission_id_hint.as_deref(), Some("mission_xyz"));
    // The mission service must not see any user-message text — the
    // request only carries explicit `mission_intent` + optional id hint +
    // data.
    assert_eq!(fires[0].data.as_deref(), Some("{\"trigger\":\"manual\"}"));

    // MissionSubmitted is a terminal success ack and must settle.
    assert_eq!(ledger.settled_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
}

#[tokio::test]
async fn mission_action_deferred_busy_returns_mission_suppressed_busy_thread() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let mission = Arc::new(FakeMissionService::new());
    mission.program_deferred_busy();

    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_mission_service(mission.clone());

    let envelope = sample_mission_action_envelope("mission-busy", "fire", Some("mission_busy"));
    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("mission deferred busy ack");

    // DeferredBusy on the mission outcome maps to MissionSuppressed
    // { BusyThread } at the wire boundary — see workflow.rs
    // `dispatch_mission_action`.
    match ack {
        ProductInboundAck::MissionSuppressed {
            mission_fire_ref,
            reason,
        } => {
            assert_ne!(mission_fire_ref.as_uuid(), Uuid::nil());
            assert_eq!(reason, AdapterMissionFireSuppressionReason::BusyThread);
        }
        other => panic!("expected MissionSuppressed {{ BusyThread }}, got {other:?}"),
    }

    assert_eq!(mission.fire_count(), 1);
}

#[tokio::test]
async fn mission_action_suppressed_cadence_returns_mission_suppressed_cadence() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let mission = Arc::new(FakeMissionService::new());
    mission.program_suppressed(WorkflowMissionFireSuppressionReason::Cadence);

    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_mission_service(mission.clone());

    let envelope = sample_mission_action_envelope("mission-cadence", "fire", None);
    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("mission cadence ack");

    match ack {
        ProductInboundAck::MissionSuppressed {
            mission_fire_ref,
            reason,
        } => {
            assert_ne!(mission_fire_ref.as_uuid(), Uuid::nil());
            assert_eq!(reason, AdapterMissionFireSuppressionReason::Cadence);
        }
        other => panic!("expected MissionSuppressed {{ Cadence }}, got {other:?}"),
    }
}

#[tokio::test]
async fn mission_action_suppressed_cooldown_returns_mission_suppressed_cooldown() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let mission = Arc::new(FakeMissionService::new());
    mission.program_suppressed(WorkflowMissionFireSuppressionReason::Cooldown);

    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_mission_service(mission.clone());

    let envelope = sample_mission_action_envelope("mission-cooldown", "fire", None);
    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("mission cooldown ack");

    match ack {
        ProductInboundAck::MissionSuppressed { reason, .. } => {
            assert_eq!(reason, AdapterMissionFireSuppressionReason::Cooldown);
        }
        other => panic!("expected MissionSuppressed {{ Cooldown }}, got {other:?}"),
    }
}

#[tokio::test]
async fn mission_action_suppressed_deduplicated_returns_mission_suppressed_deduplicated() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let mission = Arc::new(FakeMissionService::new());
    mission.program_suppressed(WorkflowMissionFireSuppressionReason::Deduplicated);

    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_mission_service(mission.clone());

    let envelope = sample_mission_action_envelope("mission-dedup", "fire", None);
    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("mission dedup ack");

    match ack {
        ProductInboundAck::MissionSuppressed { reason, .. } => {
            assert_eq!(reason, AdapterMissionFireSuppressionReason::Deduplicated);
        }
        other => panic!("expected MissionSuppressed {{ Deduplicated }}, got {other:?}"),
    }
}

#[tokio::test]
async fn mission_action_rejected_returns_redacted_permanent_rejection() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let mission = Arc::new(FakeMissionService::new());
    mission.program_rejected(MissionFireRejectionReason::UnknownMission);

    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_mission_service(mission.clone());

    let envelope =
        sample_mission_action_envelope("mission-rejected", "fire", Some("mission_unknown"));
    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("mission rejection ack");

    match ack {
        ProductInboundAck::Rejected(rejection) => {
            assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
            assert_eq!(
                rejection.disposition(),
                ProductRejectionDisposition::Permanent
            );
            // The rejection reason is wrapped in `RedactedString`, whose
            // `Display`/`Debug` impls both emit `<redacted>` — adapters
            // cannot inspect the underlying string from outside the
            // `ironclaw_product_adapters` crate. This pins the redaction
            // contract: even though the workflow currently builds the
            // reason with `format!("...{reason:?}")` (which would
            // otherwise leak the `MissionFireRejectionReason` variant
            // name and let an adapter probe mission existence), the
            // `RedactedString` seal prevents that leak from crossing the
            // boundary.
            //
            // TODO(#3280 follow-up): once the workflow uses a typed
            // wire-side rejection enum, also stop embedding the
            // unredacted variant name in the inner string — defence in
            // depth in case `RedactedString` ever gains an `expose` API
            // public to adapters.
            assert_eq!(format!("{}", rejection.reason), "<redacted>");
            assert_eq!(format!("{:?}", rejection.reason), "<redacted>");
        }
        other => panic!("expected Rejected, got {other:?}"),
    }

    assert_eq!(mission.fire_count(), 1);
    // Rejected is a terminal outcome and must settle.
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn mission_action_without_configured_service_settles_unsupported() {
    // No `.with_mission_service(...)` wired — the workflow must surface a
    // terminal rejection naming the unsupported action kind.
    let (workflow, _inbound, ledger) = build_workflow();
    let envelope = sample_mission_action_envelope("mission-unsupported", "fire", Some("mission_x"));

    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("missing mission service must surface unsupported error");
    assert!(!err.is_retryable());

    // The settled outcome is a permanent rejection. Replay returns
    // Duplicate { prior: Rejected }.
    assert_eq!(ledger.settled_count(), 1);
    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("duplicate replay");
    let ProductInboundAck::Duplicate { prior } = replay else {
        panic!("expected duplicate replay, got {replay:?}");
    };
    let ProductInboundAck::Rejected(rejection) = *prior else {
        panic!("expected rejected prior outcome");
    };
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
    assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
    // As with `mission_action_rejected_returns_redacted_permanent_rejection`,
    // the rejection reason crosses the boundary inside `RedactedString` and
    // adapters cannot inspect "mission_action" directly. The workflow's
    // `UnsupportedActionKind { kind: "mission_action" }` error is the only
    // producer of this terminal ack on this envelope, so the (kind,
    // disposition) pair and the dispatch path being the unsupported branch
    // together pin the contract.
    assert_eq!(format!("{}", rejection.reason), "<redacted>");
}

#[tokio::test]
async fn mission_action_settles_idempotency_ledger_and_duplicate_replays() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let mission = Arc::new(FakeMissionService::new());
    mission.program_submitted();

    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_mission_service(mission.clone());

    let envelope = sample_mission_action_envelope("mission-dup", "fire", Some("mission_idem"));

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("first mission submit");
    assert!(matches!(first, ProductInboundAck::MissionSubmitted { .. }));
    assert_eq!(mission.fire_count(), 1);
    assert_eq!(ledger.settled_count(), 1);

    // Re-program the fake so a second mission service call would now
    // produce a different outcome. If the workflow mistakenly dispatched
    // again instead of replaying the prior settled outcome, the
    // assertions below would catch it.
    mission.program_rejected(MissionFireRejectionReason::UnknownMission);

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect("duplicate replay");

    let ProductInboundAck::Duplicate { prior } = second else {
        panic!("expected duplicate replay, got {second:?}");
    };
    assert!(
        matches!(*prior, ProductInboundAck::MissionSubmitted { .. }),
        "prior outcome must be MissionSubmitted, got {prior:?}"
    );
    // Critically: the second accept must NOT have invoked the mission
    // service again. The ledger replays the prior settled outcome.
    assert_eq!(
        mission.fire_count(),
        1,
        "duplicate envelope must not re-invoke mission service"
    );
}

// ---------------------------------------------------------------------------
// CRITICAL: no-auto-attach guard
//
// This test closes the v1 bug shape from
// `src/bridge/router.rs::fire_event_missions_for_message`, which scanned
// inbound user-message text against active `MissionCadence::OnEvent`
// regexes and side-fired missions in parallel to the turn. AC #12 of
// issue #3280 forbids that path in Reborn: missions only fire via an
// explicit `MissionActionPayload`. A regression that re-introduces a
// text-pattern auto-attach must fail this test.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ordinary_user_message_text_never_reaches_mission_service() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let mission = Arc::new(FakeMissionService::new());
    // Program the mission service so that if anything ever reached it,
    // the fire would succeed (and the assertions below would catch the
    // call).
    mission.program_submitted();

    // Wire every other service so the workflow is fully configured and
    // can't blame an unwired dispatch arm for not calling MissionService.
    let approval = Arc::new(FakeApprovalInteractionService::new());
    let auth = Arc::new(FakeAuthInteractionService::new());
    let linked_thread = Arc::new(FakeLinkedThreadActionService::new());
    let system_action = Arc::new(FakeSystemActionService::new());
    let command_router = Arc::new(FakeProductCommandRouter::new());
    let authority = Arc::new(FakeProjectionSubscriptionAuthority::new());

    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_mission_service(mission.clone())
        .with_approval_service(approval)
        .with_auth_service(auth)
        .with_linked_thread_service(linked_thread)
        .with_system_action_service(system_action)
        .with_command_router(command_router)
        .with_projection_authority(authority);

    // Each of these envelopes carries text that v1's
    // `fire_event_missions_for_message` would have pattern-matched
    // against a `MissionCadence::OnEvent { event_pattern }` regex. The
    // Reborn workflow must route none of them to the mission service.
    let texts = [
        ("text-summarize", "please summarize today"),
        ("text-mission-word", "start the daily report mission"),
        ("text-trigger-fire", "trigger:fire mission_alpha"),
        ("text-bot-fire", "@bot fire mission"),
    ];

    for (suffix, text) in texts {
        let envelope = sample_user_message_envelope_with_text(suffix, text);
        let ack = workflow
            .accept_inbound(envelope)
            .await
            .expect("user message accepted");
        // Sanity check: user messages route through the inbound turn
        // service and produce an Accepted ack.
        assert!(
            matches!(ack, ProductInboundAck::Accepted { .. }),
            "user-message envelope must produce Accepted ack, got {ack:?} (text: {text:?})"
        );

        // The load-bearing assertion: the MissionService MUST NOT have
        // been called as a side effect of pattern-matching the text.
        assert_eq!(
            mission.fire_count(),
            0,
            "MissionService.fire_count must remain 0 — v1 \
             fire_event_missions_for_message auto-attach has been \
             reintroduced for text: {text:?}"
        );
        assert!(
            mission.fires().is_empty(),
            "MissionService.fires must remain empty — no \
             MissionFireRequest may be recorded from text: {text:?}"
        );
    }

    // Belt-and-suspenders: the inbound turn service is the ONLY service
    // that should have seen these envelopes.
    assert_eq!(inbound.accepted_count(), texts.len());
    assert_eq!(mission.fire_count(), 0);
}

// ---------------------------------------------------------------------------
// ApprovalInteractionService dispatch arm
// ---------------------------------------------------------------------------

#[tokio::test]
async fn approval_resolution_dispatches_through_approval_interaction_service() {
    let (_, inbound, ledger) = build_workflow();
    let approval = Arc::new(FakeApprovalInteractionService::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_approval_service(approval.clone());

    let envelope = sample_approval_envelope("approval-handled", "gate:abc123");
    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    let ProductInboundAck::GateHandled { gate_ref } = ack else {
        panic!("expected GateHandled ack, got {ack:?}");
    };
    assert_eq!(gate_ref.as_str(), "gate:abc123");

    let resolutions = approval.resolutions();
    assert_eq!(resolutions.len(), 1);
    let (recorded_gate, recorded_decision) = &resolutions[0];
    assert_eq!(recorded_gate.as_str(), "gate:abc123");
    assert_eq!(recorded_decision, &ApprovalDecision::ApproveOnce);
}

#[tokio::test]
async fn stale_approval_settles_terminal_rejection() {
    let (_, inbound, ledger) = build_workflow();
    let approval = Arc::new(FakeApprovalInteractionService::new());
    approval.program_stale();
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_approval_service(approval.clone());

    let envelope = sample_approval_envelope("approval-stale", "gate:stale-ref");
    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    // Adapter must not learn whether the gate existed; surface as a redacted
    // permanent PolicyDenied. The reason text crosses the redaction boundary
    // (RedactedString::expose is pub(crate)), so we assert kind + disposition
    // — those are the wire-stable categories adapters branch on.
    let ProductInboundAck::Rejected(rejection) = ack else {
        panic!("expected Rejected ack, got {ack:?}");
    };
    assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
    // Sanity: the redacted reason does not leak the raw inner text.
    assert!(!rejection.reason.to_string().contains("gate:stale-ref"));
}

#[tokio::test]
async fn approval_resolution_does_not_call_turn_coordinator_resume() {
    // Boundary regression for #3280 AC #11: the workflow must NOT bypass the
    // ApprovalInteractionService to resume the parked loop directly. The
    // workflow has no TurnCoordinator handle at all in this construction —
    // the only direct dispatch surface is
    // `InboundTurnService::accept_user_message`, which is also the only place
    // a turn could be (re-)submitted from this layer. Asserting
    // `accept_user_message` was never called while the approval service was
    // called exactly once is the strongest available proof at the workflow
    // boundary.
    //
    // Gap: the workflow crate does not own TurnCoordinator and
    // FakeInboundTurnService does not expose a separate `resume_count()` —
    // gate-resume happens inside the host-side service that the production
    // `ApprovalInteractionService` wraps. That deeper contract is owned by
    // `ironclaw_approvals` (issue #3094).
    let (_, inbound, ledger) = build_workflow();
    let approval = Arc::new(FakeApprovalInteractionService::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_approval_service(approval.clone());

    let envelope = sample_approval_envelope("approval-no-resume", "gate:no-resume");
    let _ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert_eq!(
        approval.resolution_count(),
        1,
        "approval dispatch must reach the approval service exactly once"
    );
    assert_eq!(
        inbound.attempt_count(),
        0,
        "approval dispatch must NOT route through the inbound turn service"
    );
    assert_eq!(
        inbound.accepted_count(),
        0,
        "approval dispatch must NOT submit/resume a user-message turn"
    );
}

#[tokio::test]
async fn approval_without_configured_service_settles_unsupported() {
    let (workflow, _inbound, ledger) = build_workflow();

    let envelope = sample_approval_envelope("approval-unsupported", "gate:no-service");
    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("missing approval service should reject");
    assert!(!err.is_retryable());
    assert_eq!(ledger.settled_count(), 1);

    // Replay the duplicate to inspect the terminal rejection that was settled.
    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("duplicate replay");
    let ProductInboundAck::Duplicate { prior } = replay else {
        panic!("expected duplicate replay, got {replay:?}");
    };
    let ProductInboundAck::Rejected(rejection) = *prior else {
        panic!("expected rejected prior outcome");
    };
    assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
    // The `unsupported action kind: approval_resolution` reason crosses the
    // RedactedString boundary (expose() is pub(crate)). The wire-stable
    // ProductWorkflowError::UnsupportedActionKind path is exercised by the
    // settle (terminal) versus release (retryable) outcome above.
}

// ---------------------------------------------------------------------------
// AuthInteractionService dispatch arm
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resolution_dispatches_through_auth_interaction_service() {
    let (_, inbound, ledger) = build_workflow();
    let auth = Arc::new(FakeAuthInteractionService::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_auth_service(auth.clone());

    let envelope = sample_auth_envelope("auth-handled", "auth_req_xyz");
    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    // Auth refs project into the same wire-stable LoopGateRef container as
    // approval refs — both surface through GateHandled.
    let ProductInboundAck::GateHandled { gate_ref } = ack else {
        panic!("expected GateHandled ack, got {ack:?}");
    };
    assert_eq!(gate_ref.as_str(), "auth_req_xyz");

    let resolutions = auth.resolutions();
    assert_eq!(resolutions.len(), 1);
    let (recorded_ref, recorded_result) = &resolutions[0];
    assert_eq!(recorded_ref.as_str(), "auth_req_xyz");
    assert!(matches!(
        recorded_result,
        AuthResolutionResult::CredentialProvided { credential_ref }
            if credential_ref == "cred_abc"
    ));
}

#[tokio::test]
async fn stale_auth_settles_terminal_rejection() {
    let (_, inbound, ledger) = build_workflow();
    let auth = Arc::new(FakeAuthInteractionService::new());
    auth.program_stale();
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_auth_service(auth.clone());

    let envelope = sample_auth_envelope("auth-stale", "auth_req_stale");
    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    let ProductInboundAck::Rejected(rejection) = ack else {
        panic!("expected Rejected ack, got {ack:?}");
    };
    assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
    // Sanity: the redacted reason does not leak the raw inner text.
    assert!(!rejection.reason.to_string().contains("auth_req_stale"));
}

#[tokio::test]
async fn auth_resolution_does_not_call_turn_coordinator_resume() {
    // Same boundary regression as
    // `approval_resolution_does_not_call_turn_coordinator_resume` but for the
    // auth arm: the workflow must not bypass the AuthInteractionService to
    // resume the parked loop directly. See that test's doc-comment for the
    // gap on the missing TurnCoordinator handle.
    let (_, inbound, ledger) = build_workflow();
    let auth = Arc::new(FakeAuthInteractionService::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_auth_service(auth.clone());

    let envelope = sample_auth_envelope("auth-no-resume", "auth_req_no_resume");
    let _ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert_eq!(
        auth.resolution_count(),
        1,
        "auth dispatch must reach the auth service exactly once"
    );
    assert_eq!(
        inbound.attempt_count(),
        0,
        "auth dispatch must NOT route through the inbound turn service"
    );
    assert_eq!(
        inbound.accepted_count(),
        0,
        "auth dispatch must NOT submit/resume a user-message turn"
    );
}

#[tokio::test]
async fn auth_without_configured_service_settles_unsupported() {
    let (workflow, _inbound, ledger) = build_workflow();

    let envelope = sample_auth_envelope("auth-unsupported", "auth_req_no_service");
    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("missing auth service should reject");
    assert!(!err.is_retryable());
    assert_eq!(ledger.settled_count(), 1);

    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("duplicate replay");
    let ProductInboundAck::Duplicate { prior } = replay else {
        panic!("expected duplicate replay, got {replay:?}");
    };
    let ProductInboundAck::Rejected(rejection) = *prior else {
        panic!("expected rejected prior outcome");
    };
    assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
}

// ---------------------------------------------------------------------------
// Both arms: ledger lifecycle and idempotency replay
// ---------------------------------------------------------------------------

#[tokio::test]
async fn approval_and_auth_both_settle_idempotency_ledger() {
    // Approval arm — fresh workflow + ledger.
    {
        let (_, inbound, ledger) = build_workflow();
        let approval = Arc::new(FakeApprovalInteractionService::new());
        let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
            .with_approval_service(approval.clone());

        let envelope = sample_approval_envelope("approval-ledger", "gate:ledger");
        let _ack = workflow.accept_inbound(envelope).await.expect("accept");
        assert_eq!(
            ledger.settled_count(),
            1,
            "approval dispatch must settle the idempotency ledger"
        );
        assert_eq!(ledger.in_flight_count(), 0);
    }
    // Auth arm — fresh workflow + ledger.
    {
        let (_, inbound, ledger) = build_workflow();
        let auth = Arc::new(FakeAuthInteractionService::new());
        let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
            .with_auth_service(auth.clone());

        let envelope = sample_auth_envelope("auth-ledger", "auth_req_ledger");
        let _ack = workflow.accept_inbound(envelope).await.expect("accept");
        assert_eq!(
            ledger.settled_count(),
            1,
            "auth dispatch must settle the idempotency ledger"
        );
        assert_eq!(ledger.in_flight_count(), 0);
    }
}

#[tokio::test]
async fn duplicate_approval_envelope_replays_prior_handled_ack() {
    let (_, inbound, ledger) = build_workflow();
    let approval = Arc::new(FakeApprovalInteractionService::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_approval_service(approval.clone());

    let envelope = sample_approval_envelope("approval-dup", "gate:dup-1");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("first approval");
    let ProductInboundAck::GateHandled {
        gate_ref: first_ref,
    } = first
    else {
        panic!("expected first GateHandled ack");
    };
    assert_eq!(first_ref.as_str(), "gate:dup-1");

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect("duplicate replay");
    let ProductInboundAck::Duplicate { prior } = second else {
        panic!("expected duplicate replay, got {second:?}");
    };
    let ProductInboundAck::GateHandled {
        gate_ref: prior_ref,
    } = *prior
    else {
        panic!("expected GateHandled in prior replay");
    };
    assert_eq!(prior_ref.as_str(), "gate:dup-1");
    assert_eq!(
        approval.resolution_count(),
        1,
        "duplicate envelope must NOT re-invoke the approval service"
    );
}
