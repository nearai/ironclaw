//! Trusted `LoopExit` applier adapters for the Reborn turn-runner composition.
//!
//! `ironclaw_turns` owns the trusted applier and the private validation policy.
//! This module provides Reborn-specific evidence adapters.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_loop_support::RunCancellationFactory;
use ironclaw_threads::{
    MessageKind, MessageStatus, SessionThreadService, ThreadHistory, ThreadHistoryRequest,
    ThreadMessageId, ThreadMessageRecord, ThreadScope, ToolResultReferenceEnvelope,
};
use ironclaw_turns::{
    CheckpointStateStore, GateRef, GetCheckpointStateRequest, GetLoopCheckpointRequest,
    GetRunStateRequest, LoopBlockedKind, LoopCheckpointKind, LoopGateRef, LoopMessageRef,
    LoopResultRef, TurnError, TurnId, TurnRunId, TurnScope, TurnStateStore, TurnStatus,
};

pub use ironclaw_turns::loop_exit::{
    BlockedEvidenceRequest, CompletionEvidenceRequest, FailureEvidenceRequest,
    FinalCheckpointEvidenceRequest, LoopExitApplier, LoopExitEvidencePort,
};

/// Strict test/local evidence port. Defaults to distrust everything.
///
/// Production builds expose only the distrust-by-default constructor; permissive
/// evidence mutators are test-gated so production code cannot mint fully trusted
/// loop-exit evidence through this in-memory adapter.
#[derive(Debug, Clone)]
pub struct InMemoryLoopExitEvidencePort {
    completion_refs_verified: bool,
    final_checkpoint_verified: bool,
    blocked_evidence_verified: bool,
    failure_evidence_verified: bool,
    cancellation_observed: bool,
    latest_checkpoint_kind: Option<LoopCheckpointKind>,
}

impl InMemoryLoopExitEvidencePort {
    pub fn new() -> Self {
        Self {
            completion_refs_verified: false,
            final_checkpoint_verified: false,
            blocked_evidence_verified: false,
            failure_evidence_verified: false,
            cancellation_observed: false,
            latest_checkpoint_kind: None,
        }
    }

    #[cfg(test)]
    pub fn all_verified() -> Self {
        Self::new()
            .with_completion_refs_verified(true)
            .with_final_checkpoint_verified(true)
            .with_blocked_evidence_verified(true)
            .with_failure_evidence_verified(true)
            .with_cancellation_observed(true)
    }

    #[cfg(test)]
    pub fn with_completion_refs_verified(mut self, verified: bool) -> Self {
        self.completion_refs_verified = verified;
        self
    }

    #[cfg(test)]
    pub fn with_final_checkpoint_verified(mut self, verified: bool) -> Self {
        self.final_checkpoint_verified = verified;
        self
    }

    #[cfg(test)]
    pub fn with_blocked_evidence_verified(mut self, verified: bool) -> Self {
        self.blocked_evidence_verified = verified;
        self
    }

    #[cfg(test)]
    pub fn with_failure_evidence_verified(mut self, verified: bool) -> Self {
        self.failure_evidence_verified = verified;
        self
    }

    #[cfg(test)]
    pub fn with_cancellation_observed(mut self, observed: bool) -> Self {
        self.cancellation_observed = observed;
        self
    }

    #[cfg(test)]
    pub fn with_latest_checkpoint_kind(mut self, kind: Option<LoopCheckpointKind>) -> Self {
        self.latest_checkpoint_kind = kind;
        self
    }
}

impl Default for InMemoryLoopExitEvidencePort {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LoopExitEvidencePort for InMemoryLoopExitEvidencePort {
    async fn verify_completion_refs(
        &self,
        _request: CompletionEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        Ok(self.completion_refs_verified)
    }

    async fn verify_final_checkpoint(
        &self,
        _request: FinalCheckpointEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        Ok(self.final_checkpoint_verified)
    }

    async fn verify_blocked_evidence(
        &self,
        _request: BlockedEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        Ok(self.blocked_evidence_verified)
    }

    async fn verify_failure_evidence(
        &self,
        _request: FailureEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        Ok(self.failure_evidence_verified)
    }

