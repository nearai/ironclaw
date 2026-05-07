use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use ironclaw_conversations::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AdapterInstallationId, AdapterKind,
    ConversationBindingService, ExternalActorRef, ExternalConversationRef, ExternalEventId,
    InMemoryConversationServices, InboundMessageContentRef, InboundTurnError, InboundTurnRequest,
    InboundTurnService, LinkConversationRequest, MessageIdempotencyStatus, SessionThreadService,
    ThreadAccessDecision,
};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, GetRunStateRequest, ResumeTurnRequest,
    ResumeTurnResponse, RunProfileId, RunProfileVersion, SubmitTurnRequest, SubmitTurnResponse,
    TurnActor, TurnCoordinator, TurnError, TurnRunId, TurnRunState, TurnStatus,
};

#[tokio::test]
async fn paired_actor_without_binding_creates_thread_binding_message_and_submits_turn() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());

    let response = inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-1", Some("thread-1")),
            "telegram-event-1",
        ))
        .await
        .unwrap();

    assert_eq!(response.resolution.tenant_id, tenant());
    assert_eq!(response.resolution.actor.user_id, user("alice"));
    assert_eq!(
        response.accepted_message.idempotency,
        MessageIdempotencyStatus::Inserted
    );
    assert_eq!(coordinator.submissions().len(), 1);
    let submitted = &coordinator.submissions()[0];
    assert_eq!(submitted.scope, response.resolution.turn_scope);
    assert_eq!(submitted.actor, TurnActor::new(user("alice")));
    assert_eq!(
        submitted.accepted_message_ref,
        response.accepted_message.message_ref
    );
    assert_eq!(
        submitted.source_binding_ref,
        response.accepted_message.source_binding_ref
    );
    assert_eq!(
        submitted.reply_target_binding_ref,
        response.accepted_message.reply_target_binding_ref
    );
}

#[tokio::test]
async fn unpaired_external_actor_returns_binding_required_before_message_or_turn_submission() {
    let services = InMemoryConversationServices::default();
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());

    let err = inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("unknown-user"),
            external_conversation("chat-1", None),
            "telegram-event-unpaired",
        ))
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::BindingRequired { .. }));
    assert!(coordinator.submissions().is_empty());
    assert!(services.accepted_messages().await.is_empty());
}

#[tokio::test]
async fn pairing_is_scoped_by_tenant_and_adapter_installation() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;

    let cross_tenant = services
        .resolve_or_create_binding(resolve_request_with(
            TenantId::new("tenant-b").unwrap(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            external_conversation("chat-1", None),
            "tenant-b-event-1",
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        cross_tenant,
        InboundTurnError::BindingRequired { .. }
    ));

    let cross_installation = services
        .resolve_or_create_binding(resolve_request_with(
            tenant(),
            telegram(),
            AdapterInstallationId::new("other-installation").unwrap(),
            external_actor("telegram-user-1"),
            external_conversation("chat-1", None),
            "other-install-event-1",
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        cross_installation,
        InboundTurnError::BindingRequired { .. }
    ));
}

#[tokio::test]
async fn external_ref_keying_cannot_be_collided_with_delimiter_characters() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            ExternalActorRef::new("user;id=x", "y").unwrap(),
            user("alice"),
        )
        .await;

    let colliding_actor = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            ExternalActorRef::new("user", "x;id=y").unwrap(),
            external_conversation("chat-1", None),
            "actor-collision-event",
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        colliding_actor,
        InboundTurnError::BindingRequired { .. }
    ));

    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let first = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("telegram-user-1"),
            ExternalConversationRef::new(None, "a;thread=b", Some("c"), None).unwrap(),
            "conversation-collision-a",
        ))
        .await
        .unwrap();
    let second = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("telegram-user-1"),
            ExternalConversationRef::new(None, "a", Some("b;thread=c"), None).unwrap(),
            "conversation-collision-b",
        ))
        .await
        .unwrap();
    assert_ne!(first.turn_scope.thread_id, second.turn_scope.thread_id);
}

