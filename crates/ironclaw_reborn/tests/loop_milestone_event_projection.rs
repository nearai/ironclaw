use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_event_projections::{
    EventStreamManager, ProjectionRequest, ProjectionScope, ReplayAuditProjectionService,
    ReplayEventProjectionService, RunProjectionStatus, TimelineEntryKind,
};
use ironclaw_events::{
    DurableAuditLog, DurableEventLog, EventStreamKey, InMemoryDurableAuditLog,
    InMemoryDurableEventLog, ReadScope,
};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_reborn::{
    DurableLoopHostMilestoneSink, RebornLoopDriverHostFactory, RebornLoopDriverHostRequest,
    TextOnlyLoopHostConfig,
};
use ironclaw_reborn_event_store::{
    RebornEventStoreConfig, RebornProfile, build_reborn_event_stores,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    SessionThreadService, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, EventCursor, InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore,
    InMemoryRunProfileResolver, ReplyTargetBindingRef, RunProfileId, RunProfileResolutionRequest,
    RunProfileResolver, RunProfileVersion, SourceBindingRef, TurnId, TurnLeaseToken, TurnRunId,
    TurnRunState, TurnRunnerId, TurnScope, TurnStatus,
    run_profile::{
        AgentLoopHostErrorKind, FinalizeAssistantMessage, LoopHostMilestoneSink, LoopModelPort,
        LoopModelRequest, LoopRunContext, LoopTranscriptPort, ParentLoopOutput,
    },
    runner::ClaimedTurnRun,
};

const RAW_PROMPT_SENTINEL: &str = "RAW_PROMPT_SENTINEL sk-secret /Users/firat/private.txt";
const RAW_ASSISTANT_SENTINEL: &str = "RAW_ASSISTANT_SENTINEL sk-secret /tmp/assistant.txt";
const RAW_PROVIDER_SENTINEL: &str = "RAW_PROVIDER_ERROR sk-secret /var/provider.log";

#[tokio::test]
async fn in_memory_durable_log_replays_loop_model_reply_milestones() {
    let events: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let audit: Arc<dyn DurableAuditLog> = Arc::new(InMemoryDurableAuditLog::new());

    drive_model_reply_milestones_and_assert_projection(events, audit).await;
}

#[tokio::test]
async fn jsonl_durable_log_replays_loop_model_reply_milestones() {
    let temp_dir = tempfile::tempdir().unwrap();
    let stores = build_reborn_event_stores(
        RebornProfile::Test,
        RebornEventStoreConfig::Jsonl {
            root: temp_dir.path().join("reborn-events"),
            accept_single_node_durable: true,
        },
    )
    .await
    .unwrap();

    drive_model_reply_milestones_and_assert_projection(stores.events, stores.audit).await;
}

async fn drive_model_reply_milestones_and_assert_projection(
    events: Arc<dyn DurableEventLog>,
    audit: Arc<dyn DurableAuditLog>,
) {
    let success = HostFixture::new(
        Arc::clone(&events),
        "thread-loop-events-success",
        RAW_PROMPT_SENTINEL,
        ControlledGateway::reply(RAW_ASSISTANT_SENTINEL),
    )
    .await;
    let success_host = success.build_host().await;
    let model_response = success_host
        .stream_model(LoopModelRequest {
            messages: Vec::new(),
            surface_version: None,
            model_preference: None,
        })
        .await
        .unwrap();
    let ParentLoopOutput::AssistantReply(reply) = model_response.output else {
        panic!("expected assistant reply output");
    };
    success_host
        .finalize_assistant_message(FinalizeAssistantMessage { reply })
        .await
        .unwrap();

    let failure = HostFixture::new(
        Arc::clone(&events),
        "thread-loop-events-failure",
        RAW_PROMPT_SENTINEL,
        ControlledGateway::fail(HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            RAW_PROVIDER_SENTINEL,
        )),
    )
    .await;
    let failure_host = failure.build_host().await;
    let error = failure_host
        .stream_model(LoopModelRequest {
            messages: Vec::new(),
            surface_version: None,
            model_preference: None,
        })
        .await
        .unwrap_err();
    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);

    let manager = event_stream_manager(events, audit);
    let snapshot = manager
        .runtime_snapshot(ProjectionRequest {
            scope: projection_scope(),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    let kinds = snapshot
        .timeline
        .entries
        .iter()
        .map(|entry| entry.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            TimelineEntryKind::ModelStarted,
            TimelineEntryKind::ModelCompleted,
            TimelineEntryKind::AssistantReplyFinalized,
            TimelineEntryKind::ModelStarted,
            TimelineEntryKind::ModelFailed,
        ]
    );

    let failed = snapshot
        .timeline
        .entries
        .iter()
        .find(|entry| entry.kind == TimelineEntryKind::ModelFailed)
        .expect("model_failed event should replay");
    assert_eq!(failed.error_kind.as_deref(), Some("unavailable"));

    let statuses = snapshot
        .runs
        .iter()
        .map(|run| run.status)
        .collect::<Vec<_>>();
    assert!(statuses.contains(&RunProjectionStatus::Completed));
    assert!(statuses.contains(&RunProjectionStatus::Failed));

    let wire = serde_json::to_string(&snapshot).unwrap();
    for forbidden in [
        "RAW_PROMPT_SENTINEL",
        "RAW_ASSISTANT_SENTINEL",
        "RAW_PROVIDER_ERROR",
        "sk-secret",
        "/Users/firat",
        "/var/provider.log",
    ] {
        assert!(!wire.contains(forbidden), "projection leaked {forbidden}");
    }
}

