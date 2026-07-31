use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::ids::{ProcessId, TenantId, ThreadId};
use ironclaw_processes::{
    ClaimProcessesRequest, ClaimedProcess, FailProcessRequest, JournaledProcessSnapshot,
    ProcessJournalCursor, ProcessLeaseRequest, ProcessLifecycleStatus,
    ProcessStateTransitionRequest, ProcessSuspension, ProcessTransitionPort,
    RecoverExpiredProcessLeasesRequest, RecoverExpiredProcessLeasesResponse, SuspendProcessRequest,
};
use ironclaw_threads::{
    AppendToolResultReferenceRequest, InMemorySessionThreadService, SessionThreadService,
    ThreadMessageRecord, ThreadScope, ToolResultSafeSummary,
};
use ironclaw_turns::{
    AcceptedMessageRef, AgentTurnRuntimePort, CancelRunRequest, CancelRunResponse, EventCursor,
    GetLoopCheckpointRequest, GetRunStateRequest, LoopBlocked, LoopBlockedKind, LoopCheckpointKind,
    LoopCheckpointRecord, LoopCheckpointStateRef, LoopCheckpointStore, LoopCompleted,
    LoopCompletionKind, LoopExit, LoopExitId, LoopGateRef, LoopMessageRef, LoopResultRef,
    PutLoopCheckpointRequest, RedactedCheckpointPayload, ReplyTargetBindingRef, ResumeTurnRequest,
    ResumeTurnResponse, RunProfileVersion, SanitizedFailure, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCheckpointId, TurnError, TurnGateRef, TurnId,
    TurnLeaseToken, TurnRunId, TurnRunState, TurnRunnerId, TurnScope, TurnStatus,
    run_profile::{CheckpointSchemaId, LoopDriverId},
    runner::ClaimedTurnRun,
};

use crate::loop_exit_applier::{
    AwaitDependentRunEvidenceStore, InMemoryLoopExitEvidencePort, LoopExitApplier,
    ThreadCheckpointLoopExitEvidencePort,
};

/// Always-empty test fixture — no filesystem/CAS store needed here, this
/// test module only ever wants "no awaited-child evidence exists."
struct NoAwaitDependentRunEvidence;

#[async_trait]
impl AwaitDependentRunEvidenceStore for NoAwaitDependentRunEvidence {
    async fn has_awaited_child_gate(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
        _gate_ref: &LoopGateRef,
    ) -> Result<bool, TurnError> {
        Ok(false)
    }
}

pub(super) fn empty_await_dependent_run_evidence() -> Arc<dyn AwaitDependentRunEvidenceStore> {
    Arc::new(NoAwaitDependentRunEvidence)
}

/// Minimal in-memory test double recording one `(scope, run_id, gate_ref,
/// mode)` tuple — enough for the two real-evidence tests in `mod.rs`
/// (accept a matching blocking-mode gate, reject a background-mode one)
/// without pulling in the process-journal-backed `AwaitEdgeStore`.
pub(super) struct RecordingAwaitDependentRunEvidence {
    scope: TurnScope,
    run_id: TurnRunId,
    gate_ref: TurnGateRef,
    mode: ironclaw_loop_host::SpawnSubagentMode,
}

impl RecordingAwaitDependentRunEvidence {
    pub(super) fn new(
        scope: TurnScope,
        run_id: TurnRunId,
        gate_ref: TurnGateRef,
        mode: ironclaw_loop_host::SpawnSubagentMode,
    ) -> Self {
        Self {
            scope,
            run_id,
            gate_ref,
            mode,
        }
    }
}

#[async_trait]
impl AwaitDependentRunEvidenceStore for RecordingAwaitDependentRunEvidence {
    async fn has_awaited_child_gate(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        gate_ref: &LoopGateRef,
    ) -> Result<bool, TurnError> {
        Ok(*scope == self.scope
            && run_id == self.run_id
            && gate_ref.as_str() == self.gate_ref.as_str()
            && self.mode == ironclaw_loop_host::SpawnSubagentMode::Blocking)
    }
}