#[tokio::test]
async fn per_message_external_ids_do_not_fork_conversation_bindings() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());

    let first = inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            ExternalConversationRef::new(None, "chat-1", Some("topic-a"), Some("message-1"))
                .unwrap(),
            "telegram-event-message-1",
        ))
        .await
        .unwrap();
    let second = inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            ExternalConversationRef::new(None, "chat-1", Some("topic-a"), Some("message-2"))
                .unwrap(),
            "telegram-event-message-2",
        ))
        .await
        .unwrap();

    assert_eq!(second.resolution.turn_scope, first.resolution.turn_scope);
    assert_eq!(
        second.resolution.source_binding_ref,
        first.resolution.source_binding_ref
    );
    assert_eq!(
        second.resolution.reply_target_binding_ref,
        first.resolution.reply_target_binding_ref
    );
    assert_ne!(
        second.accepted_message.reply_target_binding_ref,
        first.accepted_message.reply_target_binding_ref,
        "accepted inbound messages need message-scoped reply targets even when binding identity is stable"
    );
    let first_target = services
        .validate_reply_target(
            &tenant(),
            &user("alice"),
            &first.resolution.turn_scope.thread_id,
            &first.accepted_message.reply_target_binding_ref,
        )
        .await
        .unwrap();
    let second_target = services
        .validate_reply_target(
            &tenant(),
            &user("alice"),
            &second.resolution.turn_scope.thread_id,
            &second.accepted_message.reply_target_binding_ref,
        )
        .await
        .unwrap();
    assert_eq!(
        first_target.external_conversation_ref.message_id(),
        Some("message-1")
    );
    assert_eq!(
        second_target.external_conversation_ref.message_id(),
        Some("message-2")
    );
    assert_eq!(coordinator.submissions().len(), 2);
}

#[tokio::test]
async fn explicit_link_reuses_binding_when_only_external_message_id_changes() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    let web_resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session", None),
            "web-event-1",
        ))
        .await
        .unwrap();
    let first = services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: tenant(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: ExternalConversationRef::new(
                Some("workspace-a"),
                "chat-1",
                Some("topic-a"),
                Some("message-1"),
            )
            .unwrap(),
            target_thread_id: web_resolution.turn_scope.thread_id.clone(),
            target_agent_id: web_resolution.turn_scope.agent_id.clone(),
            target_project_id: web_resolution.turn_scope.project_id.clone(),
        })
        .await
        .unwrap();
    let replay = services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: tenant(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: ExternalConversationRef::new(
                Some("workspace-a"),
                "chat-1",
                Some("topic-a"),
                Some("message-2"),
            )
            .unwrap(),
            target_thread_id: web_resolution.turn_scope.thread_id,
            target_agent_id: web_resolution.turn_scope.agent_id,
            target_project_id: web_resolution.turn_scope.project_id,
        })
        .await
        .unwrap();

    assert_eq!(replay.thread_id, first.thread_id);
    assert_eq!(replay.source_binding_ref, first.source_binding_ref);
    assert_eq!(
        replay.reply_target_binding_ref,
        first.reply_target_binding_ref
    );
}

#[tokio::test]
async fn validated_reply_target_preserves_adapter_installation_and_external_route() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            AdapterInstallationId::new("workspace-a-installation").unwrap(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    let conversation_ref = ExternalConversationRef::new(
        Some("workspace-a"),
        "channel-1",
        Some("thread-1"),
        Some("message-1"),
    )
    .unwrap();
    let resolution = services
        .resolve_or_create_binding(resolve_request_with(
            tenant(),
            telegram(),
            AdapterInstallationId::new("workspace-a-installation").unwrap(),
            external_actor("alice-telegram"),
            conversation_ref,
            "telegram-event-1",
        ))
        .await
        .unwrap();

    let target = services
        .validate_reply_target(
            &tenant(),
            &user("alice"),
            &resolution.turn_scope.thread_id,
            &resolution.reply_target_binding_ref,
        )
        .await
        .unwrap();

    assert_eq!(target.adapter_kind, telegram());
    assert_eq!(
        target.adapter_installation_id,
        AdapterInstallationId::new("workspace-a-installation").unwrap()
    );
    assert_eq!(
        target.external_conversation_ref.space_id(),
        Some("workspace-a")
    );
    assert_eq!(
        target.external_conversation_ref.conversation_id(),
        "channel-1"
    );
    assert_eq!(
        target.external_conversation_ref.thread_id(),
        Some("thread-1")
    );
    assert_eq!(
        target.external_conversation_ref.message_id(),
        Some("message-1")
    );
}

