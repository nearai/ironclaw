//! Contract tests for the product workflow facade.

use std::sync::Arc;

use chrono::{Duration, Utc};
use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{
    AdapterInstallationId, ApprovalDecision, ApprovalResolutionPayload, AuthRequirement,
    AuthResolutionPayload, AuthResolutionResult, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, InboundCommandPayload, LinkedThreadActionPayload, ParsedProductInbound,
    ProductAdapterError, ProductAdapterId, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload, ProductRejection, ProductRejectionDisposition, ProductRejectionKind,
    ProductTriggerReason, ProductWorkflow, ProductWorkflowRejectionKind, ProtocolAuthEvidence,
    TrustedInboundContext, UserMessagePayload,
};
use ironclaw_product_workflow::{
    ActionDispatchKind, ActionFingerprintKey, AuthRequestRef, DefaultProductWorkflow,
    FakeBeforeInboundPolicy, FakeIdempotencyLedger, FakeInboundTurnService, IdempotencyDecision,
    IdempotencyLedger, InboundTurnOutcome, LinkedThreadActionId, ProductCommandName,
    ProductWorkflowError, ResolvedBinding, SourceBindingKey,
};
use ironclaw_turns::{AcceptedMessageRef, LoopGateRef, TurnError, TurnRunId};

fn sample_envelope(event_suffix: &str) -> ProductInboundEnvelope {
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
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("valid"),
        ),
    )
    .expect("parsed");

    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
}

fn sample_noop_envelope(event_suffix: &str) -> ProductInboundEnvelope {
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
        ProductInboundPayload::NoOp,
    )
    .expect("parsed");

    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
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

fn build_workflow_with_policy() -> (
    DefaultProductWorkflow,
    Arc<FakeInboundTurnService>,
    Arc<FakeIdempotencyLedger>,
    Arc<FakeBeforeInboundPolicy>,
) {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let policy = Arc::new(FakeBeforeInboundPolicy::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone())
        .with_before_inbound_policy(policy.clone());
    (workflow, inbound, ledger, policy)
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
async fn before_inbound_policy_rewrite_reaches_inbound_turn_service() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.rewrite_user_message(
        UserMessagePayload::new(
            "rewritten by policy",
            vec![],
            ProductTriggerReason::DirectChat,
        )
        .expect("valid rewrite"),
    );
    let envelope = sample_envelope("policy-rewrite");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    assert_eq!(policy.request_count(), 1);
    assert_eq!(policy.requests()[0].user_message.text, "hello");
    let accepted = inbound.accepted_envelopes();
    assert_eq!(accepted.len(), 1);
    let ProductInboundPayload::UserMessage(payload) = accepted[0].payload() else {
        panic!("expected rewritten user message payload")
    };
    assert_eq!(payload.text, "rewritten by policy");
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn before_inbound_policy_rejection_skips_transcript_and_turn_path() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.reject(ProductRejection::permanent(
        ProductRejectionKind::PolicyDenied,
        "blocked by before-inbound policy",
    ));
    let envelope = sample_envelope("policy-reject");

    let ack = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("policy rejection ack");

    let ProductInboundAck::Rejected(rejection) = ack else {
        panic!("expected rejected ack")
    };
    assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
    let actions = ledger.settled_actions();
    assert_eq!(
        actions[0].dispatch_kind,
        Some(ActionDispatchKind::Rejected {
            kind: ProductRejectionKind::PolicyDenied
        })
    );

    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("policy rejection replay");
    assert!(matches!(replay, ProductInboundAck::Duplicate { .. }));
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
}

#[tokio::test]
async fn before_inbound_policy_transient_failure_releases_fingerprint() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.force_failure(ProductWorkflowError::Transient {
        reason: "policy store unavailable".into(),
    });
    let envelope = sample_envelope("policy-transient");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("policy failure should be retryable");
    assert!(first.is_retryable());
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("released fingerprint should retry policy");
    assert!(second.is_retryable());
    assert_eq!(policy.request_count(), 2);
    assert_eq!(inbound.accepted_count(), 0);
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
