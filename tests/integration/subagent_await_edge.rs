use std::sync::Arc;

use chrono::Utc;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::turn::{LoopResultRef, TurnGateRef, TurnRunId, TurnScope};
use ironclaw_host_api::{
    ids::{AgentId, CapabilityId, InvocationId, ProcessId, ProjectId, TenantId, ThreadId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::ResourceScope,
};
use ironclaw_loop_contracts::{LoopInputCursorToken, LoopRunContext};
use ironclaw_loop_host::{
    AwaitedChildSetRecord, DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID, EnqueueQueuedMessageRequest,
    HostInputAckEffectHandler, HostInputBatch, HostInputEnqueuePort, HostInputEnvelope,
    HostInputQueue, HostInputQueueError, HostInputQueueReconcile, InMemoryHostInputQueue,
    LoopCapabilityResultWriter, ResolveOutcome, SpawnSubagentMode, SubagentKindId,
};
use ironclaw_processes::{
    ClaimProcessesRequest, CloseProcessDependencyRequest, OpenProcessDependencyRequest,
    ProcessDependencyPort, ProcessDependencyQuery, ProcessDependencyState,
    ProcessDependencySubmission, ProcessJournalStore, ProcessKind, ProcessLeaseRequest,
    ProcessLifecycleStatus, ProcessOperationId, ProcessStateTransitionRequest,
    ProcessSubmissionPort, ProcessTerminalEvidence, ProcessTransitionPort, ProcessWorkerId,
    SettleProcessDependencyRequest, SubmitProcessRequest, TransitionProcessDependencyRequest,
};
use ironclaw_threads::{
    AppendFinalizedAssistantMessageRequest, EnsureThreadRequest, InMemorySessionThreadService,
    MessageContent, MessageKind, SessionThreadService, ThreadHistoryRequest, ThreadScope,
};
use ironclaw_turn_runner::subagent::await_edge::{
    AwaitEdgeState, EdgeTerminalKind, resolver::AwaitEdgeResolver, store::AwaitEdgeStore,
};
use ironclaw_turns::{
    AcceptedMessageRef, ActivateThreadRequest, ActivationProvenance, AgentTurnRuntimePort,
    AgentTurnSpawnTreeRuntimePort, DefaultTurnCoordinator, IdempotencyKey, SYSTEM_WAKE_STREAK_CAP,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError,
    TurnSpawnTreePort,
    process_projection::AgentTurnProcessRuntime,
    test_support::{InMemoryAgentTurnProcessSystem, in_memory_agent_turn_process_system},
};

#[tokio::test]
async fn runner_await_edge_is_a_projection_over_process_dependencies() {
    let filesystem = processes_filesystem();
    let journal = Arc::new(ProcessJournalStore::new(Arc::clone(&filesystem)));
    let parent_scope = turn_scope("parent-thread");
    let child_scope = turn_scope("child-thread");
    let parent_run_id = TurnRunId::new();
    let child_run_id = TurnRunId::new();
    submit_root(
        &journal,
        &parent_scope.to_resource_scope(),
        ProcessId::from_uuid(parent_run_id.as_uuid()),
    )
    .await;

    let mut parent_context = ironclaw_agent_loop::test_support::test_run_context("parent");
    parent_context.run_id = parent_run_id;
    parent_context.scope = parent_scope.clone();
    parent_context.thread_id = parent_scope.thread_id.clone();
    let dependencies = Arc::clone(&journal)
        as Arc<dyn ProcessDependencyPort<Error = ironclaw_processes::ProcessJournalStoreError>>;
    let store = AwaitEdgeStore::new(dependencies);
    let dependency_metadata = serde_json::to_value(AwaitedChildSetRecord {
        gate_ref: TurnGateRef::new("gate:child").expect("gate"),
        parent_run_context: parent_context,
        tree_root_run_id: parent_run_id,
        child_scope: child_scope.clone(),
        child_run_id,
        child_thread_id: child_scope.thread_id.clone(),
        subagent_kind: SubagentKindId::new("general").expect("subagent kind"),
        spawn_capability_id: CapabilityId::new("builtin.subagent.spawn").expect("capability"),
        spawn_provider_call_id: None,
        result_ref: LoopResultRef::new("result:child").expect("result ref"),
        mode: SpawnSubagentMode::Blocking,
    })
    .expect("dependency metadata");
    submit_child(
        &journal,
        &child_scope.to_resource_scope(),
        parent_run_id,
        child_run_id,
        1,
        Some(ProcessDependencySubmission {
            dependent_process_id: ProcessId::from_uuid(parent_run_id.as_uuid()),
            root_process_id: ProcessId::from_uuid(parent_run_id.as_uuid()),
            group_ref: Some("gate:child".to_string()),
            metadata: dependency_metadata,
        }),
    )
    .await;

    let settled = store
        .settle(
            &child_scope,
            parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(17),
            None,
        )
        .await
        .expect("settle edge")
        .expect("edge exists");
    assert_eq!(settled.terminal_kind, Some(EdgeTerminalKind::Completed));
    store
        .consume(&child_scope, parent_run_id, child_run_id)
        .await
        .expect("consume edge");
    assert!(
        store
            .peek(&child_scope, parent_run_id, child_run_id)
            .await
            .expect("peek consumed edge")
            .is_none()
    );
    assert!(
        journal
            .unresolved_process_dependencies()
            .await
            .expect("unresolved dependencies")
            .is_empty()
    );
}

#[tokio::test]
async fn process_dependency_journal_stress_closes_each_record_and_releases_capacity() {
    const CHILDREN: u32 = 96;

    let filesystem = processes_filesystem();
    let journal = Arc::new(ProcessJournalStore::new(Arc::clone(&filesystem)));
    let parent_scope = resource_scope("stress-parent");
    let parent_id = ProcessId::new();
    submit_root(&journal, &parent_scope, parent_id).await;

    let mut children = Vec::with_capacity(CHILDREN as usize);
    for index in 0..CHILDREN {
        let child_id = ProcessId::new();
        let child_scope = resource_scope(&format!("stress-child-{index}"));
        journal
            .submit_process(SubmitProcessRequest {
                process_id: child_id,
                process_kind: ProcessKind::AgentTurn,
                scope: child_scope.clone(),
                exclusive_within_scope: false,
                operation_id: Some(ProcessOperationId::from_trusted(format!("child-{index}"))),
                owner_user_id: Some(child_scope.user_id.clone()),
                concurrency_class: None,
                parent_process_id: Some(parent_id),
                root_process_id: Some(parent_id),
                spawn_tree_descendant_cap: Some(CHILDREN),
                dependency: Some(ProcessDependencySubmission {
                    dependent_process_id: parent_id,
                    root_process_id: parent_id,
                    group_ref: Some("stress-group".to_string()),
                    metadata: serde_json::json!({"index": index}),
                }),
                checkpoint_ref: None,
                input: None,
                created_at: Utc::now(),
                metadata: serde_json::Value::Null,
            })
            .await
            .expect("submit child with dependency");
        children.push((child_id, child_scope));
    }

    let mut tasks = Vec::with_capacity(CHILDREN as usize);
    for (child_id, child_scope) in children {
        let filesystem = Arc::clone(&filesystem);
        tasks.push(tokio::spawn(async move {
            let handle = ProcessJournalStore::new(filesystem);
            handle
                .settle_process_dependency(SettleProcessDependencyRequest {
                    dependent_process_id: parent_id,
                    dependency_process_id: child_id,
                    scope: child_scope.clone(),
                    terminal: ProcessTerminalEvidence {
                        status: ProcessLifecycleStatus::Completed,
                        output_bytes: Some(1),
                        sanitized_reason: None,
                    },
                    settled_at: Utc::now(),
                })
                .await
                .expect("settle dependency");
            handle
                .consume_process_dependency(CloseProcessDependencyRequest {
                    dependent_process_id: parent_id,
                    dependency_process_id: child_id,
                    scope: child_scope,
                    closed_at: Utc::now(),
                })
                .await
                .expect("consume dependency");
        }));
    }
    for task in tasks {
        task.await.expect("dependency task");
    }

    assert!(
        journal
            .unresolved_process_dependencies()
            .await
            .expect("unresolved dependencies")
            .is_empty()
    );
    let closed = journal
        .query_process_dependencies(ProcessDependencyQuery {
            scope: parent_scope.clone(),
            dependent_process_id: Some(parent_id),
            group_ref: Some("stress-group".to_string()),
            allowed_states: None,
            include_closed: true,
            after: None,
            limit: None,
        })
        .await
        .expect("closed dependency query");
    assert_eq!(closed.len(), CHILDREN as usize);
    assert!(
        closed
            .iter()
            .all(|record| record.state == ProcessDependencyState::Consumed)
    );

    submit_child(
        &journal,
        &resource_scope("stress-replacement"),
        TurnRunId::from_uuid(parent_id.as_uuid()),
        TurnRunId::new(),
        CHILDREN,
        None,
    )
    .await;
}

async fn submit_root<F>(
    journal: &ProcessJournalStore<F>,
    scope: &ResourceScope,
    process_id: ProcessId,
) where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    journal
        .submit_process(SubmitProcessRequest {
            process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit root process");
}

async fn submit_child<F>(
    journal: &ProcessJournalStore<F>,
    scope: &ResourceScope,
    parent_run_id: TurnRunId,
    child_run_id: TurnRunId,
    cap: u32,
    dependency: Option<ProcessDependencySubmission>,
) where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    journal
        .submit_process(SubmitProcessRequest {
            process_id: ProcessId::from_uuid(child_run_id.as_uuid()),
            process_kind: ProcessKind::AgentTurn,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: Some(ProcessId::from_uuid(parent_run_id.as_uuid())),
            root_process_id: Some(ProcessId::from_uuid(parent_run_id.as_uuid())),
            spawn_tree_descendant_cap: Some(cap),
            dependency,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit child process");
}

fn turn_scope(thread: &str) -> TurnScope {
    let scope = resource_scope(thread);
    TurnScope::new_with_owner(
        scope.tenant_id,
        scope.agent_id,
        scope.project_id,
        scope.thread_id.expect("thread"),
        Some(scope.user_id),
    )
}

fn resource_scope(thread: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-dependencies").expect("tenant"),
        user_id: UserId::new("user-dependencies").expect("user"),
        agent_id: Some(AgentId::new("agent-dependencies").expect("agent")),
        project_id: Some(ProjectId::new("project-dependencies").expect("project")),
        mission_id: None,
        thread_id: Some(ThreadId::new(thread).expect("thread")),
        invocation_id: InvocationId::new(),
    }
}

fn processes_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").expect("processes alias"),
        VirtualPath::new("/engine/processes").expect("processes target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("processes mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}
// ─── Task 8 (2c): background-delivery integration scenarios ───────────────
//
// Composes the REAL production components the resolver tail runs against in
// `ironclaw_composition::runtime` — `DefaultTurnCoordinator` over the shared
// process journal, `InMemorySessionThreadService`, and `InMemoryHostInputQueue`
// — rather than the crate-tier test doubles `resolver/tests.rs` uses for the
// same scenarios (`RecordingResumeCoordinator`/`RecordingEnqueue`/
// `StubBackgroundRuntime`). Per-child submission goes through the same
// `TurnSpawnTreePort::submit_child_run` primitive `finish_spawn` calls
// (`group_ref = "bg:{parent_thread_id}"`, the deterministic run-start-sweep
// key) — no tool-call surface is exercised; edges are fixture-constructed
// directly against the real journal, per the harness guidance in
// `tests/integration/CLAUDE.md`.

struct RecordingResultWriter {
    updates: std::sync::Mutex<Vec<serde_json::Value>>,
}

impl Default for RecordingResultWriter {
    fn default() -> Self {
        Self {
            updates: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl LoopCapabilityResultWriter for RecordingResultWriter {
    async fn write_capability_result(
        &self,
        _write: ironclaw_loop_host::CapabilityResultWrite<'_>,
    ) -> Result<
        ironclaw_loop_host::CapabilityWriteResult,
        ironclaw_loop_contracts::AgentLoopHostError,
    > {
        Err(ironclaw_loop_contracts::AgentLoopHostError::new(
            ironclaw_loop_contracts::AgentLoopHostErrorKind::InvalidInvocation,
            "write is not used by background-delivery integration scenarios",
        ))
    }

    async fn update_capability_result(
        &self,
        _run_context: &LoopRunContext,
        _result_ref: &LoopResultRef,
        output: serde_json::Value,
    ) -> Result<u64, ironclaw_loop_contracts::AgentLoopHostError> {
        let byte_len = serde_json::to_vec(&output)
            .map_err(|error| {
                ironclaw_loop_contracts::AgentLoopHostError::new(
                    ironclaw_loop_contracts::AgentLoopHostErrorKind::Unavailable,
                    error.to_string(),
                )
            })?
            .len() as u64;
        self.updates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(output);
        Ok(byte_len)
    }
}

/// Claim + complete the sole unclaimed `AgentTurn` process on `scope` — the
/// same recipe `ironclaw_turns::tests::activation_contract`'s
/// `complete_active_run` uses to free a thread's one-active-run slot between
/// consecutive real admissions.
async fn complete_active_run(
    transitions: &Arc<dyn ProcessTransitionPort<Error = TurnError>>,
    scope: &TurnScope,
) {
    let claimed = transitions
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("bg-integration-worker"),
            scope_filter: Some(scope.to_resource_scope()),
            process_id_filter: None,
            process_kind_filter: Some(ProcessKind::AgentTurn),
            max_processes: 1,
        })
        .await
        .expect("claim succeeds")
        .pop()
        .expect("a run is claimable");
    transitions
        .complete_process(ProcessStateTransitionRequest {
            lease: ProcessLeaseRequest {
                process_id: claimed.state.process_id,
                worker_id: claimed.worker_id.clone(),
                lease_token: claimed.lease_token.clone(),
            },
            metadata: None,
        })
        .await
        .expect("run completes");
}

struct BgIntegrationFixture {
    #[allow(dead_code)] // keeps the in-memory journal filesystem alive
    system: InMemoryAgentTurnProcessSystem,
    runtime: Arc<AgentTurnProcessRuntime>,
    coordinator: Arc<DefaultTurnCoordinator<AgentTurnProcessRuntime>>,
    transitions: Arc<dyn ProcessTransitionPort<Error = TurnError>>,
    edge_store: Arc<AwaitEdgeStore>,
    thread_service: Arc<InMemorySessionThreadService>,
    queue: Arc<InMemoryHostInputQueue>,
    resolver: Arc<AwaitEdgeResolver<InMemorySessionThreadService>>,
    tenant_id: TenantId,
    agent_id: AgentId,
    owner_user_id: UserId,
    parent_thread_id: ThreadId,
    parent_scope: TurnScope,
    parent_run_id: TurnRunId,
    parent_context: LoopRunContext,
}

type DependenciesPort =
    Arc<dyn ProcessDependencyPort<Error = ironclaw_processes::ProcessJournalStoreError>>;
type EnqueueFactory = Box<
    dyn FnOnce(
        Arc<InMemoryHostInputQueue>,
        Arc<dyn ProcessTransitionPort<Error = TurnError>>,
        TurnScope,
    ) -> Arc<dyn HostInputEnqueuePort>,
>;
type DependenciesWrap = Box<dyn FnOnce(DependenciesPort) -> DependenciesPort>;

/// Builds one parent thread + one real, live (`Queued`) parent run over a
/// fresh in-memory process journal, wired the same way production wires the
/// resolver (`ironclaw_composition::runtime`). `enqueue_factory`/
/// `dependencies_wrap` let individual scenarios substitute a decorated
/// enqueue port or process-dependency port around the real ones — every
/// other seam (the coordinator, the journal, the thread service) stays real
/// for every scenario.
async fn build_bg_fixture_custom(
    suffix: &str,
    enqueue_factory: Option<EnqueueFactory>,
    dependencies_wrap: Option<DependenciesWrap>,
) -> BgIntegrationFixture {
    let system = in_memory_agent_turn_process_system();
    let runtime = Arc::new(system.runtime());
    let coordinator = Arc::new(DefaultTurnCoordinator::new(Arc::clone(&runtime)));
    let transitions = system.transitions();
    let process_store = system.store();
    let dependencies: DependenciesPort = Arc::clone(&process_store) as DependenciesPort;
    let dependencies = match dependencies_wrap {
        Some(wrap) => wrap(dependencies),
        None => dependencies,
    };
    let edge_store = Arc::new(AwaitEdgeStore::new(dependencies));
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let queue = Arc::new(InMemoryHostInputQueue::new(
        Arc::clone(&thread_service) as Arc<dyn SessionThreadService>
    ));

    let tenant_id = TenantId::new(format!("bg-int-tenant-{suffix}")).expect("tenant");
    let agent_id = AgentId::new(format!("bg-int-agent-{suffix}")).expect("agent");
    let owner_user_id = UserId::new(format!("bg-int-owner-{suffix}")).expect("owner");
    let parent_thread_id =
        ThreadId::new(format!("bg-int-parent-thread-{suffix}")).expect("parent thread");
    let parent_scope = TurnScope::new_with_owner(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        parent_thread_id.clone(),
        Some(owner_user_id.clone()),
    );
    let thread_scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: None,
        owner_user_id: Some(owner_user_id.clone()),
        mission_id: None,
    };
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope,
            thread_id: Some(parent_thread_id.clone()),
            created_by_actor_id: owner_user_id.to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("ensure parent thread");

    let SubmitTurnResponse::Accepted {
        run_id: parent_run_id,
        ..
    } = coordinator
        .submit_turn(SubmitTurnRequest {
            scope: parent_scope.clone(),
            actor: TurnActor::new(owner_user_id.clone()),
            accepted_message_ref: AcceptedMessageRef::new(format!(
                "accepted-bg-int-parent-{suffix}"
            ))
            .expect("accepted"),
            requested_run_profile: None,
            output_contract: None,
            requested_model: None,
            idempotency_key: IdempotencyKey::new(format!("bg-int-parent-{suffix}"))
                .expect("idempotency key"),
            received_at: Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
            subagent_activation_provenance: None,
        })
        .await
        .expect("submit parent run");

    let mut parent_context = ironclaw_agent_loop::test_support::test_run_context(suffix);
    parent_context.scope = parent_scope.clone();
    parent_context.thread_id = parent_thread_id.clone();
    parent_context.run_id = parent_run_id;
    parent_context.actor = Some(TurnActor::new(owner_user_id.clone()));

    let result_writer: Arc<dyn LoopCapabilityResultWriter> =
        Arc::new(RecordingResultWriter::default());
    let resolver = Arc::new(AwaitEdgeResolver::new_unbound(
        Arc::clone(&edge_store),
        Arc::clone(&runtime) as Arc<dyn AgentTurnSpawnTreeRuntimePort>,
        result_writer,
        Arc::clone(&thread_service),
    ));
    resolver
        .bind_coordinator(Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>)
        .expect("bind coordinator");
    let enqueue_port: Arc<dyn HostInputEnqueuePort> = match enqueue_factory {
        Some(factory) => factory(
            Arc::clone(&queue),
            Arc::clone(&transitions),
            parent_scope.clone(),
        ),
        None => Arc::clone(&queue) as Arc<dyn HostInputEnqueuePort>,
    };
    resolver
        .bind_input_enqueue(enqueue_port)
        .expect("bind input enqueue");
    queue
        .bind_ack_effect_handler(Arc::clone(&resolver) as Arc<dyn HostInputAckEffectHandler>)
        .expect("bind await-edge acknowledgment effect handler");

    BgIntegrationFixture {
        system,
        runtime,
        coordinator,
        transitions,
        edge_store,
        thread_service,
        queue,
        resolver,
        tenant_id,
        agent_id,
        owner_user_id,
        parent_thread_id,
        parent_scope,
        parent_run_id,
        parent_context,
    }
}

async fn build_bg_fixture(suffix: &str) -> BgIntegrationFixture {
    build_bg_fixture_custom(suffix, None, None).await
}

/// Submits one background child (real `submit_child_run`, matching
/// `finish_spawn`'s own call site exactly) with a fixture-constructed
/// `AwaitedChildSetRecord` dependency, ensures its thread, and lands its
/// final assistant message. Returns `(child_run_id, child_scope,
/// terminal_event)` — the event `resolver.handle_child_terminal` expects.
async fn open_background_child(
    fixture: &BgIntegrationFixture,
    child_suffix: &str,
    final_text: &str,
) -> (TurnRunId, TurnScope, ironclaw_turns::TurnLifecycleEvent) {
    let child_run_id = TurnRunId::new();
    let child_thread_id =
        ThreadId::new(format!("bg-int-child-thread-{child_suffix}")).expect("child thread");
    let child_scope = TurnScope::new_with_owner(
        fixture.tenant_id.clone(),
        Some(fixture.agent_id.clone()),
        None,
        child_thread_id.clone(),
        Some(fixture.owner_user_id.clone()),
    );
    let thread_scope = ThreadScope {
        tenant_id: fixture.tenant_id.clone(),
        agent_id: fixture.agent_id.clone(),
        project_id: None,
        owner_user_id: Some(fixture.owner_user_id.clone()),
        mission_id: None,
    };
    fixture
        .thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(child_thread_id.clone()),
            created_by_actor_id: fixture.owner_user_id.to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("ensure child thread");
    fixture
        .thread_service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: thread_scope,
            thread_id: child_thread_id.clone(),
            turn_run_id: child_run_id.to_string(),
            content: MessageContent::text(final_text),
        })
        .await
        .expect("append child final output");

    let gate_ref = TurnGateRef::new(format!("gate:subagent-bg-{child_run_id}")).expect("gate ref");
    let result_ref =
        LoopResultRef::new(format!("result:bg-int-{child_suffix}")).expect("result ref");
    let submitted = AwaitedChildSetRecord {
        gate_ref,
        parent_run_context: fixture.parent_context.clone(),
        tree_root_run_id: fixture.parent_run_id,
        child_scope: child_scope.clone(),
        child_run_id,
        child_thread_id: child_thread_id.clone(),
        subagent_kind: SubagentKindId::new("general").expect("kind"),
        spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
            .expect("capability"),
        spawn_provider_call_id: Some(format!("spawn-call-bg-int-{child_suffix}")),
        result_ref,
        mode: SpawnSubagentMode::Background,
    };
    let dependency_metadata = serde_json::to_value(&submitted).expect("edge metadata");
    let parent_process_id = ProcessId::from_uuid(fixture.parent_run_id.as_uuid());

    let SubmitTurnResponse::Accepted { .. } = fixture
        .coordinator
        .submit_child_run(ironclaw_turns::SubmitChildRunRequest {
            parent_scope: fixture.parent_scope.clone(),
            parent_run_id: fixture.parent_run_id,
            child_scope: child_scope.clone(),
            actor: TurnActor::new(fixture.owner_user_id.clone()),
            accepted_message_ref: AcceptedMessageRef::new(format!(
                "accepted-bg-int-child-{child_suffix}"
            ))
            .expect("accepted"),
            requested_run_profile: None,
            output_contract: None,
            idempotency_key: IdempotencyKey::new(format!("bg-int-child-{child_suffix}"))
                .expect("idempotency key"),
            received_at: Utc::now(),
            requested_run_id: Some(child_run_id),
            spawn_tree_descendant_cap: 4,
            process_dependency: Some(ProcessDependencySubmission {
                dependent_process_id: parent_process_id,
                root_process_id: parent_process_id,
                group_ref: Some(format!("bg:{}", fixture.parent_thread_id)),
                metadata: dependency_metadata,
            }),
            process_input: None,
        })
        .await
        .expect("submit background child run");

    let event = ironclaw_turns::TurnLifecycleEvent {
        cursor: ironclaw_host_api::turn::EventCursor(2),
        scope: child_scope.clone(),
        occurred_at: Some(Utc::now()),
        owner_user_id: Some(fixture.owner_user_id.clone()),
        run_id: child_run_id,
        status: ironclaw_host_api::turn::TurnStatus::Completed,
        kind: ironclaw_turns::TurnEventKind::Completed,
        blocked_gate: None,
        sanitized_reason: None,
        retryable: None,
        detail: None,
    };

    (child_run_id, child_scope, event)
}