#[tokio::test]
async fn explicit_link_cannot_cross_tenant_by_reusing_a_thread_id() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            TenantId::new("tenant-b").unwrap(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    let tenant_a = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session", None),
            "web-event-1",
        ))
        .await
        .unwrap();

    let err = services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: TenantId::new("tenant-b").unwrap(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: external_conversation("chat-tenant-b", None),
            target_thread_id: tenant_a.turn_scope.thread_id,
            target_agent_id: tenant_a.turn_scope.agent_id,
            target_project_id: tenant_a.turn_scope.project_id,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::ThreadNotFound { .. }));
}

#[tokio::test]
async fn webui_and_telegram_default_to_separate_threads_for_same_user() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;

    let web_resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session", None),
            "web-event-1",
        ))
        .await
        .unwrap();
    let telegram_resolution = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("alice-telegram"),
            external_conversation("chat-1", None),
            "telegram-event-1",
        ))
        .await
        .unwrap();

    assert_eq!(web_resolution.actor.user_id, user("alice"));
    assert_eq!(telegram_resolution.actor.user_id, user("alice"));
    assert_ne!(
        web_resolution.turn_scope.thread_id, telegram_resolution.turn_scope.thread_id,
        "different product surfaces must not auto-merge conversations for the same user"
    );
}

#[tokio::test]
async fn explicit_link_attaches_conversation_to_existing_thread_after_access_checks() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;

    let web_resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session", None),
            "web-event-1",
        ))
        .await
        .unwrap();
    let link = services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: tenant(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: external_conversation("chat-1", None),
            target_thread_id: web_resolution.turn_scope.thread_id.clone(),
            target_agent_id: web_resolution.turn_scope.agent_id.clone(),
            target_project_id: web_resolution.turn_scope.project_id.clone(),
        })
        .await
        .unwrap();

    assert_eq!(link.thread_id, web_resolution.turn_scope.thread_id);
    let telegram_resolution = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("alice-telegram"),
            external_conversation("chat-1", None),
            "telegram-event-2",
        ))
        .await
        .unwrap();
    assert_eq!(
        telegram_resolution.turn_scope.thread_id,
        web_resolution.turn_scope.thread_id
    );
}

#[tokio::test]
async fn repeated_explicit_link_replays_existing_binding_refs() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    let web_resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session", None),
            "web-event-1",
        ))
        .await
        .unwrap();
    let request = LinkConversationRequest {
        tenant_id: tenant(),
        adapter_kind: telegram(),
        adapter_installation_id: default_installation(),
        external_actor_ref: external_actor("alice-telegram"),
        external_conversation_ref: external_conversation("chat-1", None),
        target_thread_id: web_resolution.turn_scope.thread_id.clone(),
        target_agent_id: web_resolution.turn_scope.agent_id.clone(),
        target_project_id: web_resolution.turn_scope.project_id.clone(),
    };

    let first = services
        .link_conversation_to_thread(request.clone())
        .await
        .unwrap();
    let duplicate = services.link_conversation_to_thread(request).await.unwrap();

    assert_eq!(duplicate.source_binding_ref, first.source_binding_ref);
    assert_eq!(
        duplicate.reply_target_binding_ref,
        first.reply_target_binding_ref
    );
}

#[tokio::test]
async fn explicit_link_refuses_to_retarget_existing_conversation_binding() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    let first_thread = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session-a", None),
            "web-event-a",
        ))
        .await
        .unwrap();
    let second_thread = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session-b", None),
            "web-event-b",
        ))
        .await
        .unwrap();
    services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: tenant(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: external_conversation("chat-1", None),
            target_thread_id: first_thread.turn_scope.thread_id,
            target_agent_id: first_thread.turn_scope.agent_id,
            target_project_id: first_thread.turn_scope.project_id,
        })
        .await
        .unwrap();

    let err = services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: tenant(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: external_conversation("chat-1", None),
            target_thread_id: second_thread.turn_scope.thread_id,
            target_agent_id: second_thread.turn_scope.agent_id,
            target_project_id: second_thread.turn_scope.project_id,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::BindingConflict { .. }));
}

#[tokio::test]
async fn explicit_link_uses_existing_thread_scope_not_spoofed_link_scope() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;

    let web_resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session", None),
            "web-event-1",
        ))
        .await
        .unwrap();
    services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: tenant(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: external_conversation("chat-1", None),
            target_thread_id: web_resolution.turn_scope.thread_id.clone(),
            target_agent_id: Some(AgentId::new("spoofed-agent").unwrap()),
            target_project_id: Some(ProjectId::new("spoofed-project").unwrap()),
        })
        .await
        .unwrap();

    let telegram_resolution = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("alice-telegram"),
            external_conversation("chat-1", None),
            "telegram-event-2",
        ))
        .await
        .unwrap();
    assert_eq!(telegram_resolution.turn_scope, web_resolution.turn_scope);
}