pub(super) fn text_checkpoint_evidence(
    loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
) -> ThreadCheckpointLoopExitEvidencePort<InMemorySessionThreadService> {
    ThreadCheckpointLoopExitEvidencePort::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(StaticAgentTurnRuntime::new(claimed_run().state)),
        loop_checkpoint_store,
        empty_await_dependent_run_evidence(),
    )
}

pub(super) async fn put_final_checkpoint(
    store: &dyn LoopCheckpointStore,
    claimed: &ClaimedTurnRun,
    payload: Vec<u8>,
) -> LoopCheckpointRecord {
    let state_ref = LoopCheckpointStateRef::new(format!(
        "checkpoint:{}:{}",
        claimed.state.run_id,
        TurnRunId::new()
    ))
    .expect("valid run-scoped checkpoint ref");
    store
        .put_loop_checkpoint(PutLoopCheckpointRequest {
            scope: claimed.state.scope.clone(),
            turn_id: claimed.state.turn_id,
            run_id: claimed.state.run_id,
            state_ref,
            payload: RedactedCheckpointPayload::new(payload).expect("bounded checkpoint payload"),
            schema_id: claimed.resolved_run_profile.checkpoint_schema_id.clone(),
            schema_version: claimed.resolved_run_profile.checkpoint_schema_version,
            kind: LoopCheckpointKind::Final,
            gate_ref: None,
        })
        .await
        .expect("loop checkpoint")
}

pub(super) async fn append_tool_result_reference<S>(
    thread_service: &S,
    thread_scope: ThreadScope,
    thread_id: ThreadId,
    run_id: TurnRunId,
    result_ref: LoopResultRef,
) -> ThreadMessageRecord
where
    S: SessionThreadService + ?Sized,
{
    thread_service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            scope: thread_scope,
            thread_id,
            turn_run_id: run_id.to_string(),
            result_ref: result_ref.as_str().to_string(),
            safe_summary: ToolResultSafeSummary::new("tool completed").expect("safe summary"),
            provider_call: None,
            model_observation: None,
        })
        .await
        .expect("tool result reference")
}

/// Build a minimal `Running` run state for a given scope/run, carrying
/// the supplied authenticated actor. Used to exercise the applier's
/// per-caller owner resolution.
pub(super) fn running_run_state(
    scope: TurnScope,
    run_id: TurnRunId,
    actor: Option<TurnActor>,
) -> TurnRunState {
    TurnRunState {
        scope,
        actor,
        turn_id: TurnId::new(),
        run_id,
        status: TurnStatus::Running,
        accepted_message_ref: AcceptedMessageRef::new("msg:accepted").expect("valid"),
        source_binding_ref: SourceBindingRef::new("source").expect("valid"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply").expect("valid"),
        resolved_run_profile_id: ironclaw_turns::RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        model_usage: None,
        received_at: chrono::Utc::now(),
        checkpoint_id: None,
        gate_ref: None,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor(0),
        product_context: None,
        resume_disposition: None,
    }
}

pub(super) struct StaticAgentTurnRuntime {
    state: TurnRunState,
}

impl StaticAgentTurnRuntime {
    pub(super) fn new(state: TurnRunState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl AgentTurnRuntimePort for StaticAgentTurnRuntime {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        _run_profile_resolver: &dyn ironclaw_turns::RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("submit_turn should not be called by evidence tests")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("resume_turn should not be called by evidence tests")
    }

    async fn retry_turn(
        &self,
        request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        // WS-3 implements this.
        Err(TurnError::RunNotRetryable {
            run_id: request.run_id,
        })
    }

    async fn request_cancel(
        &self,
        _request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        panic!("request_cancel should not be called by evidence tests")
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        assert_eq!(request.scope, self.state.scope);
        assert_eq!(request.run_id, self.state.run_id);
        Ok(self.state.clone())
    }
}

pub(super) struct PanicLoopCheckpointStore;

#[async_trait]
impl LoopCheckpointStore for PanicLoopCheckpointStore {
    async fn put_loop_checkpoint(
        &self,
        _request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        panic!("put_loop_checkpoint should not be called by evidence tests")
    }

    async fn get_loop_checkpoint(
        &self,
        _request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        panic!("get_loop_checkpoint should not be called by fail-closed evidence tests")
    }
}

pub(super) struct StaticLoopCheckpointStore {
    record: Option<LoopCheckpointRecord>,
}

impl StaticLoopCheckpointStore {
    pub(super) fn new(record: LoopCheckpointRecord) -> Self {
        Self {
            record: Some(record),
        }
    }
}

#[async_trait]
impl LoopCheckpointStore for StaticLoopCheckpointStore {
    async fn put_loop_checkpoint(
        &self,
        _request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        panic!("put_loop_checkpoint should not be called by evidence tests")
    }

    async fn get_loop_checkpoint(
        &self,
        request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        Ok(self.record.as_ref().and_then(|record| {
            if record.scope == request.scope
                && record.turn_id == request.turn_id
                && record.run_id == request.run_id
                && record.checkpoint_id == request.checkpoint_id
            {
                Some(record.clone())
            } else {
                None
            }
        }))
    }
}

pub(super) fn test_exit_id() -> LoopExitId {
    LoopExitId::new("exit:test").expect("valid")
}

pub(super) fn completed_exit(
    reply_message_refs: Vec<LoopMessageRef>,
    final_checkpoint_id: Option<TurnCheckpointId>,
) -> LoopExit {
    LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::FinalReply,
        reply_message_refs,
        result_refs: vec![],
        final_checkpoint_id,
        model_usage: None,
        exit_id: test_exit_id(),
    })
}