fn parent_thread_scope(fixture: &BgIntegrationFixture) -> ThreadScope {
    ThreadScope {
        tenant_id: fixture.tenant_id.clone(),
        agent_id: fixture.agent_id.clone(),
        project_id: None,
        owner_user_id: Some(fixture.owner_user_id.clone()),
        mission_id: None,
    }
}

async fn system_messages(
    fixture: &BgIntegrationFixture,
) -> Vec<ironclaw_threads::ThreadMessageRecord> {
    fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: parent_thread_scope(fixture),
            thread_id: fixture.parent_thread_id.clone(),
        })
        .await
        .expect("read parent thread")
        .messages
        .into_iter()
        .filter(|message| message.kind == MessageKind::System)
        .collect()
}

async fn ack_batch(fixture: &BgIntegrationFixture, run_id: TurnRunId, batch: &HostInputBatch) {
    fixture
        .queue
        .ack_consumed(
            run_id,
            batch
                .inputs
                .iter()
                .map(|input| input.ack_token.clone())
                .collect(),
        )
        .await
        .expect("ack queued subagent result inputs");
}

// ─── Scenario 1: per-child delivery while the parent keeps running ────────

/// Two background children settle (in this call order) while the parent's
/// own spawning run is still live — the delivery tail's live-run branch, not
/// the parked branch. Each settle produces its own framed transcript row and
/// its own `LoopInput::SubagentSettled` queue entry, and both land in settle
/// order (D6: one typed input per child, never a batched/coalesced one).
#[tokio::test]
async fn background_child_result_is_delivered_per_child_while_parent_runs() {
    let fixture = build_bg_fixture("per-child").await;

    let (child_a, _scope_a, event_a) =
        open_background_child(&fixture, "a", "child A background output").await;
    let (child_b, _scope_b, event_b) =
        open_background_child(&fixture, "b", "child B background output").await;

    let outcome_a = fixture
        .resolver
        .handle_child_terminal(&event_a)
        .await
        .expect("child A delivers to the live parent");
    assert_eq!(outcome_a, ResolveOutcome::Drained);
    let outcome_b = fixture
        .resolver
        .handle_child_terminal(&event_b)
        .await
        .expect("child B delivers to the live parent");
    assert_eq!(outcome_b, ResolveOutcome::Drained);

    let rows = system_messages(&fixture).await;
    assert_eq!(rows.len(), 2, "each child gets its own framed row");
    assert!(
        rows[0]
            .content
            .as_deref()
            .unwrap_or_default()
            .contains("child A background output")
    );
    assert!(
        rows[1]
            .content
            .as_deref()
            .unwrap_or_default()
            .contains("child B background output")
    );

    let batch = fixture
        .queue
        .next_after(fixture.parent_run_id, LoopInputCursorToken::origin(), 10)
        .await
        .expect("poll queued inputs");
    assert_eq!(
        batch.inputs.len(),
        2,
        "one typed SubagentSettled input per child (D6)"
    );
    let ironclaw_loop_contracts::LoopInput::SubagentSettled {
        child_run_id: first_child,
        ..
    } = &batch.inputs[0].input
    else {
        panic!("expected SubagentSettled, got {:?}", batch.inputs[0].input);
    };
    let ironclaw_loop_contracts::LoopInput::SubagentSettled {
        child_run_id: second_child,
        ..
    } = &batch.inputs[1].input
    else {
        panic!("expected SubagentSettled, got {:?}", batch.inputs[1].input);
    };
    assert_eq!(
        (*first_child, *second_child),
        (child_a, child_b),
        "arrival order matches settle order"
    );

    for child_run_id in [child_a, child_b] {
        assert_eq!(
            fixture
                .edge_store
                .peek(&fixture.parent_scope, fixture.parent_run_id, child_run_id)
                .await
                .expect("peek edge before input ack")
                .expect("the edge remains recoverable until input ack")
                .state,
            AwaitEdgeState::ResultAppended
        );
    }
    ack_batch(&fixture, fixture.parent_run_id, &batch).await;

    for (scope, child_run_id) in [
        (&fixture.parent_scope, child_a),
        (&fixture.parent_scope, child_b),
    ] {
        assert!(
            fixture
                .edge_store
                .peek(scope, fixture.parent_run_id, child_run_id)
                .await
                .expect("peek edge")
                .is_none(),
            "a delivered edge must be closed"
        );
    }
}

