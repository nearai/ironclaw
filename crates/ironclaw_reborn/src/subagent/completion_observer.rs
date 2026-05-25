use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;
use ironclaw_loop_support::{
    AwaitedChildSetRecord, DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID, LoopCapabilityResultWriter,
    SpawnSubagentMode, SubagentGateResolutionStore, SubagentSpawnGoalStore, SubagentThreadKind,
    SubagentThreadMetadata,
};
use ironclaw_threads::{
    MessageKind, MessageStatus, SessionThreadService, ThreadHistoryRequest, ThreadScope,
    ToolResultSafeSummary, UpdateToolResultReferenceRequest,
};
use ironclaw_turns::{
    CancelRunRequest, CancelRunResponse, GateRef, GetRunStateRequest, IdempotencyKey,
    ResumeTurnPrecondition, ResumeTurnRequest, ResumeTurnResponse, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnEventKind, TurnLifecycleEvent,
    TurnRunId, TurnRunRecord, TurnRunState, TurnStateStore, TurnStatus,
    run_profile::{AgentLoopHostError, LoopRunContext, sanitize_model_visible_text},
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordModelRouteSnapshotRequest, RecordRecoveryRequiredRequest,
        RecoverExpiredLeasesRequest, RecoverExpiredLeasesResponse, TurnRunTransitionPort,
    },
};

use crate::subagent::gate_resolution::{
    AwaitedChildTerminalEvent, BoundedSubagentGateResolutionStore,
};

#[derive(Clone)]
pub struct SubagentCompletionObserver<S: SessionThreadService + ?Sized> {
    gate_store: Arc<BoundedSubagentGateResolutionStore>,
    goal_store: Arc<dyn SubagentSpawnGoalStore>,
    turn_state_store: Arc<dyn TurnStateStore>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    coordinator: Arc<dyn TurnCoordinator>,
    thread_service: Arc<S>,
}

