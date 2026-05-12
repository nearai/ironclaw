//! Trusted `LoopExit` applier adapters for the Reborn turn-runner composition.
//!
//! `ironclaw_turns` owns the trusted applier and the private validation policy.
//! This module provides Reborn-specific evidence adapters.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_threads::{
    MessageStatus, SessionThreadService, ThreadHistoryRequest, ThreadMessageId, ThreadScope,
};
use ironclaw_turns::{LoopCheckpointKind, LoopMessageRef, TurnError, TurnId, TurnRunId, TurnScope};

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

/// Durable text/checkpoint-backed evidence adapter for the current Reborn
/// text-only host. Capability-result and gate/process evidence deliberately
/// remain untrusted until dedicated durable stores are wired.
pub struct ThreadCheckpointLoopExitEvidencePort<S>
where
    S: SessionThreadService + ?Sized,
{
    thread_service: Arc<S>,
    loop_checkpoint_store: Arc<dyn ironclaw_turns::LoopCheckpointStore>,
}

impl<S> ThreadCheckpointLoopExitEvidencePort<S>
where
    S: SessionThreadService + ?Sized,
{
    pub fn new(
        thread_service: Arc<S>,
        loop_checkpoint_store: Arc<dyn ironclaw_turns::LoopCheckpointStore>,
    ) -> Self {
        Self {
            thread_service,
            loop_checkpoint_store,
        }
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
        if !request.result_refs.is_empty() {
            return Ok(false);
        }
        if request.reply_message_refs.is_empty() {
            return Ok(true);
        }
        let thread_scope = thread_scope_from_turn_scope(request.scope)?;
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
        Ok(request.reply_message_refs.iter().all(|message_ref| {
            let Some(message_id) = message_id_from_ref(message_ref) else {
                return false;
            };
            history.messages.iter().any(|message| {
                message.message_id == message_id
                    && message.status == MessageStatus::Finalized
                    && message.turn_run_id.as_deref() == Some(expected_run_id.as_str())
            })
        }))
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
        _request: BlockedEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        // A BeforeBlock checkpoint alone is not sufficient: #3424 requires a
        // durable pending gate/process ref. The current text-only adapter has
        // no gate/process outcome store, so it must fail closed without doing
        // unrelated checkpoint I/O.
        Ok(false)
    }

    async fn verify_failure_evidence(
        &self,
        _request: FailureEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        // Failure exits require durable diagnostic evidence before trusting the
        // driver-supplied failure kind. The text-only adapter does not yet own
        // that diagnostics store, so it fails closed.
        Ok(false)
    }

    async fn is_cancellation_observed(
        &self,
        _scope: &TurnScope,
        _turn_id: TurnId,
        _run_id: TurnRunId,
    ) -> Result<bool, TurnError> {
        Ok(false)
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

fn message_id_from_ref(message_ref: &LoopMessageRef) -> Option<ThreadMessageId> {
    let raw = message_ref.as_str().strip_prefix("msg:")?;
    ThreadMessageId::parse(raw).ok()
}

#[cfg(test)]
mod tests;
