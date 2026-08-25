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
//! via [`HostInputQueueReconcile::reject_unconsumed`].
//!
//! [`SteeringReconcilingProcessTransitions`] applies the same reconciliation
//! at the other choke point — the ONE composed
//! [`ProcessTransitionPort`](ironclaw_processes::ProcessTransitionPort) every
//! terminal run transition flows through (loop-exit completion, failure,
//! cancellation, and the executor/scheduler failure fallbacks) — so a run
//! that reaches `Completed` or `Failed` without a cancel also reclaims its
//! queue record and settles any stranded rows, instead of leaking one queue
//! record per steered run for the life of the deployment.
//!
//! The flip is best-effort by design — the terminal outcome must never be
//! failed or delayed by transcript bookkeeping. A run that consumed an input
//! first keeps its `Submitted` row (`reject_unconsumed` claims-then-flips, so
//! the race resolves to whichever side claimed first, and a consumed row
//! whose `Submitted` flip is still pending is repaired rather than
//! rejected).

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ids::ProcessId;
use ironclaw_loop_host::HostInputQueueReconcile;
use ironclaw_processes::{
    ClaimProcessesRequest, ClaimedProcess, FailProcessRequest, JournaledProcessSnapshot,
    ProcessJournalCommit, ProcessJournalCommitObserver, ProcessJournalCursor, ProcessJournalKind,
    ProcessKind, ProcessLeaseRequest, ProcessStateTransitionRequest, ProcessTransitionPort,
    RecoverExpiredProcessLeasesRequest, RecoverExpiredProcessLeasesResponse, SuspendProcessRequest,
};
use ironclaw_turns::{
    ActivateThreadRequest, CancelRunRequest, CancelRunResponse, GetRunStateRequest,
    ResumeTurnRequest, ResumeTurnResponse, RetryTurnRequest, RetryTurnResponse, SubmitTurnRequest,
    SubmitTurnResponse, TurnCoordinator, TurnError, TurnRunId, TurnRunState, TurnScope,
};
use tracing::debug;

/// [`TurnCoordinator`] decorator that reconciles the run's steering input
/// queue after a successful `cancel_run`. Every other method forwards.
pub struct CancelReconcilingTurnCoordinator {
    inner: Arc<dyn TurnCoordinator>,
    input_queue: Arc<dyn HostInputQueueReconcile>,
}

