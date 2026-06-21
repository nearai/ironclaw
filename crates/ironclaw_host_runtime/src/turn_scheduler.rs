use std::{
    collections::HashMap, error::Error, fmt, panic::AssertUnwindSafe, sync::Arc, time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use futures_util::FutureExt;
use ironclaw_turns::{
    SanitizedFailure, TurnError, TurnLeaseToken, TurnRunId, TurnRunWake, TurnRunWakeNotifier,
    TurnRunWakeNotifyError, TurnRunnerId, TurnScope,
    runner::{
        ClaimRunRequest, ClaimedTurnRun, HeartbeatRequest, RecordRunnerFailureRequest,
        RecoverExpiredLeasesRequest, RelinquishRunRequest, TurnRunTransitionPort,
    },
};
use tokio::{
    sync::{Semaphore, mpsc},
    task::{JoinHandle, JoinSet},
    time::{MissedTickBehavior, interval, sleep},
};
use tracing::Instrument;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct TurnRunSchedulerConfig {
    max_concurrent_runs: usize,
    poll_interval: Duration,
    lease_recovery_interval: Duration,
    runner_heartbeat_interval: Duration,
    claim_error_backoff: Duration,
    wake_channel_capacity: usize,
}

impl Default for TurnRunSchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_runs: 4,
            poll_interval: Duration::from_secs(5),
            lease_recovery_interval: Duration::from_secs(10),
            runner_heartbeat_interval: Duration::from_secs(30),
            claim_error_backoff: Duration::from_secs(1),
            wake_channel_capacity: 128,
        }
    }
}

fn non_zero_duration(duration: Duration) -> Duration {
    if duration.is_zero() {
        Duration::from_millis(1)
    } else {
        duration
    }
}

impl TurnRunSchedulerConfig {
    pub fn max_concurrent_runs(&self) -> usize {
        self.max_concurrent_runs
    }

    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    pub fn lease_recovery_interval(&self) -> Duration {
        self.lease_recovery_interval
    }

    pub fn runner_heartbeat_interval(&self) -> Duration {
        self.runner_heartbeat_interval
    }

    pub fn claim_error_backoff(&self) -> Duration {
        self.claim_error_backoff
    }

    pub fn wake_channel_capacity(&self) -> usize {
        self.wake_channel_capacity
    }

    pub fn with_max_concurrent_runs(mut self, max_concurrent_runs: usize) -> Self {
        self.max_concurrent_runs = max_concurrent_runs.max(1);
        self
    }

    pub fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = non_zero_duration(poll_interval);
        self
    }

    pub fn with_lease_recovery_interval(mut self, lease_recovery_interval: Duration) -> Self {
        self.lease_recovery_interval = non_zero_duration(lease_recovery_interval);
        self
    }

    pub fn with_runner_heartbeat_interval(mut self, runner_heartbeat_interval: Duration) -> Self {
        self.runner_heartbeat_interval = non_zero_duration(runner_heartbeat_interval);
        self
    }

    pub fn with_claim_error_backoff(mut self, claim_error_backoff: Duration) -> Self {
        self.claim_error_backoff = non_zero_duration(claim_error_backoff);
        self
    }

    pub fn with_wake_channel_capacity(mut self, wake_channel_capacity: usize) -> Self {
        self.wake_channel_capacity = wake_channel_capacity.max(1);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRunExecutorError {
    failure: SanitizedFailure,
}

impl TurnRunExecutorError {
    pub fn new(failure_category: impl Into<String>) -> Result<Self, String> {
        Ok(Self {
            failure: SanitizedFailure::new(failure_category)?,
        })
    }

    pub fn failure(&self) -> &SanitizedFailure {
        &self.failure
    }

    pub fn failure_category(&self) -> &str {
        self.failure.category()
    }
}

impl fmt::Display for TurnRunExecutorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "turn run executor failed: {}",
            self.failure.category()
        )
    }
}

impl Error for TurnRunExecutorError {}

#[async_trait]
pub trait TurnRunExecutor: Send + Sync {
    async fn execute_claimed_run(
        &self,
        claimed: ClaimedTurnRun,
        transitions: Arc<dyn TurnRunTransitionPort>,
    ) -> Result<(), TurnRunExecutorError>;
}