pub(super) fn blocked_exit(kind: LoopBlockedKind) -> LoopExit {
    blocked_exit_with_checkpoint(
        kind,
        TurnCheckpointId::new(),
        LoopCheckpointStateRef::new("checkpoint:blocked-state").expect("valid"),
    )
}

pub(super) fn blocked_exit_with_checkpoint(
    kind: LoopBlockedKind,
    checkpoint_id: TurnCheckpointId,
    state_ref: LoopCheckpointStateRef,
) -> LoopExit {
    LoopExit::Blocked(LoopBlocked {
        kind,
        gate_ref: LoopGateRef::new("gate:test").expect("valid"),
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        checkpoint_id,
        state_ref,
        exit_id: test_exit_id(),
    })
}

pub(super) fn loop_checkpoint_record(
    claimed: &ClaimedTurnRun,
    checkpoint_id: TurnCheckpointId,
    state_ref: LoopCheckpointStateRef,
    kind: LoopCheckpointKind,
) -> LoopCheckpointRecord {
    loop_checkpoint_record_with_gate(claimed, checkpoint_id, state_ref, kind, None)
}

pub(super) fn loop_checkpoint_record_with_gate(
    claimed: &ClaimedTurnRun,
    checkpoint_id: TurnCheckpointId,
    state_ref: LoopCheckpointStateRef,
    kind: LoopCheckpointKind,
    gate_ref: Option<LoopGateRef>,
) -> LoopCheckpointRecord {
    LoopCheckpointRecord {
        checkpoint_id,
        scope: claimed.state.scope.clone(),
        turn_id: claimed.state.turn_id,
        run_id: claimed.state.run_id,
        state_ref,
        payload: None,
        schema_id: claimed.resolved_run_profile.checkpoint_schema_id.clone(),
        schema_version: claimed.resolved_run_profile.checkpoint_schema_version,
        kind,
        gate_ref,
        created_at: chrono::Utc::now(),
    }
}

pub(super) struct Fixture {
    pub(super) claimed: ClaimedTurnRun,
    pub(super) transition: Arc<RecordingTransitionPort>,
    pub(super) applier: Arc<LoopExitApplier>,
}

impl Fixture {
    pub(super) fn new(evidence: InMemoryLoopExitEvidencePort) -> Self {
        let claimed = claimed_run();
        let transition = Arc::new(RecordingTransitionPort::new());
        let applier = Arc::new(LoopExitApplier::new(transition.clone(), Arc::new(evidence)));
        Self {
            claimed,
            transition,
            applier,
        }
    }
}