impl CancelReconcilingTurnCoordinator {
    pub fn new(
        inner: Arc<dyn TurnCoordinator>,
        input_queue: Arc<dyn HostInputQueueReconcile>,
    ) -> Self {
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

    async fn activate(
        &self,
        request: ActivateThreadRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.inner.activate(request).await
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

/// [`ProcessJournalCommitObserver`] that reconciles the run's steering queue
/// on every TERMINAL agent-turn journal commit, whichever caller performed
/// the transition. This is the completeness net over
/// [`SteeringReconcilingProcessTransitions`]: the scheduler/supervisor
/// terminalizes crash-reclaimed and panicked runs through the raw
/// [`ProcessRuntimePort`](ironclaw_processes::ProcessRuntimePort) handle
/// (which the decorator never sees), but every terminal transition — from any
/// writer — lands in the process journal, and the observer registry delivers
/// commits durably (cursor-tracked, retried, replayed across restarts). The
/// reconciliation is idempotent, so double delivery via decorator + observer
/// is a no-op.
///
/// Delivery-contract note: reconciliation failures return `Ok` (logged at
/// debug) rather than `Err` — an `Err` would wedge this observer's durable
/// cursor behind one permanently unreconcilable run and stall delivery for
/// every later terminal commit. The retained queue record remains the durable
/// retry source.
pub struct SteeringReconcileCommitObserver {
    input_queue: Arc<dyn HostInputQueueReconcile>,
}

impl SteeringReconcileCommitObserver {
    pub fn new(input_queue: Arc<dyn HostInputQueueReconcile>) -> Self {
        Self { input_queue }
    }
}

#[async_trait]
impl ProcessJournalCommitObserver for SteeringReconcileCommitObserver {
    fn process_observer_id(&self) -> &'static str {
        "steering-queue-terminal-reconcile-v1"
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        if commit.state.process_kind != ProcessKind::AgentTurn {
            return Ok(());
        }
        if !matches!(
            commit.kind,
            ProcessJournalKind::Completed
                | ProcessJournalKind::Failed
                | ProcessJournalKind::Cancelled
                | ProcessJournalKind::Stopped
                | ProcessJournalKind::Killed
        ) {
            return Ok(());
        }
        let run_id = TurnRunId::from_uuid(commit.state.process_id.as_uuid());
        match self.input_queue.reject_unconsumed(run_id).await {
            Ok(rejected) if !rejected.is_empty() => {
                debug!(
                    %run_id,
                    kind = ?commit.kind,
                    rejected = rejected.len(),
                    "flipped stranded queued messages to rejected-busy after terminal journal commit"
                );
            }
            Ok(_) => {}
            Err(error) => {
                // silent-ok: best-effort post-terminal reconciliation — an Err
                // here would wedge the observer's durable cursor behind one
                // unreconcilable run; the retained queue record keeps the rows
                // reconcilable instead.
                debug!(
                    %run_id,
                    kind = ?commit.kind,
                    error = %error,
                    "steering-queue reconciliation after terminal journal commit failed; \
                     queued rows may lag"
                );
            }
        }
        Ok(())
    }
}

/// [`ProcessTransitionPort`] decorator that reconciles the run's steering
/// queue after every successful terminal transition (`complete_process`,
/// `fail_process`, `cancel_process`). Non-terminal methods forward untouched.
///
/// This is the reclamation path for runs that end WITHOUT a cancel: the
/// (idempotent) `reject_unconsumed` closes the run's queue, settles any rows
/// still `Queued`, and reclaims the per-run queue record once settled — so
/// normally-completed steered runs do not accumulate queue records. Process
/// ids are the run uuid (`process_id_from_turn_run_id` in `ironclaw_turns`),
/// so the run identity is recovered from the transition request. Scheduler /
/// supervisor terminal writes that bypass this decorated handle are covered
/// by [`SteeringReconcileCommitObserver`].
pub struct SteeringReconcilingProcessTransitions<E> {
    inner: Arc<dyn ProcessTransitionPort<Error = E>>,
    input_queue: Arc<dyn HostInputQueueReconcile>,
}

impl<E> SteeringReconcilingProcessTransitions<E> {
    pub fn new(
        inner: Arc<dyn ProcessTransitionPort<Error = E>>,
        input_queue: Arc<dyn HostInputQueueReconcile>,
    ) -> Self {
        Self { inner, input_queue }
    }

    /// Best-effort post-terminal reconciliation: the terminal transition is
    /// already committed and must not be failed or delayed by transcript
    /// bookkeeping.
    async fn reconcile(&self, process_id: &ProcessId, operation: &'static str) {
        let run_id = TurnRunId::from_uuid(process_id.as_uuid());
        match self.input_queue.reject_unconsumed(run_id).await {
            Ok(rejected) if !rejected.is_empty() => {
                debug!(
                    %run_id,
                    operation,
                    rejected = rejected.len(),
                    "flipped stranded queued messages to rejected-busy after terminal transition"
                );
            }
            Ok(_) => {}
            Err(error) => {
                // silent-ok: post-terminal best-effort reconciliation; the
                // terminal outcome is already settled and must not be failed
                // or delayed by transcript bookkeeping — the retained queue
                // record keeps the rows reconcilable.
                debug!(
                    %run_id,
                    operation,
                    error = %error,
                    "steering-queue reconciliation after terminal transition failed; \
                     queued rows may lag"
                );
            }
        }
    }
}

#[async_trait]
impl<E> ProcessTransitionPort for SteeringReconcilingProcessTransitions<E>
where
    E: Send + Sync + 'static,
{
    type Error = E;

    async fn claim_next_processes(
        &self,
        request: ClaimProcessesRequest,
    ) -> Result<Vec<ClaimedProcess>, Self::Error> {
        self.inner.claim_next_processes(request).await
    }

    async fn heartbeat_process(
        &self,
        request: ProcessLeaseRequest,
    ) -> Result<ProcessJournalCursor, Self::Error> {
        self.inner.heartbeat_process(request).await
    }

    async fn recover_expired_process_leases(
        &self,
        request: RecoverExpiredProcessLeasesRequest,
    ) -> Result<RecoverExpiredProcessLeasesResponse, Self::Error> {
        self.inner.recover_expired_process_leases(request).await
    }

    async fn suspend_process(
        &self,
        request: SuspendProcessRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.inner.suspend_process(request).await
    }

    async fn complete_process(
        &self,
        request: ProcessStateTransitionRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        let process_id = request.lease.process_id;
        let snapshot = self.inner.complete_process(request).await?;
        self.reconcile(&process_id, "complete_process").await;
        Ok(snapshot)
    }

    async fn cancel_process(
        &self,
        request: ProcessStateTransitionRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        let process_id = request.lease.process_id;
        let snapshot = self.inner.cancel_process(request).await?;
        self.reconcile(&process_id, "cancel_process").await;
        Ok(snapshot)
    }

    async fn fail_process(
        &self,
        request: FailProcessRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        let process_id = request.process_id;
        let snapshot = self.inner.fail_process(request).await?;
        self.reconcile(&process_id, "fail_process").await;
        Ok(snapshot)
    }

    async fn relinquish_process(
        &self,
        request: ProcessLeaseRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.inner.relinquish_process(request).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId, UserId};
    use ironclaw_loop_host::HostInputQueueError;
    use ironclaw_threads::ThreadMessageId;
    use ironclaw_turns::{
        AcceptedMessageRef, ActivateThreadRequest, ActivationProvenance, EventCursor,
        IdempotencyKey, SanitizedCancelReason, TurnActor, TurnStatus,
    };

    use super::*;

    /// The decorator's contract is "every other method forwards". A method it
    /// forgets silently inherits the trait's fail-closed default, and because
    /// this is the ONE coordinator production composes, that turns into "the
    /// feature is simply off in production" with no compile error to catch it.
    #[tokio::test]
    async fn activate_forwards_to_the_inner_coordinator() {
        struct ActivateRecordingCoordinator {
            seen: StdMutex<Vec<ActivationProvenance>>,
        }

        #[async_trait]
        impl TurnCoordinator for ActivateRecordingCoordinator {
            async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
                panic!("prepare_turn is not used by this test")
            }
            async fn submit_turn(
                &self,
                _request: SubmitTurnRequest,
            ) -> Result<SubmitTurnResponse, TurnError> {
                panic!("submit_turn is not used by this test")
            }
            async fn activate(
                &self,
                request: ActivateThreadRequest,
            ) -> Result<SubmitTurnResponse, TurnError> {
                self.seen
                    .lock()
                    .expect("activation recorder poisoned")
                    .push(request.provenance);
                Err(TurnError::Unavailable {
                    reason: "inner reached".to_string(),
                })
            }
            async fn resume_turn(
                &self,
                _request: ResumeTurnRequest,
            ) -> Result<ResumeTurnResponse, TurnError> {
                panic!("resume_turn is not used by this test")
            }
            async fn retry_turn(
                &self,
                _request: RetryTurnRequest,
            ) -> Result<RetryTurnResponse, TurnError> {
                panic!("retry_turn is not used by this test")
            }
            async fn cancel_run(
                &self,
                _request: CancelRunRequest,
            ) -> Result<CancelRunResponse, TurnError> {
                panic!("cancel_run is not used by this test")
            }
            async fn get_run_state(
                &self,
                _request: GetRunStateRequest,
            ) -> Result<TurnRunState, TurnError> {
                panic!("get_run_state is not used by this test")
            }
        }

        let inner = Arc::new(ActivateRecordingCoordinator {
            seen: StdMutex::new(Vec::new()),
        });
        let decorated = CancelReconcilingTurnCoordinator::new(
            Arc::clone(&inner) as Arc<dyn TurnCoordinator>,
            Arc::new(RecordingQueue::default()),
        );

        let error = decorated
            .activate(ActivateThreadRequest {
                scope: TurnScope::new(
                    TenantId::new("tenant-reconcile").expect("tenant"),
                    Some(AgentId::new("agent-reconcile").expect("agent")),
                    Some(ProjectId::new("project-reconcile").expect("project")),
                    ThreadId::new("thread-reconcile").expect("thread"),
                ),
                actor: TurnActor::new(UserId::new("user-forward").expect("user")),
                accepted_message_ref: AcceptedMessageRef::new("accepted-forward")
                    .expect("accepted"),
                provenance: ActivationProvenance::System,
                idempotency_key: IdempotencyKey::new("forward-key").expect("key"),
                received_at: chrono::Utc::now(),
                requested_run_profile: None,
                resolved_run_profile: None,
            })
            .await
            .expect_err("the recording inner coordinator always errors");

        assert!(
            matches!(error, TurnError::Unavailable { .. }),
            "the decorator must surface the INNER coordinator's outcome, not the \
             trait's fail-closed default; got {error:?}"
        );
        assert_eq!(
            inner
                .seen
                .lock()
                .expect("activation recorder poisoned")
                .as_slice(),
            &[ActivationProvenance::System],
            "activate must reach the inner coordinator exactly once"
        );
    }

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

    /// A queue that fails every reconciliation — the failure-isolation double
    /// `RecordingQueue` (always `Ok`) cannot reach the decorators' `Err` arms.
    struct FailingQueue {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl HostInputQueueReconcile for FailingQueue {
        async fn reject_unconsumed(
            &self,
            _run_id: TurnRunId,
        ) -> Result<Vec<ThreadMessageId>, HostInputQueueError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(HostInputQueueError::Unavailable {
                reason: "scripted reconcile failure".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn failed_reconciliation_does_not_fail_the_successful_cancel() {
        // The decorator's contract: reconciliation is best-effort — a
        // successful cancellation stays successful when the queue's
        // reject_unconsumed fails.
        let run_id = TurnRunId::new();
        let queue = Arc::new(FailingQueue {
            calls: AtomicUsize::new(0),
        });
        let coordinator = CancelReconcilingTurnCoordinator::new(
            Arc::new(ScriptedCancelCoordinator {
                cancel_result: StdMutex::new(Some(Ok(cancelled_response(run_id)))),
            }),
            queue.clone(),
        );

        let response = coordinator
            .cancel_run(cancel_request(run_id))
            .await
            .expect("cancel outcome survives the failed reconciliation");

        assert_eq!(response.status, TurnStatus::Cancelled);
        assert_eq!(queue.calls.load(Ordering::SeqCst), 1);
    }
}

#[cfg(test)]
mod transition_decorator_tests {
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_host_api::{
        ids::{InvocationId, TenantId, UserId},
        resource::ResourceScope,
        turn::SanitizedFailure,
    };
    use ironclaw_loop_host::HostInputQueueError;
    use ironclaw_processes::{
        ProcessJournalCursor, ProcessKind, ProcessLeaseToken, ProcessLifecycleStatus,
        ProcessWorkerId,
    };
    use ironclaw_threads::ThreadMessageId;
    use ironclaw_turns::{TurnError, TurnRunId};

    use super::*;

    fn scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-reconcile").expect("tenant"),
            user_id: UserId::new("user-reconcile").expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn snapshot(process_id: ProcessId, status: ProcessLifecycleStatus) -> JournaledProcessSnapshot {
        JournaledProcessSnapshot {
            process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: scope(),
            status,
            suspension: None,
            checkpoint_ref: None,
            checkpoint_kind: None,
            input_ref: None,
            failure: None,
            journal_cursor: ProcessJournalCursor(0),
            lease: None,
            crash_reclaim_count: 0,
            created_at: chrono::Utc::now(),
            owner_user_id: None,
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            metadata: serde_json::Value::Null,
        }
    }

    fn lease(process_id: ProcessId) -> ProcessLeaseRequest {
        ProcessLeaseRequest {
            process_id,
            worker_id: ProcessWorkerId::from_trusted("worker-reconcile"),
            lease_token: ProcessLeaseToken::from_trusted("lease-reconcile"),
        }
    }

    fn transition_request(process_id: ProcessId) -> ProcessStateTransitionRequest {
        ProcessStateTransitionRequest {
            lease: lease(process_id),
            metadata: None,
        }
    }

    /// Forwards terminal transitions with a scripted terminal status; panics
    /// on the non-terminal methods the decorator must forward untouched
    /// (exercised separately via `suspend`).
    struct ScriptedTransitions;

    #[async_trait]
    impl ProcessTransitionPort for ScriptedTransitions {
        type Error = TurnError;

        async fn claim_next_processes(
            &self,
            _request: ClaimProcessesRequest,
        ) -> Result<Vec<ClaimedProcess>, Self::Error> {
            Ok(Vec::new())
        }

        async fn heartbeat_process(
            &self,
            request: ProcessLeaseRequest,
        ) -> Result<ProcessJournalCursor, Self::Error> {
            let _ = request;
            Ok(ProcessJournalCursor(0))
        }

        async fn recover_expired_process_leases(
            &self,
            _request: RecoverExpiredProcessLeasesRequest,
        ) -> Result<RecoverExpiredProcessLeasesResponse, Self::Error> {
            panic!("recover_expired_process_leases is not used by these tests")
        }

        async fn suspend_process(
            &self,
            request: SuspendProcessRequest,
        ) -> Result<JournaledProcessSnapshot, Self::Error> {
            Ok(snapshot(
                request.process_id,
                ProcessLifecycleStatus::Suspended,
            ))
        }

        async fn complete_process(
            &self,
            request: ProcessStateTransitionRequest,
        ) -> Result<JournaledProcessSnapshot, Self::Error> {
            Ok(snapshot(
                request.lease.process_id,
                ProcessLifecycleStatus::Completed,
            ))
        }

        async fn cancel_process(
            &self,
            request: ProcessStateTransitionRequest,
        ) -> Result<JournaledProcessSnapshot, Self::Error> {
            Ok(snapshot(
                request.lease.process_id,
                ProcessLifecycleStatus::Cancelled,
            ))
        }

        async fn fail_process(
            &self,
            request: FailProcessRequest,
        ) -> Result<JournaledProcessSnapshot, Self::Error> {
            Ok(snapshot(request.process_id, ProcessLifecycleStatus::Failed))
        }

        async fn relinquish_process(
            &self,
            request: ProcessLeaseRequest,
        ) -> Result<JournaledProcessSnapshot, Self::Error> {
            Ok(snapshot(request.process_id, ProcessLifecycleStatus::Queued))
        }
    }

    #[derive(Default)]
    struct RecordingQueue {
        reconciled_runs: StdMutex<Vec<TurnRunId>>,
    }

    #[async_trait]
    impl HostInputQueueReconcile for RecordingQueue {
        async fn reject_unconsumed(
            &self,
            run_id: TurnRunId,
        ) -> Result<Vec<ThreadMessageId>, HostInputQueueError> {
            self.reconciled_runs.lock().expect("lock").push(run_id);
            Ok(Vec::new())
        }
    }

    fn decorated(
        queue: Arc<dyn HostInputQueueReconcile>,
    ) -> SteeringReconcilingProcessTransitions<TurnError> {
        SteeringReconcilingProcessTransitions::new(Arc::new(ScriptedTransitions), queue)
    }

    #[tokio::test]
    async fn every_terminal_transition_reconciles_the_run_queue() {
        // The reclamation guarantee: Completed and Failed runs (not only
        // cancels) close and reclaim their steering queue.
        let queue = Arc::new(RecordingQueue::default());
        let port = decorated(queue.clone());

        let complete_run = TurnRunId::new();
        port.complete_process(transition_request(ProcessId::from_uuid(
            complete_run.as_uuid(),
        )))
        .await
        .expect("complete forwards");

        let failed_run = TurnRunId::new();
        port.fail_process(FailProcessRequest {
            process_id: ProcessId::from_uuid(failed_run.as_uuid()),
            worker_id: ProcessWorkerId::from_trusted("worker-reconcile"),
            lease_token: ProcessLeaseToken::from_trusted("lease-reconcile"),
            failure: SanitizedFailure::from_trusted_static("unknown_failure"),
            recovery: Default::default(),
            checkpoint_ref: None,
            metadata: None,
        })
        .await
        .expect("fail forwards");

        let cancelled_run = TurnRunId::new();
        port.cancel_process(transition_request(ProcessId::from_uuid(
            cancelled_run.as_uuid(),
        )))
        .await
        .expect("cancel forwards");

        assert_eq!(
            *queue.reconciled_runs.lock().expect("lock"),
            vec![complete_run, failed_run, cancelled_run],
            "every terminal transition must reconcile the run's queue, keyed by the run id"
        );
    }

    #[tokio::test]
    async fn non_terminal_transitions_do_not_touch_the_queue() {
        let queue = Arc::new(RecordingQueue::default());
        let port = decorated(queue.clone());
        let run_id = TurnRunId::new();

        port.suspend_process(SuspendProcessRequest {
            process_id: ProcessId::from_uuid(run_id.as_uuid()),
            worker_id: ProcessWorkerId::from_trusted("worker-reconcile"),
            lease_token: ProcessLeaseToken::from_trusted("lease-reconcile"),
            checkpoint_ref: ironclaw_processes::ProcessCheckpointRef::from_trusted(
                "checkpoint-reconcile",
            ),
            suspension: suspension(),
            metadata: None,
        })
        .await
        .expect("suspend forwards");
        port.heartbeat_process(lease(ProcessId::from_uuid(run_id.as_uuid())))
            .await
            .expect("heartbeat forwards");

        assert!(
            queue.reconciled_runs.lock().expect("lock").is_empty(),
            "non-terminal transitions must not reconcile the queue"
        );
    }

    #[tokio::test]
    async fn failed_reconciliation_does_not_fail_the_terminal_transition() {
        struct FailingQueue {
            calls: AtomicUsize,
        }

        #[async_trait]
        impl HostInputQueueReconcile for FailingQueue {
            async fn reject_unconsumed(
                &self,
                _run_id: TurnRunId,
            ) -> Result<Vec<ThreadMessageId>, HostInputQueueError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Err(HostInputQueueError::Unavailable {
                    reason: "scripted reconcile failure".to_string(),
                })
            }
        }

        let queue = Arc::new(FailingQueue {
            calls: AtomicUsize::new(0),
        });
        let port = decorated(queue.clone());
        let run_id = TurnRunId::new();

        let snapshot = port
            .complete_process(transition_request(ProcessId::from_uuid(run_id.as_uuid())))
            .await
            .expect("the committed terminal transition survives the failed reconciliation");

        assert_eq!(snapshot.status, ProcessLifecycleStatus::Completed);
        assert_eq!(queue.calls.load(Ordering::SeqCst), 1);
    }

    fn suspension() -> ironclaw_processes::ProcessSuspension {
        ironclaw_processes::ProcessSuspension {
            kind: ironclaw_processes::ProcessSuspensionKind::Approval,
            gate_ref: None,
            activity_id: None,
            credential_requirements: Vec::new(),
            detail: None,
        }
    }
}

#[cfg(test)]
mod observer_tests {
    use std::sync::Mutex as StdMutex;

    use chrono::Utc;
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::ids::{TenantId, UserId};
    use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
    use ironclaw_host_api::path::{MountAlias, VirtualPath};
    use ironclaw_host_api::resource::ResourceScope;
    use ironclaw_loop_host::HostInputQueueError;
    use ironclaw_processes::{
        ProcessFailureRecovery, ProcessJournalObserverRegistry, ProcessJournalStore,
        ProcessOperationId, ProcessSubmissionPort, ProcessWorkerId, SubmitProcessRequest,
    };
    use ironclaw_threads::ThreadMessageId;
    use ironclaw_turns::TurnRunId;

    use super::*;

    fn processes_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/processes").expect("mount alias"),
            VirtualPath::new("/engine/processes").expect("virtual path"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            mounts,
        ))
    }

    #[derive(Default)]
    struct RecordingQueue {
        reconciled_runs: StdMutex<Vec<TurnRunId>>,
    }

    #[async_trait]
    impl HostInputQueueReconcile for RecordingQueue {
        async fn reject_unconsumed(
            &self,
            run_id: TurnRunId,
        ) -> Result<Vec<ThreadMessageId>, HostInputQueueError> {
            self.reconciled_runs.lock().expect("lock").push(run_id);
            Ok(Vec::new())
        }
    }

    fn scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-observer").expect("tenant"),
            user_id: UserId::new("user-observer").expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::ids::InvocationId::new(),
        }
    }

