use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use ironclaw_conversations::{
    AdapterInstallationId, AdapterKind, ConversationBindingService, ExternalActorRef,
    ExternalConversationRef, ExternalEventId, InMemoryConversationServices,
    InboundMessageContentRef, InboundTurnError, InboundTurnRequest, InboundTurnService,
    LinkConversationRequest, MessageIdempotencyStatus, ThreadAccessDecision,
};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_turns::{
    CancelRunRequest, CancelRunResponse, GetRunStateRequest, ResumeTurnRequest, ResumeTurnResponse,
    RunProfileId, RunProfileVersion, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
    TurnCoordinator, TurnError, TurnRunId, TurnRunState, TurnStatus,
};

#[tokio::test]
async fn paired_actor_without_binding_creates_thread_binding_message_and_submits_turn() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(telegram(), external_actor("telegram-user-1"), user("alice"))
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
async fn webui_and_telegram_default_to_separate_threads_for_same_user() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(web(), external_actor("alice-web"), user("alice"))
        .await;
    services
        .pair_external_actor(telegram(), external_actor("alice-telegram"), user("alice"))
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
        .pair_external_actor(web(), external_actor("alice-web"), user("alice"))
        .await;
    services
        .pair_external_actor(telegram(), external_actor("alice-telegram"), user("alice"))
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
async fn explicit_link_uses_existing_thread_scope_not_spoofed_link_scope() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(web(), external_actor("alice-web"), user("alice"))
        .await;
    services
        .pair_external_actor(telegram(), external_actor("alice-telegram"), user("alice"))
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
async fn duplicate_external_event_replays_message_and_does_not_submit_duplicate_turn() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(telegram(), external_actor("telegram-user-1"), user("alice"))
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
        .pair_external_actor(telegram(), external_actor("alice-telegram"), user("alice"))
        .await;
    services
        .pair_external_actor(telegram(), external_actor("bob-telegram"), user("bob"))
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
async fn reply_target_validation_is_scoped_to_actor_and_binding() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(web(), external_actor("alice-web"), user("alice"))
        .await;
    services
        .pair_external_actor(web(), external_actor("bob-web"), user("bob"))
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
        .validate_reply_target(&tenant(), &user("alice"), &alice.reply_target_binding_ref)
        .await
        .unwrap();
    assert_eq!(
        target.external_conversation_ref.conversation_id(),
        "alice-browser"
    );

    let err = services
        .validate_reply_target(&tenant(), &user("alice"), &bob.reply_target_binding_ref)
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
    ironclaw_conversations::ResolveConversationRequest {
        tenant_id: tenant(),
        adapter_kind,
        adapter_installation_id: default_installation(),
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

#[derive(Default)]
struct RecordingTurnCoordinator {
    submissions: Mutex<Vec<SubmitTurnRequest>>,
}

impl RecordingTurnCoordinator {
    fn submissions(&self) -> Vec<SubmitTurnRequest> {
        self.submissions.lock().unwrap().clone()
    }
}

#[async_trait]
impl TurnCoordinator for RecordingTurnCoordinator {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.submissions.lock().unwrap().push(request.clone());
        Ok(SubmitTurnResponse::Accepted {
            turn_id: ironclaw_turns::TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            event_cursor: ironclaw_turns::events::EventCursor(1),
            accepted_message_ref: request.accepted_message_ref,
            reply_target_binding_ref: request.reply_target_binding_ref,
        })
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