    async fn is_cancellation_observed(
        &self,
        _scope: &TurnScope,
        _turn_id: TurnId,
        _run_id: TurnRunId,
    ) -> Result<bool, TurnError> {
        Ok(self.cancellation_observed)
    }

    async fn latest_checkpoint_kind(
        &self,
        _scope: &TurnScope,
        _turn_id: TurnId,
        _run_id: TurnRunId,
    ) -> Result<Option<LoopCheckpointKind>, TurnError> {
        Ok(self.latest_checkpoint_kind)
    }
}

/// Durable text/checkpoint-backed evidence adapter for the current Reborn host.
///
/// Completions are trusted only when every reported reply ref and result ref is
/// backed by same-run finalized thread evidence. Result-ref-only completions
/// are allowed once matching finalized `ToolResultReference` records exist.
pub struct ThreadCheckpointLoopExitEvidencePort<S>
where
    S: SessionThreadService + ?Sized,
{
    thread_service: Arc<S>,
    turn_state_store: Arc<dyn TurnStateStore>,
    loop_checkpoint_store: Arc<dyn ironclaw_turns::LoopCheckpointStore>,
    checkpoint_state_store: Option<Arc<dyn CheckpointStateStore>>,
    approval_gate_evidence: Option<Arc<dyn ApprovalGateEvidenceStore>>,
    await_dependent_run_evidence: Option<Arc<dyn AwaitDependentRunEvidenceStore>>,
    thread_scope: Option<ThreadScope>,
    cancellation_factory: Option<Arc<dyn RunCancellationFactory>>,
}

#[async_trait]
pub trait ApprovalGateEvidenceStore: Send + Sync {
    async fn pending_approval_gate(
        &self,
        scope: &TurnScope,
        gate_ref: &ironclaw_turns::LoopGateRef,
    ) -> Result<bool, TurnError>;
}

#[async_trait]
pub trait AwaitDependentRunEvidenceStore: Send + Sync {
    async fn pending_awaited_child_gate(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        gate_ref: &LoopGateRef,
    ) -> Result<bool, TurnError>;
}

impl<S> ThreadCheckpointLoopExitEvidencePort<S>
where
    S: SessionThreadService + ?Sized,
{
    pub fn new(
        thread_service: Arc<S>,
        turn_state_store: Arc<dyn TurnStateStore>,
        loop_checkpoint_store: Arc<dyn ironclaw_turns::LoopCheckpointStore>,
    ) -> Self {
        Self {
            thread_service,
            turn_state_store,
            loop_checkpoint_store,
            checkpoint_state_store: None,
            approval_gate_evidence: None,
            await_dependent_run_evidence: None,
            thread_scope: None,
            cancellation_factory: None,
        }
    }

    pub fn new_with_thread_scope(
        thread_service: Arc<S>,
        turn_state_store: Arc<dyn TurnStateStore>,
        loop_checkpoint_store: Arc<dyn ironclaw_turns::LoopCheckpointStore>,
        thread_scope: ThreadScope,
    ) -> Self {
        Self {
            thread_service,
            turn_state_store,
            loop_checkpoint_store,
            checkpoint_state_store: None,
            approval_gate_evidence: None,
            await_dependent_run_evidence: None,
            thread_scope: Some(thread_scope),
            cancellation_factory: None,
        }
    }

    pub fn with_checkpoint_state_store(
        mut self,
        checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    ) -> Self {
        self.checkpoint_state_store = Some(checkpoint_state_store);
        self
    }

    pub fn with_approval_gate_evidence(
        mut self,
        approval_gate_evidence: Arc<dyn ApprovalGateEvidenceStore>,
    ) -> Self {
        self.approval_gate_evidence = Some(approval_gate_evidence);
        self
    }

    pub fn with_await_dependent_run_evidence(
        mut self,
        await_dependent_run_evidence: Arc<dyn AwaitDependentRunEvidenceStore>,
    ) -> Self {
        self.await_dependent_run_evidence = Some(await_dependent_run_evidence);
        self
    }

    pub fn with_cancellation_factory(
        mut self,
        cancellation_factory: Arc<dyn RunCancellationFactory>,
    ) -> Self {
        self.cancellation_factory = Some(cancellation_factory);
        self
    }
}

#[async_trait]
impl<S> LoopExitEvidencePort for ThreadCheckpointLoopExitEvidencePort<S>
where
    S: SessionThreadService + ?Sized + Send + Sync,
{
    async fn verify_completion_refs(
        &self,
        request: CompletionEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        if request.reply_message_refs.is_empty() && request.result_refs.is_empty() {
            return Ok(true);
        }
        let mut thread_scope = match &self.thread_scope {
            Some(thread_scope) => {
                ensure_thread_scope_matches_turn_scope(thread_scope, request.scope)?;
                thread_scope.clone()
            }
            None => thread_scope_from_turn_scope(request.scope)?,
        };
        // Multi-user: the loop host wrote this thread under the run's
        // authenticated owner (`owners/<caller>`), so the completion-ref
        // read must use the same owner — otherwise it looks in the wrong
        // subtree and fails with `unknown thread`. Apply the SAME
        // owner-rewrite rule the loop host uses, via the shared
        // [`ThreadScopeResolver`], so the two cannot drift. The run-state
        // read (for the actor) only runs when the base scope is
        // owner-scoped; an owner-less applier keeps its shared/system slot.
        if request.scope.has_explicit_thread_owner() {
            thread_scope = crate::thread_scope::ThreadScopeResolver::resolve_for_turn(
                &thread_scope,
                request.scope,
                None,
            );
        } else if thread_scope.owner_user_id.is_some() {
            let run_state = self
                .turn_state_store
                .get_run_state(GetRunStateRequest {
                    scope: request.scope.clone(),
                    run_id: request.run_id,
                })
                .await?;
            thread_scope = crate::thread_scope::ThreadScopeResolver::resolve_for_turn(
                &thread_scope,
                request.scope,
                run_state.actor.as_ref(),
            );
        }
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope,
                thread_id: request.scope.thread_id.clone(),
            })
            .await
            .map_err(|error| TurnError::Unavailable {
                reason: error.to_string(),
            })?;
        let expected_run_id = request.run_id.to_string();
        let replies_verified = request.reply_message_refs.iter().all(|message_ref| {
            verify_reply_message_ref(&history, message_ref, expected_run_id.as_str())
        });
        let results_verified = request.result_refs.iter().all(|result_ref| {
            verify_tool_result_ref(&history, result_ref, expected_run_id.as_str())
        });
        Ok(replies_verified && results_verified)
    }

    async fn verify_final_checkpoint(
        &self,
        request: FinalCheckpointEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        let checkpoint = self
            .loop_checkpoint_store
            .get_loop_checkpoint(ironclaw_turns::GetLoopCheckpointRequest {
                scope: request.scope.clone(),
                turn_id: request.turn_id,
                run_id: request.run_id,
                checkpoint_id: *request.checkpoint_id,
            })
            .await?;
        Ok(checkpoint
            .map(|record| record.kind == LoopCheckpointKind::Final)
            .unwrap_or(false))
    }

    async fn verify_blocked_evidence(
        &self,
        request: BlockedEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        match request.blocked.kind {
            LoopBlockedKind::Auth => {}
            LoopBlockedKind::Approval => {
                if !self.verify_pending_approval_gate(&request).await? {
                    return Ok(false);
                }
            }
            LoopBlockedKind::AwaitDependentRun => {
                if !self.verify_pending_awaited_child_gate(&request).await? {
                    return Ok(false);
                }
            }
            LoopBlockedKind::Resource => {
                // A BeforeBlock checkpoint alone is not sufficient for resource
                // gates: #3424 requires a durable pending process/resource ref.
                // Auth gates use
                // the blocked turn state itself as the product-visible pending ref,
                // so verifying the pre-block checkpoint is enough to let the
                // applier persist that state.
                return Ok(false);
            }
            // `LoopBlockedKind` is `#[non_exhaustive]` in a sibling crate, so
            // the compiler requires a wildcard arm. Any unknown variant is
            // intentionally failed closed: we have no evidence model for it
            // yet, and treating it as unverified prevents a new variant from
            // silently routing through the Auth carve-out.
            _ => return Ok(false),
        }

        let Some(checkpoint) = self
            .loop_checkpoint_store
            .get_loop_checkpoint(GetLoopCheckpointRequest {
                scope: request.scope.clone(),
                turn_id: request.turn_id,
                run_id: request.run_id,
                checkpoint_id: request.blocked.checkpoint_id,
            })
            .await?
        else {
            return Ok(false);
        };

        // Bind gate identity: the checkpoint must have been created for the
        // exact same gate that the blocked exit reports. This prevents a rogue
        // driver from reusing a legitimate BeforeBlock checkpoint (e.g. from
        // an Approval or Resource gate in the same run) as Auth evidence.
        Ok(checkpoint.kind == LoopCheckpointKind::BeforeBlock
            && checkpoint.state_ref == request.blocked.state_ref
            && checkpoint.gate_ref.as_ref() == Some(&request.blocked.gate_ref))
    }

    async fn verify_failure_evidence(
        &self,
        request: FailureEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        let Some(checkpoint_id) = request.failed.checkpoint_id else {
            return Ok(false);
        };
        let Some(checkpoint_state_store) = &self.checkpoint_state_store else {
            return Ok(false);
        };
        let Some(checkpoint) = self
            .loop_checkpoint_store
            .get_loop_checkpoint(GetLoopCheckpointRequest {
                scope: request.scope.clone(),
                turn_id: request.turn_id,
                run_id: request.run_id,
                checkpoint_id,
            })
            .await?
        else {
            return Ok(false);
        };
        if checkpoint.kind != LoopCheckpointKind::Final {
            return Ok(false);
        }
        let Some(checkpoint_state) = checkpoint_state_store
            .get_checkpoint_state(GetCheckpointStateRequest {
                scope: request.scope.clone(),
                turn_id: request.turn_id,
                run_id: request.run_id,
                state_ref: checkpoint.state_ref,
                schema_id: checkpoint.schema_id,
                schema_version: checkpoint.schema_version,
                kind: checkpoint.kind,
            })
            .await?
        else {
            return Ok(false);
        };
        let state = match ironclaw_agent_loop::state::LoopExecutionState::from_checkpoint_payload(
            checkpoint_state.payload.as_bytes(),
            ironclaw_agent_loop::state::CheckpointKind::Final,
        ) {
            Ok(state) => state,
            Err(_) => return Ok(false),
        };
        Ok(state
            .recent_failure_kinds
            .iter()
            .any(|kind| *kind == request.failed.reason_kind))
    }

    async fn is_cancellation_observed(
        &self,
        scope: &TurnScope,
        _turn_id: TurnId,
        run_id: TurnRunId,
    ) -> Result<bool, TurnError> {
        if let Some(cancellation_factory) = self.cancellation_factory.as_ref()
            && cancellation_factory
                .is_product_cancellation_observed(run_id)
                .map_err(|error| TurnError::Unavailable {
                    reason: error.safe_summary,
                })?
        {
            return Ok(true);
        }
        let state = self
            .turn_state_store
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await?;
        Ok(matches!(
            state.status,
            TurnStatus::CancelRequested | TurnStatus::Cancelled
        ))
    }

    async fn latest_checkpoint_kind(
        &self,
        _scope: &TurnScope,
        _turn_id: TurnId,
        _run_id: TurnRunId,
    ) -> Result<Option<LoopCheckpointKind>, TurnError> {
        // This adapter cannot query the latest checkpoint yet. Assume side
        // effects may have happened so invalid exits recover instead of
        // terminally failing a partially-applied run.
        Ok(Some(LoopCheckpointKind::BeforeSideEffect))
    }
}