// ─── Scenario 2: RunClosed race healed by activation ──────────────────────

/// Simulates the `RunClosed` race: an independent terminal-reconciliation
/// pass closes the live run's input queue AND completes its process record
/// between `deliver_background`'s live-run read (which must see the run as
/// genuinely live) and the enqueue attempt that follows it — reproducing
/// "the read observed a live run; by write time it had gone terminal and the
/// queue had closed" deterministically, with real components on both sides
/// of the race.
struct RaceClosingEnqueue {
    inner: Arc<InMemoryHostInputQueue>,
    transitions: Arc<dyn ProcessTransitionPort<Error = TurnError>>,
    scope: TurnScope,
}

#[async_trait::async_trait]
impl HostInputEnqueuePort for RaceClosingEnqueue {
    async fn enqueue_queued_message(
        &self,
        request: EnqueueQueuedMessageRequest,
    ) -> Result<HostInputEnvelope, HostInputQueueError> {
        complete_active_run(&self.transitions, &self.scope).await;
        let _ = self.inner.reject_unconsumed(request.run_id).await;
        Err(HostInputQueueError::RunClosed)
    }
}

#[tokio::test]
async fn run_closed_race_is_healed_by_activation() {
    let fixture = build_bg_fixture_custom(
        "run-closed",
        Some(Box::new(|queue, transitions, scope| {
            Arc::new(RaceClosingEnqueue {
                inner: queue,
                transitions,
                scope,
            }) as Arc<dyn HostInputEnqueuePort>
        })),
        None,
    )
    .await;

    let (child_run_id, _child_scope, event) =
        open_background_child(&fixture, "race", "child race output").await;

    let outcome = fixture
        .resolver
        .handle_child_terminal(&event)
        .await
        .expect("a RunClosed race is healed by activation, not a hard error");
    assert_eq!(outcome, ResolveOutcome::Drained);

    assert!(
        fixture
            .edge_store
            .peek(&fixture.parent_scope, fixture.parent_run_id, child_run_id)
            .await
            .expect("peek edge")
            .is_none(),
        "a healed-by-activation edge must be closed"
    );
    let rows = system_messages(&fixture).await;
    assert_eq!(rows.len(), 1);

    let recent = fixture
        .runtime
        .recent_runs_for_thread(&fixture.parent_scope, 1)
        .await
        .expect("recent runs");
    let newest = recent.first().expect("an activated run exists");
    assert_eq!(
        newest.subagent_activation_provenance,
        Some(ActivationProvenance::System),
        "the race is healed by a System-provenance activation, not a queued input"
    );
}