impl<S> SubagentCompletionObserver<S>
where
    S: SessionThreadService + ?Sized,
{
    pub fn new(
        gate_store: Arc<BoundedSubagentGateResolutionStore>,
        goal_store: Arc<dyn SubagentSpawnGoalStore>,
        turn_state_store: Arc<dyn TurnStateStore>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        coordinator: Arc<dyn TurnCoordinator>,
        thread_service: Arc<S>,
    ) -> Self {
        Self {
            gate_store,
            goal_store,
            turn_state_store,
            result_writer,
            coordinator,
            thread_service,
        }
    }

    async fn handle_terminal(&self, event: &TurnLifecycleEvent) -> Result<(), TurnError> {
        let has_gate_record = self
            .gate_store
            .flavor_id_for_child(event.run_id)
            .map_err(map_host_error)?
            .is_some();
        if !has_gate_record && !self.is_subagent_child(event).await? {
            return Ok(());
        }
        self.gate_store
            .record_child_terminal(event.run_id, terminal_event_from_lifecycle(event))
            .map_err(map_host_error)?;
        self.recover_missing_gate_record(event).await?;
        while let Some(state) = self
            .gate_store
            .claim_next_terminal_state_for_child(event.run_id)
            .map_err(map_host_error)?
        {
            if let Err(error) = self.handle_claimed_terminal_state(state).await {
                let (gate_ref, error) = error;
                let _ = self.gate_store.release_terminal_claim(&gate_ref);
                return Err(error);
            }
        }
        Ok(())
    }

    async fn handle_claimed_terminal_state(
        &self,
        state: crate::subagent::gate_resolution::AwaitedChildState,
    ) -> Result<(), (GateRef, TurnError)> {
        let terminal_event = state.terminal_event.ok_or_else(|| {
            (
                state.record.gate_ref.clone(),
                TurnError::Unavailable {
                    reason: "subagent gate replay selected state without terminal metadata"
                        .to_string(),
                },
            )
        })?;
        let result = async {
            match state.record.mode {
                SpawnSubagentMode::Blocking => {
                    self.write_terminal_result(&state.record, &terminal_event)
                        .await?;
                    self.resume_parent(&terminal_event, &state.record).await?;
                }
                SpawnSubagentMode::Background => {
                    self.write_terminal_result(&state.record, &terminal_event)
                        .await?;
                }
            }
            self.release_descendant_reservation(&state.record).await?;
            self.goal_store
                .delete_goal(&state.record.child_scope, state.record.child_run_id)
                .await
                .map_err(map_host_error)?;
            self.gate_store
                .mark_delivered(&state.record.gate_ref)
                .map_err(map_host_error)?;
            self.gate_store
                .delete_awaited_child(&state.record.gate_ref)
                .await
                .map_err(map_host_error)?;
            Ok(())
        }
        .await;
        result.map_err(|error| (state.record.gate_ref, error))
    }

    async fn is_subagent_child(&self, event: &TurnLifecycleEvent) -> Result<bool, TurnError> {
        let Some(record) = self
            .turn_state_store
            .get_run_record(&event.scope, event.run_id)
            .await?
        else {
            return Ok(false);
        };
        Ok(record.parent_run_id.is_some() && record.subagent_depth > 0)
    }

    pub async fn handle_terminal_state(
        &self,
        state: &TurnRunState,
        kind: TurnEventKind,
    ) -> Result<(), TurnError> {
        if !is_subagent_terminal_status(state.status) {
            return Ok(());
        }
        let owner_user_id = state.actor.as_ref().map(|actor| actor.user_id.clone());
        self.handle_terminal(&TurnLifecycleEvent {
            cursor: state.event_cursor,
            scope: state.scope.clone(),
            occurred_at: None,
            owner_user_id,
            run_id: state.run_id,
            status: state.status,
            kind,
            blocked_gate: None,
            sanitized_reason: None,
        })
        .await
    }

    async fn recover_missing_gate_record(
        &self,
        event: &TurnLifecycleEvent,
    ) -> Result<(), TurnError> {
        if self
            .gate_store
            .flavor_id_for_child(event.run_id)
            .map_err(map_host_error)?
            .is_some()
        {
            return Ok(());
        }
        let Some(record) = self.reconstruct_record(event).await? else {
            return Ok(());
        };
        self.gate_store
            .record_awaited_child(record)
            .await
            .map_err(map_host_error)?;
        self.gate_store
            .record_child_terminal(event.run_id, terminal_event_from_lifecycle(event))
            .map_err(map_host_error)?;
        Ok(())
    }

    async fn reconstruct_record(
        &self,
        event: &TurnLifecycleEvent,
    ) -> Result<Option<AwaitedChildSetRecord>, TurnError> {
        let Some(child_record) = self
            .turn_state_store
            .get_run_record(&event.scope, event.run_id)
            .await?
        else {
            return Ok(None);
        };
        let child_thread_scope = thread_scope_from_turn_scope(&child_record.scope, event)?;
        let child_thread = self
            .thread_service
            .read_thread(ThreadHistoryRequest {
                scope: child_thread_scope,
                thread_id: child_record.scope.thread_id.clone(),
            })
            .await
            .map_err(|error| TurnError::Unavailable {
                reason: format!("subagent thread metadata unavailable: {error}"),
            })?;
        let Some(metadata) = child_thread
            .metadata_json
            .as_deref()
            .and_then(parse_subagent_thread_metadata)
        else {
            return Ok(None);
        };
        if metadata.child_run_id != event.run_id {
            return Ok(None);
        }
        let parent_scope = ironclaw_turns::TurnScope::new(
            child_record.scope.tenant_id.clone(),
            child_record.scope.agent_id.clone(),
            child_record.scope.project_id.clone(),
            metadata.parent_thread_id.clone(),
        );
        let Some(parent_record) = self
            .turn_state_store
            .get_run_record(&parent_scope, metadata.parent_run_id)
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(awaited_child_record_from_persisted(
            parent_record,
            child_record,
            metadata,
        )?))
    }

    async fn release_descendant_reservation(
        &self,
        record: &ironclaw_loop_support::AwaitedChildSetRecord,
    ) -> Result<(), TurnError> {
        if !self
            .gate_store
            .mark_descendant_reservation_released(&record.gate_ref)
            .map_err(map_host_error)?
        {
            return Ok(());
        }
        self.turn_state_store
            .release_tree_descendants(&record.parent_scope, record.tree_root_run_id, 1)
            .await
    }

    async fn resume_parent(
        &self,
        event: &AwaitedChildTerminalEvent,
        record: &ironclaw_loop_support::AwaitedChildSetRecord,
    ) -> Result<(), TurnError> {
        let actor = actor_from_terminal_event(event)?;
        self.coordinator
            .resume_turn(ResumeTurnRequest {
                scope: record.parent_scope.clone(),
                actor,
                run_id: record.parent_run_id,
                gate_resolution_ref: record.gate_ref.clone(),
                source_binding_ref: record.source_binding_ref.clone(),
                reply_target_binding_ref: record.reply_target_binding_ref.clone(),
                idempotency_key: IdempotencyKey::new(format!(
                    "subagent-resume:{}:{}",
                    record.parent_run_id, record.child_run_id
                ))
                .map_err(|reason| TurnError::InvalidRequest { reason })?,
                // Pin the resume to the dependent-run gate so a child
                // termination cannot unblock a parent that is actually
                // waiting on an unrelated approval/auth/resource gate.
                precondition: ResumeTurnPrecondition::BlockedDependentRunGate,
            })
            .await
            .map(|_| ())
            .or_else(|error| match error {
                TurnError::Conflict { .. } | TurnError::InvalidTransition { .. } => Ok(()),
                other => Err(other),
            })?;
        Ok(())
    }

    async fn write_terminal_result(
        &self,
        record: &ironclaw_loop_support::AwaitedChildSetRecord,
        event: &AwaitedChildTerminalEvent,
    ) -> Result<(), TurnError> {
        let Some(result_ref) = record.background_result_ref.as_ref() else {
            return Err(TurnError::Unavailable {
                reason: "subagent result ref is missing".to_string(),
            });
        };
        let child_output = self.child_terminal_output(record, event).await?;
        let safe_summary = parent_result_summary(event, &child_output)?;
        let payload = background_completion_payload(event, record, &child_output);
        match self
            .result_writer
            .update_capability_result(&record.parent_run_context, result_ref, payload)
            .await
        {
            Ok(()) => {}
            Err(error) => return Err(map_host_error(error)),
        }
        self.update_parent_result_reference(record, event, result_ref, safe_summary)
            .await?;
        Ok(())
    }

    async fn child_terminal_output(
        &self,
        record: &ironclaw_loop_support::AwaitedChildSetRecord,
        event: &AwaitedChildTerminalEvent,
    ) -> Result<ChildTerminalOutput, TurnError> {
        let Some(agent_id) = record.child_scope.agent_id.clone() else {
            return Err(TurnError::InvalidRequest {
                reason: "child scope missing agent id for subagent result".to_string(),
            });
        };
        let child_thread_scope = ThreadScope {
            tenant_id: record.child_scope.tenant_id.clone(),
            agent_id,
            project_id: record.child_scope.project_id.clone(),
            owner_user_id: event.owner_user_id.clone(),
            mission_id: None,
        };
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: child_thread_scope,
                thread_id: record.child_thread_id.clone(),
            })
            .await
            .map_err(|error| TurnError::Unavailable {
                reason: format!("subagent child result history unavailable: {error}"),
            })?;
        let final_text = history
            .messages
            .iter()
            .rev()
            .find(|message| {
                message.kind == MessageKind::Assistant && message.status == MessageStatus::Finalized
            })
            .and_then(|message| message.content.clone());
        let failure_summary = match event.status {
            TurnStatus::Failed | TurnStatus::Cancelled | TurnStatus::RecoveryRequired => {
                event.sanitized_reason.clone()
            }
            _ => None,
        };
        Ok(ChildTerminalOutput {
            final_text,
            failure_summary,
        })
    }

    async fn update_parent_result_reference(
        &self,
        record: &ironclaw_loop_support::AwaitedChildSetRecord,
        event: &AwaitedChildTerminalEvent,
        result_ref: &ironclaw_turns::LoopResultRef,
        safe_summary: ToolResultSafeSummary,
    ) -> Result<(), TurnError> {
        let Some(agent_id) = record.parent_scope.agent_id.clone() else {
            return Err(TurnError::InvalidRequest {
                reason: "parent scope missing agent id for subagent result update".to_string(),
            });
        };
        let thread_scope = ThreadScope {
            tenant_id: record.parent_scope.tenant_id.clone(),
            agent_id,
            project_id: record.parent_scope.project_id.clone(),
            owner_user_id: event.owner_user_id.clone(),
            mission_id: None,
        };
        self.thread_service
            .update_tool_result_reference(UpdateToolResultReferenceRequest {
                scope: thread_scope,
                thread_id: record.parent_scope.thread_id.clone(),
                turn_run_id: record.parent_run_id.to_string(),
                result_ref: result_ref.as_str().to_string(),
                safe_summary,
            })
            .await
            .map_err(|error| TurnError::Unavailable {
                reason: format!("subagent result reference update failed: {error}"),
            })?;
        Ok(())
    }
}

