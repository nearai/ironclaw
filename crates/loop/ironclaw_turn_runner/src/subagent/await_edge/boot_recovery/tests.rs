use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ids::{AgentId, CapabilityId, ProcessId, TenantId, ThreadId, UserId};
use ironclaw_host_api::turn::{
    AcceptedMessageRef, ActivationProvenance, LoopMessageRef, LoopResultRef, TurnActor,
    TurnGateRef, TurnRunId, TurnScope,
};
use ironclaw_loop_host::{
    AwaitedChildSetRecord, DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID, SpawnSubagentMode, SubagentKindId,
};
use ironclaw_processes::{
    ProcessDependencySubmission, ProcessJournalStore, ProcessKind, ProcessOperationId,
    ProcessSubmissionPort, SubmitProcessRequest,
};
use ironclaw_threads::{EnsureThreadRequest, SessionThreadService, ThreadScope};
use ironclaw_turns::{
    ActivateThreadRequest, AgentTurnSpawnTreeRuntimePort, CancelRunRequest, CancelRunResponse,
    GetRunStateRequest, ResumeTurnRequest, ResumeTurnResponse, RetryTurnRequest, RetryTurnResponse,
    SubmitTurnRequest, SubmitTurnResponse, TurnCoordinator, TurnError, TurnRunRecord, TurnRunState,
    TurnStatus,
};

use super::*;
use crate::subagent::await_edge::{AttentionOutcome, EdgeTerminalKind};

/// Bare `TurnCoordinator` double: only `activate` is real (records the
/// request and replays a scripted outcome). Every other method is
/// unreachable — recovering a parked background parent never resumes,
/// submits, or cancels.
#[derive(Default)]
struct RecordingActivateCoordinator {
    activations: std::sync::Mutex<Vec<ActivateThreadRequest>>,
}

impl RecordingActivateCoordinator {
    fn activations(&self) -> Vec<ActivateThreadRequest> {
        self.activations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl TurnCoordinator for RecordingActivateCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        unreachable!("boot recovery tests do not prepare turns")
    }

    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        unreachable!("boot recovery tests do not submit turns")
    }

    async fn activate(
        &self,
        request: ActivateThreadRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.activations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        Ok(SubmitTurnResponse::Accepted {
            turn_id: ironclaw_host_api::turn::TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            resolved_run_profile_id: ironclaw_host_api::turn::RunProfileId::default_profile(),
            resolved_run_profile_version: ironclaw_host_api::turn::RunProfileVersion::new(1),
            event_cursor: ironclaw_host_api::turn::EventCursor(1),
            accepted_message_ref: AcceptedMessageRef::new("accepted-boot-recovery-activation")
                .expect("accepted message ref"),
        })
    }

    // Blocking-mode `Settled` recovery (`drain_settled_group`) always
    // resumes the parent. This double has no real turn machinery to resume
    // into, so it returns the benign "already past this gate" shape
    // (`from` in `resolver.rs`'s `is_benign_already_resumed` set) —
    // `resume_parent` treats that as success, letting the group-drain path
    // complete and prove itself via `ResolveOutcome::Resumed`, which only
    // that path ever produces.
    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        Err(TurnError::InvalidTransition {
            from: TurnStatus::Completed,
            to: TurnStatus::Running,
        })
    }

    async fn retry_turn(&self, _request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        unreachable!("boot recovery tests do not retry turns")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        unreachable!("boot recovery tests do not cancel runs")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        unreachable!("boot recovery tests do not read run state")
    }
}

/// `AgentTurnSpawnTreeRuntimePort` double reporting no live run for the
/// parent thread — boot recovery always finds a parked (not live) parent, so
/// `deliver_background`'s attend step always falls to `activate_parked_parent`.
struct NoLiveRunRuntime;

