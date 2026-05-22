use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;
use ironclaw_loop_support::{
    AwaitedChildSetRecord, DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID, LoopCapabilityResultWriter,
    SpawnSubagentMode, SubagentGateResolutionStore, SubagentSpawnGoalStore, SubagentThreadKind,
    SubagentThreadMetadata,
};
use ironclaw_threads::{
    LatestThreadMessageRequest, MessageKind, MessageStatus, SessionThreadService,
    ThreadHistoryRequest, ThreadScope, ToolResultSafeSummary, UpdateToolResultReferenceRequest,
};
use ironclaw_turns::{
    GateRef, IdempotencyKey, ResumeTurnPrecondition, ResumeTurnRequest, TurnActor,
    TurnCommittedEventObserver, TurnCoordinator, TurnError, TurnEventKind, TurnLifecycleEvent,
    TurnRunId, TurnRunRecord, TurnRunState, TurnSpawnTreeStateStore, TurnStatus,
    run_profile::{AgentLoopHostError, LoopRunContext, sanitize_model_visible_text},
};

use crate::subagent::gate_resolution::{
    AwaitedChildTerminalEvent, BoundedSubagentGateResolutionStore,
};
use crate::subagent::spawn_result::{
    SpawnedChildRunPayload, SubagentSpawnMode as PayloadSpawnMode,
    SubagentSpawnStatus as PayloadSpawnStatus, SubagentTerminalEventKind,
    SubagentTerminalEventPayload,
};

#[derive(Clone)]
pub struct SubagentCompletionObserver<S: SessionThreadService + ?Sized> {
    gate_store: Arc<BoundedSubagentGateResolutionStore>,
    goal_store: Arc<dyn SubagentSpawnGoalStore>,
    turn_state_store: Arc<dyn TurnSpawnTreeStateStore>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    coordinator: Arc<OnceLock<Arc<dyn TurnCoordinator>>>,
    thread_service: Arc<S>,
}