pub struct SubagentCompletionTransitionPort<S: SessionThreadService + ?Sized> {
    inner: Arc<dyn TurnRunTransitionPort>,
    observer: Arc<SubagentCompletionObserver<S>>,
}

impl<S> SubagentCompletionTransitionPort<S>
where
    S: SessionThreadService + ?Sized,
{
    pub fn new(
        inner: Arc<dyn TurnRunTransitionPort>,
        observer: Arc<SubagentCompletionObserver<S>>,
    ) -> Self {
        Self { inner, observer }
    }
}

#[async_trait]
impl<S> TurnRunTransitionPort for SubagentCompletionTransitionPort<S>
where
    S: SessionThreadService + ?Sized,
{
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        self.inner.claim_next_run(request).await
    }

    async fn heartbeat(
        &self,
        request: HeartbeatRequest,
    ) -> Result<ironclaw_turns::EventCursor, TurnError> {
        self.inner.heartbeat(request).await
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        self.inner.recover_expired_leases(request).await
    }

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.inner.record_model_route_snapshot(request).await
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        self.inner.block_run(request).await
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        let state = self.inner.complete_run(request).await?;
        self.observer
            .handle_terminal_state(&state, TurnEventKind::Completed)
            .await?;
        Ok(state)
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.cancel_run(request).await?;
        self.observer
            .handle_terminal_state(&state, TurnEventKind::Cancelled)
            .await?;
        Ok(state)
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        let state = self.inner.fail_run(request).await?;
        self.observer
            .handle_terminal_state(&state, TurnEventKind::Failed)
            .await?;
        Ok(state)
    }

    async fn record_recovery_required(
        &self,
        request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.record_recovery_required(request).await?;
        self.observer
            .handle_terminal_state(&state, TurnEventKind::RecoveryRequired)
            .await?;
        Ok(state)
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.apply_validated_loop_exit(request).await?;
        let kind = match state.status {
            TurnStatus::Completed => TurnEventKind::Completed,
            TurnStatus::Cancelled => TurnEventKind::Cancelled,
            TurnStatus::Failed => TurnEventKind::Failed,
            _ => return Ok(state),
        };
        self.observer.handle_terminal_state(&state, kind).await?;
        Ok(state)
    }
}

pub struct SubagentCompletionCoordinator<S: SessionThreadService + ?Sized> {
    inner: Arc<dyn TurnCoordinator>,
    observer: Arc<SubagentCompletionObserver<S>>,
}

impl<S> SubagentCompletionCoordinator<S>
where
    S: SessionThreadService + ?Sized,
{
    pub fn new(
        inner: Arc<dyn TurnCoordinator>,
        observer: Arc<SubagentCompletionObserver<S>>,
    ) -> Self {
        Self { inner, observer }
    }
}

#[async_trait]
impl<S> TurnCoordinator for SubagentCompletionCoordinator<S>
where
    S: SessionThreadService + ?Sized,
{
    async fn prepare_turn(&self, scope: ironclaw_turns::TurnScope) -> Result<TurnRunId, TurnError> {
        self.inner.prepare_turn(scope).await
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.inner.submit_turn(request).await
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        self.inner.resume_turn(request).await
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        let scope = request.scope.clone();
        let response = self.inner.cancel_run(request).await?;
        if response.status.is_terminal() && !response.already_terminal {
            let state = self
                .inner
                .get_run_state(GetRunStateRequest {
                    scope,
                    run_id: response.run_id,
                })
                .await?;
            self.observer
                .handle_terminal_state(&state, TurnEventKind::Cancelled)
                .await?;
        }
        Ok(response)
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.inner.get_run_state(request).await
    }
}