#[async_trait]
impl ironclaw_turns::AgentTurnRuntimePort for NoLiveRunRuntime {
    async fn submit_turn(
        &self,
        _request: ironclaw_turns::SubmitTurnRequest,
        _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        _run_profile_resolver: &dyn ironclaw_loop_contracts::RunProfileResolver,
    ) -> Result<ironclaw_turns::SubmitTurnResponse, TurnError> {
        unreachable!("boot recovery tests do not submit turns")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ironclaw_turns::ResumeTurnResponse, TurnError> {
        unreachable!("boot recovery tests do not resume turns")
    }

    async fn retry_turn(
        &self,
        request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        Err(TurnError::RunNotRetryable {
            run_id: request.run_id,
        })
    }

    async fn request_cancel(
        &self,
        _request: ironclaw_turns::CancelRunRequest,
    ) -> Result<ironclaw_turns::CancelRunResponse, TurnError> {
        unreachable!("boot recovery tests do not cancel")
    }

    async fn get_run_state(
        &self,
        _request: GetRunStateRequest,
    ) -> Result<ironclaw_turns::TurnRunState, TurnError> {
        unreachable!("boot recovery tests do not get run state")
    }

    async fn recent_runs_for_thread(
        &self,
        _scope: &TurnScope,
        _limit: u32,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl AgentTurnSpawnTreeRuntimePort for NoLiveRunRuntime {
    async fn submit_child_turn(
        &self,
        _request: ironclaw_turns::SubmitChildRunRequest,
        _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        _run_profile_resolver: &dyn ironclaw_loop_contracts::RunProfileResolver,
    ) -> Result<ironclaw_turns::SubmitTurnResponse, TurnError> {
        unreachable!("boot recovery tests do not submit child turns")
    }

    async fn children_of(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        Ok(Vec::new())
    }

    async fn get_run_record(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        unreachable!("boot recovery's deliver_background re-drive never reads a run record")
    }

    async fn reserve_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        _cap: u32,
    ) -> Result<ironclaw_turns::SpawnTreeReservation, TurnError> {
        Ok(ironclaw_turns::SpawnTreeReservation {
            scope: scope.clone(),
            root_run_id,
            descendant_count: u64::from(delta),
            released_children: std::collections::BTreeSet::new(),
        })
    }

    async fn release_tree_descendants(
        &self,
        _scope: &TurnScope,
        _root_run_id: TurnRunId,
        _delta: u32,
        _idempotency_key: TurnRunId,
    ) -> Result<(), TurnError> {
        Ok(())
    }

    async fn prune_released_child(
        &self,
        _scope: &TurnScope,
        _root_run_id: TurnRunId,
        _child_run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        Ok(())
    }
}

#[derive(Default)]
struct RecordingUpdateWriter {
    updates: std::sync::Mutex<Vec<serde_json::Value>>,
}

#[async_trait]
impl ironclaw_loop_host::LoopCapabilityResultWriter for RecordingUpdateWriter {
    async fn write_capability_result(
        &self,
        _write: ironclaw_loop_host::CapabilityResultWrite<'_>,
    ) -> Result<
        ironclaw_loop_host::CapabilityWriteResult,
        ironclaw_loop_contracts::AgentLoopHostError,
    > {
        Err(ironclaw_loop_contracts::AgentLoopHostError::new(
            ironclaw_loop_contracts::AgentLoopHostErrorKind::InvalidInvocation,
            "write is not used by boot recovery tests",
        ))
    }

    async fn update_capability_result(
        &self,
        _run_context: &ironclaw_loop_contracts::LoopRunContext,
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

/// Harness for `recover_scope` tests: one parent thread/scope/run over the
/// real process journal, plus a resolver wired the way production wires one
/// — except the runtime always reports no live run (boot recovery only ever
/// finds a parked parent) and the coordinator only implements `activate`.
struct RecoveryFixture {
    resolver: Arc<AwaitEdgeResolver<ironclaw_threads::InMemorySessionThreadService>>,
    edge_store: Arc<AwaitEdgeStore>,
    process_store: Arc<ProcessJournalStore<ironclaw_filesystem::InMemoryBackend>>,
    thread_service: Arc<ironclaw_threads::InMemorySessionThreadService>,
    coordinator: Arc<RecordingActivateCoordinator>,
    parent_scope: TurnScope,
    parent_run_id: TurnRunId,
    parent_context: ironclaw_loop_contracts::LoopRunContext,
}

impl RecoveryFixture {
    /// Opens one dependency edge (`Open`) directly against the real journal,
    /// sharing this fixture's parent. `mode` selects blocking vs background;
    /// background edges get the exact `bg:{thread_id}` group tag production
    /// writes, blocking edges get their own `gate_ref` (the pre-existing
    /// blocking-mode group key).
    async fn open_edge(&self, suffix: &str, mode: SpawnSubagentMode) -> (TurnRunId, TurnScope) {
        self.open_edge_for(
            &self.parent_scope,
            self.parent_run_id,
            &self.parent_context,
            suffix,
            mode,
            true,
        )
        .await
    }

    /// Same as `open_edge`, generalized over which parent the edge belongs
    /// to (for cross-scope sweep tests, which need a second, independent
    /// parent sharing this fixture's process/thread stores and resolver —
    /// see `spawn_extra_parent`) and whether the child thread is ensured
    /// (`ensure_child_thread: false` starves `deliver_background`'s first
    /// read of a thread to read, forcing exactly that one edge's delivery to
    /// fail — used by the "one edge fails, the rest of the pass continues"
    /// sweep test).
    #[allow(clippy::too_many_arguments)]
    async fn open_edge_for(
        &self,
        parent_scope: &TurnScope,
        parent_run_id: TurnRunId,
        parent_context: &ironclaw_loop_contracts::LoopRunContext,
        suffix: &str,
        mode: SpawnSubagentMode,
        ensure_child_thread: bool,
    ) -> (TurnRunId, TurnScope) {
        let child_run_id = TurnRunId::new();
        let child_thread_id =
            ThreadId::new(format!("recovery-child-thread-{suffix}")).expect("child thread");
        let tenant_id = parent_scope.tenant_id.clone();
        let agent_id = parent_scope.agent_id.clone();
        let owner_user_id = parent_scope
            .explicit_owner_user_id()
            .cloned()
            .expect("fixture parent scope carries an explicit owner");
        let child_scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            agent_id.clone(),
            None,
            child_thread_id.clone(),
            Some(owner_user_id.clone()),
        );
        // `child_terminal_output` (deliver_background's step 1) reads the
        // child thread's latest message — the thread itself must exist even
        // when nothing was ever posted to it (an unknown thread fails
        // closed, an empty one just yields no final text).
        if ensure_child_thread {
            self.thread_service
                .ensure_thread(EnsureThreadRequest {
                    scope: ThreadScope {
                        tenant_id,
                        agent_id: agent_id
                            .clone()
                            .expect("fixture parent scope carries an agent id"),
                        project_id: None,
                        owner_user_id: Some(owner_user_id.clone()),
                        mission_id: None,
                    },
                    thread_id: Some(child_thread_id.clone()),
                    created_by_actor_id: owner_user_id.to_string(),
                    title: None,
                    metadata_json: None,
                })
                .await
                .expect("ensure child thread");
        }
        let parent_process_id = ProcessId::from_uuid(parent_run_id.as_uuid());
        let gate_ref =
            TurnGateRef::new(format!("gate:recovery-{suffix}")).expect("gate ref for edge");
        let result_ref =
            LoopResultRef::new(format!("result:recovery-{suffix}")).expect("result ref");
        let group_ref = match mode {
            SpawnSubagentMode::Background => format!("bg:{}", parent_scope.thread_id),
            SpawnSubagentMode::Blocking => gate_ref.as_str().to_string(),
        };
        if mode == SpawnSubagentMode::Blocking {
            // `drain_settled_group` (blocking-mode recovery) writes the
            // child's result via `update_tool_result_reference`, which
            // updates an existing placeholder keyed on
            // `(parent_thread_id, parent_run_id, result_ref)` — the same one
            // the real spawn tool call seeds. Recovery never creates this
            // placeholder itself, so the fixture must.
            self.thread_service
                .append_tool_result_reference(ironclaw_threads::AppendToolResultReferenceRequest {
                    intrinsic_outcome: None,
                    scope: ThreadScope {
                        tenant_id: parent_scope.tenant_id.clone(),
                        agent_id: parent_scope
                            .agent_id
                            .clone()
                            .expect("fixture parent scope carries an agent id"),
                        project_id: None,
                        owner_user_id: parent_scope.explicit_owner_user_id().cloned(),
                        mission_id: None,
                    },
                    thread_id: parent_scope.thread_id.clone(),
                    turn_run_id: parent_run_id.to_string(),
                    result_ref: result_ref.as_str().to_string(),
                    safe_summary: ironclaw_threads::ToolResultSafeSummary::new(
                        "subagent still running",
                    )
                    .expect("initial summary"),
                    provider_call: None,
                    model_observation: None,
                })
                .await
                .expect("seed parent tool-result-reference placeholder");
        }
        let submitted = AwaitedChildSetRecord {
            gate_ref,
            parent_run_context: parent_context.clone(),
            tree_root_run_id: parent_run_id,
            child_scope: child_scope.clone(),
            child_run_id,
            child_thread_id: child_thread_id.clone(),
            subagent_kind: SubagentKindId::new("general").expect("kind"),
            spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                .expect("capability"),
            spawn_provider_call_id: Some(format!("spawn-call-recovery-{suffix}")),
            result_ref,
            mode,
        };
        self.process_store
            .submit_process(SubmitProcessRequest {
                process_id: ProcessId::from_uuid(child_run_id.as_uuid()),
                process_kind: ProcessKind::AgentTurn,
                scope: child_scope.to_resource_scope(),
                exclusive_within_scope: false,
                operation_id: Some(ProcessOperationId::from_trusted(format!(
                    "recovery-child-{suffix}"
                ))),
                owner_user_id: Some(owner_user_id),
                concurrency_class: None,
                parent_process_id: Some(parent_process_id),
                root_process_id: Some(parent_process_id),
                spawn_tree_descendant_cap: Some(16),
                dependency: Some(ProcessDependencySubmission {
                    dependent_process_id: parent_process_id,
                    root_process_id: parent_process_id,
                    group_ref: Some(group_ref),
                    metadata: serde_json::to_value(submitted).expect("edge metadata"),
                }),
                checkpoint_ref: None,
                input: None,
                created_at: chrono::Utc::now(),
                metadata: serde_json::Value::Null,
            })
            .await
            .expect("submit child process");
        (child_run_id, child_scope)
    }

    /// Submits a second, independent parent process + thread sharing this
    /// fixture's process store, thread service, and resolver — for
    /// cross-scope sweep tests. `sweep_unclosed_background_edges` scans
    /// across every scope in one pass; this is the only way to prove that
    /// without standing up a second full fixture (a second resolver would
    /// hide the "one sweep, many scopes" property under test).
    async fn spawn_extra_parent(
        &self,
        suffix: &str,
    ) -> (
        TurnScope,
        TurnRunId,
        ironclaw_loop_contracts::LoopRunContext,
    ) {
        self.spawn_extra_parent_with_run_id(suffix, TurnRunId::new())
            .await
    }

    /// Same as `spawn_extra_parent`, but with the run id supplied by the
    /// caller instead of generated randomly — see
    /// `recovery_fixture_with_parent_run_id` for why.
    async fn spawn_extra_parent_with_run_id(
        &self,
        suffix: &str,
        parent_run_id: TurnRunId,
    ) -> (
        TurnScope,
        TurnRunId,
        ironclaw_loop_contracts::LoopRunContext,
    ) {
        let tenant_id = self.parent_scope.tenant_id.clone();
        let agent_id = self.parent_scope.agent_id.clone();
        let owner_user_id =
            UserId::new(format!("recovery-owner-{suffix}")).expect("extra parent owner");
        let parent_thread_id =
            ThreadId::new(format!("recovery-parent-thread-{suffix}")).expect("parent thread");
        let parent_scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            agent_id.clone(),
            None,
            parent_thread_id.clone(),
            Some(owner_user_id.clone()),
        );
        let parent_process_id = ProcessId::from_uuid(parent_run_id.as_uuid());
        self.process_store
            .submit_process(SubmitProcessRequest {
                process_id: parent_process_id,
                process_kind: ProcessKind::AgentTurn,
                scope: parent_scope.to_resource_scope(),
                exclusive_within_scope: false,
                operation_id: None,
                owner_user_id: Some(owner_user_id.clone()),
                concurrency_class: None,
                parent_process_id: None,
                root_process_id: None,
                spawn_tree_descendant_cap: None,
                dependency: None,
                checkpoint_ref: None,
                input: None,
                created_at: chrono::Utc::now(),
                metadata: serde_json::Value::Null,
            })
            .await
            .expect("submit extra parent process");

        self.thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: ThreadScope {
                    tenant_id,
                    agent_id: agent_id.expect("fixture parent scope carries an agent id"),
                    project_id: None,
                    owner_user_id: Some(owner_user_id.clone()),
                    mission_id: None,
                },
                thread_id: Some(parent_thread_id.clone()),
                created_by_actor_id: owner_user_id.to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("ensure extra parent thread");

        let mut parent_context = ironclaw_agent_loop::test_support::test_run_context(&format!(
            "recovery-parent-{suffix}"
        ));
        parent_context.scope = parent_scope.clone();
        parent_context.thread_id = parent_thread_id;
        parent_context.run_id = parent_run_id;
        parent_context.actor = Some(TurnActor::new(owner_user_id));

        (parent_scope, parent_run_id, parent_context)
    }
}

async fn recovery_fixture(profile_id: ironclaw_host_api::turn::RunProfileId) -> RecoveryFixture {
    recovery_fixture_with_parent_run_id(profile_id, TurnRunId::new()).await
}

/// Same as `recovery_fixture`, but with the fixture's parent run id supplied
/// by the caller instead of generated randomly — needed by
/// `sweep_is_fair_across_scopes_when_one_scope_exceeds_the_cap`, which must
/// control the scan's sort order (`dependent_process_id`, derived from this
/// run id) rather than leave it to chance.
async fn recovery_fixture_with_parent_run_id(
    profile_id: ironclaw_host_api::turn::RunProfileId,
    parent_run_id: TurnRunId,
) -> RecoveryFixture {
    let tenant_id = TenantId::new("recovery-tenant").expect("tenant");
    let agent_id = AgentId::new("recovery-agent").expect("agent");
    let owner_user_id = UserId::new("recovery-owner").expect("owner");
    let parent_thread_id = ThreadId::new("recovery-parent-thread").expect("parent thread");
    let parent_scope = TurnScope::new_with_owner(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        parent_thread_id.clone(),
        Some(owner_user_id.clone()),
    );
    let parent_process_id = ProcessId::from_uuid(parent_run_id.as_uuid());

    let process_store = Arc::new(ironclaw_processes::in_memory_backed_process_store());
    process_store
        .submit_process(SubmitProcessRequest {
            process_id: parent_process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: parent_scope.to_resource_scope(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(owner_user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit parent process");

    let dependencies = Arc::clone(&process_store)
        as Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >;
    let edge_store = Arc::new(AwaitEdgeStore::new(dependencies));

    let thread_service = Arc::new(ironclaw_threads::InMemorySessionThreadService::default());
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: ThreadScope {
                tenant_id: tenant_id.clone(),
                agent_id: agent_id.clone(),
                project_id: None,
                owner_user_id: Some(owner_user_id.clone()),
                mission_id: None,
            },
            thread_id: Some(parent_thread_id.clone()),
            created_by_actor_id: owner_user_id.to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("ensure parent thread");

    let mut parent_context = ironclaw_agent_loop::test_support::test_run_context("recovery-parent");
    parent_context.scope = parent_scope.clone();
    parent_context.thread_id = parent_thread_id;
    parent_context.run_id = parent_run_id;
    parent_context.actor = Some(TurnActor::new(owner_user_id));
    parent_context.resolved_run_profile.profile_id = profile_id;

    let runtime = Arc::new(NoLiveRunRuntime) as Arc<dyn AgentTurnSpawnTreeRuntimePort>;
    let result_writer: Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter> =
        Arc::new(RecordingUpdateWriter::default());
    let resolver = Arc::new(AwaitEdgeResolver::new_unbound(
        Arc::clone(&edge_store),
        runtime,
        result_writer,
        Arc::clone(&thread_service),
    ));
    let coordinator = Arc::new(RecordingActivateCoordinator::default());
    resolver
        .bind_coordinator(Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>)
        .expect("bind coordinator");

    RecoveryFixture {
        resolver,
        edge_store,
        process_store,
        thread_service,
        coordinator,
        parent_scope,
        parent_run_id,
        parent_context,
    }
}

/// A `Settled` background edge recovers through the full `deliver_background`
/// flow: append the framed result, then activate the parked parent with
/// `System` provenance, preserving the parent's own resolved run profile.
#[tokio::test]
async fn recover_scope_delivers_a_settled_background_edge_through_activation() {
    let profile_id = ironclaw_host_api::turn::RunProfileId::long_running_mission();
    let fixture = recovery_fixture(profile_id.clone()).await;
    let (child_run_id, child_scope) = fixture
        .open_edge("settled", SpawnSubagentMode::Background)
        .await;
    fixture
        .edge_store
        .settle(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(7),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");

    let report = recover_scope(
        &fixture.resolver,
        &fixture.edge_store,
        &fixture.parent_scope,
    )
    .await;
    assert_eq!(report.failed, 0, "recovery must not fail this edge");
    assert_eq!(report.drained, 1);

    assert!(
        fixture
            .edge_store
            .peek(&child_scope, fixture.parent_run_id, child_run_id)
            .await
            .expect("peek edge")
            .is_none(),
        "a recovered edge must close"
    );
    let activations = fixture.coordinator.activations();
    assert_eq!(activations.len(), 1, "recovery must activate exactly once");
    assert_eq!(activations[0].provenance, ActivationProvenance::System);
    assert_eq!(
        activations[0].requested_run_profile, None,
        "the parent's own resolved run profile must survive recovery"
    );
    assert_eq!(
        activations[0]
            .resolved_run_profile
            .as_ref()
            .map(|profile| &profile.profile_id),
        Some(&profile_id),
        "recovery must carry the parent's full profile snapshot"
    );
}

/// A `ResultAppended` background edge recovers by re-attending (append is a
/// no-op replay) and activating the parked parent.
#[tokio::test]
async fn recover_scope_delivers_a_result_appended_background_edge() {
    let fixture = recovery_fixture(ironclaw_host_api::turn::RunProfileId::default_profile()).await;
    let (child_run_id, child_scope) = fixture
        .open_edge("result-appended", SpawnSubagentMode::Background)
        .await;
    fixture
        .edge_store
        .settle(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(5),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");
    fixture
        .edge_store
        .record_result_appended(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            LoopMessageRef::new(format!(
                "msg:{}",
                ironclaw_threads::ThreadMessageId::new().as_uuid()
            ))
            .expect("valid ref"),
        )
        .await
        .expect("append")
        .expect("edge exists");

    let report = recover_scope(
        &fixture.resolver,
        &fixture.edge_store,
        &fixture.parent_scope,
    )
    .await;
    assert_eq!(report.failed, 0);
    assert_eq!(report.drained, 1);
    assert!(
        fixture
            .edge_store
            .peek(&child_scope, fixture.parent_run_id, child_run_id)
            .await
            .expect("peek edge")
            .is_none(),
        "a recovered edge must close"
    );
    assert_eq!(fixture.coordinator.activations().len(), 1);
}

/// An `AttentionScheduled` background edge closes with no duplicate
/// delivery — attention is already durable, recovery must not call
/// `activate` again.
#[tokio::test]
async fn recover_scope_closes_attention_scheduled_edge_without_duplicate_delivery() {
    let fixture = recovery_fixture(ironclaw_host_api::turn::RunProfileId::default_profile()).await;
    let (child_run_id, child_scope) = fixture
        .open_edge("attention-scheduled", SpawnSubagentMode::Background)
        .await;
    fixture
        .edge_store
        .settle(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(3),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");
    fixture
        .edge_store
        .record_result_appended(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            LoopMessageRef::new(format!(
                "msg:{}",
                ironclaw_threads::ThreadMessageId::new().as_uuid()
            ))
            .expect("valid ref"),
        )
        .await
        .expect("append")
        .expect("edge exists");
    fixture
        .edge_store
        .record_attention(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            AttentionOutcome::Activated,
        )
        .await
        .expect("attention recorded")
        .expect("edge exists");

    let report = recover_scope(
        &fixture.resolver,
        &fixture.edge_store,
        &fixture.parent_scope,
    )
    .await;
    assert_eq!(report.failed, 0);
    assert_eq!(report.drained, 1);
    assert!(
        fixture
            .edge_store
            .peek(&child_scope, fixture.parent_run_id, child_run_id)
            .await
            .expect("peek edge")
            .is_none(),
        "an already-attended edge must close"
    );
    assert!(
        fixture.coordinator.activations().is_empty(),
        "attention is already durable — recovery must not activate again"
    );
}

/// Re-running `recover_scope` after every edge closed is a no-op: no more
/// candidates, no duplicate activation.
#[tokio::test]
async fn recover_scope_is_idempotent_on_a_second_pass() {
    let fixture = recovery_fixture(ironclaw_host_api::turn::RunProfileId::default_profile()).await;
    let (child_run_id, child_scope) = fixture
        .open_edge("idempotent", SpawnSubagentMode::Background)
        .await;
    fixture
        .edge_store
        .settle(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(2),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");

    let first = recover_scope(
        &fixture.resolver,
        &fixture.edge_store,
        &fixture.parent_scope,
    )
    .await;
    assert_eq!(first.drained, 1);
    assert_eq!(fixture.coordinator.activations().len(), 1);

    let second = recover_scope(
        &fixture.resolver,
        &fixture.edge_store,
        &fixture.parent_scope,
    )
    .await;
    assert_eq!(second.drained, 0);
    assert_eq!(second.failed, 0);
    assert_eq!(
        fixture.coordinator.activations().len(),
        1,
        "a second pass over an already-closed edge must not activate again"
    );
}

#[tokio::test]
async fn check_scope_recovered_rejects_when_recovery_reports_a_failure() {
    let fixture = recovery_fixture(ironclaw_host_api::turn::RunProfileId::default_profile()).await;
    let (child_run_id, child_scope) = fixture
        .open_edge("failed-admission", SpawnSubagentMode::Background)
        .await;
    fixture
        .edge_store
        .settle(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(2),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");

    // Leave the resolver's coordinator unbound so recovery reaches a real
    // delivery failure after the durable edge is found.
    let resolver = Arc::new(AwaitEdgeResolver::new_unbound(
        Arc::clone(&fixture.edge_store),
        Arc::new(NoLiveRunRuntime),
        Arc::new(RecordingUpdateWriter::default()),
        Arc::clone(&fixture.thread_service),
    ));
    let recovery_driver = ScopeRecoveryDriver::new(resolver, Arc::clone(&fixture.edge_store));

    let result = ironclaw_loop_host::AwaitEdgeWriter::check_scope_recovered(
        &recovery_driver,
        &fixture.parent_scope,
    )
    .await;
    assert!(
        result.is_err(),
        "scope admission must reject when recovery reports a failure"
    );
}

/// Blocking-mode edges keep their pre-existing `drain_settled_group` path —
/// unaffected by the background delivery-chain arms this task adds.
#[tokio::test]
async fn recover_scope_drains_a_blocking_mode_settled_edge_through_the_group_path() {
    let fixture = recovery_fixture(ironclaw_host_api::turn::RunProfileId::default_profile()).await;
    let (child_run_id, child_scope) = fixture
        .open_edge("blocking", SpawnSubagentMode::Blocking)
        .await;
    fixture
        .edge_store
        .settle(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(1),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");

    let report = recover_scope(
        &fixture.resolver,
        &fixture.edge_store,
        &fixture.parent_scope,
    )
    .await;
    // `ResolveOutcome::Resumed` is produced only by `drain_settled_group`
    // (`deliver_background`'s re-drive only ever yields `Drained`) — seeing
    // it here proves the blocking-mode edge took the pre-existing group
    // path, not the background delivery arm this task adds.
    assert_eq!(report.failed, 0);
    assert_eq!(report.resumed, 1);
    assert_eq!(report.drained, 0);
    assert!(
        fixture.coordinator.activations().is_empty(),
        "the group path resumes the parent — it must never call activate()"
    );
    assert!(
        fixture
            .edge_store
            .peek(&child_scope, fixture.parent_run_id, child_run_id)
            .await
            .expect("peek edge")
            .is_none(),
        "the group path must close the edge too"
    );
}

/// The whole point of the sweep: edges in two different scopes are both
/// delivered by one call, with no per-thread call needed.
#[tokio::test]
async fn sweep_delivers_edges_across_two_scopes_in_one_pass() {
    let fixture = recovery_fixture(ironclaw_host_api::turn::RunProfileId::default_profile()).await;
    let (child_a, scope_a) = fixture
        .open_edge("sweep-scope-a", SpawnSubagentMode::Background)
        .await;
    fixture
        .edge_store
        .settle(
            &scope_a,
            fixture.parent_run_id,
            child_a,
            EdgeTerminalKind::Completed,
            Some(4),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");

    let (parent_scope_b, parent_run_id_b, parent_context_b) =
        fixture.spawn_extra_parent("sweep-scope-b").await;
    let (child_b, scope_b) = fixture
        .open_edge_for(
            &parent_scope_b,
            parent_run_id_b,
            &parent_context_b,
            "sweep-scope-b-child",
            SpawnSubagentMode::Background,
            true,
        )
        .await;
    fixture
        .edge_store
        .settle(
            &scope_b,
            parent_run_id_b,
            child_b,
            EdgeTerminalKind::Completed,
            Some(4),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");

    let report = sweep_unclosed_background_edges(&fixture.resolver, &fixture.edge_store).await;
    assert_eq!(report.failed, 0, "sweep must not fail either edge");
    assert_eq!(
        report.drained, 2,
        "one sweep call must deliver both scopes' edges"
    );

    assert!(
        fixture
            .edge_store
            .peek(&scope_a, fixture.parent_run_id, child_a)
            .await
            .expect("peek scope a edge")
            .is_none(),
        "scope a's edge must close"
    );
    assert!(
        fixture
            .edge_store
            .peek(&scope_b, parent_run_id_b, child_b)
            .await
            .expect("peek scope b edge")
            .is_none(),
        "scope b's edge must close in the same pass, with no per-thread call"
    );
    assert_eq!(
        fixture.coordinator.activations().len(),
        2,
        "both parked parents must be activated"
    );
}

/// A streak-capped edge stays parked at boot — a boot pass is neither a
/// permitted nor a human-initiated start.
#[tokio::test]
async fn sweep_leaves_a_streak_capped_edge_alone() {
    let fixture = recovery_fixture(ironclaw_host_api::turn::RunProfileId::default_profile()).await;
    let (child_run_id, child_scope) = fixture
        .open_edge("sweep-deferred", SpawnSubagentMode::Background)
        .await;
    fixture
        .edge_store
        .settle(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(3),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");
    fixture
        .edge_store
        .record_result_appended(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            LoopMessageRef::new(format!(
                "msg:{}",
                ironclaw_threads::ThreadMessageId::new().as_uuid()
            ))
            .expect("valid ref"),
        )
        .await
        .expect("append")
        .expect("edge exists");
    fixture
        .edge_store
        .defer_streak_capped(&child_scope, fixture.parent_run_id, child_run_id)
        .await
        .expect("defer")
        .expect("edge exists");

    let report = sweep_unclosed_background_edges(&fixture.resolver, &fixture.edge_store).await;
    assert_eq!(report.failed, 0);
    assert_eq!(
        report.drained, 0,
        "a streak-capped edge must not be drained by a boot pass"
    );
    assert!(
        fixture.coordinator.activations().is_empty(),
        "a boot pass must not activate a streak-capped edge's parent"
    );

    let edge = fixture
        .edge_store
        .peek(&child_scope, fixture.parent_run_id, child_run_id)
        .await
        .expect("peek edge")
        .expect("edge stays unclosed, parked");
    assert_eq!(edge.state, AwaitEdgeState::AttentionDeferredStreakCap);
}

/// An `AttentionScheduled` edge closes without a second delivery or a second
/// activate — the sweep's idempotent re-drive contract.
#[tokio::test]
async fn sweep_closes_attention_scheduled_edge_without_duplicate_delivery() {
    let fixture = recovery_fixture(ironclaw_host_api::turn::RunProfileId::default_profile()).await;
    let (child_run_id, child_scope) = fixture
        .open_edge("sweep-attention-scheduled", SpawnSubagentMode::Background)
        .await;
    fixture
        .edge_store
        .settle(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(3),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");
    fixture
        .edge_store
        .record_result_appended(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            LoopMessageRef::new(format!(
                "msg:{}",
                ironclaw_threads::ThreadMessageId::new().as_uuid()
            ))
            .expect("valid ref"),
        )
        .await
        .expect("append")
        .expect("edge exists");
    fixture
        .edge_store
        .record_attention(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            AttentionOutcome::Activated,
        )
        .await
        .expect("attention recorded")
        .expect("edge exists");

    let report = sweep_unclosed_background_edges(&fixture.resolver, &fixture.edge_store).await;
    assert_eq!(report.failed, 0);
    assert_eq!(report.drained, 1);
    assert!(
        fixture
            .edge_store
            .peek(&child_scope, fixture.parent_run_id, child_run_id)
            .await
            .expect("peek edge")
            .is_none(),
        "an already-attended edge must close"
    );
    assert!(
        fixture.coordinator.activations().is_empty(),
        "attention is already durable — the sweep must not activate again"
    );
}

/// Fairness: scope A has more settled edges than the per-pass delivery
/// budget allows; scope B's lone edge must still be served in the same
/// pass. Replacing the round-robin drain with a scope-by-scope drain would
/// spend the entire capped budget on scope A and leave scope B untouched —
/// this assertion is exactly what would fail under that regression.
#[tokio::test]
async fn sweep_is_fair_across_scopes_when_one_scope_exceeds_the_cap() {
    // Fixed (not `TurnRunId::new()`) run ids: the scan sorts by
    // `dependent_process_id`, derived from each parent's run id, so which
    // scope's records come first in scan order is otherwise a coin flip —
    // when the lone scope B record happened to sort first, a regression to
    // bucketing by `record.scope` (instead of `group_ref`) would still
    // deliver it early and this test would pass anyway. Pinning scope A's
    // (busy) run id below scope B's (lone) run id makes round-robin the only
    // way B is served within the delivery cap. Do not revert this to random
    // ids.
    let busy_parent_run_id = TurnRunId::from_uuid(uuid::Uuid::from_u128(1));
    let lone_parent_run_id = TurnRunId::from_uuid(uuid::Uuid::from_u128(2));

    let fixture = recovery_fixture_with_parent_run_id(
        ironclaw_host_api::turn::RunProfileId::default_profile(),
        busy_parent_run_id,
    )
    .await;

    let mut scope_a_edges = Vec::new();
    for index in 0..5 {
        let suffix = format!("fair-a-{index}");
        let (child_run_id, child_scope) = fixture
            .open_edge(&suffix, SpawnSubagentMode::Background)
            .await;
        fixture
            .edge_store
            .settle(
                &child_scope,
                fixture.parent_run_id,
                child_run_id,
                EdgeTerminalKind::Completed,
                Some(1),
                None,
            )
            .await
            .expect("settle")
            .expect("edge exists");
        scope_a_edges.push((child_run_id, child_scope));
    }

    let (parent_scope_b, parent_run_id_b, parent_context_b) = fixture
        .spawn_extra_parent_with_run_id("fair-b", lone_parent_run_id)
        .await;
    let (child_b, scope_b) = fixture
        .open_edge_for(
            &parent_scope_b,
            parent_run_id_b,
            &parent_context_b,
            "fair-b-child",
            SpawnSubagentMode::Background,
            true,
        )
        .await;
    fixture
        .edge_store
        .settle(
            &scope_b,
            parent_run_id_b,
            child_b,
            EdgeTerminalKind::Completed,
            Some(1),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");

    // scan_cap covers the whole backlog (6 edges) so nothing is truncated
    // before bucketing; the delivery cap (3) is what fairness has to work
    // under.
    let report =
        sweep_unclosed_background_edges_bounded(&fixture.resolver, &fixture.edge_store, 32, 3, 32)
            .await;
    assert_eq!(report.failed, 0);
    assert_eq!(report.drained, 3, "the delivery cap bounds one pass's work");

    assert!(
        fixture
            .edge_store
            .peek(&scope_b, parent_run_id_b, child_b)
            .await
            .expect("peek scope b edge")
            .is_none(),
        "round-robin must serve scope b within the same capped pass, \
         not starve it behind scope a's larger backlog"
    );

    let mut scope_a_closed = 0;
    for (child_run_id, child_scope) in &scope_a_edges {
        if fixture
            .edge_store
            .peek(child_scope, fixture.parent_run_id, *child_run_id)
            .await
            .expect("peek scope a edge")
            .is_none()
        {
            scope_a_closed += 1;
        }
    }
    assert_eq!(
        scope_a_closed, 2,
        "the cap leaves most of scope a's backlog for a later pass"
    );
}

/// The returned `ResolveReport` carries the right counts, including a
/// failure counted (not swallowed) when one edge's delivery errors — here,
/// an edge whose child thread was never created, so `deliver_background`'s
/// first read fails. The rest of the pass still proceeds.
#[tokio::test]
async fn sweep_report_counts_a_failed_edge_without_swallowing_it() {
    let fixture = recovery_fixture(ironclaw_host_api::turn::RunProfileId::default_profile()).await;

    let (failing_child, failing_scope) = fixture
        .open_edge_for(
            &fixture.parent_scope,
            fixture.parent_run_id,
            &fixture.parent_context,
            "sweep-failure",
            SpawnSubagentMode::Background,
            false,
        )
        .await;
    fixture
        .edge_store
        .settle(
            &failing_scope,
            fixture.parent_run_id,
            failing_child,
            EdgeTerminalKind::Completed,
            Some(2),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");

    let (parent_scope_b, parent_run_id_b, parent_context_b) =
        fixture.spawn_extra_parent("sweep-failure-b").await;
    let (ok_child, ok_scope) = fixture
        .open_edge_for(
            &parent_scope_b,
            parent_run_id_b,
            &parent_context_b,
            "sweep-failure-b-child",
            SpawnSubagentMode::Background,
            true,
        )
        .await;
    fixture
        .edge_store
        .settle(
            &ok_scope,
            parent_run_id_b,
            ok_child,
            EdgeTerminalKind::Completed,
            Some(2),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");

    let report = sweep_unclosed_background_edges(&fixture.resolver, &fixture.edge_store).await;
    assert_eq!(
        report.failed, 1,
        "the failing edge's delivery error must be counted, not swallowed"
    );
    assert_eq!(
        report.drained, 1,
        "the other scope's edge must still be delivered despite the failure"
    );
    assert!(
        fixture
            .edge_store
            .peek(&failing_scope, fixture.parent_run_id, failing_child)
            .await
            .expect("peek failing edge")
            .is_some(),
        "a failed delivery must leave its edge unclosed for a later pass"
    );
    assert!(
        fixture
            .edge_store
            .peek(&ok_scope, parent_run_id_b, ok_child)
            .await
            .expect("peek ok edge")
            .is_none(),
        "the unaffected scope's edge must still close"
    );
}

/// A second sweep over the same state is idempotent: nothing delivered
/// twice.
#[tokio::test]
async fn sweep_second_pass_over_same_state_is_idempotent() {
    let fixture = recovery_fixture(ironclaw_host_api::turn::RunProfileId::default_profile()).await;
    let (child_run_id, child_scope) = fixture
        .open_edge("sweep-idempotent", SpawnSubagentMode::Background)
        .await;
    fixture
        .edge_store
        .settle(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(2),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");

    let first = sweep_unclosed_background_edges(&fixture.resolver, &fixture.edge_store).await;
    assert_eq!(first.drained, 1);
    assert_eq!(fixture.coordinator.activations().len(), 1);

    let second = sweep_unclosed_background_edges(&fixture.resolver, &fixture.edge_store).await;
    assert_eq!(second.drained, 0);
    assert_eq!(second.failed, 0);
    assert_eq!(
        fixture.coordinator.activations().len(),
        1,
        "a second sweep over an already-closed edge must not activate again"
    );
}

/// The scan's keyset cursor still works when a backlog spans more than one
/// page — proven directly through the `_bounded` entry point with an
/// explicit small `page_size`, independent of the scan cap. See
/// `sweep_unclosed_background_edges`'s doc comment for why production always
/// requests one page sized to the scan cap instead.
#[tokio::test]
async fn sweep_scan_pages_through_a_backlog_larger_than_one_page() {
    let fixture = recovery_fixture(ironclaw_host_api::turn::RunProfileId::default_profile()).await;

    let mut edges = Vec::new();
    for index in 0..5 {
        let suffix = format!("paged-{index}");
        let (child_run_id, child_scope) = fixture
            .open_edge(&suffix, SpawnSubagentMode::Background)
            .await;
        fixture
            .edge_store
            .settle(
                &child_scope,
                fixture.parent_run_id,
                child_run_id,
                EdgeTerminalKind::Completed,
                Some(1),
                None,
            )
            .await
            .expect("settle")
            .expect("edge exists");
        edges.push((child_run_id, child_scope));
    }

    // page_size (2) is smaller than both the backlog (5 edges) and scan_cap
    // (32), forcing the scan loop through multiple `scan_unclosed_background`
    // calls and exercising the keyset cursor.
    let report =
        sweep_unclosed_background_edges_bounded(&fixture.resolver, &fixture.edge_store, 32, 32, 2)
            .await;
    assert_eq!(report.failed, 0);
    assert_eq!(
        report.drained, 5,
        "every paged-in edge must still be delivered"
    );

    for (child_run_id, child_scope) in &edges {
        assert!(
            fixture
                .edge_store
                .peek(child_scope, fixture.parent_run_id, *child_run_id)
                .await
                .expect("peek paged edge")
                .is_none(),
            "a multi-page scan must still close every backlogged edge"
        );
    }
}