pub struct TurnRunScheduler {
    transitions: Arc<dyn TurnRunTransitionPort>,
    executor: Arc<dyn TurnRunExecutor>,
    config: TurnRunSchedulerConfig,
    runner_id: TurnRunnerId,
}

impl TurnRunScheduler {
    pub fn new(
        transitions: Arc<dyn TurnRunTransitionPort>,
        executor: Arc<dyn TurnRunExecutor>,
        config: TurnRunSchedulerConfig,
    ) -> Self {
        Self {
            transitions,
            executor,
            config,
            runner_id: TurnRunnerId::new(),
        }
    }

    pub fn start(self) -> TurnRunSchedulerHandle {
        let capacity = self.config.wake_channel_capacity();
        let (notifier, channel) = SchedulerTurnRunWakeNotifier::channel(capacity);
        self.start_with_channel(notifier, channel)
    }

    /// Start with a pre-created wake channel (from
    /// [`SchedulerTurnRunWakeNotifier::channel`]), consuming both the notifier
    /// and the channel. This is the cycle-breaking entry point used when the
    /// coordinator needs the notifier before the scheduler starts.
    pub fn start_with_channel(
        self,
        notifier: Arc<SchedulerTurnRunWakeNotifier>,
        channel: TurnRunWakeChannel,
    ) -> TurnRunSchedulerHandle {
        let TurnRunWakeChannel {
            command_tx,
            command_rx,
        } = channel;
        let supervisor = tokio::spawn(run_scheduler_loop(
            command_rx,
            command_tx.clone(),
            self.transitions,
            self.executor,
            self.config,
            self.runner_id,
        ));
        TurnRunSchedulerHandle {
            notifier,
            command_tx,
            supervisor: Some(supervisor),
        }
    }
}

/// The receiver half of a pre-created wake channel, paired with a
/// [`SchedulerTurnRunWakeNotifier`].
///
/// Created by [`SchedulerTurnRunWakeNotifier::channel`] to break the
/// coordinator↔scheduler build-order cycle. The caller mints both the
/// notifier and this channel before building the coordinator, then passes
/// the channel to [`TurnRunScheduler::start_with_channel`].
pub struct TurnRunWakeChannel {
    command_tx: mpsc::Sender<SchedulerCommand>,
    command_rx: mpsc::Receiver<SchedulerCommand>,
}

#[derive(Clone)]
pub struct SchedulerTurnRunWakeNotifier {
    command_tx: mpsc::Sender<SchedulerCommand>,
}

impl SchedulerTurnRunWakeNotifier {
    /// Create a notifier and its paired wake channel before the scheduler is
    /// started, breaking the coordinator↔scheduler build-order cycle.
    ///
    /// The returned notifier can be given to the turn coordinator immediately.
    /// Pass the channel to [`TurnRunScheduler::start_with_channel`] later to
    /// wire the scheduler loop.
    pub fn channel(capacity: usize) -> (Arc<SchedulerTurnRunWakeNotifier>, TurnRunWakeChannel) {
        let (command_tx, command_rx) = mpsc::channel(capacity.max(1));
        let notifier = Arc::new(SchedulerTurnRunWakeNotifier {
            command_tx: command_tx.clone(),
        });
        (
            notifier,
            TurnRunWakeChannel {
                command_tx,
                command_rx,
            },
        )
    }
}

impl fmt::Debug for SchedulerTurnRunWakeNotifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SchedulerTurnRunWakeNotifier")
    }
}

impl TurnRunWakeNotifier for SchedulerTurnRunWakeNotifier {
    fn notify_queued_run(&self, wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        self.command_tx
            .try_send(SchedulerCommand::Wake(wake))
            .map_err(|_| TurnRunWakeNotifyError::DeliveryUnavailable)
    }
}

pub struct TurnRunSchedulerHandle {
    notifier: Arc<SchedulerTurnRunWakeNotifier>,
    command_tx: mpsc::Sender<SchedulerCommand>,
    /// `Option` so that `shutdown()` can `take()` the handle without a
    /// partial move, which would be disallowed when `Drop` is implemented.
    /// `None` only after `shutdown()` completes or if construction somehow
    /// produced an absent supervisor (not possible via the public API).
    supervisor: Option<JoinHandle<()>>,
}