fn actor_from_terminal_event(event: &AwaitedChildTerminalEvent) -> Result<TurnActor, TurnError> {
    let user_id = event
        .owner_user_id
        .clone()
        .ok_or_else(|| TurnError::InvalidRequest {
            reason: "subagent terminal event missing owner user id".to_string(),
        })?;
    Ok(TurnActor::new(user_id))
}

fn map_host_error(error: AgentLoopHostError) -> TurnError {
    TurnError::Unavailable {
        reason: error.safe_summary,
    }
}

fn is_subagent_terminal_status(status: TurnStatus) -> bool {
    status.is_terminal() || status == TurnStatus::RecoveryRequired
}

fn background_completion_payload(
    event: &AwaitedChildTerminalEvent,
    record: &ironclaw_loop_support::AwaitedChildSetRecord,
    child_output: &ChildTerminalOutput,
) -> serde_json::Value {
    let final_text = child_output
        .final_text
        .as_deref()
        .map(|text| sanitize_tool_result_summary(text.to_string()));
    serde_json::json!({
        "child_run_id": record.child_run_id,
        "child_thread_id": record.child_thread_id,
        "flavor": record.flavor_id,
        "mode": mode_label(record.mode),
        "status": status_label(event.status),
        "output_available": event.status == TurnStatus::Completed,
        "final_text": final_text,
        "failure_summary": child_output.failure_summary.clone(),
        "terminal_event": {
            "kind": event_kind_label(&event.kind),
            "cursor": event.cursor.0,
            "reason": event.sanitized_reason,
        }
    })
}

#[derive(Debug, Clone)]
struct ChildTerminalOutput {
    final_text: Option<String>,
    failure_summary: Option<String>,
}

fn parent_result_summary(
    event: &AwaitedChildTerminalEvent,
    child_output: &ChildTerminalOutput,
) -> Result<ToolResultSafeSummary, TurnError> {
    let mut summary = match child_output.final_text.as_deref() {
        Some(final_text) if !final_text.trim().is_empty() => {
            let final_text = sanitize_tool_result_summary(final_text.to_string());
            format!("Subagent completed with answer {}", final_text)
        }
        _ => match child_output.failure_summary.as_deref() {
            Some(failure) if !failure.trim().is_empty() => {
                let failure = sanitize_tool_result_summary(failure.to_string());
                format!(
                    "Subagent finished with status {} and failure {}",
                    status_label(event.status),
                    failure
                )
            }
            _ => format!(
                "Subagent finished with status {}",
                status_label(event.status)
            ),
        },
    };
    summary = sanitize_tool_result_summary(summary);
    ToolResultSafeSummary::new(summary).map_err(|reason| TurnError::InvalidRequest { reason })
}

fn sanitize_tool_result_summary(value: String) -> String {
    let mut safe = sanitize_model_visible_text(value)
        .chars()
        .map(|character| match character {
            '{' | '}' | '[' | ']' | '`' | '<' | '>' | '/' | '\\' => ' ',
            character if character == '\0' || character.is_control() => ' ',
            character => character,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if safe.len() > 512 {
        truncate_to_char_boundary(&mut safe, 512);
    }
    if ToolResultSafeSummary::new(safe.clone()).is_ok() {
        safe
    } else {
        "Subagent result available".to_string()
    }
}

fn truncate_to_char_boundary(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

fn terminal_event_from_lifecycle(event: &TurnLifecycleEvent) -> AwaitedChildTerminalEvent {
    AwaitedChildTerminalEvent {
        status: event.status,
        kind: event.kind.clone(),
        cursor: event.cursor,
        sanitized_reason: event.sanitized_reason.clone(),
        owner_user_id: event.owner_user_id.clone(),
    }
}

fn awaited_child_record_from_persisted(
    parent_record: TurnRunRecord,
    child_record: TurnRunRecord,
    metadata: SubagentThreadMetadata,
) -> Result<AwaitedChildSetRecord, TurnError> {
    let gate_ref = GateRef::new(match metadata.mode {
        SpawnSubagentMode::Blocking => format!("gate:subagent:{}", child_record.run_id),
        SpawnSubagentMode::Background => format!("gate:subagent-bg:{}", child_record.run_id),
    })
    .map_err(|reason| TurnError::InvalidRequest { reason })?;
    let parent_run_context = LoopRunContext::new(
        parent_record.scope.clone(),
        parent_record.turn_id,
        parent_record.run_id,
        parent_record.profile.resolved,
    );
    Ok(AwaitedChildSetRecord {
        gate_ref,
        parent_run_context,
        parent_scope: parent_record.scope,
        parent_run_id: parent_record.run_id,
        tree_root_run_id: metadata.tree_root_run_id,
        child_scope: child_record.scope.clone(),
        child_run_id: child_record.run_id,
        child_thread_id: child_record.scope.thread_id.clone(),
        source_binding_ref: child_record.source_binding_ref,
        reply_target_binding_ref: child_record.reply_target_binding_ref,
        flavor_id: metadata.flavor,
        spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).map_err(
            |reason| TurnError::InvalidRequest {
                reason: reason.to_string(),
            },
        )?,
        background_result_ref: Some(metadata.result_ref),
        mode: metadata.mode,
    })
}

fn parse_subagent_thread_metadata(raw: &str) -> Option<SubagentThreadMetadata> {
    serde_json::from_str::<SubagentThreadMetadata>(raw)
        .ok()
        .filter(|metadata| metadata.kind == SubagentThreadKind::Subagent)
}

fn thread_scope_from_turn_scope(
    scope: &ironclaw_turns::TurnScope,
    event: &TurnLifecycleEvent,
) -> Result<ThreadScope, TurnError> {
    let agent_id = scope
        .agent_id
        .clone()
        .ok_or_else(|| TurnError::InvalidRequest {
            reason: "subagent run scope is missing agent id".to_string(),
        })?;
    Ok(ThreadScope {
        tenant_id: scope.tenant_id.clone(),
        agent_id,
        project_id: scope.project_id.clone(),
        owner_user_id: event.owner_user_id.clone(),
        mission_id: None,
    })
}

fn mode_label(mode: SpawnSubagentMode) -> &'static str {
    match mode {
        SpawnSubagentMode::Blocking => "blocking",
        SpawnSubagentMode::Background => "background",
    }
}

fn status_label(status: TurnStatus) -> &'static str {
    match status {
        TurnStatus::Queued => "queued",
        TurnStatus::Running => "running",
        TurnStatus::BlockedApproval => "blocked_approval",
        TurnStatus::BlockedAuth => "blocked_auth",
        TurnStatus::BlockedResource => "blocked_resource",
        TurnStatus::BlockedDependentRun => "blocked_dependent_run",
        TurnStatus::CancelRequested => "cancel_requested",
        TurnStatus::Cancelled => "cancelled",
        TurnStatus::Completed => "completed",
        TurnStatus::Failed => "failed",
        TurnStatus::RecoveryRequired => "recovery_required",
    }
}