// ─── Scenario 3: parked/completed parent activated with System provenance ─

#[tokio::test]
async fn parked_parent_is_activated_with_system_provenance() {
    let fixture = build_bg_fixture("parked").await;
    complete_active_run(&fixture.transitions, &fixture.parent_scope).await;

    let (child_run_id, _child_scope, event) =
        open_background_child(&fixture, "parked", "child parked output").await;

    let outcome = fixture
        .resolver
        .handle_child_terminal(&event)
        .await
        .expect("parked-parent activation succeeds");
    assert_eq!(outcome, ResolveOutcome::Drained);

    assert!(
        fixture
            .edge_store
            .peek(&fixture.parent_scope, fixture.parent_run_id, child_run_id)
            .await
            .expect("peek edge")
            .is_none(),
        "an activated edge must be closed"
    );

    let recent = fixture
        .runtime
        .recent_runs_for_thread(&fixture.parent_scope, 1)
        .await
        .expect("recent runs");
    let newest = recent.first().expect("an activated run exists");
    assert_eq!(
        newest.subagent_activation_provenance,
        Some(ActivationProvenance::System),
        "assert via the submitted run record's journaled subagent_activation_provenance"
    );
}

// ─── Scenario 4: idempotent replay ─────────────────────────────────────────

/// Fails exactly one scripted `transition_process_dependency` call to the
/// requested target state, simulating a crash between a durable side effect
/// (the thread acceptance) and the store CAS that would have recorded it —
/// the same technique `resolver/tests.rs`'s `ScriptedDependencyFailures`
/// uses for the exhaustive per-boundary crate-tier matrix. This integration
/// scenario re-drives one representative boundary (crash before
/// `ResultAppended`) through the REAL `DefaultTurnCoordinator` +
/// `InMemoryHostInputQueue` + `InMemorySessionThreadService` stack, proving
/// the same idempotency guarantee holds end-to-end, not just against test
/// doubles.
struct FailOnceDependencies {
    inner: DependenciesPort,
    fail_transition_to: std::sync::Mutex<Option<ProcessDependencyState>>,
}

