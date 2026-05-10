//! Contract tests for the product workflow facade.

use std::sync::Arc;

use chrono::{Duration, Utc};
use ironclaw_product_adapters::{
    AdapterInstallationId, ApprovalDecision, ApprovalResolutionPayload, AuthRequirement,
    AuthResolutionPayload, AuthResolutionResult, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, InboundCommandPayload, LinkedThreadActionPayload, ParsedProductInbound,
    ProductAdapterId, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    ProductTriggerReason, ProductWorkflow, ProtocolAuthEvidence, TrustedInboundContext,
    UserMessagePayload,
};
use ironclaw_product_workflow::{
    ActionDispatchKind, ActionFingerprintKey, AuthRequestRef, DefaultProductWorkflow,
    FakeIdempotencyLedger, FakeInboundTurnService, IdempotencyDecision, IdempotencyLedger,
    LinkedThreadActionId, ProductCommandName, ProductWorkflowError, SourceBindingKey,
};
use ironclaw_turns::LoopGateRef;

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
async fn transient_dispatch_failure_keeps_fingerprint_reserved_until_recovery() {
    let (workflow, inbound, ledger) = build_workflow();
    inbound.force_failure(ProductWorkflowError::Transient {
        reason: "turn coordinator unavailable".into(),
    });

    let envelope = sample_envelope("transient-reserved");
    let first_err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("first attempt should be retryable");
    assert!(first_err.is_retryable());
    assert_eq!(inbound.attempt_count(), 1);

    let second_err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("same in-flight fingerprint should be reserved");
    assert!(second_err.is_retryable());
    assert_eq!(
        inbound.attempt_count(),
        1,
        "reserved fingerprint must not dispatch the same action twice before recovery cleanup"
    );
    assert_eq!(ledger.settled_count(), 0);
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