pub(super) fn claimed_run() -> ClaimedTurnRun {
    let descriptor = ironclaw_turns::AgentLoopDriverDescriptor {
        id: LoopDriverId::new("test_loop").expect("valid"),
        version: RunProfileVersion::new(1),
        checkpoint_schema_id: Some(CheckpointSchemaId::new("test_checkpoint").expect("valid")),
        checkpoint_schema_version: Some(RunProfileVersion::new(1)),
    };
    let scope = TurnScope::new(
        TenantId::new("tenant").expect("valid"),
        None,
        None,
        ThreadId::new("thread").expect("valid"),
    );
    let mut profile = test_profile(descriptor);
    profile.checkpoint_policy.require_final_checkpoint = false;
    profile.checkpoint_policy.allow_no_reply_completion = false;
    ClaimedTurnRun {
        state: TurnRunState {
            scope,
            actor: None,
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Running,
            accepted_message_ref: AcceptedMessageRef::new("msg:accepted").expect("valid"),
            source_binding_ref: SourceBindingRef::new("source").expect("valid"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply").expect("valid"),
            resolved_run_profile_id: ironclaw_turns::RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            model_usage: None,
            received_at: chrono::Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: EventCursor(0),
            product_context: None,
            resume_disposition: None,
        },
        resolved_run_profile: profile,
        subagent_depth: 0,
        spawn_tree_descendant_cap: None,
        runner_id: TurnRunnerId::new(),
        lease_token: TurnLeaseToken::new(),
    }
}

fn test_profile(
    descriptor: ironclaw_turns::AgentLoopDriverDescriptor,
) -> ironclaw_turns::ResolvedRunProfile {
    use ironclaw_turns::run_profile::*;
    use ironclaw_turns::*;

    ResolvedRunProfile {
        run_class_id: RunClassId::new("test_class").expect("valid"),
        profile_id: RunProfileId::default_profile(),
        profile_version: RunProfileVersion::new(1),
        loop_driver: descriptor.clone(),
        checkpoint_schema_id: descriptor.checkpoint_schema_id.clone().expect("schema"),
        checkpoint_schema_version: descriptor.checkpoint_schema_version.expect("version"),
        model_profile_id: ModelProfileId::new("test_model").expect("valid"),
        capability_surface_profile_id: CapabilitySurfaceProfileId::new("test_capabilities")
            .expect("valid"),
        context_profile_id: ContextProfileId::new("test_context").expect("valid"),
        steering_policy: SteeringPolicy {
            allow_steering: false,
            allow_interrupt: true,
            allow_driver_specific_nudges: false,
        },
        cancellation_policy: CancellationPolicy {
            allow_cancel: true,
            require_checkpoint_before_cancel: false,
        },
        checkpoint_policy: CheckpointPolicy {
            require_before_model: false,
            require_before_side_effect: false,
            require_before_block: true,
            max_checkpoint_bytes: 64 * 1024,
            require_final_checkpoint: false,
            allow_no_reply_completion: false,
        },
        resource_budget_policy: ResourceBudgetPolicy {
            tier: ResourceBudgetTier::new("test_tier").expect("valid"),
            max_model_calls: 32,
            max_capability_invocations: 64,
        },
        personal_context_policy: ironclaw_turns::run_profile::PersonalContextPolicy::Excluded,
        runtime_constraints: RuntimeProfileConstraints {
            allow_raw_runtime_backend_selection: false,
            allow_broad_capability_surface: false,
        },
        runner_pool_id: None,
        scheduling_class: SchedulingClass::new("interactive").expect("valid"),
        concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
        resolution_fingerprint: RunProfileFingerprint::new("test-fingerprint-v1").expect("valid"),
        provenance: RedactedRunProfileProvenance {
            sources: vec![],
            effective_privileges: vec![],
        },
    }
}

pub(super) struct RecordingTransitionPort {
    raw_failures: Mutex<Vec<String>>,
    apply_calls: Mutex<usize>,
}

impl Default for RecordingTransitionPort {
    fn default() -> Self {
        Self {
            raw_failures: Mutex::new(Vec::new()),
            apply_calls: Mutex::new(0),
        }
    }
}

impl RecordingTransitionPort {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn raw_failure_texts(&self) -> Vec<String> {
        self.raw_failures.lock().expect("lock").clone()
    }

    pub(super) fn apply_count(&self) -> usize {
        *self.apply_calls.lock().expect("lock")
    }
}

#[async_trait]
impl ProcessTransitionPort for RecordingTransitionPort {
    type Error = TurnError;

    async fn claim_next_processes(
        &self,
        _request: ClaimProcessesRequest,
    ) -> Result<Vec<ClaimedProcess>, TurnError> {
        Ok(Vec::new())
    }

    async fn heartbeat_process(
        &self,
        _request: ProcessLeaseRequest,
    ) -> Result<ProcessJournalCursor, TurnError> {
        Ok(ProcessJournalCursor(0))
    }

    async fn recover_expired_process_leases(
        &self,
        _request: RecoverExpiredProcessLeasesRequest,
    ) -> Result<RecoverExpiredProcessLeasesResponse, TurnError> {
        Ok(RecoverExpiredProcessLeasesResponse { recovered: vec![] })
    }

    async fn suspend_process(
        &self,
        request: SuspendProcessRequest,
    ) -> Result<JournaledProcessSnapshot, TurnError> {
        *self.apply_calls.lock().expect("lock") += 1;
        Ok(process_state_for_mapping(
            ProcessLifecycleStatus::Suspended,
            request.process_id,
            None,
            Some(request.suspension),
            Some(request.checkpoint_ref),
        ))
    }

    async fn complete_process(
        &self,
        request: ProcessStateTransitionRequest,
    ) -> Result<JournaledProcessSnapshot, TurnError> {
        *self.apply_calls.lock().expect("lock") += 1;
        let mut snapshot = process_state_for_mapping(
            ProcessLifecycleStatus::Completed,
            request.lease.process_id,
            None,
            None,
            None,
        );
        if let Some(metadata) = request.metadata {
            snapshot.metadata = metadata;
        }
        Ok(snapshot)
    }

    async fn cancel_process(
        &self,
        request: ProcessStateTransitionRequest,
    ) -> Result<JournaledProcessSnapshot, TurnError> {
        *self.apply_calls.lock().expect("lock") += 1;
        let mut snapshot = process_state_for_mapping(
            ProcessLifecycleStatus::Cancelled,
            request.lease.process_id,
            None,
            None,
            None,
        );
        if let Some(metadata) = request.metadata {
            snapshot.metadata = metadata;
        }
        Ok(snapshot)
    }

    async fn fail_process(
        &self,
        request: FailProcessRequest,
    ) -> Result<JournaledProcessSnapshot, TurnError> {
        *self.apply_calls.lock().expect("lock") += 1;
        self.raw_failures
            .lock()
            .expect("lock")
            .push(request.failure.category().to_string());
        Ok(process_state_for_mapping(
            ProcessLifecycleStatus::Failed,
            request.process_id,
            Some(request.failure),
            None,
            request.checkpoint_ref,
        ))
    }

    async fn relinquish_process(
        &self,
        request: ProcessLeaseRequest,
    ) -> Result<JournaledProcessSnapshot, TurnError> {
        Ok(process_state_for_mapping(
            ProcessLifecycleStatus::Queued,
            request.process_id,
            None,
            None,
            None,
        ))
    }
}

fn process_state_for_mapping(
    status: ProcessLifecycleStatus,
    process_id: ProcessId,
    failure: Option<SanitizedFailure>,
    suspension: Option<ProcessSuspension>,
    checkpoint_ref: Option<ironclaw_processes::ProcessCheckpointRef>,
) -> JournaledProcessSnapshot {
    let mut claimed = claimed_run();
    claimed.state.run_id = TurnRunId::from_uuid(process_id.as_uuid());
    let mut snapshot = ClaimedProcess::from(&claimed).state;
    snapshot.status = status;
    snapshot.failure = failure;
    snapshot.suspension = suspension;
    snapshot.checkpoint_ref = checkpoint_ref;
    snapshot
}