#[async_trait::async_trait]
impl ProcessDependencyPort for FailOnceDependencies {
    type Error = ironclaw_processes::ProcessJournalStoreError;

    async fn open_process_dependency(
        &self,
        request: OpenProcessDependencyRequest,
    ) -> Result<ironclaw_processes::ProcessDependencyRecord, Self::Error> {
        self.inner.open_process_dependency(request).await
    }

    async fn settle_process_dependency(
        &self,
        request: SettleProcessDependencyRequest,
    ) -> Result<Option<ironclaw_processes::ProcessDependencyRecord>, Self::Error> {
        self.inner.settle_process_dependency(request).await
    }

    async fn consume_process_dependency(
        &self,
        request: CloseProcessDependencyRequest,
    ) -> Result<Option<ironclaw_processes::ProcessDependencyRecord>, Self::Error> {
        self.inner.consume_process_dependency(request).await
    }

    async fn abandon_process_dependency(
        &self,
        request: CloseProcessDependencyRequest,
    ) -> Result<Option<ironclaw_processes::ProcessDependencyRecord>, Self::Error> {
        self.inner.abandon_process_dependency(request).await
    }

    async fn transition_process_dependency(
        &self,
        request: TransitionProcessDependencyRequest,
    ) -> Result<Option<ironclaw_processes::ProcessDependencyRecord>, Self::Error> {
        let armed = {
            let mut guard = self
                .fail_transition_to
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if *guard == Some(request.next) {
                *guard = None;
                true
            } else {
                false
            }
        };
        if armed {
            return Err(
                ironclaw_processes::ProcessJournalStoreError::InvalidRequest(format!(
                    "scripted transition failure for {:?}",
                    request.next
                )),
            );
        }
        self.inner.transition_process_dependency(request).await
    }