#[tokio::test]
async fn duplicate_external_event_after_transient_submit_failure_retries_same_message_ref() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(FailFirstTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());
    let request = inbound_request(
        telegram(),
        external_actor("telegram-user-1"),
        external_conversation("chat-1", None),
        "telegram-event-transient",
    );

    let err = inbound
        .handle_inbound_turn(request.clone())
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::TurnSubmissionFailed { .. }));
    assert_eq!(services.accepted_messages().await.len(), 1);
    assert_eq!(coordinator.submissions().len(), 1);

    let retry = inbound.handle_inbound_turn(request).await.unwrap();

    assert_eq!(services.accepted_messages().await.len(), 1);
    assert_eq!(coordinator.submissions().len(), 2);
    assert_eq!(
        coordinator.submissions()[0].accepted_message_ref,
        coordinator.submissions()[1].accepted_message_ref,
        "adapter retry must reuse the accepted message ref instead of getting stuck after a pre-submit failure"
    );
    assert!(retry.turn_submission.is_some());
}

#[tokio::test]
async fn max_length_accepted_message_ref_is_valid_as_submit_idempotency_key() {
    let binding = InMemoryConversationServices::default();
    binding
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let long_ref = "m".repeat(256);
    let session =
        FixedMessageSessionService::new(AcceptedMessageRef::new(long_ref.clone()).unwrap());
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(binding, session, coordinator.clone());

    inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-1", None),
            "telegram-event-long-ref",
        ))
        .await
        .unwrap();

    assert_eq!(coordinator.submissions().len(), 1);
    assert_eq!(
        coordinator.submissions()[0].idempotency_key.as_str(),
        long_ref
    );
}

#[tokio::test]
async fn duplicate_external_event_replays_message_and_does_not_submit_duplicate_turn() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());
    let request = inbound_request(
        telegram(),
        external_actor("telegram-user-1"),
        external_conversation("chat-1", None),
        "telegram-event-1",
    );

    let first = inbound.handle_inbound_turn(request.clone()).await.unwrap();
    let duplicate = inbound.handle_inbound_turn(request).await.unwrap();

    assert_eq!(
        duplicate.accepted_message.idempotency,
        MessageIdempotencyStatus::Duplicate
    );
    assert_eq!(
        duplicate.accepted_message.message_ref,
        first.accepted_message.message_ref
    );
    assert_eq!(coordinator.submissions().len(), 1);
}