impl<S> ThreadCheckpointLoopExitEvidencePort<S>
where
    S: SessionThreadService + ?Sized + Send + Sync,
{
    async fn verify_pending_approval_gate(
        &self,
        request: &BlockedEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        let Some(evidence) = &self.approval_gate_evidence else {
            return Ok(false);
        };
        evidence
            .pending_approval_gate(request.scope, &request.blocked.gate_ref)
            .await
    }

    async fn verify_pending_awaited_child_gate(
        &self,
        request: &BlockedEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        let Some(evidence) = &self.await_dependent_run_evidence else {
            return Ok(false);
        };
        evidence
            .pending_awaited_child_gate(request.scope, request.run_id, &request.blocked.gate_ref)
            .await
    }
}

#[async_trait]
impl AwaitDependentRunEvidenceStore
    for crate::subagent::gate_resolution::BoundedSubagentGateResolutionStore
{
    async fn pending_awaited_child_gate(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        gate_ref: &LoopGateRef,
    ) -> Result<bool, TurnError> {
        let gate_ref = GateRef::new(gate_ref.as_str()).map_err(|_| TurnError::InvalidRequest {
            reason: "awaited child gate evidence has invalid gate ref".to_string(),
        })?;
        let state = self
            .state_for_gate(&gate_ref)
            .map_err(|error| TurnError::Unavailable {
                reason: error.safe_summary,
            })?;
        Ok(state.is_some_and(|state| {
            state.record.parent_run_context.scope == *scope
                && state.record.parent_run_context.run_id == run_id
                && state.record.gate_ref == gate_ref
        }))
    }
}

