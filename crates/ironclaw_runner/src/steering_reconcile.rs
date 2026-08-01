//! Terminal reconciliation of stranded steering inputs.
//!
//! A message queued for a busy run is only ever delivered by that run's drain.
//! When the run is cancelled before its next drain, the queued input has no
//! consumer left: the transcript row would sit `Queued` forever — invisible to
//! the model, rendered as a live "queued" badge, with no resend affordance.
//!
//! [`CancelReconcilingTurnCoordinator`] decorates the ONE composed
//! [`TurnCoordinator`] so every cancel caller (WebUI cancel, gate deny, auth
//! cancel, automation) inherits the reconciliation: after a successful
//! `cancel_run`, the run's undrained queue entries are claimed and their rows
//! flipped `Queued` → `RejectedBusy` (resend affordance, never auto-resubmit)
//! via [`HostInputQueue::reject_unconsumed`].
//!
//! The flip is best-effort by design — the cancel outcome must never be failed
//! or delayed by transcript bookkeeping. A run that consumed an input first
//! keeps its `Submitted` row (`reject_unconsumed` claims-then-flips, so the
//! race resolves to whichever side acked first). Runs that reach `Failed`
//! without a cancel are not reconciled here; their queued rows remain the
//! documented stranded-terminal gap.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_loop_host::HostInputQueueReconcile;
use ironclaw_turns::{
    CancelRunRequest, CancelRunResponse, GetRunStateRequest, ResumeTurnRequest, ResumeTurnResponse,
    RetryTurnRequest, RetryTurnResponse, SubmitTurnRequest, SubmitTurnResponse, TurnCoordinator,
    TurnError, TurnRunId, TurnRunState, TurnScope,
};
use tracing::debug;

/// [`TurnCoordinator`] decorator that reconciles the run's steering input
/// queue after a successful `cancel_run`. Every other method forwards.
pub struct CancelReconcilingTurnCoordinator {
    inner: Arc<dyn TurnCoordinator>,
    input_queue: Arc<dyn HostInputQueueReconcile>,
}

impl CancelReconcilingTurnCoordinator {
    pub fn new(inner: Arc<dyn TurnCoordinator>, input_queue: Arc<dyn HostInputQueueReconcile>) -> Self {
        Self { inner, input_queue }
    }
}

#[async_trait]
impl TurnCoordinator for CancelReconcilingTurnCoordinator {
    async fn prepare_turn(&self, scope: TurnScope) -> Result<TurnRunId, TurnError> {
        self.inner.prepare_turn(scope).await
    }

    async fn abort_prepared_turn(&self, run_id: TurnRunId) -> Result<(), TurnError> {
        self.inner.abort_prepared_turn(run_id).await
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

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        self.inner.retry_turn(request).await
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        let run_id = request.run_id;
        let response = self.inner.cancel_run(request).await?;
        // The cancel is accepted (or the run was already terminal): the run
        // will not drain its queue again, so claim the stranded entries and
        // flip their rows to the resend affordance. Best-effort — the cancel
        // outcome is already settled and must not be failed by bookkeeping.
        match self.input_queue.reject_unconsumed(run_id).await {
            Ok(rejected) if !rejected.is_empty() => {
                debug!(
                    %run_id,
                    rejected = rejected.len(),
                    "flipped stranded queued messages to rejected-busy after cancel"
                );
            }
            Ok(_) => {}
            Err(error) => {
                // silent-ok: post-terminal best-effort reconciliation; the
                // cancel outcome is already settled and must not be failed or
                // delayed by transcript bookkeeping — the row stays visibly
                // Queued and remains reconcilable.
                debug!(
                    %run_id,
                    error = %error,
                    "steering-queue reconciliation after cancel failed; queued rows may lag"
                );
            }
        }
        Ok(response)
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.inner.get_run_state(request).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId, UserId};
    use ironclaw_loop_contracts::{LoopInputAckToken, LoopInputCursorToken};
    use ironclaw_loop_host::{HostInputBatch, HostInputQueueError};
    use ironclaw_threads::ThreadMessageId;
    use ironclaw_turns::{
        EventCursor, IdempotencyKey, SanitizedCancelReason, TurnActor, TurnStatus,
    };

    use super::*;

    struct ScriptedCancelCoordinator {
        cancel_result: StdMutex<Option<Result<CancelRunResponse, TurnError>>>,
    }

    #[async_trait]
    impl TurnCoordinator for ScriptedCancelCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            panic!("prepare_turn is not used by the reconcile decorator tests")
        }

        async fn submit_turn(
            &self,
            _request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            panic!("submit_turn is not used by the reconcile decorator tests")
        }

        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            panic!("resume_turn is not used by the reconcile decorator tests")
        }

        async fn retry_turn(
            &self,
            _request: RetryTurnRequest,
        ) -> Result<RetryTurnResponse, TurnError> {
            panic!("retry_turn is not used by the reconcile decorator tests")
        }

        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            self.cancel_result
                .lock()
                .expect("lock")
                .take()
                .expect("cancel scripted once")
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            panic!("get_run_state is not used by the reconcile decorator tests")
        }
    }

    #[derive(Default)]
    struct RecordingQueue {
        rejected_runs: StdMutex<Vec<TurnRunId>>,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl HostInputQueueReconcile for RecordingQueue {
        async fn reject_unconsumed(
            &self,
            run_id: TurnRunId,
        ) -> Result<Vec<ThreadMessageId>, HostInputQueueError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.rejected_runs.lock().expect("lock").push(run_id);
            Ok(Vec::new())
        }
    }

    fn cancel_request(run_id: TurnRunId) -> CancelRunRequest {
        CancelRunRequest {
            scope: TurnScope::new(
                TenantId::new("tenant-reconcile").expect("tenant"),
                Some(AgentId::new("agent-reconcile").expect("agent")),
                Some(ProjectId::new("project-reconcile").expect("project")),
                ThreadId::new("thread-reconcile").expect("thread"),
            ),
            actor: TurnActor::new(UserId::new("user-reconcile").expect("user")),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("cancel-reconcile").expect("key"),
        }
    }

    fn cancelled_response(run_id: TurnRunId) -> CancelRunResponse {
        CancelRunResponse {
            run_id,
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor::default(),
            already_terminal: false,
            actor: None,
        }
    }

    #[tokio::test]
    async fn successful_cancel_reconciles_the_run_queue() {
        let run_id = TurnRunId::new();
        let queue = Arc::new(RecordingQueue::default());
        let coordinator = CancelReconcilingTurnCoordinator::new(
            Arc::new(ScriptedCancelCoordinator {
                cancel_result: StdMutex::new(Some(Ok(cancelled_response(run_id)))),
            }),
            queue.clone(),
        );

        let response = coordinator
            .cancel_run(cancel_request(run_id))
            .await
            .expect("cancel forwards");

        assert_eq!(response.status, TurnStatus::Cancelled);
        assert_eq!(*queue.rejected_runs.lock().expect("lock"), vec![run_id]);
    }

    #[tokio::test]
    async fn failed_cancel_does_not_touch_the_queue() {
        let run_id = TurnRunId::new();
        let queue = Arc::new(RecordingQueue::default());
        let coordinator = CancelReconcilingTurnCoordinator::new(
            Arc::new(ScriptedCancelCoordinator {
                cancel_result: StdMutex::new(Some(Err(TurnError::Unauthorized))),
            }),
            queue.clone(),
        );

        let result = coordinator.cancel_run(cancel_request(run_id)).await;

        assert!(matches!(result, Err(TurnError::Unauthorized)));
        assert_eq!(queue.calls.load(Ordering::SeqCst), 0);
    }
}