impl<S> SubagentCompletionObserver<S>
where
    S: SessionThreadService + ?Sized,
{
    pub fn new(
        gate_store: Arc<BoundedSubagentGateResolutionStore>,
        goal_store: Arc<dyn SubagentSpawnGoalStore>,
        turn_state_store: Arc<dyn TurnSpawnTreeStateStore>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        coordinator: Arc<dyn TurnCoordinator>,
        thread_service: Arc<S>,
    ) -> Result<Self, TurnError> {
        let observer = Self::new_unbound(
            gate_store,
            goal_store,
            turn_state_store,
            result_writer,
            thread_service,
        );
        observer.bind_coordinator(coordinator)?;
        Ok(observer)
    }

    pub fn new_unbound(
        gate_store: Arc<BoundedSubagentGateResolutionStore>,
        goal_store: Arc<dyn SubagentSpawnGoalStore>,
        turn_state_store: Arc<dyn TurnSpawnTreeStateStore>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        thread_service: Arc<S>,
    ) -> Self {
        Self {
            gate_store,
            goal_store,
            turn_state_store,
            result_writer,
            coordinator: Arc::new(OnceLock::new()),
            thread_service,
        }
    }

    pub fn bind_coordinator(&self, coordinator: Arc<dyn TurnCoordinator>) -> Result<(), TurnError> {
        self.coordinator
            .set(coordinator)
            .map_err(|_| TurnError::InvalidRequest {
                reason: "subagent completion observer coordinator already bound".to_string(),
            })
    }

    async fn handle_terminal(&self, event: &TurnLifecycleEvent) -> Result<(), TurnError> {
        let has_gate_record = self
            .gate_store
            .subagent_kind_for_child(event.run_id)
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

    async fn recover_missing_gate_record(
        &self,
        event: &TurnLifecycleEvent,
    ) -> Result<(), TurnError> {
        if self
            .gate_store
            .subagent_kind_for_child(event.run_id)
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
            .claim_descendant_reservation_release(&record.gate_ref)
            .map_err(map_host_error)?
        {
            return Ok(());
        }
        match self
            .turn_state_store
            .release_tree_descendants(&record.parent_run_context.scope, record.tree_root_run_id, 1)
            .await
        {
            Ok(()) => self
                .gate_store
                .mark_descendant_reservation_released(&record.gate_ref)
                .map_err(map_host_error),
            Err(error) => {
                let _ = self
                    .gate_store
                    .release_descendant_reservation_claim(&record.gate_ref);
                Err(error)
            }
        }
    }

    async fn resume_parent(
        &self,
        event: &AwaitedChildTerminalEvent,
        record: &ironclaw_loop_support::AwaitedChildSetRecord,
    ) -> Result<(), TurnError> {
        let actor = actor_from_terminal_event(event)?;
        let coordinator = self
            .coordinator
            .get()
            .ok_or_else(|| TurnError::Unavailable {
                reason: "subagent completion observer coordinator is not bound".to_string(),
            })?;
        coordinator
            .resume_turn(ResumeTurnRequest {
                scope: record.parent_run_context.scope.clone(),
                actor,
                run_id: record.parent_run_context.run_id,
                gate_resolution_ref: record.gate_ref.clone(),
                source_binding_ref: record.source_binding_ref.clone(),
                reply_target_binding_ref: record.reply_target_binding_ref.clone(),
                idempotency_key: IdempotencyKey::new(format!(
                    "subagent-resume:{}:{}",
                    record.parent_run_context.run_id, record.child_run_id
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
        let result_ref = &record.result_ref;
        let child_output = self.child_terminal_output(record, event).await?;
        let safe_summary = parent_result_summary(event, &child_output)?;
        let payload = background_completion_payload(event, record, &child_output)?;
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
        let final_text = self
            .thread_service
            .latest_thread_message(LatestThreadMessageRequest {
                scope: child_thread_scope,
                thread_id: record.child_thread_id.clone(),
                kind: MessageKind::Assistant,
                status: MessageStatus::Finalized,
            })
            .await
            .map_err(|error| TurnError::Unavailable {
                reason: format!("subagent child final message unavailable: {error}"),
            })?
            .and_then(|message| message.content);
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
        let Some(agent_id) = record.parent_run_context.scope.agent_id.clone() else {
            return Err(TurnError::InvalidRequest {
                reason: "parent scope missing agent id for subagent result update".to_string(),
            });
        };
        let thread_scope = ThreadScope {
            tenant_id: record.parent_run_context.scope.tenant_id.clone(),
            agent_id,
            project_id: record.parent_run_context.scope.project_id.clone(),
            owner_user_id: event.owner_user_id.clone(),
            mission_id: None,
        };
        self.thread_service
            .update_tool_result_reference(UpdateToolResultReferenceRequest {
                scope: thread_scope,
                thread_id: record.parent_run_context.scope.thread_id.clone(),
                turn_run_id: record.parent_run_context.run_id.to_string(),
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

#[async_trait]
impl<S> TurnCommittedEventObserver for SubagentCompletionObserver<S>
where
    S: SessionThreadService + ?Sized,
{
    fn observes_state(&self, state: &TurnRunState) -> bool {
        is_subagent_terminal_status(state.status)
    }

    fn observes_event(&self, event: &TurnLifecycleEvent) -> bool {
        is_subagent_terminal_status(event.status)
    }

    async fn observe_committed_state(&self, state: TurnRunState) -> Result<(), TurnError> {
        let event = terminal_event_from_state(&state)?;
        self.handle_terminal(&event).await
    }

    async fn observe_committed_event(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        self.handle_terminal(&event).await
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
) -> Result<serde_json::Value, TurnError> {
    let final_text = child_output
        .final_text
        .as_deref()
        .map(|text| sanitize_tool_result_summary(text.to_string()));
    let payload = SpawnedChildRunPayload {
        child_run_id: record.child_run_id,
        child_thread_id: record.child_thread_id.clone(),
        subagent_kind: record.subagent_kind.clone(),
        mode: payload_spawn_mode(record.mode),
        status: payload_spawn_status(event.status)?,
        output_available: event.status == TurnStatus::Completed,
        final_text,
        failure_summary: child_output.failure_summary.clone(),
        terminal_event: Some(SubagentTerminalEventPayload {
            kind: terminal_event_kind(&event.kind),
            cursor: event.cursor,
            reason: event.sanitized_reason.clone(),
        }),
    };
    serde_json::to_value(payload).map_err(|error| TurnError::Unavailable {
        reason: format!("subagent completion payload serialization failed: {error}"),
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

fn terminal_event_from_state(state: &TurnRunState) -> Result<TurnLifecycleEvent, TurnError> {
    Ok(TurnLifecycleEvent {
        cursor: state.event_cursor,
        scope: state.scope.clone(),
        occurred_at: None,
        owner_user_id: state.actor.clone().map(|actor| actor.user_id),
        run_id: state.run_id,
        status: state.status,
        kind: event_kind_from_terminal_status(state.status)?,
        blocked_gate: None,
        sanitized_reason: state
            .failure
            .as_ref()
            .map(|failure| failure.category().to_string()),
    })
}

fn event_kind_from_terminal_status(status: TurnStatus) -> Result<TurnEventKind, TurnError> {
    match status {
        TurnStatus::Completed => Ok(TurnEventKind::Completed),
        TurnStatus::Failed => Ok(TurnEventKind::Failed),
        TurnStatus::Cancelled => Ok(TurnEventKind::Cancelled),
        TurnStatus::RecoveryRequired => Ok(TurnEventKind::RecoveryRequired),
        other => Err(TurnError::InvalidRequest {
            reason: format!("subagent completion observer received non-terminal status {other:?}"),
        }),
    }
}

fn awaited_child_record_from_persisted(
    parent_record: TurnRunRecord,
    child_record: TurnRunRecord,
    metadata: SubagentThreadMetadata,
) -> Result<AwaitedChildSetRecord, TurnError> {
    // Mirrors the spawn path's `LoopGateRef`-compatible gate token format.
    // The separator after the `gate:` prefix must stay colon-free because
    // `LoopGateRef` rejects additional colons in the opaque id.
    let gate_ref = GateRef::new(match metadata.mode {
        SpawnSubagentMode::Blocking => format!("gate:subagent.{}", child_record.run_id),
        SpawnSubagentMode::Background => format!("gate:subagent-bg.{}", child_record.run_id),
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
        tree_root_run_id: metadata.tree_root_run_id,
        child_scope: child_record.scope.clone(),
        child_run_id: child_record.run_id,
        child_thread_id: child_record.scope.thread_id.clone(),
        source_binding_ref: child_record.source_binding_ref,
        reply_target_binding_ref: child_record.reply_target_binding_ref,
        subagent_kind: metadata.subagent_kind,
        spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).map_err(
            |reason| TurnError::InvalidRequest {
                reason: reason.to_string(),
            },
        )?,
        result_ref: metadata.result_ref,
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

fn payload_spawn_mode(mode: SpawnSubagentMode) -> PayloadSpawnMode {
    match mode {
        SpawnSubagentMode::Blocking => PayloadSpawnMode::Blocking,
        SpawnSubagentMode::Background => PayloadSpawnMode::Background,
    }
}

fn payload_spawn_status(status: TurnStatus) -> Result<PayloadSpawnStatus, TurnError> {
    match status {
        TurnStatus::Completed => Ok(PayloadSpawnStatus::Completed),
        TurnStatus::Failed => Ok(PayloadSpawnStatus::Failed),
        TurnStatus::Cancelled => Ok(PayloadSpawnStatus::Cancelled),
        TurnStatus::RecoveryRequired => Ok(PayloadSpawnStatus::RecoveryRequired),
        other => Err(TurnError::InvalidRequest {
            reason: format!("subagent completion payload received non-terminal status {other:?}"),
        }),
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

fn terminal_event_kind(kind: &TurnEventKind) -> SubagentTerminalEventKind {
    match kind {
        TurnEventKind::Submitted => SubagentTerminalEventKind::Submitted,
        TurnEventKind::Resumed => SubagentTerminalEventKind::Resumed,
        TurnEventKind::RunnerClaimed => SubagentTerminalEventKind::RunnerClaimed,
        TurnEventKind::RunnerHeartbeat => SubagentTerminalEventKind::RunnerHeartbeat,
        TurnEventKind::RecoveryRequired => SubagentTerminalEventKind::RecoveryRequired,
        TurnEventKind::Blocked => SubagentTerminalEventKind::Blocked,
        TurnEventKind::CancelRequested => SubagentTerminalEventKind::CancelRequested,
        TurnEventKind::Cancelled => SubagentTerminalEventKind::Cancelled,
        TurnEventKind::Completed => SubagentTerminalEventKind::Completed,
        TurnEventKind::Failed => SubagentTerminalEventKind::Failed,
    }
}

#[allow(dead_code)]
fn _assert_terminal_statuses_are_covered(status: TurnStatus) -> bool {
    matches!(
        status,
        TurnStatus::Completed
            | TurnStatus::Failed
            | TurnStatus::Cancelled
            | TurnStatus::RecoveryRequired
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, CapabilityId, InvocationId, TenantId, ThreadId, UserId};
    use ironclaw_loop_support::{
        AwaitedChildSetRecord, SubagentGateResolutionStore, SubagentKindId,
    };
    use ironclaw_threads::{
        AppendAssistantDraftRequest, AppendToolResultReferenceRequest, EnsureThreadRequest,
        InMemorySessionThreadService, MessageContent, ThreadHistoryRequest,
    };
    use ironclaw_turns::{
        AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GateRef,
        GetRunStateRequest, LoopResultRef, ReplyTargetBindingRef, ResumeTurnResponse, RunProfileId,
        RunProfileVersion, SourceBindingRef, SpawnTreeReservation, SubmitTurnRequest,
        SubmitTurnResponse, TurnRunId, TurnRunProfile, TurnRunRecord, TurnRunState, TurnScope,
        TurnStateStore, events::TurnLifecycleEvent,
    };

    use crate::subagent::goal_store::InMemoryBoundedSubagentGoalStore;

    use super::*;

    #[derive(Default)]
    struct RecordingCoordinator {
        resumed: Mutex<Vec<ResumeTurnRequest>>,
    }

    #[async_trait]
    impl TurnCoordinator for RecordingCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            Ok(TurnRunId::new())
        }

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
            _input_ref: &ironclaw_turns::run_profile::CapabilityInputRef,
            _invocation_id: InvocationId,
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
        records: Mutex<Vec<TurnRunRecord>>,
    }

    impl RecordingTurnStateStore {
        fn releases(&self) -> Vec<(TurnScope, TurnRunId, u32)> {
            self.releases.lock().unwrap().clone()
        }

        fn add_record(&self, record: TurnRunRecord) {
            self.records.lock().unwrap().push(record);
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
    }

    #[async_trait]
    impl TurnSpawnTreeStateStore for RecordingTurnStateStore {
        async fn submit_child_turn(
            &self,
            _request: ironclaw_turns::SubmitChildRunRequest,
            _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
            _run_profile_resolver: &dyn ironclaw_turns::RunProfileResolver,
        ) -> Result<SubmitTurnResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "submit_child_turn not used by completion observer tests".to_string(),
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
            scope: &TurnScope,
            run_id: TurnRunId,
        ) -> Result<Option<TurnRunRecord>, TurnError> {
            Ok(self
                .records
                .lock()
                .unwrap()
                .iter()
                .find(|record| record.scope == *scope && record.run_id == run_id)
                .cloned())
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

    fn empty_observer() -> SubagentCompletionObserver<InMemorySessionThreadService> {
        SubagentCompletionObserver::new_unbound(
            Arc::new(BoundedSubagentGateResolutionStore::new()),
            Arc::new(InMemoryBoundedSubagentGoalStore::new()),
            Arc::new(RecordingTurnStateStore::default()),
            Arc::new(RecordingResultWriter::new(
                LoopResultRef::new("result:test").unwrap(),
            )),
            Arc::new(InMemorySessionThreadService::default()),
        )
    }

    fn test_state(status: TurnStatus) -> TurnRunState {
        TurnRunState {
            scope: TurnScope::new(
                TenantId::new("tenant1").unwrap(),
                Some(AgentId::new("agent1").unwrap()),
                None,
                ThreadId::new("thread-test").unwrap(),
            ),
            actor: Some(TurnActor::new(UserId::new("user-test").unwrap())),
            turn_id: ironclaw_turns::TurnId::new(),
            run_id: TurnRunId::new(),
            status,
            accepted_message_ref: AcceptedMessageRef::new("message-test").unwrap(),
            source_binding_ref: SourceBindingRef::new("source-test").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-test").unwrap(),
            resolved_run_profile_id: RunProfileId::new("default").unwrap(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            received_at: chrono::Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            failure: None,
            event_cursor: EventCursor(1),
        }
    }

    fn turn_record_for_context(
        context: &ironclaw_turns::run_profile::LoopRunContext,
        parent_run_id: Option<TurnRunId>,
        subagent_depth: u32,
        spawn_tree_root_run_id: Option<TurnRunId>,
    ) -> TurnRunRecord {
        TurnRunRecord {
            run_id: context.run_id,
            turn_id: context.turn_id,
            scope: context.scope.clone(),
            accepted_message_ref: AcceptedMessageRef::new(format!("msg-{}", context.run_id))
                .unwrap(),
            source_binding_ref: SourceBindingRef::new(format!("source-{}", context.run_id))
                .unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new(format!(
                "reply-{}",
                context.run_id
            ))
            .unwrap(),
            status: TurnStatus::Queued,
            profile: TurnRunProfile::from_resolved(context.resolved_run_profile.clone()),
            resolved_model_route: None,
            checkpoint_id: None,
            gate_ref: None,
            failure: None,
            event_cursor: EventCursor(1),
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: chrono::Utc::now(),
            parent_run_id,
            subagent_depth,
            spawn_tree_root_run_id,
        }
    }

    fn turn_state_for_context(
        context: &ironclaw_turns::run_profile::LoopRunContext,
        status: TurnStatus,
    ) -> TurnRunState {
        let profile = TurnRunProfile::from_resolved(context.resolved_run_profile.clone());
        TurnRunState {
            scope: context.scope.clone(),
            actor: None,
            turn_id: context.turn_id,
            run_id: context.run_id,
            status,
            accepted_message_ref: AcceptedMessageRef::new(format!("msg-{}", context.run_id))
                .unwrap(),
            source_binding_ref: SourceBindingRef::new(format!("source-{}", context.run_id))
                .unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new(format!(
                "reply-{}",
                context.run_id
            ))
            .unwrap(),
            resolved_run_profile_id: profile.id,
            resolved_run_profile_version: profile.version,
            resolved_model_route: None,
            received_at: chrono::Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            failure: None,
            event_cursor: EventCursor(1),
        }
    }

    #[tokio::test]
    async fn bind_coordinator_rejects_double_bind() {
        let observer = empty_observer();
        observer
            .bind_coordinator(Arc::new(RecordingCoordinator::default()))
            .unwrap();

        let error = observer
            .bind_coordinator(Arc::new(RecordingCoordinator::default()))
            .unwrap_err();

        assert!(
            matches!(error, TurnError::InvalidRequest { reason } if reason.contains("already bound"))
        );
    }

    #[tokio::test]
    async fn observe_committed_state_rejects_non_terminal_status() {
        let observer = empty_observer();

        let error = observer
            .observe_committed_state(test_state(TurnStatus::Running))
            .await
            .unwrap_err();

        assert!(
            matches!(error, TurnError::InvalidRequest { reason } if reason.contains("non-terminal status"))
        );
    }

    #[tokio::test]
    async fn handle_terminal_state_ignores_non_terminal_statuses() {
        let context =
            ironclaw_agent_loop::test_support::test_run_context("completion-observer-nonterminal");
        let turn_state_store = Arc::new(RecordingTurnStateStore::default());
        let result_ref = LoopResultRef::new("result:subagent.nonterminal").unwrap();
        let result_writer = Arc::new(RecordingResultWriter::new(result_ref));
        let observer = SubagentCompletionObserver::new(
            Arc::new(BoundedSubagentGateResolutionStore::new()),
            Arc::new(InMemoryBoundedSubagentGoalStore::new()),
            turn_state_store.clone(),
            result_writer.clone(),
            Arc::new(RecordingCoordinator::default()),
            Arc::new(InMemorySessionThreadService::default()),
        );

        observer
            .handle_terminal_state(
                &turn_state_for_context(&context, TurnStatus::Running),
                TurnEventKind::RunnerHeartbeat,
            )
            .await
            .unwrap();

        assert!(turn_state_store.releases().is_empty());
        assert!(result_writer.writes().is_empty());
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
        let goal_store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
        let mut parent_run_context =
            ironclaw_agent_loop::test_support::test_run_context("completion-observer");
        parent_run_context.scope = parent_scope.clone();
        parent_run_context.thread_id = parent_scope.thread_id.clone();
        parent_run_context.run_id = parent_run_id;
        gate_store
            .record_awaited_child(AwaitedChildSetRecord {
                gate_ref: GateRef::new("gate:subagent-bg:test").unwrap(),
                parent_run_context,
                tree_root_run_id,
                child_scope: child_scope.clone(),
                child_run_id,
                child_thread_id: child_scope.thread_id.clone(),
                source_binding_ref: SourceBindingRef::new("subagent-source:test").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("subagent-reply:test")
                    .unwrap(),
                subagent_kind: SubagentKindId::new("general").unwrap(),
                spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                    .unwrap(),
                result_ref: result_ref.clone(),
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
        )
        .unwrap();

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
        let goal_store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
        let mut parent_run_context =
            ironclaw_agent_loop::test_support::test_run_context("completion-observer-recovery");
        parent_run_context.scope = parent_scope.clone();
        parent_run_context.thread_id = parent_scope.thread_id.clone();
        parent_run_context.run_id = parent_run_id;
        gate_store
            .record_awaited_child(AwaitedChildSetRecord {
                gate_ref: GateRef::new("gate:subagent-bg:recovered").unwrap(),
                parent_run_context,
                tree_root_run_id: parent_run_id,
                child_scope: child_scope.clone(),
                child_run_id,
                child_thread_id: child_scope.thread_id.clone(),
                source_binding_ref: SourceBindingRef::new("subagent-source:recovered").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("subagent-reply:recovered")
                    .unwrap(),
                subagent_kind: SubagentKindId::new("general").unwrap(),
                spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                    .unwrap(),
                result_ref: result_ref.clone(),
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
        )
        .unwrap();

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
    async fn terminal_event_after_restart_reconstructs_missing_gate_from_thread_metadata() {
        let tenant = TenantId::new("tenant").unwrap();
        let agent = AgentId::new("agent").unwrap();
        let owner = UserId::new("owner").unwrap();
        let parent_scope = TurnScope::new(
            tenant.clone(),
            Some(agent.clone()),
            None,
            ThreadId::new("parent-thread-reconstructed").unwrap(),
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
            ThreadId::new("child-thread-reconstructed").unwrap(),
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
        let result_ref = LoopResultRef::new("result:subagent.reconstructed").unwrap();

        let mut parent_run_context =
            ironclaw_agent_loop::test_support::test_run_context("completion-observer-parent");
        parent_run_context.scope = parent_scope.clone();
        parent_run_context.thread_id = parent_scope.thread_id.clone();
        parent_run_context.run_id = parent_run_id;

        let mut child_run_context =
            ironclaw_agent_loop::test_support::test_run_context("completion-observer-child");
        child_run_context.scope = child_scope.clone();
        child_run_context.thread_id = child_scope.thread_id.clone();
        child_run_context.run_id = child_run_id;

        let turn_state_store = Arc::new(RecordingTurnStateStore::default());
        turn_state_store.add_record(turn_record_for_context(&parent_run_context, None, 0, None));
        turn_state_store.add_record(turn_record_for_context(
            &child_run_context,
            Some(parent_run_id),
            1,
            Some(parent_run_id),
        ));

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
                metadata_json: Some(
                    serde_json::to_string(&SubagentThreadMetadata {
                        kind: SubagentThreadKind::Subagent,
                        parent_run_id,
                        parent_thread_id: parent_scope.thread_id.clone(),
                        tree_root_run_id: parent_run_id,
                        child_run_id,
                        subagent_kind: SubagentKindId::new("general").unwrap(),
                        mode: SpawnSubagentMode::Background,
                        result_ref: result_ref.clone(),
                        handoff: None,
                    })
                    .unwrap(),
                ),
            })
            .await
            .unwrap();
        let child_reply = thread_service
            .append_assistant_draft(AppendAssistantDraftRequest {
                scope: child_thread_scope,
                thread_id: child_scope.thread_id.clone(),
                turn_run_id: child_run_id.to_string(),
                content: MessageContent::text("draft reconstructed answer"),
            })
            .await
            .unwrap();
        thread_service
            .finalize_assistant_message(
                &ThreadScope {
                    tenant_id: parent_thread_scope.tenant_id.clone(),
                    agent_id: parent_thread_scope.agent_id.clone(),
                    project_id: None,
                    owner_user_id: Some(owner.clone()),
                    mission_id: None,
                },
                &child_scope.thread_id,
                child_reply.message_id,
                MessageContent::text("final reconstructed answer"),
            )
            .await
            .unwrap();

        let gate_store = Arc::new(BoundedSubagentGateResolutionStore::new());
        let goal_store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
        let result_writer = Arc::new(RecordingResultWriter::new(result_ref));
        let observer = SubagentCompletionObserver::new(
            Arc::clone(&gate_store),
            goal_store,
            turn_state_store.clone(),
            result_writer.clone(),
            Arc::new(RecordingCoordinator::default()),
            thread_service,
        );

        observer
            .handle_terminal(&TurnLifecycleEvent {
                cursor: EventCursor(11),
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
            vec![(parent_scope.clone(), parent_run_id, 1)]
        );
        let writes = result_writer.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0]["status"], "completed");
        assert_eq!(writes[0]["final_text"], "final reconstructed answer");
        assert_eq!(writes[0]["terminal_event"]["kind"], "completed");
        assert_eq!(writes[0]["terminal_event"]["cursor"], 11);
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
        let goal_store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
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
                subagent_kind: SubagentKindId::new("general").unwrap(),
                spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                    .unwrap(),
                result_ref: result_ref.clone(),
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
        )
        .unwrap();

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

    #[tokio::test]
    async fn blocking_terminal_event_invokes_resume_parent_with_dependent_run_precondition() {
        let tenant = TenantId::new("tenant").unwrap();
        let agent = AgentId::new("agent").unwrap();
        let owner = UserId::new("owner").unwrap();
        let parent_scope = TurnScope::new(
            tenant.clone(),
            Some(agent.clone()),
            None,
            ThreadId::new("blocking-parent-thread").unwrap(),
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
            ThreadId::new("blocking-child-thread").unwrap(),
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
        let result_ref = LoopResultRef::new("result:subagent.blocking").unwrap();

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
                safe_summary: ToolResultSafeSummary::new("subagent spawned blocking").unwrap(),
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
                content: MessageContent::text("draft"),
            })
            .await
            .unwrap();
        thread_service
            .finalize_assistant_message(
                &child_thread_scope,
                &child_scope.thread_id,
                child_reply.message_id,
                MessageContent::text("blocking final reply"),
            )
            .await
            .unwrap();

        let gate_ref = GateRef::new(format!("gate:subagent:{}", child_run_id)).unwrap();
        let gate_store = Arc::new(BoundedSubagentGateResolutionStore::new());
        let goal_store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
        let mut parent_run_context =
            ironclaw_agent_loop::test_support::test_run_context("completion-observer-blocking");
        parent_run_context.scope = parent_scope.clone();
        parent_run_context.thread_id = parent_scope.thread_id.clone();
        parent_run_context.run_id = parent_run_id;
        gate_store
            .record_awaited_child(AwaitedChildSetRecord {
                gate_ref: gate_ref.clone(),
                parent_run_context,
                parent_scope: parent_scope.clone(),
                parent_run_id,
                tree_root_run_id: parent_run_id,
                child_scope: child_scope.clone(),
                child_run_id,
                child_thread_id: child_scope.thread_id.clone(),
                source_binding_ref: SourceBindingRef::new("subagent-source:blocking").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("subagent-reply:blocking")
                    .unwrap(),
                flavor_id: "general".to_string(),
                spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                    .unwrap(),
                background_result_ref: Some(result_ref.clone()),
                mode: SpawnSubagentMode::Blocking,
            })
            .await
            .unwrap();

        let result_writer = Arc::new(RecordingResultWriter::new(result_ref));
        let coordinator = Arc::new(RecordingCoordinator::default());
        let observer = SubagentCompletionObserver::new(
            Arc::clone(&gate_store),
            goal_store,
            turn_state_store.clone(),
            result_writer.clone(),
            coordinator.clone(),
            thread_service.clone(),
        )
        .unwrap();

        observer
            .handle_terminal(&TurnLifecycleEvent {
                cursor: EventCursor(11),
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

        let resumed = coordinator.resumed.lock().unwrap().clone();
        assert_eq!(resumed.len(), 1, "blocking mode must resume parent");
        assert_eq!(resumed[0].run_id, parent_run_id);
        assert_eq!(resumed[0].gate_resolution_ref, gate_ref);
        assert_eq!(
            resumed[0].precondition,
            ResumeTurnPrecondition::AwaitDependentRunGate,
        );
        assert_eq!(
            turn_state_store.releases(),
            vec![(parent_scope, parent_run_id, 1)]
        );
    }

    #[tokio::test]
    async fn blocking_terminal_event_on_unbound_coordinator_returns_unavailable() {
        let tenant = TenantId::new("tenant").unwrap();
        let agent = AgentId::new("agent").unwrap();
        let owner = UserId::new("owner").unwrap();
        let parent_scope = TurnScope::new(
            tenant.clone(),
            Some(agent.clone()),
            None,
            ThreadId::new("unbound-parent-thread").unwrap(),
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
            ThreadId::new("unbound-child-thread").unwrap(),
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
        let result_ref = LoopResultRef::new("result:subagent.unbound").unwrap();

        let turn_state_store = Arc::new(RecordingTurnStateStore::default());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: parent_thread_scope,
                thread_id: Some(parent_scope.thread_id.clone()),
                created_by_actor_id: "test".to_string(),
                title: None,
                metadata_json: None,
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
                content: MessageContent::text("draft"),
            })
            .await
            .unwrap();
        thread_service
            .finalize_assistant_message(
                &child_thread_scope,
                &child_scope.thread_id,
                child_reply.message_id,
                MessageContent::text("final"),
            )
            .await
            .unwrap();

        let gate_ref = GateRef::new(format!("gate:subagent:{}", child_run_id)).unwrap();
        let gate_store = Arc::new(BoundedSubagentGateResolutionStore::new());
        let goal_store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
        let mut parent_run_context =
            ironclaw_agent_loop::test_support::test_run_context("completion-observer-unbound");
        parent_run_context.scope = parent_scope.clone();
        parent_run_context.thread_id = parent_scope.thread_id.clone();
        parent_run_context.run_id = parent_run_id;
        gate_store
            .record_awaited_child(AwaitedChildSetRecord {
                gate_ref,
                parent_run_context,
                parent_scope,
                parent_run_id,
                tree_root_run_id: parent_run_id,
                child_scope: child_scope.clone(),
                child_run_id,
                child_thread_id: child_scope.thread_id.clone(),
                source_binding_ref: SourceBindingRef::new("subagent-source:unbound").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("subagent-reply:unbound")
                    .unwrap(),
                flavor_id: "general".to_string(),
                spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                    .unwrap(),
                background_result_ref: Some(result_ref.clone()),
                mode: SpawnSubagentMode::Blocking,
            })
            .await
            .unwrap();

        let result_writer = Arc::new(RecordingResultWriter::new(result_ref));
        let observer = SubagentCompletionObserver::new_unbound(
            gate_store,
            goal_store,
            turn_state_store,
            result_writer,
            thread_service,
        );

        let error = observer
            .handle_terminal(&TurnLifecycleEvent {
                cursor: EventCursor(11),
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
            .unwrap_err();

        assert!(
            matches!(error, TurnError::Unavailable { .. }),
            "unbound coordinator must yield Unavailable, got {error:?}",
        );
    }
}