    async fn query_process_dependencies(
        &self,
        request: ProcessDependencyQuery,
    ) -> Result<Vec<ironclaw_processes::ProcessDependencyRecord>, Self::Error> {
        self.inner.query_process_dependencies(request).await
    }

    async fn unresolved_process_dependencies(
        &self,
    ) -> Result<Vec<ironclaw_processes::ProcessDependencyRecord>, Self::Error> {
        self.inner.unresolved_process_dependencies().await
    }
}

#[tokio::test]
async fn background_delivery_replay_is_idempotent() {
    let fixture = build_bg_fixture_custom(
        "replay",
        None,
        Some(Box::new(|inner| {
            Arc::new(FailOnceDependencies {
                inner,
                fail_transition_to: std::sync::Mutex::new(Some(
                    ProcessDependencyState::ResultAppended,
                )),
            }) as DependenciesPort
        })),
    )
    .await;

    let (child_run_id, _child_scope, event) =
        open_background_child(&fixture, "replay", "child replay output").await;

    let first = fixture.resolver.handle_child_terminal(&event).await;
    assert!(
        first.is_err(),
        "the scripted crash before ResultAppended must surface as an error"
    );

    let second = fixture
        .resolver
        .handle_child_terminal(&event)
        .await
        .expect("re-drive recovers once the scripted crash is spent");
    assert_eq!(second, ResolveOutcome::Drained);

    let rows = system_messages(&fixture).await;
    assert_eq!(rows.len(), 1, "exactly one row survives the replay");
    let batch = fixture
        .queue
        .next_after(fixture.parent_run_id, LoopInputCursorToken::origin(), 10)
        .await
        .expect("poll queued inputs");
    assert_eq!(
        batch.inputs.len(),
        1,
        "exactly one attention outcome survives the replay"
    );
    ack_batch(&fixture, fixture.parent_run_id, &batch).await;
    assert!(
        fixture
            .edge_store
            .peek(&fixture.parent_scope, fixture.parent_run_id, child_run_id)
            .await
            .expect("peek edge")
            .is_none()
    );
}