fn thread_scope_from_turn_scope(scope: &TurnScope) -> Result<ThreadScope, TurnError> {
    // `ironclaw_threads::ThreadScope` is currently agent-scoped. Reject
    // agentless Reborn turns explicitly until the thread store grows an
    // agentless scope representation.
    let Some(agent_id) = scope.agent_id.clone() else {
        return Err(TurnError::InvalidRequest {
            reason: "thread checkpoint loop-exit evidence requires agent-scoped turn scope"
                .to_string(),
        });
    };

    Ok(ThreadScope {
        tenant_id: scope.tenant_id.clone(),
        agent_id,
        project_id: scope.project_id.clone(),
        owner_user_id: None,
        mission_id: None,
    })
}

fn ensure_thread_scope_matches_turn_scope(
    thread_scope: &ThreadScope,
    turn_scope: &TurnScope,
) -> Result<(), TurnError> {
    let Some(agent_id) = turn_scope.agent_id.as_ref() else {
        return Err(TurnError::InvalidRequest {
            reason: "thread checkpoint loop-exit evidence requires agent-scoped turn scope"
                .to_string(),
        });
    };
    if thread_scope.tenant_id != turn_scope.tenant_id
        || &thread_scope.agent_id != agent_id
        || thread_scope.project_id.as_ref() != turn_scope.project_id.as_ref()
    {
        return Err(TurnError::InvalidRequest {
            reason: "thread checkpoint loop-exit evidence scope does not match turn scope"
                .to_string(),
        });
    }
    Ok(())
}