fn event_stream_manager(
    events: Arc<dyn DurableEventLog>,
    audit: Arc<dyn DurableAuditLog>,
) -> EventStreamManager {
    EventStreamManager::from_services(
        Arc::new(ReplayEventProjectionService::from_runtime_log(events)),
        Arc::new(ReplayAuditProjectionService::from_audit_log(audit)),
    )
}

fn projection_scope() -> ProjectionScope {
    ProjectionScope {
        stream: EventStreamKey::new(tenant_id(), user_id(), Some(agent_id())),
        read_scope: ReadScope {
            project_id: Some(project_id()),
            mission_id: None,
            thread_id: None,
            process_id: None,
        },
    }
}

struct HostFixture {
    thread_service: Arc<InMemorySessionThreadService>,
    checkpoint_state_store: Arc<InMemoryCheckpointStateStore>,
    loop_checkpoint_store: Arc<InMemoryLoopCheckpointStore>,
    gateway: Arc<ControlledGateway>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    thread_scope: ThreadScope,
    claimed: ClaimedTurnRun,
    context: LoopRunContext,
}

impl HostFixture {
    async fn new(
        events: Arc<dyn DurableEventLog>,
        thread_name: &str,
        user_content: &str,
        gateway: ControlledGateway,
    ) -> Self {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
        let loop_checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
        let gateway = Arc::new(gateway);
        let thread_id = ThreadId::new(thread_name).unwrap();
        let thread_scope = ThreadScope {
            tenant_id: tenant_id(),
            agent_id: agent_id(),
            project_id: Some(project_id()),
            owner_user_id: Some(user_id()),
            mission_id: None,
        };
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: user_id().to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        let accepted = thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
                actor_id: user_id().to_string(),
                source_binding_id: Some("source-web".to_string()),
                reply_target_binding_id: Some("reply-web".to_string()),
                external_event_id: Some(format!("event-{thread_name}")),
                content: MessageContent::text(user_content),
            })
            .await
            .unwrap();

        let turn_scope = TurnScope::new(
            tenant_id(),
            Some(agent_id()),
            Some(project_id()),
            thread_id.clone(),
        );
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        let turn_id = TurnId::new();
        let run_id = TurnRunId::new();
        let state = TurnRunState {
            scope: turn_scope.clone(),
            turn_id,
            run_id,
            status: TurnStatus::Running,
            accepted_message_ref: AcceptedMessageRef::new(format!("accepted-{thread_name}"))
                .unwrap(),
            source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            failure: None,
            event_cursor: EventCursor(1),
        };
        let claimed = ClaimedTurnRun {
            state,
            resolved_run_profile: resolved.clone(),
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
        };
        let context = LoopRunContext::new(turn_scope, turn_id, run_id, resolved);
        thread_service
            .mark_message_submitted(
                &thread_scope,
                &thread_id,
                accepted.message_id,
                turn_id.to_string(),
                run_id.to_string(),
            )
            .await
            .unwrap();
        let milestone_sink: Arc<dyn LoopHostMilestoneSink> =
            Arc::new(DurableLoopHostMilestoneSink::new(events, user_id()));

        Self {
            thread_service,
            checkpoint_state_store,
            loop_checkpoint_store,
            gateway,
            milestone_sink,
            thread_scope,
            claimed,
            context,
        }
    }

    async fn build_host(&self) -> ironclaw_reborn::RebornLoopDriverHost {
        RebornLoopDriverHostFactory::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            Arc::clone(&self.gateway),
            self.checkpoint_state_store.clone(),
            self.loop_checkpoint_store.clone(),
            Arc::clone(&self.milestone_sink),
            TextOnlyLoopHostConfig { max_messages: 8 },
        )
        .build_text_only_host(RebornLoopDriverHostRequest {
            claimed_run: self.claimed.clone(),
            loop_run_context: self.context.clone(),
        })
        .await
        .unwrap()
    }
}

struct ControlledGateway {
    response: Mutex<Option<Result<HostManagedModelResponse, HostManagedModelError>>>,
}

impl ControlledGateway {
    fn reply(content: &str) -> Self {
        Self {
            response: Mutex::new(Some(Ok(HostManagedModelResponse::assistant_reply(content)))),
        }
    }

    fn fail(error: HostManagedModelError) -> Self {
        Self {
            response: Mutex::new(Some(Err(error))),
        }
    }
}

#[async_trait]
impl HostManagedModelGateway for ControlledGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.response
            .lock()
            .unwrap()
            .take()
            .expect("test gateway response configured")
    }
}

fn tenant_id() -> TenantId {
    TenantId::new("tenant-loop-events").unwrap()
}

fn agent_id() -> AgentId {
    AgentId::new("agent-loop-events").unwrap()
}

fn project_id() -> ProjectId {
    ProjectId::new("project-loop-events").unwrap()
}

fn user_id() -> UserId {
    UserId::new("user-loop-events").unwrap()
}