// ─── Scenario 5: streak-capped result waits for a human ───────────────────

#[tokio::test]
async fn streak_capped_result_waits_for_human() {
    let fixture = build_bg_fixture("streak").await;
    complete_active_run(&fixture.transitions, &fixture.parent_scope).await;

    for index in 0..SYSTEM_WAKE_STREAK_CAP {
        let SubmitTurnResponse::Accepted { .. } = fixture
            .coordinator
            .activate(ActivateThreadRequest {
                scope: fixture.parent_scope.clone(),
                actor: TurnActor::new(fixture.owner_user_id.clone()),
                accepted_message_ref: AcceptedMessageRef::new(format!(
                    "accepted-bg-int-streak-{index}"
                ))
                .expect("accepted"),
                provenance: ActivationProvenance::System,
                idempotency_key: IdempotencyKey::new(format!("bg-int-streak-{index}"))
                    .expect("idempotency key"),
                received_at: Utc::now(),
                requested_run_profile: None,
                resolved_run_profile: None,
            })
            .await
            .unwrap_or_else(|error| panic!("streak wake {index} must be admitted, got {error:?}"));
        complete_active_run(&fixture.transitions, &fixture.parent_scope).await;
    }

    let (child_run_id, child_scope, event) =
        open_background_child(&fixture, "streak", "child streak output").await;

    let outcome = fixture
        .resolver
        .handle_child_terminal(&event)
        .await
        .expect("a streak-capped activation refusal is not a hard error");
    assert_eq!(outcome, ResolveOutcome::Drained);

    let parked = fixture
        .edge_store
        .peek(&child_scope, fixture.parent_run_id, child_run_id)
        .await
        .expect("peek edge")
        .expect("a streak-deferred edge stays unclosed");
    assert_eq!(parked.state, AwaitEdgeState::AttentionDeferredStreakCap);

    // An autonomous re-drive (human_initiated = false) must not touch it.
    fixture
        .resolver
        .sweep_thread_on_run_start(&fixture.parent_scope, false)
        .await
        .expect("sweep succeeds");
    let still_parked = fixture
        .edge_store
        .peek(&child_scope, fixture.parent_run_id, child_run_id)
        .await
        .expect("peek edge")
        .expect("an autonomous sweep must not drain a streak-capped edge");
    assert_eq!(
        still_parked.state,
        AwaitEdgeState::AttentionDeferredStreakCap
    );

    // A human-provenance run starts on the same (now idle) thread — an
    // ordinary submission breaks the System streak window (any non-System
    // entry in the top-CAP window admits) — then its run-start sweep drains
    // the parked edge forward.
    let SubmitTurnResponse::Accepted {
        run_id: human_run_id,
        ..
    } = fixture
        .coordinator
        .submit_turn(SubmitTurnRequest {
            scope: fixture.parent_scope.clone(),
            actor: TurnActor::new(fixture.owner_user_id.clone()),
            accepted_message_ref: AcceptedMessageRef::new("accepted-bg-int-streak-human")
                .expect("accepted"),
            requested_run_profile: None,
            output_contract: None,
            requested_model: None,
            idempotency_key: IdempotencyKey::new("bg-int-streak-human").expect("idempotency key"),
            received_at: Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
            subagent_activation_provenance: None,
        })
        .await
        .expect("human submission admits on the now-idle thread");

    fixture
        .resolver
        .sweep_thread_on_run_start(&fixture.parent_scope, true)
        .await
        .expect("human-initiated sweep drains the streak-capped edge");

    assert_eq!(
        fixture
            .edge_store
            .peek(&child_scope, fixture.parent_run_id, child_run_id)
            .await
            .expect("peek edge")
            .expect("the edge remains recoverable until input ack")
            .state,
        AwaitEdgeState::AttentionDeferredStreakCap
    );
    let batch = fixture
        .queue
        .next_after(human_run_id, LoopInputCursorToken::origin(), 10)
        .await
        .expect("poll queued inputs");
    assert_eq!(
        batch.inputs.len(),
        1,
        "the streak-parked result is delivered into the human run's queue"
    );
    ack_batch(&fixture, human_run_id, &batch).await;
    assert!(
        fixture
            .edge_store
            .peek(&child_scope, fixture.parent_run_id, child_run_id)
            .await
            .expect("peek edge after input ack")
            .is_none(),
        "the human-provenance queue acknowledgment closes the parked edge"
    );
    let rows = system_messages(&fixture).await;
    assert_eq!(rows.len(), 1);
}