fn message_id_from_ref(message_ref: &LoopMessageRef) -> Option<ThreadMessageId> {
    let raw = message_ref.as_str().strip_prefix("msg:")?;
    ThreadMessageId::parse(raw).ok()
}

fn verify_reply_message_ref(
    history: &ThreadHistory,
    message_ref: &LoopMessageRef,
    expected_run_id: &str,
) -> bool {
    let Some(message_id) = message_id_from_ref(message_ref) else {
        return false;
    };
    history.messages.iter().any(|message| {
        message.message_id == message_id
            && message.kind == MessageKind::Assistant
            && message.status == MessageStatus::Finalized
            && message.turn_run_id.as_deref() == Some(expected_run_id)
    })
}

fn verify_tool_result_ref(
    history: &ThreadHistory,
    result_ref: &LoopResultRef,
    expected_run_id: &str,
) -> bool {
    history.messages.iter().any(|message| {
        message.kind == MessageKind::ToolResultReference
            && message.status == MessageStatus::Finalized
            && message.turn_run_id.as_deref() == Some(expected_run_id)
            && message.tool_result_ref.as_deref() == Some(result_ref.as_str())
            && message_content_matches_result_ref(message, result_ref)
    })
}

fn message_content_matches_result_ref(
    message: &ThreadMessageRecord,
    result_ref: &LoopResultRef,
) -> bool {
    let Some(content) = message.content.as_deref() else {
        return false;
    };
    // Cheap metadata checks run before this helper. Keep the envelope parse so
    // forged or malformed transcript content cannot satisfy completion evidence.
    let Ok(envelope) = ToolResultReferenceEnvelope::from_json_str(content) else {
        return false;
    };
    envelope.result_ref == result_ref.as_str()
}

#[cfg(test)]
mod tests;
