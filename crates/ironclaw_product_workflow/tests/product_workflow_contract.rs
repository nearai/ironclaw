//! Contract tests for the product workflow facade.

use std::sync::Arc;

use chrono::Utc;
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, ParsedProductInbound, ProductAdapterId, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductTriggerReason, ProductWorkflow,
    ProtocolAuthEvidence, TrustedInboundContext, UserMessagePayload,
};
use ironclaw_product_workflow::{
    ActionDispatchKind, DefaultProductWorkflow, FakeIdempotencyLedger, FakeInboundTurnService,
};

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