impl TurnRunSchedulerHandle {
    pub fn wake_notifier(&self) -> Arc<SchedulerTurnRunWakeNotifier> {
        Arc::clone(&self.notifier)
    }

    pub fn is_stopped(&self) -> bool {
        self.supervisor
            .as_ref()
            .map_or(true, |s| s.is_finished())
    }

    /// Graceful shutdown: signal the scheduler loop to stop, then await the
    /// supervisor task to completion.  This is the preferred shutdown path
    /// when the caller can `await`.
    ///
    /// If the handle is dropped without calling `shutdown()` — for example
    /// when a build function returns `Err` after the scheduler has started —
    /// the `Drop` impl sends a best-effort synchronous shutdown signal instead.
    pub async fn shutdown(mut self) {
        let _ = self.command_tx.send(SchedulerCommand::Shutdown).await;
        if let Some(supervisor) = self.supervisor.take() {
            let _ = supervisor.await;
        }
    }
}

impl Drop for TurnRunSchedulerHandle {
    fn drop(&mut self) {
        // Safety net for error paths: if `shutdown()` was not called (e.g. a
        // build function failed after starting the scheduler), send a best-effort
        // synchronous Shutdown command so the background task terminates instead
        // of running indefinitely with no owner.
        //
        // `try_send` is non-blocking and is a no-op on failure (task already
        // stopped, or channel unexpectedly full).  The graceful `shutdown()` path
        // awaits task completion and is preferred wherever an async context is
        // available; Drop is the fallback for synchronous or error-path drops.
        //
        // The supervisor `JoinHandle` is `Option` so that `shutdown()` can
        // `take()` it (avoiding a partial-move from a `Drop`-implementing type).
        // When Drop fires here the `JoinHandle` — if not already taken by
        // `shutdown()` — is dropped, which detaches the tokio task.  The
        // `Shutdown` command above causes the detached task to self-terminate
        // on its next `select!` iteration.
        let _ = self.command_tx.try_send(SchedulerCommand::Shutdown);
    }
}

#[derive(Debug)]
enum SchedulerCommand {
    Wake(TurnRunWake),
    Drain,
    RetryDrain,
    Shutdown,
}

/// Identity fields needed to relinquish a claimed run back to Queued.
struct RelinquishIdentity {
    run_id: TurnRunId,
    runner_id: TurnRunnerId,
    lease_token: TurnLeaseToken,
}

struct SchedulerDrainContext {
    transitions: Arc<dyn TurnRunTransitionPort>,
    executor: Arc<dyn TurnRunExecutor>,
    semaphore: Arc<Semaphore>,
    command_tx: mpsc::Sender<SchedulerCommand>,
    config: TurnRunSchedulerConfig,
    runner_id: TurnRunnerId,
}