    /// The regression the raw-handle gap demands: a terminal transition
    /// performed directly on the journal store — the same undecorated handle
    /// the scheduler/supervisor uses for crash-reclaimed and panicked runs —
    /// still reconciles the run's steering queue, because the subscribed
    /// commit observer sees every terminal journal commit regardless of
    /// which caller wrote it.
    #[tokio::test]
    async fn raw_store_terminal_transition_reconciles_via_commit_observer() {
        let store = ProcessJournalStore::new(processes_filesystem());
        let queue = Arc::new(RecordingQueue::default());
        store
            .subscribe_process_observer(Arc::new(SteeringReconcileCommitObserver::new(
                queue.clone(),
            )))
            .expect("subscribe observer");

        let scope = scope();
        let process_id = ironclaw_host_api::ids::ProcessId::new();
        store
            .submit_process(SubmitProcessRequest {
                process_id,
                process_kind: ProcessKind::AgentTurn,
                scope: scope.clone(),
                exclusive_within_scope: false,
                operation_id: Some(ProcessOperationId::from_trusted("observer-regression")),
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
            .expect("submit process");
        let worker_id = ProcessWorkerId::from_trusted("observer-worker");
        let claimed = store
            .claim_next_processes(ClaimProcessesRequest {
                worker_id: worker_id.clone(),
                scope_filter: Some(scope.clone()),
                process_id_filter: Some(process_id),
                process_kind_filter: Some(ProcessKind::AgentTurn),
                max_processes: 1,
            })
            .await
            .expect("claim process");
        assert_eq!(claimed.len(), 1);

        // Terminalize through the RAW store handle — no decorator in sight.
        store
            .fail_process(FailProcessRequest {
                process_id,
                worker_id,
                lease_token: claimed[0].lease_token.clone(),
                failure: ironclaw_host_api::turn::SanitizedFailure::from_trusted_static(
                    "scheduler_executor_panic",
                ),
                recovery: ProcessFailureRecovery::Terminal,
                checkpoint_ref: None,
                metadata: None,
            })
            .await
            .expect("fail process through the raw handle");

        // Observer delivery is asynchronous-but-prompt; poll briefly.
        let expected_run = TurnRunId::from_uuid(process_id.as_uuid());
        for _ in 0..100 {
            if queue
                .reconciled_runs
                .lock()
                .expect("lock")
                .contains(&expected_run)
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("terminal journal commit on the raw handle must reconcile the steering queue");
    }

    #[tokio::test]
    async fn non_terminal_and_foreign_kind_commits_do_not_reconcile() {
        let queue = Arc::new(RecordingQueue::default());
        let observer = SteeringReconcileCommitObserver::new(queue.clone());
        let process_id = ironclaw_host_api::ids::ProcessId::new();

        let commit = |process_kind, kind| ProcessJournalCommit {
            state: JournaledProcessSnapshot {
                process_id,
                process_kind,
                scope: scope(),
                status: ironclaw_processes::ProcessLifecycleStatus::Running,
                suspension: None,
                checkpoint_ref: None,
                checkpoint_kind: None,
                input_ref: None,
                failure: None,
                journal_cursor: ProcessJournalCursor(0),
                lease: None,
                crash_reclaim_count: 0,
                created_at: Utc::now(),
                owner_user_id: None,
                concurrency_class: None,
                parent_process_id: None,
                root_process_id: None,
                metadata: serde_json::Value::Null,
            },
            kind,
            occurred_at: None,
            sanitized_reason: None,
        };

        observer
            .observe_process_commit(commit(ProcessKind::AgentTurn, ProcessJournalKind::Claimed))
            .await
            .expect("non-terminal commit accepted");
        observer
            .observe_process_commit(commit(ProcessKind::Internal, ProcessJournalKind::Failed))
            .await
            .expect("foreign-kind commit accepted");
        assert!(
            queue.reconciled_runs.lock().expect("lock").is_empty(),
            "only terminal agent-turn commits may reconcile"
        );

        observer
            .observe_process_commit(commit(ProcessKind::AgentTurn, ProcessJournalKind::Failed))
            .await
            .expect("terminal commit accepted");
        assert_eq!(
            *queue.reconciled_runs.lock().expect("lock"),
            vec![TurnRunId::from_uuid(process_id.as_uuid())]
        );
    }

    #[tokio::test]
    async fn failed_reconciliation_never_wedges_the_observer_cursor() {
        struct FailingQueue;

        #[async_trait]
        impl HostInputQueueReconcile for FailingQueue {
            async fn reject_unconsumed(
                &self,
                _run_id: TurnRunId,
            ) -> Result<Vec<ThreadMessageId>, HostInputQueueError> {
                Err(HostInputQueueError::Unavailable {
                    reason: "scripted reconcile failure".to_string(),
                })
            }
        }

        let observer = SteeringReconcileCommitObserver::new(Arc::new(FailingQueue));
        let commit = ProcessJournalCommit {
            state: JournaledProcessSnapshot {
                process_id: ironclaw_host_api::ids::ProcessId::new(),
                process_kind: ProcessKind::AgentTurn,
                scope: scope(),
                status: ironclaw_processes::ProcessLifecycleStatus::Failed,
                suspension: None,
                checkpoint_ref: None,
                checkpoint_kind: None,
                input_ref: None,
                failure: None,
                journal_cursor: ProcessJournalCursor(0),
                lease: None,
                crash_reclaim_count: 0,
                created_at: Utc::now(),
                owner_user_id: None,
                concurrency_class: None,
                parent_process_id: None,
                root_process_id: None,
                metadata: serde_json::Value::Null,
            },
            kind: ProcessJournalKind::Failed,
            occurred_at: None,
            sanitized_reason: None,
        };

        observer
            .observe_process_commit(commit)
            .await
            .expect("a failed reconciliation must return Ok so the durable cursor advances");
    }
}
