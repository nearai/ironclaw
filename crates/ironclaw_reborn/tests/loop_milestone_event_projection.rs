use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_event_projections::{
    EventStreamManager, ProjectionCursor, ProjectionRequest, ProjectionScope,
    ReplayAuditProjectionService, ReplayEventProjectionService, RunProjectionStatus,
    TimelineEntryKind,
};
use ironclaw_events::{
    DurableAuditLog, DurableEventLog, EventStreamKey, InMemoryDurableAuditLog,
    InMemoryDurableEventLog, ReadScope,
};
use ironclaw_host_api::{
    AgentId, CapabilityId, ExtensionId, InvocationId, MissionId, ProjectId, RuntimeKind, TenantId,
    ThreadId, UserId,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_reborn::loop_driver_host::{
    RebornLoopDriverHost, RebornLoopDriverHostFactory, RebornLoopDriverHostRequest,
    TextOnlyLoopHostConfig,
};
use ironclaw_reborn::milestone_events::{
    DurableLoopHostMilestoneScope, DurableLoopHostMilestoneSink,
};
use ironclaw_reborn_event_store::{
    RebornEventStoreConfig, RebornProfile, build_reborn_event_stores,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    SessionThreadService, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, CapabilityActivityId, EventCursor,
    GetRunStateRequest, InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore,
    InMemoryRunProfileResolver, LoopCompletionKind, LoopExitId, LoopFailureKind,
    ReplyTargetBindingRef, ResumeTurnRequest, RunProfileId, RunProfileResolutionRequest,
    RunProfileResolver, RunProfileVersion, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
    TurnActor, TurnAdmissionPolicy, TurnCheckpointId, TurnError, TurnId, TurnLeaseToken, TurnRunId,
    TurnRunState, TurnRunnerId, TurnScope, TurnStateStore, TurnStatus,
    SpawnTreeReservation, TurnRunRecord,
    run_profile::{
        AgentLoopHostErrorKind, BatchPolicyKind, CapabilityFailureKind, FinalizeAssistantMessage,
        HookDecisionSummary, LoopCheckpointKind, LoopDriverId, LoopGateKind, LoopHostMilestone,
        LoopHostMilestoneEmitter, LoopHostMilestoneKind, LoopHostMilestoneSink, LoopModelPort,
        LoopModelRequest, LoopPromptBundleRequest, LoopPromptPort, LoopRunContext,
        LoopTranscriptPort, ParentLoopOutput, PromptMode,
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
    let event_root = temp_dir.path().join("reborn-events");
    let stores = build_reborn_event_stores(
        RebornProfile::Test,
        RebornEventStoreConfig::Jsonl {
            root: event_root.clone(),
            accept_single_node_durable: true,
        },
    )
    .await
    .unwrap();

    drive_model_reply_milestones_and_assert_projection(stores.events, stores.audit).await;

    let raw_jsonl = read_all_file_bytes_lossy(&event_root);
    for forbidden in [
        "RAW_PROMPT_SENTINEL",
        "RAW_ASSISTANT_SENTINEL",
        "RAW_PROVIDER_ERROR",
        "sk-secret",
        "/Users/firat",
        "/tmp/assistant.txt",
        "/var/provider.log",
    ] {
        assert!(
            !raw_jsonl.contains(forbidden),
            "raw JSONL leaked {forbidden}"
        );
    }
}

#[tokio::test]
async fn durable_milestone_scope_requires_thread_owner_binding() {
    let thread_scope = ThreadScope {
        tenant_id: tenant_id(),
        agent_id: agent_id(),
        project_id: Some(project_id()),
        owner_user_id: None,
        mission_id: Some(mission_id()),
    };

    let error = DurableLoopHostMilestoneScope::from_thread_scope(&thread_scope)
        .expect_err("ownerless thread scope must not build a durable event scope");

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

#[tokio::test]
async fn durable_milestone_sink_rejects_mismatched_milestone_scope() {
    let events: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let thread_scope = ThreadScope {
        tenant_id: tenant_id(),
        agent_id: agent_id(),
        project_id: Some(project_id()),
        owner_user_id: Some(user_id()),
        mission_id: Some(mission_id()),
    };
    let sink = DurableLoopHostMilestoneSink::new(
        Arc::clone(&events),
        DurableLoopHostMilestoneScope::from_thread_scope(&thread_scope).unwrap(),
    );
    let milestone = loop_milestone_for_scope(TurnScope::new(
        TenantId::new("tenant-loop-events-foreign").unwrap(),
        Some(agent_id()),
        Some(project_id()),
        ThreadId::new("thread-loop-events-mismatch").unwrap(),
    ));

    let error = sink
        .publish_loop_milestone(milestone)
        .await
        .expect_err("durable milestone sink must reject foreign turn scope");

    assert_eq!(error.kind, AgentLoopHostErrorKind::ScopeMismatch);
    let manager = event_stream_manager(events, Arc::new(InMemoryDurableAuditLog::new()));
    let snapshot = manager
        .runtime_snapshot(ProjectionRequest {
            scope: projection_scope(),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert!(snapshot.timeline.entries.is_empty());
}

#[tokio::test]
async fn durable_milestone_sink_rejects_mismatched_thread_or_run_binding() {
    let events: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let thread_scope = ThreadScope {
        tenant_id: tenant_id(),
        agent_id: agent_id(),
        project_id: Some(project_id()),
        owner_user_id: Some(user_id()),
        mission_id: Some(mission_id()),
    };
    let expected_thread_id = ThreadId::new("thread-loop-events-expected").unwrap();
    let expected_run_id = TurnRunId::new();
    let sink = DurableLoopHostMilestoneSink::new(
        Arc::clone(&events),
        DurableLoopHostMilestoneScope::from_thread_scope_for_run(
            &thread_scope,
            expected_thread_id.clone(),
            expected_run_id,
        )
        .unwrap(),
    );

    let wrong_thread = loop_milestone_for_scope(TurnScope::new(
        tenant_id(),
        Some(agent_id()),
        Some(project_id()),
        ThreadId::new("thread-loop-events-wrong").unwrap(),
    ));
    let error = sink
        .publish_loop_milestone(wrong_thread)
        .await
        .expect_err("durable milestone sink must reject foreign thread scope");
    assert_eq!(error.kind, AgentLoopHostErrorKind::ScopeMismatch);

    let wrong_run = LoopHostMilestone {
        run_id: TurnRunId::new(),
        ..loop_milestone_for_scope(TurnScope::new(
            tenant_id(),
            Some(agent_id()),
            Some(project_id()),
            expected_thread_id,
        ))
    };
    let error = sink
        .publish_loop_milestone(wrong_run)
        .await
        .expect_err("durable milestone sink must reject foreign run scope");
    assert_eq!(error.kind, AgentLoopHostErrorKind::ScopeMismatch);

    let manager = event_stream_manager(events, Arc::new(InMemoryDurableAuditLog::new()));
    let snapshot = manager
        .runtime_snapshot(ProjectionRequest {
            scope: projection_scope(),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert!(snapshot.timeline.entries.is_empty());
}

#[tokio::test]
async fn durable_milestone_sink_does_not_project_lossy_loop_progress_milestones() {
    let events: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let thread_scope = ThreadScope {
        tenant_id: tenant_id(),
        agent_id: agent_id(),
        project_id: Some(project_id()),
        owner_user_id: Some(user_id()),
        mission_id: Some(mission_id()),
    };
    let thread_id = ThreadId::new("thread-loop-events-progress").unwrap();
    let run_id = TurnRunId::new();
    let sink = DurableLoopHostMilestoneSink::new(
        Arc::clone(&events),
        DurableLoopHostMilestoneScope::from_thread_scope_for_run(
            &thread_scope,
            thread_id.clone(),
            run_id,
        )
        .unwrap(),
    );
    let scope = TurnScope::new(
        tenant_id(),
        Some(agent_id()),
        Some(project_id()),
        thread_id.clone(),
    );

    for kind in [
        LoopHostMilestoneKind::IterationStarted { iteration: 1 },
        LoopHostMilestoneKind::CapabilityBatchStarted {
            iteration: 1,
            call_count: 2,
            policy: BatchPolicyKind::Parallel,
        },
        LoopHostMilestoneKind::CapabilityBatchCompleted {
            iteration: 1,
            result_count: 1,
            denied_count: 0,
            gated_count: 1,
            failed_count: 0,
        },
        LoopHostMilestoneKind::GateBlocked {
            iteration: 1,
            gate_kind: LoopGateKind::Approval,
        },
        LoopHostMilestoneKind::CheckpointCreated {
            checkpoint_id: TurnCheckpointId::new(),
            checkpoint_kind: LoopCheckpointKind::BeforeBlock,
        },
    ] {
        sink.publish_loop_milestone(LoopHostMilestone {
            scope: scope.clone(),
            turn_id: TurnId::new(),
            run_id,
            loop_driver_id: LoopDriverId::new("test-driver").unwrap(),
            kind,
        })
        .await
        .unwrap();
    }

    let manager = event_stream_manager(events, Arc::new(InMemoryDurableAuditLog::new()));
    let snapshot = manager
        .runtime_snapshot(ProjectionRequest {
            scope: projection_scope_for_thread(thread_id),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert!(
        snapshot.timeline.entries.is_empty(),
        "progress milestones carry counters/checkpoint kinds that would be lost in RuntimeEvent"
    );
    assert!(
        snapshot.runs.is_empty(),
        "progress milestones should not synthesize partial run records"
    );

    let replay_scope = projection_scope_for_thread(scope.thread_id.clone());
    let replay = manager
        .runtime_updates(ProjectionRequest {
            scope: replay_scope.clone(),
            after: Some(ProjectionCursor::origin_for_scope(replay_scope)),
            limit: 16,
        })
        .await
        .unwrap();
    assert!(replay.updates.is_empty());
    assert!(replay.runs.is_empty());
}

async fn drive_model_reply_milestones_and_assert_projection(
    events: Arc<dyn DurableEventLog>,
    audit: Arc<dyn DurableAuditLog>,
) {
    let success_thread_id = ThreadId::new("thread-loop-events-success").unwrap();
    let failure_thread_id = ThreadId::new("thread-loop-events-failure").unwrap();
    let success = HostFixture::new(
        Arc::clone(&events),
        success_thread_id.clone(),
        RAW_PROMPT_SENTINEL,
        ControlledGateway::reply(RAW_ASSISTANT_SENTINEL),
    )
    .await;
    let success_host = success.build_host().await;
    let success_prompt = success_host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    let model_response = success_host
        .stream_model(LoopModelRequest {
            messages: success_prompt.messages,
            surface_version: None,
            model_preference: None,
            capability_view: None,
        })
        .await
        .unwrap();
    let ParentLoopOutput::AssistantReply(reply) = model_response.output else {
        panic!("expected assistant reply output");
    };

    let before_reply_finalized = event_stream_manager(Arc::clone(&events), Arc::clone(&audit))
        .runtime_snapshot(ProjectionRequest {
            scope: projection_scope_for_thread(success_thread_id.clone()),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(
        before_reply_finalized
            .timeline
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            TimelineEntryKind::ModelStarted,
            TimelineEntryKind::ModelCompleted,
        ]
    );
    assert_eq!(before_reply_finalized.runs.len(), 1);
    assert_eq!(
        before_reply_finalized.runs[0].status,
        RunProjectionStatus::Running,
        "model_completed only means provider returned; reply finalization can still fail"
    );

    success_host
        .finalize_assistant_message(FinalizeAssistantMessage { reply })
        .await
        .unwrap();
    LoopHostMilestoneEmitter::new(success.context.clone(), Arc::clone(&success.milestone_sink))
        .completed(
            LoopCompletionKind::FinalReply,
            LoopExitId::new("exit:loop-events-success").unwrap(),
        )
        .await
        .unwrap();

    let failure = HostFixture::new(
        Arc::clone(&events),
        failure_thread_id.clone(),
        RAW_PROMPT_SENTINEL,
        ControlledGateway::fail(HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            RAW_PROVIDER_SENTINEL,
        )),
    )
    .await;
    let failure_host = failure.build_host().await;
    let failure_prompt = failure_host
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    let error = failure_host
        .stream_model(LoopModelRequest {
            messages: failure_prompt.messages,
            surface_version: None,
            model_preference: None,
            capability_view: None,
        })
        .await
        .unwrap_err();
    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);

    let attempt_failure_only = event_stream_manager(Arc::clone(&events), Arc::clone(&audit))
        .runtime_snapshot(ProjectionRequest {
            scope: projection_scope_for_thread(failure_thread_id.clone()),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(attempt_failure_only.runs.len(), 1);
    assert_eq!(
        attempt_failure_only.runs[0].status,
        RunProjectionStatus::Running,
        "model_failed is attempt-level progress until trusted loop terminal failure"
    );

    LoopHostMilestoneEmitter::new(failure.context.clone(), Arc::clone(&failure.milestone_sink))
        .failed(
            LoopFailureKind::ModelError,
            LoopExitId::new("exit:loop-events-failure").unwrap(),
        )
        .await
        .unwrap();

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
            TimelineEntryKind::LoopCompleted,
            TimelineEntryKind::ModelStarted,
            TimelineEntryKind::ModelFailed,
            TimelineEntryKind::LoopFailed,
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

    let success_thread = manager
        .runtime_snapshot(ProjectionRequest {
            scope: projection_scope_for_thread(success_thread_id.clone()),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(
        success_thread
            .timeline
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            TimelineEntryKind::ModelStarted,
            TimelineEntryKind::ModelCompleted,
            TimelineEntryKind::AssistantReplyFinalized,
            TimelineEntryKind::LoopCompleted,
        ]
    );
    assert!(
        success_thread
            .timeline
            .entries
            .iter()
            .all(|entry| entry.thread_id.as_ref() == Some(&success_thread_id))
    );
    assert_eq!(success_thread.runs.len(), 1);
    assert_eq!(
        success_thread.runs[0].status,
        RunProjectionStatus::Completed
    );
    assert_eq!(
        success_thread.runs[0].capability_id,
        CapabilityId::new("loop.model").unwrap(),
        "assistant_reply_finalized must not reclassify the model run capability"
    );
    assert_eq!(success_thread.runs[0].error_kind, None);

    let success_replay_scope = projection_scope_for_thread(success_thread_id.clone());
    let success_thread_replay = manager
        .runtime_updates(ProjectionRequest {
            scope: success_replay_scope.clone(),
            after: Some(ProjectionCursor::origin_for_scope(success_replay_scope)),
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(
        success_thread_replay
            .updates
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            TimelineEntryKind::ModelStarted,
            TimelineEntryKind::ModelCompleted,
            TimelineEntryKind::AssistantReplyFinalized,
            TimelineEntryKind::LoopCompleted,
        ]
    );
    assert!(
        success_thread_replay
            .updates
            .iter()
            .all(|entry| entry.thread_id.as_ref() == Some(&success_thread_id))
    );

    let failure_thread = manager
        .runtime_snapshot(ProjectionRequest {
            scope: projection_scope_for_thread(failure_thread_id.clone()),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(
        failure_thread
            .timeline
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            TimelineEntryKind::ModelStarted,
            TimelineEntryKind::ModelFailed,
            TimelineEntryKind::LoopFailed
        ]
    );
    assert!(
        failure_thread
            .timeline
            .entries
            .iter()
            .all(|entry| entry.thread_id.as_ref() == Some(&failure_thread_id))
    );
    assert_eq!(failure_thread.runs.len(), 1);
    assert_eq!(failure_thread.runs[0].status, RunProjectionStatus::Failed);

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
            mission_id: Some(mission_id()),
            thread_id: None,
            process_id: None,
        },
    }
}

fn projection_scope_for_thread(thread_id: ThreadId) -> ProjectionScope {
    ProjectionScope {
        stream: EventStreamKey::new(tenant_id(), user_id(), Some(agent_id())),
        read_scope: ReadScope {
            project_id: Some(project_id()),
            mission_id: Some(mission_id()),
            thread_id: Some(thread_id),
            process_id: None,
        },
    }
}

fn loop_milestone_for_scope(scope: TurnScope) -> LoopHostMilestone {
    LoopHostMilestone {
        scope,
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        loop_driver_id: LoopDriverId::new("test-driver").unwrap(),
        kind: LoopHostMilestoneKind::ModelStarted {
            requested_model_profile_id: None,
        },
    }
}

fn read_all_file_bytes_lossy(root: &Path) -> String {
    let mut output = String::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let metadata = std::fs::metadata(&path).unwrap();
        if metadata.is_dir() {
            for entry in std::fs::read_dir(path).unwrap() {
                stack.push(entry.unwrap().path());
            }
        } else {
            output.push_str(&String::from_utf8_lossy(&std::fs::read(path).unwrap()));
        }
    }
    output
}

struct HostFixture {
    thread_service: Arc<InMemorySessionThreadService>,
    checkpoint_state_store: Arc<InMemoryCheckpointStateStore>,
    turn_state_store: Arc<StaticTurnStateStore>,
    loop_checkpoint_store: Arc<InMemoryLoopCheckpointStore>,
    gateway: Arc<ControlledGateway>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    thread_scope: ThreadScope,
    claimed: ClaimedTurnRun,
    context: LoopRunContext,
}

struct StaticTurnStateStore {
    state: Mutex<TurnRunState>,
}

impl StaticTurnStateStore {
    fn new(state: TurnRunState) -> Self {
        Self {
            state: Mutex::new(state),
        }
    }
}

#[async_trait]
impl TurnStateStore for StaticTurnStateStore {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("submit_turn should not be called by static test turn state store")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ironclaw_turns::ResumeTurnResponse, TurnError> {
        panic!("resume_turn should not be called by static test turn state store")
    }

    async fn request_cancel(
        &self,
        _request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        panic!("request_cancel should not be called by static test turn state store")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        Ok(self.state.lock().unwrap().clone())
    }

    async fn children_of(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        panic!("children_of should not be called by static test turn state store")
    }

    async fn get_run_record(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        panic!("get_run_record should not be called by static test turn state store")
    }

    async fn reserve_tree_descendants(
        &self,
        _scope: &TurnScope,
        _tree_root_run_id: TurnRunId,
        _amount: u32,
        _cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError> {
        panic!("reserve_tree_descendants should not be called by static test turn state store")
    }

    async fn release_tree_descendants(
        &self,
        _scope: &TurnScope,
        _root_run_id: TurnRunId,
        _delta: u32,
    ) -> Result<(), TurnError> {
        panic!("release_tree_descendants should not be called by static test turn state store")
    }
}

impl HostFixture {
    async fn new(
        events: Arc<dyn DurableEventLog>,
        thread_id: ThreadId,
        user_content: &str,
        gateway: ControlledGateway,
    ) -> Self {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
        let loop_checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
        let gateway = Arc::new(gateway);
        let thread_scope = ThreadScope {
            tenant_id: tenant_id(),
            agent_id: agent_id(),
            project_id: Some(project_id()),
            owner_user_id: Some(user_id()),
            mission_id: Some(mission_id()),
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
                external_event_id: Some(format!("event-{thread_id}")),
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
            actor: Some(TurnActor::new(user_id())),
            turn_id,
            run_id,
            status: TurnStatus::Running,
            accepted_message_ref: AcceptedMessageRef::new(format!("accepted-{thread_id}")).unwrap(),
            source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
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
        let turn_state_store = Arc::new(StaticTurnStateStore::new(claimed.state.clone()));
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
        let milestone_scope = DurableLoopHostMilestoneScope::from_thread_scope_for_run(
            &thread_scope,
            thread_id.clone(),
            run_id,
        )
        .expect("thread fixture has owner user for milestone scope");
        let milestone_sink: Arc<dyn LoopHostMilestoneSink> =
            Arc::new(DurableLoopHostMilestoneSink::new(events, milestone_scope));

        Self {
            thread_service,
            checkpoint_state_store,
            turn_state_store,
            loop_checkpoint_store,
            gateway,
            milestone_sink,
            thread_scope,
            claimed,
            context,
        }
    }

    async fn build_host(&self) -> RebornLoopDriverHost {
        RebornLoopDriverHostFactory::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            Arc::clone(&self.gateway),
            self.checkpoint_state_store.clone(),
            self.turn_state_store.clone(),
            self.loop_checkpoint_store.clone(),
            Arc::clone(&self.milestone_sink),
            TextOnlyLoopHostConfig {
                max_messages: 8,
                ..TextOnlyLoopHostConfig::default()
            },
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

fn mission_id() -> MissionId {
    MissionId::new("mission-loop-events").unwrap()
}

fn user_id() -> UserId {
    UserId::new("user-loop-events").unwrap()
}

// ─── PR #3573 deferred test: publish_loop_milestone with hook milestones ───
//
// The unit-level helper `runtime_event_for_milestone` is already covered
// inside `milestone_events.rs::tests`. Per IronClaw's "Test Through the
// Caller, Not Just the Helper" rule, the trait impl that actually appends
// to the event log (`LoopHostMilestoneSink::publish_loop_milestone`) needs
// its own coverage. These tests drive `publish_loop_milestone` end-to-end
// with each `Hook*` milestone kind and assert the event lands in the
// durable log with the sanitized hook metadata projected through the
// snapshot pipeline.

const HOOK_HEX_ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn milestone_thread_scope() -> ThreadScope {
    ThreadScope {
        tenant_id: tenant_id(),
        agent_id: agent_id(),
        project_id: Some(project_id()),
        owner_user_id: Some(user_id()),
        mission_id: Some(mission_id()),
    }
}

fn milestone_for(
    scope: TurnScope,
    run_id: TurnRunId,
    kind: LoopHostMilestoneKind,
) -> LoopHostMilestone {
    LoopHostMilestone {
        scope,
        turn_id: TurnId::new(),
        run_id,
        loop_driver_id: LoopDriverId::new("milestone-projection-driver").unwrap(),
        kind,
    }
}

#[tokio::test]
async fn publish_loop_milestone_projects_capability_lifecycle_to_runtime_events() {
    let events: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let thread_id = ThreadId::new("thread-capability-publish-lifecycle").unwrap();
    let run_id = TurnRunId::new();
    let first_activity_id = CapabilityActivityId::new();
    let second_activity_id = CapabilityActivityId::new();
    let first_capability_id = CapabilityId::new("demo.echo").unwrap();
    let second_capability_id = CapabilityId::new("demo.search").unwrap();
    let provider_id = ExtensionId::new("demo").unwrap();
    let sink = DurableLoopHostMilestoneSink::new(
        Arc::clone(&events),
        DurableLoopHostMilestoneScope::from_thread_scope_for_run(
            &milestone_thread_scope(),
            thread_id.clone(),
            run_id,
        )
        .unwrap(),
    );
    let scope = TurnScope::new(
        tenant_id(),
        Some(agent_id()),
        Some(project_id()),
        thread_id.clone(),
    );

    sink.publish_loop_milestone(milestone_for(
        scope.clone(),
        run_id,
        LoopHostMilestoneKind::ModelStarted {
            requested_model_profile_id: None,
        },
    ))
    .await
    .unwrap();

    for kind in [
        LoopHostMilestoneKind::CapabilityInvoked {
            activity_id: first_activity_id,
            capability_id: first_capability_id.clone(),
        },
        LoopHostMilestoneKind::CapabilityCompleted {
            activity_id: first_activity_id,
            capability_id: first_capability_id.clone(),
            provider: provider_id.clone(),
            runtime: RuntimeKind::FirstParty,
            output_bytes: 64,
        },
        LoopHostMilestoneKind::CapabilityInvoked {
            activity_id: second_activity_id,
            capability_id: second_capability_id.clone(),
        },
        LoopHostMilestoneKind::CapabilityFailed {
            activity_id: second_activity_id,
            capability_id: second_capability_id.clone(),
            provider: Some(provider_id.clone()),
            runtime: Some(RuntimeKind::FirstParty),
            reason_kind: CapabilityFailureKind::OperationFailed,
        },
    ] {
        sink.publish_loop_milestone(milestone_for(scope.clone(), run_id, kind))
            .await
            .unwrap();
    }

    let manager = event_stream_manager(events, Arc::new(InMemoryDurableAuditLog::new()));
    let snapshot = manager
        .runtime_snapshot(ProjectionRequest {
            scope: projection_scope_for_thread(thread_id),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 5);
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
            TimelineEntryKind::DispatchRequested,
            TimelineEntryKind::DispatchSucceeded,
            TimelineEntryKind::DispatchRequested,
            TimelineEntryKind::DispatchFailed,
        ]
    );
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Running);
    assert_eq!(snapshot.capability_activities.len(), 2);
    let completed = snapshot
        .capability_activities
        .iter()
        .find(|activity| {
            activity.invocation_id == InvocationId::from_uuid(first_activity_id.as_uuid())
        })
        .expect("first capability activity projected");
    assert_eq!(
        completed.run_id,
        Some(InvocationId::from_uuid(run_id.as_uuid()))
    );
    assert_eq!(completed.capability_id, first_capability_id);
    assert_eq!(completed.provider.as_ref(), Some(&provider_id));
    assert_eq!(completed.runtime, Some(RuntimeKind::FirstParty));
    assert_eq!(completed.output_bytes, Some(64));
    let failed = snapshot
        .capability_activities
        .iter()
        .find(|activity| {
            activity.invocation_id == InvocationId::from_uuid(second_activity_id.as_uuid())
        })
        .expect("second capability activity projected");
    assert_eq!(
        failed.run_id,
        Some(InvocationId::from_uuid(run_id.as_uuid()))
    );
    assert_eq!(failed.capability_id, second_capability_id);
    assert_eq!(failed.provider.as_ref(), Some(&provider_id));
    assert_eq!(failed.runtime, Some(RuntimeKind::FirstParty));
    assert_eq!(failed.error_kind.as_deref(), Some("operation_failed"));
}

#[tokio::test]
async fn publish_loop_milestone_projects_hook_dispatched_to_runtime_event() {
    let events: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let thread_id = ThreadId::new("thread-hook-publish-dispatched").unwrap();
    let run_id = TurnRunId::new();
    let sink = DurableLoopHostMilestoneSink::new(
        Arc::clone(&events),
        DurableLoopHostMilestoneScope::from_thread_scope_for_run(
            &milestone_thread_scope(),
            thread_id.clone(),
            run_id,
        )
        .unwrap(),
    );
    let scope = TurnScope::new(
        tenant_id(),
        Some(agent_id()),
        Some(project_id()),
        thread_id.clone(),
    );

    sink.publish_loop_milestone(milestone_for(
        scope,
        run_id,
        LoopHostMilestoneKind::HookDispatched {
            hook_id: HOOK_HEX_ID.to_string(),
            point: "before_capability".to_string(),
            trust_class: "installed".to_string(),
            owning_extension: None,
        },
    ))
    .await
    .unwrap();

    let manager = event_stream_manager(events, Arc::new(InMemoryDurableAuditLog::new()));
    let snapshot = manager
        .runtime_snapshot(ProjectionRequest {
            scope: projection_scope_for_thread(thread_id),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 1);
    let entry = &snapshot.timeline.entries[0];
    assert_eq!(entry.kind, TimelineEntryKind::HookDispatched);
    assert_eq!(entry.hook_id.as_deref(), Some(HOOK_HEX_ID));
    assert_eq!(entry.hook_point.as_deref(), Some("before_capability"));
    assert_eq!(entry.hook_trust_class.as_deref(), Some("installed"));
    // Hook lifecycle telemetry must not move the run's status away from
    // the default `Running` bootstrap state — this is also the contract
    // pinned by `hook_runtime_events_do_not_alter_run_status_projection`.
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Running);
}

#[tokio::test]
async fn publish_loop_milestone_projects_hook_decision_with_closed_vocabulary_only() {
    let events: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let thread_id = ThreadId::new("thread-hook-publish-decision").unwrap();
    let run_id = TurnRunId::new();
    let sink = DurableLoopHostMilestoneSink::new(
        Arc::clone(&events),
        DurableLoopHostMilestoneScope::from_thread_scope_for_run(
            &milestone_thread_scope(),
            thread_id.clone(),
            run_id,
        )
        .unwrap(),
    );
    let scope = TurnScope::new(
        tenant_id(),
        Some(agent_id()),
        Some(project_id()),
        thread_id.clone(),
    );

    // The raw `reason` must NOT cross into the durable event — only the
    // closed-vocabulary `kind_name()` (here, "deny") may flow through.
    const RAW_DECISION_REASON: &str = "RAW_DECISION_REASON_SENTINEL sk-leak";
    sink.publish_loop_milestone(milestone_for(
        scope,
        run_id,
        LoopHostMilestoneKind::HookDecisionEmitted {
            hook_id: HOOK_HEX_ID.to_string(),
            decision: HookDecisionSummary::Deny {
                reason: RAW_DECISION_REASON.to_string(),
            },
            audit_reason: None,
            owning_extension: None,
        },
    ))
    .await
    .unwrap();

    let manager = event_stream_manager(events, Arc::new(InMemoryDurableAuditLog::new()));
    let snapshot = manager
        .runtime_snapshot(ProjectionRequest {
            scope: projection_scope_for_thread(thread_id),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 1);
    let entry = &snapshot.timeline.entries[0];
    assert_eq!(entry.kind, TimelineEntryKind::HookDecisionEmitted);
    assert_eq!(entry.hook_decision.as_deref(), Some("deny"));
    assert_eq!(entry.hook_id.as_deref(), Some(HOOK_HEX_ID));
    // Sanity-check the contract that raw reason text never enters the
    // projection DTO — the decision label is the only model-visible carrier.
    let wire = serde_json::to_string(entry).expect("serialize entry");
    assert!(
        !wire.contains("RAW_DECISION_REASON_SENTINEL"),
        "raw decision reason leaked into projection entry: {wire}",
    );
}

#[tokio::test]
async fn publish_loop_milestone_projects_hook_failed_with_disposition() {
    let events: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let thread_id = ThreadId::new("thread-hook-publish-failed").unwrap();
    let run_id = TurnRunId::new();
    let sink = DurableLoopHostMilestoneSink::new(
        Arc::clone(&events),
        DurableLoopHostMilestoneScope::from_thread_scope_for_run(
            &milestone_thread_scope(),
            thread_id.clone(),
            run_id,
        )
        .unwrap(),
    );
    let scope = TurnScope::new(
        tenant_id(),
        Some(agent_id()),
        Some(project_id()),
        thread_id.clone(),
    );

    sink.publish_loop_milestone(milestone_for(
        scope,
        run_id,
        LoopHostMilestoneKind::HookFailed {
            hook_id: HOOK_HEX_ID.to_string(),
            category: "timeout".to_string(),
            disposition: "fail_closed".to_string(),
            owning_extension: None,
        },
    ))
    .await
    .unwrap();

    let manager = event_stream_manager(events, Arc::new(InMemoryDurableAuditLog::new()));
    let snapshot = manager
        .runtime_snapshot(ProjectionRequest {
            scope: projection_scope_for_thread(thread_id),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 1);
    let entry = &snapshot.timeline.entries[0];
    assert_eq!(entry.kind, TimelineEntryKind::HookFailed);
    assert_eq!(entry.hook_id.as_deref(), Some(HOOK_HEX_ID));
    assert_eq!(entry.hook_failure_category.as_deref(), Some("timeout"));
    assert_eq!(
        entry.hook_failure_disposition.as_deref(),
        Some("fail_closed")
    );
    // A `HookFailed` event must NOT downgrade the run to `Failed`; hook
    // telemetry is purely observability.
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Running);
}