fn event_kind_label(kind: &TurnEventKind) -> &'static str {
    match kind {
        TurnEventKind::Submitted => "submitted",
        TurnEventKind::Resumed => "resumed",
        TurnEventKind::RunnerClaimed => "runner_claimed",
        TurnEventKind::RunnerHeartbeat => "runner_heartbeat",
        TurnEventKind::RecoveryRequired => "recovery_required",
        TurnEventKind::Blocked => "blocked",
        TurnEventKind::CancelRequested => "cancel_requested",
        TurnEventKind::Cancelled => "cancelled",
        TurnEventKind::Completed => "completed",
        TurnEventKind::Failed => "failed",
    }
}

#[allow(dead_code)]
fn _assert_terminal_statuses_are_covered(status: TurnStatus) -> bool {
    matches!(
        status,
        TurnStatus::Completed | TurnStatus::Failed | TurnStatus::Cancelled
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, CapabilityId, TenantId, ThreadId, UserId};
    use ironclaw_loop_support::{AwaitedChildSetRecord, SubagentGateResolutionStore};
    use ironclaw_threads::{
        AppendAssistantDraftRequest, AppendToolResultReferenceRequest, EnsureThreadRequest,
        InMemorySessionThreadService, MessageContent, ThreadHistoryRequest,
    };
    use ironclaw_turns::{
        CancelRunRequest, CancelRunResponse, EventCursor, GateRef, GetRunStateRequest,
        LoopResultRef, ReplyTargetBindingRef, ResumeTurnResponse, SourceBindingRef,
        SpawnTreeReservation, SubmitTurnRequest, SubmitTurnResponse, TurnRunId, TurnRunRecord,
        TurnRunState, TurnScope, events::TurnLifecycleEvent,
    };

    use crate::subagent::goal_store::BoundedSubagentGoalStore;

    use super::*;

    #[derive(Default)]
    struct RecordingCoordinator {
        resumed: Mutex<Vec<ResumeTurnRequest>>,
    }

    #[async_trait]
    impl TurnCoordinator for RecordingCoordinator {
        async fn submit_turn(
            &self,
            _request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "submit not used by completion observer tests".to_string(),
            })
        }

        async fn resume_turn(
            &self,
            request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            self.resumed.lock().unwrap().push(request.clone());
            Ok(ResumeTurnResponse {
                run_id: request.run_id,
                status: TurnStatus::Queued,
                event_cursor: EventCursor(10),
            })
        }

        async fn cancel_run(
            &self,
            request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: format!(
                    "cancel not used by completion observer tests: {}",
                    request.run_id
                ),
            })
        }

        async fn get_run_state(
            &self,
            request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: format!(
                    "get_run_state not used by completion observer tests: {}",
                    request.run_id
                ),
            })
        }
    }

    struct RecordingResultWriter {
        result_ref: LoopResultRef,
        writes: Mutex<Vec<serde_json::Value>>,
    }

    impl RecordingResultWriter {
        fn new(result_ref: LoopResultRef) -> Self {
            Self {
                result_ref,
                writes: Mutex::new(Vec::new()),
            }
        }

        fn writes(&self) -> Vec<serde_json::Value> {
            self.writes.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl LoopCapabilityResultWriter for RecordingResultWriter {
        async fn write_capability_result(
            &self,
            _run_context: &ironclaw_turns::run_profile::LoopRunContext,
            _capability_id: &CapabilityId,
            output: serde_json::Value,
        ) -> Result<LoopResultRef, AgentLoopHostError> {
            self.writes.lock().unwrap().push(output);
            Ok(self.result_ref.clone())
        }

        async fn update_capability_result(
            &self,
            _run_context: &ironclaw_turns::run_profile::LoopRunContext,
            result_ref: &LoopResultRef,
            output: serde_json::Value,
        ) -> Result<(), AgentLoopHostError> {
            assert_eq!(result_ref, &self.result_ref);
            self.writes.lock().unwrap().push(output);
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingTurnStateStore {
        releases: Mutex<Vec<(TurnScope, TurnRunId, u32)>>,
    }

    impl RecordingTurnStateStore {
        fn releases(&self) -> Vec<(TurnScope, TurnRunId, u32)> {
            self.releases.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl TurnStateStore for RecordingTurnStateStore {
        async fn submit_turn(
            &self,
            _request: SubmitTurnRequest,
            _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
            _run_profile_resolver: &dyn ironclaw_turns::RunProfileResolver,
        ) -> Result<SubmitTurnResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "submit not used by completion observer tests".to_string(),
            })
        }

        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "resume not used by recording store".to_string(),
            })
        }

        async fn request_cancel(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "cancel not used by completion observer tests".to_string(),
            })
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: "get_run_state not used by completion observer tests".to_string(),
            })
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
            Ok(None)
        }

        async fn reserve_tree_descendants(
            &self,
            scope: &TurnScope,
            root_run_id: TurnRunId,
            delta: u32,
            _cap: u32,
        ) -> Result<SpawnTreeReservation, TurnError> {
            Ok(SpawnTreeReservation {
                scope: scope.clone(),
                root_run_id,
                descendant_count: u64::from(delta),
            })
        }

        async fn release_tree_descendants(
            &self,
            scope: &TurnScope,
            root_run_id: TurnRunId,
            delta: u32,
        ) -> Result<(), TurnError> {
            self.releases
                .lock()
                .unwrap()
                .push((scope.clone(), root_run_id, delta));
            Ok(())
        }
    }

    #[tokio::test]
    async fn background_terminal_event_releases_reservation_writes_result_and_delivers_message() {
        let tenant = TenantId::new("tenant").unwrap();
        let agent = AgentId::new("agent").unwrap();
        let owner = UserId::new("owner").unwrap();
        let parent_scope = TurnScope::new(
            tenant.clone(),
            Some(agent.clone()),
            None,
            ThreadId::new("parent-thread").unwrap(),
        );
        let parent_thread_scope = ThreadScope {
            tenant_id: tenant.clone(),
            agent_id: agent.clone(),
            project_id: None,
            owner_user_id: Some(owner.clone()),
            mission_id: None,
        };
        let child_scope = TurnScope::new(
            tenant.clone(),
            Some(agent.clone()),
            None,
            ThreadId::new("child-thread").unwrap(),
        );
        let child_thread_scope = ThreadScope {
            tenant_id: tenant,
            agent_id: agent,
            project_id: None,
            owner_user_id: Some(owner.clone()),
            mission_id: None,
        };
        let parent_run_id = TurnRunId::new();
        let child_run_id = TurnRunId::new();
        let tree_root_run_id = parent_run_id;
        let result_ref = LoopResultRef::new("result:subagent.background").unwrap();

        let turn_state_store = Arc::new(RecordingTurnStateStore::default());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: parent_thread_scope.clone(),
                thread_id: Some(parent_scope.thread_id.clone()),
                created_by_actor_id: "test".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        thread_service
            .append_tool_result_reference(AppendToolResultReferenceRequest {
                scope: parent_thread_scope.clone(),
                thread_id: parent_scope.thread_id.clone(),
                turn_run_id: parent_run_id.to_string(),
                result_ref: result_ref.as_str().to_string(),
                safe_summary: ToolResultSafeSummary::new("subagent spawned in background").unwrap(),
                provider_call: None,
            })
            .await
            .unwrap();
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: child_thread_scope.clone(),
                thread_id: Some(child_scope.thread_id.clone()),
                created_by_actor_id: "test".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        let child_reply = thread_service
            .append_assistant_draft(AppendAssistantDraftRequest {
                scope: child_thread_scope.clone(),
                thread_id: child_scope.thread_id.clone(),
                turn_run_id: child_run_id.to_string(),
                content: MessageContent::text("draft child answer"),
            })
            .await
            .unwrap();
        thread_service
            .finalize_assistant_message(
                &child_thread_scope,
                &child_scope.thread_id,
                child_reply.message_id,
                MessageContent::text("final child answer"),
            )
            .await
            .unwrap();

        let gate_store = Arc::new(BoundedSubagentGateResolutionStore::new());
        let goal_store = Arc::new(BoundedSubagentGoalStore::new());
        let mut parent_run_context =
            ironclaw_agent_loop::test_support::test_run_context("completion-observer");
        parent_run_context.scope = parent_scope.clone();
        parent_run_context.thread_id = parent_scope.thread_id.clone();
        parent_run_context.run_id = parent_run_id;
        gate_store
            .record_awaited_child(AwaitedChildSetRecord {
                gate_ref: GateRef::new("gate:subagent-bg:test").unwrap(),
                parent_run_context,
                parent_scope: parent_scope.clone(),
                parent_run_id,
                tree_root_run_id,
                child_scope: child_scope.clone(),
                child_run_id,
                child_thread_id: child_scope.thread_id.clone(),
                source_binding_ref: SourceBindingRef::new("subagent-source:test").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("subagent-reply:test")
                    .unwrap(),
                flavor_id: "general".to_string(),
                spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                    .unwrap(),
                background_result_ref: Some(result_ref.clone()),
                mode: SpawnSubagentMode::Background,
            })
            .await
            .unwrap();

        let result_writer = Arc::new(RecordingResultWriter::new(result_ref));
        let observer = SubagentCompletionObserver::new(
            Arc::clone(&gate_store),
            goal_store,
            turn_state_store.clone(),
            result_writer.clone(),
            Arc::new(RecordingCoordinator::default()),
            thread_service.clone(),
        );

        observer
            .handle_terminal(&TurnLifecycleEvent {
                cursor: EventCursor(7),
                scope: child_scope,
                occurred_at: None,
                owner_user_id: Some(owner),
                run_id: child_run_id,
                status: TurnStatus::Completed,
                kind: TurnEventKind::Completed,
                blocked_gate: None,
                sanitized_reason: None,
            })
            .await
            .unwrap();

        assert_eq!(
            turn_state_store.releases(),
            vec![(parent_scope.clone(), tree_root_run_id, 1)]
        );
        let writes = result_writer.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0]["status"], "completed");
        assert_eq!(writes[0]["output_available"], true);
        assert_eq!(writes[0]["final_text"], "final child answer");

        let history = thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: parent_thread_scope,
                thread_id: parent_scope.thread_id,
            })
            .await
            .unwrap();
        assert_eq!(history.messages.len(), 1);
        assert!(
            history.messages[0]
                .content
                .as_ref()
                .unwrap()
                .contains("final child answer")
        );
    }

    #[tokio::test]
    async fn terminal_event_after_restart_updates_parent_reference_without_staged_payload() {
        let tenant = TenantId::new("tenant").unwrap();
        let agent = AgentId::new("agent").unwrap();
        let owner = UserId::new("owner").unwrap();
        let parent_scope = TurnScope::new(
            tenant.clone(),
            Some(agent.clone()),
            None,
            ThreadId::new("parent-thread-recovered").unwrap(),
        );
        let parent_thread_scope = ThreadScope {
            tenant_id: tenant.clone(),
            agent_id: agent.clone(),
            project_id: None,
            owner_user_id: Some(owner.clone()),
            mission_id: None,
        };
        let child_scope = TurnScope::new(
            tenant.clone(),
            Some(agent.clone()),
            None,
            ThreadId::new("child-thread-recovered").unwrap(),
        );
        let child_thread_scope = ThreadScope {
            tenant_id: tenant,
            agent_id: agent,
            project_id: None,
            owner_user_id: Some(owner.clone()),
            mission_id: None,
        };
        let parent_run_id = TurnRunId::new();
        let child_run_id = TurnRunId::new();
        let result_ref = LoopResultRef::new("result:subagent.recovered").unwrap();

        let turn_state_store = Arc::new(RecordingTurnStateStore::default());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: parent_thread_scope.clone(),
                thread_id: Some(parent_scope.thread_id.clone()),
                created_by_actor_id: "test".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        thread_service
            .append_tool_result_reference(AppendToolResultReferenceRequest {
                scope: parent_thread_scope.clone(),
                thread_id: parent_scope.thread_id.clone(),
                turn_run_id: parent_run_id.to_string(),
                result_ref: result_ref.as_str().to_string(),
                safe_summary: ToolResultSafeSummary::new("subagent spawned in background").unwrap(),
                provider_call: None,
            })
            .await
            .unwrap();
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: child_thread_scope,
                thread_id: Some(child_scope.thread_id.clone()),
                created_by_actor_id: "test".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();

        let gate_store = Arc::new(BoundedSubagentGateResolutionStore::new());
        let goal_store = Arc::new(BoundedSubagentGoalStore::new());
        let mut parent_run_context =
            ironclaw_agent_loop::test_support::test_run_context("completion-observer-recovery");
        parent_run_context.scope = parent_scope.clone();
        parent_run_context.thread_id = parent_scope.thread_id.clone();
        parent_run_context.run_id = parent_run_id;
        gate_store
            .record_awaited_child(AwaitedChildSetRecord {
                gate_ref: GateRef::new("gate:subagent-bg:recovered").unwrap(),
                parent_run_context,
                parent_scope: parent_scope.clone(),
                parent_run_id,
                tree_root_run_id: parent_run_id,
                child_scope: child_scope.clone(),
                child_run_id,
                child_thread_id: child_scope.thread_id.clone(),
                source_binding_ref: SourceBindingRef::new("subagent-source:recovered").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("subagent-reply:recovered")
                    .unwrap(),
                flavor_id: "general".to_string(),
                spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                    .unwrap(),
                background_result_ref: Some(result_ref.clone()),
                mode: SpawnSubagentMode::Background,
            })
            .await
            .unwrap();

        let result_writer = Arc::new(RecordingResultWriter::new(result_ref));
        let observer = SubagentCompletionObserver::new(
            Arc::clone(&gate_store),
            goal_store,
            turn_state_store,
            result_writer.clone(),
            Arc::new(RecordingCoordinator::default()),
            thread_service.clone(),
        );

        observer
            .handle_terminal(&TurnLifecycleEvent {
                cursor: EventCursor(8),
                scope: child_scope,
                occurred_at: None,
                owner_user_id: Some(owner),
                run_id: child_run_id,
                status: TurnStatus::Completed,
                kind: TurnEventKind::Completed,
                blocked_gate: None,
                sanitized_reason: None,
            })
            .await
            .unwrap();

        assert_eq!(result_writer.writes().len(), 1);
        let history = thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: parent_thread_scope,
                thread_id: parent_scope.thread_id,
            })
            .await
            .unwrap();
        assert_eq!(history.messages.len(), 1);
        assert!(
            history.messages[0]
                .content
                .as_ref()
                .unwrap()
                .contains("Subagent finished with status completed")
        );
    }

    #[tokio::test]
    async fn recovery_required_child_resolves_parent_reference() {
        let tenant = TenantId::new("tenant").unwrap();
        let agent = AgentId::new("agent").unwrap();
        let owner = UserId::new("owner").unwrap();
        let parent_scope = TurnScope::new(
            tenant.clone(),
            Some(agent.clone()),
            None,
            ThreadId::new("parent-thread-recovery-required").unwrap(),
        );
        let parent_thread_scope = ThreadScope {
            tenant_id: tenant.clone(),
            agent_id: agent.clone(),
            project_id: None,
            owner_user_id: Some(owner.clone()),
            mission_id: None,
        };
        let child_scope = TurnScope::new(
            tenant,
            Some(agent.clone()),
            None,
            ThreadId::new("child-thread-recovery-required").unwrap(),
        );
        let child_thread_scope = ThreadScope {
            tenant_id: parent_thread_scope.tenant_id.clone(),
            agent_id: agent,
            project_id: None,
            owner_user_id: Some(owner.clone()),
            mission_id: None,
        };
        let parent_run_id = TurnRunId::new();
        let child_run_id = TurnRunId::new();
        let result_ref = LoopResultRef::new("result:subagent.recovery_required").unwrap();

        let turn_state_store = Arc::new(RecordingTurnStateStore::default());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: parent_thread_scope.clone(),
                thread_id: Some(parent_scope.thread_id.clone()),
                created_by_actor_id: "test".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        thread_service
            .append_tool_result_reference(AppendToolResultReferenceRequest {
                scope: parent_thread_scope.clone(),
                thread_id: parent_scope.thread_id.clone(),
                turn_run_id: parent_run_id.to_string(),
                result_ref: result_ref.as_str().to_string(),
                safe_summary: ToolResultSafeSummary::new("subagent spawned in background").unwrap(),
                provider_call: None,
            })
            .await
            .unwrap();
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: child_thread_scope,
                thread_id: Some(child_scope.thread_id.clone()),
                created_by_actor_id: "test".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();

        let gate_store = Arc::new(BoundedSubagentGateResolutionStore::new());
        let goal_store = Arc::new(BoundedSubagentGoalStore::new());
        let mut parent_run_context = ironclaw_agent_loop::test_support::test_run_context(
            "completion-observer-recovery-required",
        );
        parent_run_context.scope = parent_scope.clone();
        parent_run_context.thread_id = parent_scope.thread_id.clone();
        parent_run_context.run_id = parent_run_id;
        gate_store
            .record_awaited_child(AwaitedChildSetRecord {
                gate_ref: GateRef::new("gate:subagent-bg:recovery-required").unwrap(),
                parent_run_context,
                parent_scope: parent_scope.clone(),
                parent_run_id,
                tree_root_run_id: parent_run_id,
                child_scope: child_scope.clone(),
                child_run_id,
                child_thread_id: child_scope.thread_id.clone(),
                source_binding_ref: SourceBindingRef::new("subagent-source:recovery-required")
                    .unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new(
                    "subagent-reply:recovery-required",
                )
                .unwrap(),
                flavor_id: "general".to_string(),
                spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                    .unwrap(),
                background_result_ref: Some(result_ref.clone()),
                mode: SpawnSubagentMode::Background,
            })
            .await
            .unwrap();

        let result_writer = Arc::new(RecordingResultWriter::new(result_ref));
        let observer = SubagentCompletionObserver::new(
            Arc::clone(&gate_store),
            goal_store,
            turn_state_store,
            result_writer,
            Arc::new(RecordingCoordinator::default()),
            thread_service.clone(),
        );

        observer
            .handle_terminal(&TurnLifecycleEvent {
                cursor: EventCursor(9),
                scope: child_scope,
                occurred_at: None,
                owner_user_id: Some(owner),
                run_id: child_run_id,
                status: TurnStatus::RecoveryRequired,
                kind: TurnEventKind::RecoveryRequired,
                blocked_gate: None,
                sanitized_reason: Some("driver_bug".to_string()),
            })
            .await
            .unwrap();

        let history = thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: parent_thread_scope,
                thread_id: parent_scope.thread_id,
            })
            .await
            .unwrap();
        assert_eq!(history.messages.len(), 1);
        assert!(
            history.messages[0]
                .content
                .as_ref()
                .unwrap()
                .contains("recovery_required")
        );
    }

    #[test]
    fn tool_result_summary_sanitizes_and_truncates_on_utf8_boundary() {
        let raw = format!("answer {{with}} <markers> {}", "é".repeat(300));

        let safe = sanitize_tool_result_summary(raw);

        assert!(safe.len() <= 512);
        assert!(safe.is_char_boundary(safe.len()));
        assert!(!safe.contains('{'));
        assert!(!safe.contains('}'));
        assert!(!safe.contains('<'));
        assert!(!safe.contains('>'));
    }

    #[test]
    fn parent_result_summary_sanitizes_child_text_before_formatting() {
        let summary = parent_result_summary(
            &AwaitedChildTerminalEvent {
                status: TurnStatus::Completed,
                kind: TurnEventKind::Completed,
                cursor: EventCursor(10),
                sanitized_reason: None,
                owner_user_id: None,
            },
            &ChildTerminalOutput {
                final_text: Some(format!("{} {{secret}}", "é".repeat(300))),
                failure_summary: None,
            },
        )
        .unwrap();

        assert!(summary.as_str().len() <= 512);
        assert!(!summary.as_str().contains('{'));
        assert!(!summary.as_str().contains('}'));
    }
}