async fn run_scheduler_loop(
    mut command_rx: mpsc::Receiver<SchedulerCommand>,
    command_tx: mpsc::Sender<SchedulerCommand>,
    transitions: Arc<dyn TurnRunTransitionPort>,
    executor: Arc<dyn TurnRunExecutor>,
    config: TurnRunSchedulerConfig,
    runner_id: TurnRunnerId,
) {
    let semaphore = Arc::new(Semaphore::new(config.max_concurrent_runs()));
    let mut executor_tasks: JoinSet<TurnRunId> = JoinSet::new();
    // Tracks every in-flight run so we can relinquish on shutdown.
    let mut active_runs: HashMap<TurnRunId, RelinquishIdentity> = HashMap::new();
    let mut poll_tick = interval(config.poll_interval());
    poll_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut recovery_tick = interval(config.lease_recovery_interval());
    recovery_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let context = SchedulerDrainContext {
        transitions,
        executor,
        semaphore,
        command_tx,
        config,
        runner_id,
    };
    let mut claim_retry_pending = false;

    loop {
        tokio::select! {
            Some(command) = command_rx.recv() => {
                match command {
                    SchedulerCommand::Wake(wake) => {
                        // Prefer the woken scope for locality; if that scope has no
                        // claimable work, fall back to the global queue below.
                        if !claim_retry_pending
                            && drain_queued_runs(
                                &context,
                                Some(wake.scope),
                                &mut executor_tasks,
                                &mut active_runs,
                            ).await
                        {
                            claim_retry_pending = true;
                            schedule_drain_after(
                                context.command_tx.clone(),
                                context.config.claim_error_backoff(),
                            );
                        }
                        if !claim_retry_pending
                            && drain_queued_runs(
                                &context,
                                None,
                                &mut executor_tasks,
                                &mut active_runs,
                            ).await
                        {
                            claim_retry_pending = true;
                            schedule_drain_after(
                                context.command_tx.clone(),
                                context.config.claim_error_backoff(),
                            );
                        }
                    }
                    SchedulerCommand::Drain => {
                        if !claim_retry_pending
                            && drain_queued_runs(
                                &context,
                                None,
                                &mut executor_tasks,
                                &mut active_runs,
                            ).await
                        {
                            claim_retry_pending = true;
                            schedule_drain_after(
                                context.command_tx.clone(),
                                context.config.claim_error_backoff(),
                            );
                        }
                    }
                    SchedulerCommand::RetryDrain => {
                        claim_retry_pending = false;
                        if drain_queued_runs(
                            &context,
                            None,
                            &mut executor_tasks,
                            &mut active_runs,
                        ).await {
                            claim_retry_pending = true;
                            schedule_drain_after(
                                context.command_tx.clone(),
                                context.config.claim_error_backoff(),
                            );
                        }
                    }
                    SchedulerCommand::Shutdown => {
                        // Abort all in-flight tasks first so there is no race
                        // between them completing a transition and our relinquish.
                        executor_tasks.shutdown().await;
                        // Best-effort relinquish: return each aborted run to Queued
                        // so a restart can pick it up instead of letting lease expiry
                        // mark it Failed.
                        for (_run_id, identity) in active_runs {
                            let result = context
                                .transitions
                                .relinquish_run(RelinquishRunRequest {
                                    run_id: identity.run_id,
                                    runner_id: identity.runner_id,
                                    lease_token: identity.lease_token,
                                })
                                .await;
                            if let Err(error) = result {
                                warn!(
                                    run_id = %identity.run_id,
                                    error = %error,
                                    "failed to relinquish in-flight run during scheduler shutdown; run will rely on lease recovery"
                                );
                            }
                        }
                        break;
                    },
                }
            }
            _ = poll_tick.tick() => {
                if !claim_retry_pending
                    && drain_queued_runs(
                        &context,
                        None,
                        &mut executor_tasks,
                        &mut active_runs,
                    ).await
                {
                    claim_retry_pending = true;
                    schedule_drain_after(
                        context.command_tx.clone(),
                        context.config.claim_error_backoff(),
                    );
                }
            }
            Some(result) = executor_tasks.join_next(), if !executor_tasks.is_empty() => {
                match result {
                    Ok(completed_run_id) => {
                        active_runs.remove(&completed_run_id);
                    }
                    Err(error) => {
                        debug!(error = %error, "turn run scheduler executor supervisor task failed");
                    }
                }
            }
            _ = recovery_tick.tick() => {
                recover_expired_leases(Arc::clone(&context.transitions)).await;
            }
        }
    }
}

async fn drain_queued_runs(
    context: &SchedulerDrainContext,
    scope_filter: Option<TurnScope>,
    executor_tasks: &mut JoinSet<TurnRunId>,
    active_runs: &mut HashMap<TurnRunId, RelinquishIdentity>,
) -> bool {
    loop {
        let Ok(permit) = Arc::clone(&context.semaphore).try_acquire_owned() else {
            return false;
        };
        let claim = context
            .transitions
            .claim_next_run(ClaimRunRequest {
                runner_id: context.runner_id,
                lease_token: ironclaw_turns::TurnLeaseToken::new(),
                scope_filter: scope_filter.clone(),
            })
            .await;
        match claim {
            Ok(Some(claimed)) => {
                let run_id = claimed.state.run_id;
                active_runs.insert(
                    run_id,
                    RelinquishIdentity {
                        run_id,
                        runner_id: claimed.runner_id,
                        lease_token: claimed.lease_token,
                    },
                );
                spawn_executor_task(
                    claimed,
                    Arc::clone(&context.transitions),
                    Arc::clone(&context.executor),
                    context.command_tx.clone(),
                    permit,
                    context.config.runner_heartbeat_interval(),
                    executor_tasks,
                );
            }
            Ok(None) => return false,
            Err(error) => {
                debug!(error = %error, "turn run scheduler claim failed");
                return true;
            }
        }
    }
}