#[tokio::test]
async fn bound_group_message_from_non_participant_is_denied() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("bob-telegram"),
            user("bob"),
        )
        .await;
    let group = external_conversation("group-1", Some("topic-a"));
    let alice_resolution = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("alice-telegram"),
            group.clone(),
            "group-event-1",
        ))
        .await
        .unwrap();
    assert_eq!(alice_resolution.access, ThreadAccessDecision::Allowed);

    let err = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("bob-telegram"),
            group,
            "group-event-2",
        ))
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[tokio::test]
async fn reply_target_validation_rejects_same_actor_wrong_thread_refs() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    let first = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser-a", None),
            "alice-event-a",
        ))
        .await
        .unwrap();
    let second = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser-b", None),
            "alice-event-b",
        ))
        .await
        .unwrap();

    let err = services
        .validate_reply_target(
            &tenant(),
            &user("alice"),
            &first.turn_scope.thread_id,
            &second.reply_target_binding_ref,
        )
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[tokio::test]
async fn accept_inbound_message_rejects_external_route_mismatch() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    let resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser-a", None),
            "alice-event-a",
        ))
        .await
        .unwrap();

    let err = services
        .accept_inbound_message(AcceptInboundMessageRequest {
            tenant_id: tenant(),
            thread_id: resolution.turn_scope.thread_id,
            actor: resolution.actor,
            source_binding_ref: resolution.source_binding_ref,
            reply_target_binding_ref: resolution.reply_target_binding_ref,
            external_conversation_ref: external_conversation("alice-browser-b", None),
            external_event_id: ExternalEventId::new("route-mismatch-event").unwrap(),
            content_ref: InboundMessageContentRef::new("content:route-mismatch-event").unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 1, 0).unwrap(),
        })
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[tokio::test]
async fn accept_inbound_message_rejects_mixed_source_and_reply_bindings() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    let first = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser-a", None),
            "alice-event-a",
        ))
        .await
        .unwrap();
    let second = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser-b", None),
            "alice-event-b",
        ))
        .await
        .unwrap();

    let err = services
        .accept_inbound_message(AcceptInboundMessageRequest {
            tenant_id: tenant(),
            thread_id: first.turn_scope.thread_id,
            actor: first.actor,
            source_binding_ref: first.source_binding_ref,
            reply_target_binding_ref: second.reply_target_binding_ref,
            external_conversation_ref: external_conversation("alice-browser-a", None),
            external_event_id: ExternalEventId::new("mixed-binding-event").unwrap(),
            content_ref: InboundMessageContentRef::new("content:mixed-binding-event").unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 1, 0).unwrap(),
        })
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[test]
fn serde_deserialization_revalidates_external_ref_invariants() {
    assert!(serde_json::from_str::<AdapterKind>("\"\"").is_err());
    assert!(
        serde_json::from_str::<AdapterInstallationId>(&format!("\"{}\"", "x".repeat(513))).is_err()
    );
    assert!(serde_json::from_str::<ExternalEventId>("\"event\\u0000id\"").is_err());
    assert!(serde_json::from_str::<InboundMessageContentRef>("\"\"").is_err());
    assert!(serde_json::from_str::<ExternalActorRef>(r#"{"kind":"user","id":""}"#).is_err());
    assert!(serde_json::from_str::<ExternalConversationRef>(
        r#"{"space_id":null,"conversation_id":"chat-1","thread_id":"ok","message_id":"bad\u0001"}"#
    )
    .is_err());
}

#[tokio::test]
async fn reply_target_validation_is_scoped_to_actor_and_binding() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("bob-web"),
            user("bob"),
        )
        .await;
    let alice = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser", None),
            "alice-event-1",
        ))
        .await
        .unwrap();
    let bob = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("bob-web"),
            external_conversation("bob-browser", None),
            "bob-event-1",
        ))
        .await
        .unwrap();

    let target = services
        .validate_reply_target(
            &tenant(),
            &user("alice"),
            &alice.turn_scope.thread_id,
            &alice.reply_target_binding_ref,
        )
        .await
        .unwrap();
    assert_eq!(
        target.external_conversation_ref.conversation_id(),
        "alice-browser"
    );

    let err = services
        .validate_reply_target(
            &tenant(),
            &user("alice"),
            &bob.turn_scope.thread_id,
            &bob.reply_target_binding_ref,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

fn inbound_request(
    adapter_kind: AdapterKind,
    external_actor_ref: ExternalActorRef,
    external_conversation_ref: ExternalConversationRef,
    external_event_id: &str,
) -> InboundTurnRequest {
    InboundTurnRequest {
        tenant_id: tenant(),
        adapter_kind,
        adapter_installation_id: default_installation(),
        external_actor_ref,
        external_conversation_ref,
        external_event_id: ExternalEventId::new(external_event_id).unwrap(),
        content_ref: InboundMessageContentRef::new(format!("content:{external_event_id}")).unwrap(),
        requested_agent_id: Some(agent()),
        requested_project_id: Some(project()),
        received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap(),
        requested_run_profile: None,
    }
}

fn resolve_request(
    adapter_kind: AdapterKind,
    external_actor_ref: ExternalActorRef,
    external_conversation_ref: ExternalConversationRef,
    external_event_id: &str,
) -> ironclaw_conversations::ResolveConversationRequest {
    resolve_request_with(
        tenant(),
        adapter_kind,
        default_installation(),
        external_actor_ref,
        external_conversation_ref,
        external_event_id,
    )
}

fn resolve_request_with(
    tenant_id: TenantId,
    adapter_kind: AdapterKind,
    adapter_installation_id: AdapterInstallationId,
    external_actor_ref: ExternalActorRef,
    external_conversation_ref: ExternalConversationRef,
    external_event_id: &str,
) -> ironclaw_conversations::ResolveConversationRequest {
    ironclaw_conversations::ResolveConversationRequest {
        tenant_id,
        adapter_kind,
        adapter_installation_id,
        external_actor_ref,
        external_conversation_ref,
        external_event_id: ExternalEventId::new(external_event_id).unwrap(),
        requested_agent_id: Some(agent()),
        requested_project_id: Some(project()),
    }
}

fn tenant() -> TenantId {
    TenantId::new("tenant-a").unwrap()
}

fn user(id: &str) -> UserId {
    UserId::new(id).unwrap()
}

fn agent() -> AgentId {
    AgentId::new("agent-a").unwrap()
}

fn project() -> ProjectId {
    ProjectId::new("project-a").unwrap()
}

fn telegram() -> AdapterKind {
    AdapterKind::new("telegram").unwrap()
}

fn web() -> AdapterKind {
    AdapterKind::new("web").unwrap()
}

fn default_installation() -> AdapterInstallationId {
    AdapterInstallationId::new("default-installation").unwrap()
}

fn external_actor(id: &str) -> ExternalActorRef {
    ExternalActorRef::new("user", id).unwrap()
}

fn external_conversation(
    conversation_id: &str,
    thread_id: Option<&str>,
) -> ExternalConversationRef {
    ExternalConversationRef::new(None, conversation_id, thread_id, None).unwrap()
}

struct FixedMessageSessionService {
    message_ref: AcceptedMessageRef,
    accepted: Mutex<Option<AcceptedInboundMessage>>,
    submitted: Mutex<bool>,
}

impl FixedMessageSessionService {
    fn new(message_ref: AcceptedMessageRef) -> Self {
        Self {
            message_ref,
            accepted: Mutex::new(None),
            submitted: Mutex::new(false),
        }
    }
}

#[async_trait]
impl SessionThreadService for FixedMessageSessionService {
    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, InboundTurnError> {
        let mut accepted = self.accepted.lock().unwrap();
        if let Some(existing) = accepted.clone() {
            let mut duplicate = existing;
            duplicate.idempotency = MessageIdempotencyStatus::Duplicate;
            return Ok(duplicate);
        }
        let message = AcceptedInboundMessage {
            tenant_id: request.tenant_id,
            thread_id: request.thread_id,
            message_ref: self.message_ref.clone(),
            source_binding_ref: request.source_binding_ref,
            reply_target_binding_ref: request.reply_target_binding_ref,
            idempotency: MessageIdempotencyStatus::Inserted,
        };
        *accepted = Some(message.clone());
        Ok(message)
    }

    async fn inbound_message_turn_submitted(
        &self,
        _message_ref: &AcceptedMessageRef,
    ) -> Result<bool, InboundTurnError> {
        Ok(*self.submitted.lock().unwrap())
    }

    async fn mark_inbound_message_turn_submitted(
        &self,
        _message_ref: &AcceptedMessageRef,
    ) -> Result<(), InboundTurnError> {
        *self.submitted.lock().unwrap() = true;
        Ok(())
    }
}

#[derive(Default)]
struct RecordingTurnCoordinator {
    submissions: Mutex<Vec<SubmitTurnRequest>>,
}

impl RecordingTurnCoordinator {
    fn submissions(&self) -> Vec<SubmitTurnRequest> {
        self.submissions.lock().unwrap().clone()
    }
}

#[derive(Default)]
struct FailFirstTurnCoordinator {
    submissions: Mutex<Vec<SubmitTurnRequest>>,
}

impl FailFirstTurnCoordinator {
    fn submissions(&self) -> Vec<SubmitTurnRequest> {
        self.submissions.lock().unwrap().clone()
    }
}

#[async_trait]
impl TurnCoordinator for FailFirstTurnCoordinator {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let mut submissions = self.submissions.lock().unwrap();
        submissions.push(request.clone());
        if submissions.len() == 1 {
            return Err(TurnError::Unavailable {
                reason: "transient outage".to_string(),
            });
        }
        Ok(accepted_response(request))
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        unimplemented!("not used by inbound facade tests")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        unimplemented!("not used by inbound facade tests")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        unimplemented!("not used by inbound facade tests")
    }
}

#[async_trait]
impl TurnCoordinator for RecordingTurnCoordinator {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.submissions.lock().unwrap().push(request.clone());
        Ok(accepted_response(request))
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        unimplemented!("not used by inbound facade tests")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        unimplemented!("not used by inbound facade tests")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        unimplemented!("not used by inbound facade tests")
    }
}

fn accepted_response(request: SubmitTurnRequest) -> SubmitTurnResponse {
    SubmitTurnResponse::Accepted {
        turn_id: ironclaw_turns::TurnId::new(),
        run_id: TurnRunId::new(),
        status: TurnStatus::Queued,
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        event_cursor: ironclaw_turns::events::EventCursor(1),
        accepted_message_ref: request.accepted_message_ref,
        reply_target_binding_ref: request.reply_target_binding_ref,
    }
}
