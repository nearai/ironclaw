//! Contract tests for the InboundTurnService.

use std::sync::Arc;

use chrono::Utc;
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, ParsedProductInbound, ProductAdapterId, ProductInboundEnvelope,
    ProductInboundPayload, ProductTriggerReason, ProtocolAuthEvidence, TrustedInboundContext,
    UserMessagePayload,
};
use ironclaw_product_workflow::{
    DefaultInboundTurnService, FakeConversationBindingService, InboundTurnOutcome,
    InboundTurnService, ProductWorkflowError,
};
use ironclaw_threads::{
    InMemorySessionThreadService, MessageStatus, SessionThreadService, ThreadHistoryRequest,
    ThreadScope,
};
use ironclaw_turns::{DefaultTurnCoordinator, InMemoryTurnStateStore};

fn sample_user_message_envelope(event_suffix: &str) -> ProductInboundEnvelope {
    sample_user_message_envelope_with_text(event_suffix, "hello world")
}

fn sample_user_message_envelope_with_text(
    event_suffix: &str,
    text: &str,
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
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(text, vec![], ProductTriggerReason::DirectChat).expect("valid"),
        ),
    )
    .expect("parsed");

    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
}

#[tokio::test]
async fn user_message_resolves_binding_persists_message_and_submits_turn() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator);

    let envelope = sample_user_message_envelope("turn1");
    let outcome: InboundTurnOutcome = service
        .accept_user_message(&envelope)
        .await
        .expect("submit");

    let binding = match &outcome {
        InboundTurnOutcome::Submitted { binding, .. } => binding,
        _ => panic!("expected Submitted, got {outcome:?}"),
    };

    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].content.as_deref(), Some("hello world"));
    assert_eq!(history.messages[0].status, MessageStatus::Submitted);
    assert!(history.messages[0].turn_run_id.is_some());
}

#[tokio::test]
async fn busy_thread_persists_second_message_as_deferred() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    let service =
        DefaultInboundTurnService::new(binding_service, thread_service.clone(), coordinator);

    let first = sample_user_message_envelope("busy1");
    service.accept_user_message(&first).await.expect("first");
    let second = sample_user_message_envelope_with_text("busy2", "second");
    let outcome = service
        .accept_user_message(&second)
        .await
        .expect("second deferred");
    assert!(matches!(outcome, InboundTurnOutcome::DeferredBusy { .. }));

    let binding = match outcome {
        InboundTurnOutcome::DeferredBusy { binding, .. } => binding,
        _ => unreachable!(),
    };
    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: binding.tenant_id.clone(),
                agent_id: binding.agent_id.clone().expect("agent id"),
                project_id: binding.project_id.clone(),
                owner_user_id: Some(binding.user_id.clone()),
                mission_id: None,
            },
            thread_id: binding.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 2);
    assert_eq!(history.messages[1].content.as_deref(), Some("second"));
    assert_eq!(history.messages[1].status, MessageStatus::DeferredBusy);
}

#[tokio::test]
async fn max_valid_external_ids_do_not_overflow_turn_refs() {
    let binding_service = FakeConversationBindingService::new();
    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    let service = DefaultInboundTurnService::new(binding_service, thread_service, coordinator);

    let long_event_id = "e".repeat(250);
    let envelope = sample_user_message_envelope(&long_event_id);
    service
        .accept_user_message(&envelope)
        .await
        .expect("long ids accepted");
}

#[tokio::test]
async fn binding_failure_surfaces_workflow_error() {
    let binding_service = FakeConversationBindingService::new();
    binding_service.force_failure(ProductWorkflowError::BindingResolutionFailed {
        reason: "no tenant found".into(),
    });

    let thread_service = InMemorySessionThreadService::default();
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store);
    let service = DefaultInboundTurnService::new(binding_service, thread_service, coordinator);

    let envelope = sample_user_message_envelope("fail1");
    let err = service
        .accept_user_message(&envelope)
        .await
        .expect_err("should fail");

    assert!(matches!(
        err,
        ProductWorkflowError::BindingResolutionFailed { .. }
    ));
}