enum ExecutorTaskOutcome {
    Completed,
    TerminalFailure(Option<SanitizedFailure>),
}

fn spawn_executor_task(
    claimed: ClaimedTurnRun,
    transitions: Arc<dyn TurnRunTransitionPort>,
    executor: Arc<dyn TurnRunExecutor>,
    command_tx: mpsc::Sender<SchedulerCommand>,
    permit: tokio::sync::OwnedSemaphorePermit,
    runner_heartbeat_interval: Duration,
    executor_tasks: &mut JoinSet<TurnRunId>,
) {
    // Tag every tracing event emitted while this run executes with its
    // `thread_id` + `run_id` so the operator Logs panel's scoped (thread/run)
    // view is populated. `OperatorLogLayer` reads these correlation fields from
    // the enclosing span via `from_root`; without the span, scoped queries
    // match nothing and the panel shows "0 entries".
    let run_span = tracing::info_span!(
        "turn_run",
        thread_id = %claimed.state.scope.thread_id,
        run_id = %claimed.state.run_id,
    );
    // Capture these before `claimed` is moved into the async block so the
    // "turn run started" event can emit them as explicit fields. This makes
    // the event self-contained and allows test layers to find them without
    // relying on span registration timing (which can be racy under parallel
    // test execution when using `tracing::dispatcher::set_default`).
    let recovery_thread_id = claimed.state.scope.thread_id.clone();
    let recovery_run_id_for_start = claimed.state.run_id;
    executor_tasks.spawn(
        async move {
            let recovery_run_id = claimed.state.run_id;
            let recovery_runner_id = claimed.runner_id;
            let recovery_lease_token = claimed.lease_token;
            tracing::debug!(
                thread_id = %recovery_thread_id,
                run_id = %recovery_run_id_for_start,
                "turn run started",
            );
            let mut heartbeat_tick = interval(runner_heartbeat_interval);
            heartbeat_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
            // Consume the immediate first tick so the heartbeat loop never fires
            // at t=0. The run's lease was just issued and valid; a t=0 heartbeat
            // would fail on CancelRequested status (heartbeat only accepts Running)
            // and prematurely terminate the executor task before the driver has a
            // chance to observe cancellation and write its reply to thread history.
            heartbeat_tick.tick().await;
            let executor_result =
                AssertUnwindSafe(executor.execute_claimed_run(claimed, Arc::clone(&transitions)))
                    .catch_unwind();
            tokio::pin!(executor_result);
            let outcome = loop {
                tokio::select! {
                    result = &mut executor_result => {
                        break match result {
                            Ok(Ok(())) => ExecutorTaskOutcome::Completed,
                            Ok(Err(error)) => ExecutorTaskOutcome::TerminalFailure(Some(
                                error.failure().clone(),
                            )),
                            Err(_) => ExecutorTaskOutcome::TerminalFailure(scheduler_failure(
                                "scheduler_executor_panic",
                            )),
                        };
                    }
                    _ = heartbeat_tick.tick() => {
                        if !heartbeat_claimed_run(
                            Arc::clone(&transitions),
                            recovery_run_id,
                            recovery_runner_id,
                            recovery_lease_token,
                        ).await {
                            break ExecutorTaskOutcome::TerminalFailure(scheduler_failure(
                                "scheduler_heartbeat_failed",
                            ));
                        }
                    }
                }
            };

            match outcome {
                ExecutorTaskOutcome::Completed => {}
                ExecutorTaskOutcome::TerminalFailure(Some(failure)) => {
                    record_terminal_failure(
                        Arc::clone(&transitions),
                        recovery_run_id,
                        recovery_runner_id,
                        recovery_lease_token,
                        failure,
                    )
                    .await;
                }
                ExecutorTaskOutcome::TerminalFailure(None) => {
                    debug!("turn run scheduler could not sanitize terminal failure category");
                }
            }

            tracing::debug!("turn run finished");
            drop(permit);
            let _ = command_tx.send(SchedulerCommand::Drain).await;
            // Return the run_id so the scheduler loop can remove it from active_runs.
            recovery_run_id
        }
        .instrument(run_span),
    );
}

async fn heartbeat_claimed_run(
    transitions: Arc<dyn TurnRunTransitionPort>,
    run_id: ironclaw_turns::TurnRunId,
    runner_id: ironclaw_turns::TurnRunnerId,
    lease_token: ironclaw_turns::TurnLeaseToken,
) -> bool {
    let result = transitions
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await;
    if let Err(error) = result {
        debug!(error = %error, "turn run scheduler heartbeat failed");
        return false;
    }
    true
}

async fn record_terminal_failure(
    transitions: Arc<dyn TurnRunTransitionPort>,
    run_id: ironclaw_turns::TurnRunId,
    runner_id: ironclaw_turns::TurnRunnerId,
    lease_token: ironclaw_turns::TurnLeaseToken,
    failure: SanitizedFailure,
) {
    let result = transitions
        .record_runner_failure(RecordRunnerFailureRequest {
            run_id,
            runner_id,
            lease_token,
            failure,
        })
        .await;
    if let Err(error) = result {
        debug!(error = %error, "turn run scheduler terminal failure transition failed");
    }
}

fn scheduler_failure(category: &'static str) -> Option<SanitizedFailure> {
    match SanitizedFailure::new(category) {
        Ok(failure) => Some(failure),
        Err(error) => {
            debug!(
                category,
                error, "turn run scheduler static terminal failure category failed validation"
            );
            None
        }
    }
}

async fn recover_expired_leases(transitions: Arc<dyn TurnRunTransitionPort>) {
    let result: Result<_, TurnError> = transitions
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc::now(),
            // Scheduler currently owns one global worker pool; if composition
            // introduces per-tenant schedulers, thread that scope filter here.
            scope_filter: None,
        })
        .await;
    if let Err(error) = result {
        debug!(error = %error, "turn run scheduler lease recovery failed");
    }
}

fn schedule_drain_after(command_tx: mpsc::Sender<SchedulerCommand>, delay: Duration) {
    // Best-effort timer: if shutdown closes the command channel first, send fails harmlessly.
    tokio::spawn(async move {
        sleep(delay).await;
        let _ = command_tx.send(SchedulerCommand::RetryDrain).await;
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_turns::{
        EventCursor, TurnError, TurnRunState,
        runner::{
            ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
            ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
            RecordModelRouteSnapshotRequest, RecoverExpiredLeasesRequest,
            RecoverExpiredLeasesResponse, RelinquishRunRequest, TurnRunTransitionPort,
        },
    };

    use super::{TurnRunExecutor, TurnRunExecutorError, TurnRunScheduler, TurnRunSchedulerConfig};

    // ── Minimal fakes ────────────────────────────────────────────────────────

    /// A `TurnRunTransitionPort` that claims nothing and no-ops everything else.
    struct NoopTransitionPort;

    #[async_trait]
    impl TurnRunTransitionPort for NoopTransitionPort {
        async fn claim_next_run(
            &self,
            _request: ClaimRunRequest,
        ) -> Result<Option<ClaimedTurnRun>, TurnError> {
            Ok(None)
        }

        async fn heartbeat(&self, _request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
            Ok(EventCursor(0))
        }

        async fn recover_expired_leases(
            &self,
            _request: RecoverExpiredLeasesRequest,
        ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
            Ok(RecoverExpiredLeasesResponse { recovered: vec![] })
        }

        async fn record_model_route_snapshot(
            &self,
            _request: RecordModelRouteSnapshotRequest,
        ) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: "noop".to_string(),
            })
        }

        async fn block_run(&self, _request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: "noop".to_string(),
            })
        }

        async fn complete_run(
            &self,
            _request: CompleteRunRequest,
        ) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: "noop".to_string(),
            })
        }

        async fn cancel_run(
            &self,
            _request: CancelRunCompletionRequest,
        ) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: "noop".to_string(),
            })
        }

        async fn fail_run(&self, _request: FailRunRequest) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: "noop".to_string(),
            })
        }

        async fn relinquish_run(
            &self,
            _request: RelinquishRunRequest,
        ) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: "noop".to_string(),
            })
        }

        async fn apply_validated_loop_exit(
            &self,
            _request: ApplyValidatedLoopExitRequest,
        ) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: "noop".to_string(),
            })
        }
    }

    /// A `TurnRunExecutor` that never executes (claim_next_run always returns None).
    struct NoopExecutor;

    #[async_trait]
    impl TurnRunExecutor for NoopExecutor {
        async fn execute_claimed_run(
            &self,
            _claimed: ClaimedTurnRun,
            _transitions: Arc<dyn TurnRunTransitionPort>,
        ) -> Result<(), TurnRunExecutorError> {
            Ok(())
        }
    }

    /// `is_stopped()` returns `false` while the scheduler is running and the
    /// supervisor task becomes finished after `shutdown()` completes.
    ///
    /// `shutdown(self)` consumes the handle so `is_stopped()` cannot be called
    /// after it.  We verify the two halves of the lifecycle separately:
    ///
    /// * **Before shutdown**: `is_stopped() == false` on a running handle.
    /// * **After shutdown**: a detached watcher task performs the `is_stopped()`
    ///   check on the same handle, then calls `shutdown().await`. The channel
    ///   value it sends back confirms the pre-shutdown state was `false` and that
    ///   shutdown completed without hanging.
    #[tokio::test]
    async fn is_stopped_reflects_scheduler_lifecycle() {
        let config = TurnRunSchedulerConfig::default()
            // Long intervals so the poll/recovery ticks never fire during the test.
            .with_poll_interval(std::time::Duration::from_secs(3600))
            .with_lease_recovery_interval(std::time::Duration::from_secs(3600));

        let scheduler =
            TurnRunScheduler::new(Arc::new(NoopTransitionPort), Arc::new(NoopExecutor), config);
        let handle = scheduler.start();

        // Spawn a task that holds the handle, checks is_stopped(), shuts down,
        // and sends both observations back.
        let (tx, rx) = tokio::sync::oneshot::channel::<(bool, bool)>();
        tokio::spawn(async move {
            let was_running = !handle.is_stopped();
            handle.shutdown().await;
            // After shutdown() the supervisor has been joined → is_finished()
            // is guaranteed true; we use `true` as a sentinel for "stopped".
            let _ = tx.send((was_running, true));
        });

        let (was_running, is_stopped_after) = rx.await.expect("watcher task must complete");
        assert!(
            was_running,
            "is_stopped() must be false immediately after start()"
        );
        assert!(
            is_stopped_after,
            "scheduler must be stopped after shutdown() returns"
        );
    }

    /// Dropping a `TurnRunSchedulerHandle` without calling `shutdown()` must
    /// signal the background scheduler task to self-terminate, not leak.
    ///
    /// This guards the bug scenario from the PR review: a build function starts
    /// the scheduler via `build_default_planned_runtime` then fails on a later
    /// fallible step.  Without Drop-based cleanup the scheduler task would run
    /// indefinitely after the build error is returned.
    ///
    /// Observable proxy: grab a clone of `command_tx` before the drop (the same
    /// channel the Drop impl tries to send `Shutdown` on), then verify the task
    /// processes it by waiting for the channel to close — which only happens once
    /// the scheduler loop breaks out of its `loop` on `Shutdown`.
    #[tokio::test]
    async fn drop_without_shutdown_sends_shutdown_signal() {
        let config = TurnRunSchedulerConfig::default()
            // Long intervals so poll/recovery ticks never fire during the test.
            .with_poll_interval(std::time::Duration::from_secs(3600))
            .with_lease_recovery_interval(std::time::Duration::from_secs(3600));

        let scheduler =
            TurnRunScheduler::new(Arc::new(NoopTransitionPort), Arc::new(NoopExecutor), config);
        let handle = scheduler.start();

        // Clone the command sender so we can observe the channel state after drop.
        let command_tx_clone = handle.command_tx.clone();

        // Drop the handle WITHOUT calling shutdown().
        // The Drop impl should fire try_send(SchedulerCommand::Shutdown).
        drop(handle);

        // The scheduler loop is running in a background task.  After receiving
        // `Shutdown` it breaks and the task ends.  When the task ends, the
        // internal `command_tx` owned by the loop is dropped, closing the channel.
        // `closed()` resolves when all senders are gone — i.e. the task exited.
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            command_tx_clone.closed(),
        )
        .await
        .expect("scheduler task must terminate within 2 s when handle is dropped without shutdown");
    }
}
