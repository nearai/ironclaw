//! Generic execution supervision over the process journal.

use std::{collections::HashMap, fmt, panic::AssertUnwindSafe, sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use futures::FutureExt;
use ironclaw_host_api::{ProcessId, ResourceScope, SanitizedFailure};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch},
    task::{JoinHandle, JoinSet},
    time::{MissedTickBehavior, interval, sleep},
};
use tracing::{Instrument, debug};

use crate::{
    ClaimProcessesRequest, ClaimedProcess, FailProcessRequest, ProcessFailureRecovery,
    ProcessJournalStoreError, ProcessKind, ProcessLeaseRequest, ProcessLeaseToken,
    ProcessTransitionPort, ProcessWorkerId, RecoverExpiredProcessLeasesRequest,
};

const MAX_CLAIMS_PER_DRAIN_BATCH: usize = 128;

#[derive(Debug, Clone)]
pub struct ProcessSupervisorConfig {
    max_concurrent_processes: usize,
    poll_interval: Duration,
    lease_recovery_interval: Duration,
    heartbeat_interval: Duration,
    max_consecutive_heartbeat_failures: usize,
    terminal_failure_record_attempts: usize,
    terminal_failure_record_backoff: Duration,
    claim_error_backoff: Duration,
    wake_channel_capacity: usize,
}

impl Default for ProcessSupervisorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_processes: 4,
            poll_interval: Duration::from_secs(5),
            lease_recovery_interval: Duration::from_secs(10),
            heartbeat_interval: Duration::from_secs(30),
            max_consecutive_heartbeat_failures: 3,
            terminal_failure_record_attempts: 3,
            terminal_failure_record_backoff: Duration::from_millis(100),
            claim_error_backoff: Duration::from_secs(1),
            wake_channel_capacity: 128,
        }
    }
}

impl ProcessSupervisorConfig {
    pub fn max_concurrent_processes(&self) -> usize {
        self.max_concurrent_processes
    }

    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    pub fn lease_recovery_interval(&self) -> Duration {
        self.lease_recovery_interval
    }

    pub fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }

    pub fn max_consecutive_heartbeat_failures(&self) -> usize {
        self.max_consecutive_heartbeat_failures
    }

    pub fn terminal_failure_record_attempts(&self) -> usize {
        self.terminal_failure_record_attempts
    }

    pub fn terminal_failure_record_backoff(&self) -> Duration {
        self.terminal_failure_record_backoff
    }

    pub fn claim_error_backoff(&self) -> Duration {
        self.claim_error_backoff
    }

    pub fn wake_channel_capacity(&self) -> usize {
        self.wake_channel_capacity
    }

    pub fn with_max_concurrent_processes(mut self, maximum: usize) -> Self {
        self.max_concurrent_processes = maximum.max(1);
        self
    }

    pub fn with_poll_interval(mut self, value: Duration) -> Self {
        self.poll_interval = non_zero_duration(value);
        self
    }

    pub fn with_lease_recovery_interval(mut self, value: Duration) -> Self {
        self.lease_recovery_interval = non_zero_duration(value);
        self
    }

    pub fn with_heartbeat_interval(mut self, value: Duration) -> Self {
        self.heartbeat_interval = non_zero_duration(value);
        self
    }

    pub fn with_max_consecutive_heartbeat_failures(mut self, maximum: usize) -> Self {
        self.max_consecutive_heartbeat_failures = maximum.max(1);
        self
    }

    pub fn with_terminal_failure_record_attempts(mut self, maximum: usize) -> Self {
        self.terminal_failure_record_attempts = maximum.max(1);
        self
    }

    pub fn with_terminal_failure_record_backoff(mut self, value: Duration) -> Self {
        self.terminal_failure_record_backoff = non_zero_duration(value);
        self
    }

    pub fn with_claim_error_backoff(mut self, value: Duration) -> Self {
        self.claim_error_backoff = non_zero_duration(value);
        self
    }

    pub fn with_wake_channel_capacity(mut self, capacity: usize) -> Self {
        self.wake_channel_capacity = capacity.max(1);
        self
    }
}

fn non_zero_duration(value: Duration) -> Duration {
    if value.is_zero() {
        Duration::from_millis(1)
    } else {
        value
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessExecutorFailure {
    failure: Option<SanitizedFailure>,
    fallback_category: String,
    recovery: ProcessFailureRecovery,
}

impl ProcessExecutorFailure {
    pub fn new(category: impl Into<String>) -> Self {
        let fallback_category = category.into();
        let failure = SanitizedFailure::new(fallback_category.clone()).ok();
        Self {
            failure,
            fallback_category,
            recovery: ProcessFailureRecovery::Terminal,
        }
    }

    pub fn from_failure(failure: SanitizedFailure) -> Self {
        let fallback_category = failure.category().to_string();
        Self {
            failure: Some(failure),
            fallback_category,
            recovery: ProcessFailureRecovery::Terminal,
        }
    }

    pub fn with_recovery(mut self, recovery: ProcessFailureRecovery) -> Self {
        self.recovery = recovery;
        self
    }

    pub fn failure(&self) -> Option<&SanitizedFailure> {
        self.failure.as_ref()
    }

    pub fn failure_category(&self) -> &str {
        self.failure
            .as_ref()
            .map_or(self.fallback_category.as_str(), SanitizedFailure::category)
    }

    pub fn recovery(&self) -> ProcessFailureRecovery {
        self.recovery
    }
}

impl fmt::Display for ProcessExecutorFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "process executor failed: {}",
            self.failure_category()
        )
    }
}

impl std::error::Error for ProcessExecutorFailure {}

#[async_trait]
pub trait JournalProcessExecutor: Send + Sync {
    async fn execute_claimed_process(
        &self,
        claimed: ClaimedProcess,
    ) -> Result<(), ProcessExecutorFailure>;

    fn panic_failure(&self) -> ProcessExecutorFailure {
        ProcessExecutorFailure::new("process_executor_panic")
    }

    fn heartbeat_failure(&self) -> ProcessExecutorFailure {
        ProcessExecutorFailure::new("process_heartbeat_failed")
    }
}

pub struct ProcessSupervisor {
    runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>>,
    executor: Arc<dyn JournalProcessExecutor>,
    process_kind: ProcessKind,
    config: ProcessSupervisorConfig,
    worker_id: ProcessWorkerId,
}

impl ProcessSupervisor {
    pub fn new(
        runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>>,
        executor: Arc<dyn JournalProcessExecutor>,
        process_kind: ProcessKind,
        config: ProcessSupervisorConfig,
    ) -> Self {
        Self {
            runtime,
            executor,
            process_kind,
            config,
            worker_id: ProcessWorkerId::from_trusted(ProcessId::new().to_string()),
        }
    }

    pub fn start(self) -> ProcessSupervisorHandle {
        let (notifier, channel) = ProcessWakeNotifier::channel(self.config.wake_channel_capacity());
        self.start_with_channel(notifier, channel)
    }

    pub fn start_with_channel(
        self,
        notifier: Arc<ProcessWakeNotifier>,
        channel: ProcessWakeChannel,
    ) -> ProcessSupervisorHandle {
        let ProcessWakeChannel {
            command_tx,
            command_rx,
        } = channel;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let supervisor = tokio::spawn(run_supervisor_loop(
            command_rx,
            SupervisorLoop {
                command_tx,
                runtime: self.runtime,
                executor: self.executor,
                process_kind: self.process_kind,
                config: self.config,
                worker_id: self.worker_id,
                shutdown_rx,
            },
        ));
        ProcessSupervisorHandle {
            notifier,
            supervisor: Some(supervisor),
            shutdown_tx,
        }
    }
}

pub struct ProcessWakeChannel {
    command_tx: mpsc::Sender<SupervisorCommand>,
    command_rx: mpsc::Receiver<SupervisorCommand>,
}

#[derive(Clone)]
pub struct ProcessWakeNotifier {
    command_tx: mpsc::Sender<SupervisorCommand>,
}

impl ProcessWakeNotifier {
    pub fn channel(capacity: usize) -> (Arc<Self>, ProcessWakeChannel) {
        let (command_tx, command_rx) = mpsc::channel(capacity.max(1));
        (
            Arc::new(Self {
                command_tx: command_tx.clone(),
            }),
            ProcessWakeChannel {
                command_tx,
                command_rx,
            },
        )
    }

    pub fn notify_scope(&self, scope: ResourceScope) -> Result<(), ProcessWakeError> {
        self.command_tx
            .try_send(SupervisorCommand::Wake(scope))
            .map_err(|_| ProcessWakeError::DeliveryUnavailable)
    }
}

impl fmt::Debug for ProcessWakeNotifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProcessWakeNotifier")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProcessWakeError {
    #[error("process supervisor wake delivery unavailable")]
    DeliveryUnavailable,
}

pub struct ProcessSupervisorHandle {
    notifier: Arc<ProcessWakeNotifier>,
    supervisor: Option<JoinHandle<()>>,
    shutdown_tx: watch::Sender<bool>,
}

impl ProcessSupervisorHandle {
    pub fn wake_notifier(&self) -> Arc<ProcessWakeNotifier> {
        Arc::clone(&self.notifier)
    }

    pub fn is_stopped(&self) -> bool {
        self.supervisor
            .as_ref()
            .is_none_or(tokio::task::JoinHandle::is_finished)
    }

    pub async fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(supervisor) = self.supervisor.take() {
            let _ = supervisor.await;
        }
    }
}

impl Drop for ProcessSupervisorHandle {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
    }
}

#[derive(Debug)]
enum SupervisorCommand {
    Wake(ResourceScope),
    Drain,
    RetryDrain,
}

struct ClaimedIdentity {
    process_id: ProcessId,
    worker_id: ProcessWorkerId,
    lease_token: ProcessLeaseToken,
}

struct SupervisorLoop {
    command_tx: mpsc::Sender<SupervisorCommand>,
    runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>>,
    executor: Arc<dyn JournalProcessExecutor>,
    process_kind: ProcessKind,
    config: ProcessSupervisorConfig,
    worker_id: ProcessWorkerId,
    shutdown_rx: watch::Receiver<bool>,
}

struct DrainContext {
    command_tx: mpsc::Sender<SupervisorCommand>,
    runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>>,
    executor: Arc<dyn JournalProcessExecutor>,
    process_kind: ProcessKind,
    config: ProcessSupervisorConfig,
    worker_id: ProcessWorkerId,
    semaphore: Arc<Semaphore>,
}

async fn run_supervisor_loop(
    mut command_rx: mpsc::Receiver<SupervisorCommand>,
    loop_state: SupervisorLoop,
) {
    let mut shutdown_rx = loop_state.shutdown_rx;
    let context = DrainContext {
        command_tx: loop_state.command_tx,
        runtime: loop_state.runtime,
        executor: loop_state.executor,
        process_kind: loop_state.process_kind,
        semaphore: Arc::new(Semaphore::new(loop_state.config.max_concurrent_processes())),
        config: loop_state.config,
        worker_id: loop_state.worker_id,
    };
    let mut tasks = JoinSet::new();
    let mut active = HashMap::new();
    let mut poll_tick = interval(context.config.poll_interval());
    poll_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut recovery_tick = interval(context.config.lease_recovery_interval());
    recovery_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut claim_retry_pending = false;

    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    shutdown_supervisor(&context, &mut tasks, active).await;
                    break;
                }
            }
            Some(command) = command_rx.recv() => {
                match command {
                    SupervisorCommand::Wake(scope) => {
                        drain_or_retry(&context, Some(scope), &mut tasks, &mut active, &mut claim_retry_pending).await;
                        drain_or_retry(&context, None, &mut tasks, &mut active, &mut claim_retry_pending).await;
                    }
                    SupervisorCommand::Drain => {
                        drain_or_retry(&context, None, &mut tasks, &mut active, &mut claim_retry_pending).await;
                    }
                    SupervisorCommand::RetryDrain => {
                        claim_retry_pending = false;
                        drain_or_retry(&context, None, &mut tasks, &mut active, &mut claim_retry_pending).await;
                    }
                }
            }
            _ = poll_tick.tick() => {
                drain_or_retry(&context, None, &mut tasks, &mut active, &mut claim_retry_pending).await;
            }
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                match result {
                    Ok(process_id) => {
                        active.remove(&process_id);
                    }
                    Err(error) => debug!(%error, "process executor supervisor task failed"),
                }
            }
            _ = recovery_tick.tick() => {
                recover_expired_leases(&context).await;
            }
        }
    }
}

async fn drain_or_retry(
    context: &DrainContext,
    scope: Option<ResourceScope>,
    tasks: &mut JoinSet<ProcessId>,
    active: &mut HashMap<ProcessId, ClaimedIdentity>,
    retry_pending: &mut bool,
) {
    if !*retry_pending && drain_queued(context, scope, tasks, active).await {
        *retry_pending = true;
        schedule_retry(
            context.command_tx.clone(),
            context.config.claim_error_backoff(),
        );
    }
}

async fn drain_queued(
    context: &DrainContext,
    scope_filter: Option<ResourceScope>,
    tasks: &mut JoinSet<ProcessId>,
    active: &mut HashMap<ProcessId, ClaimedIdentity>,
) -> bool {
    loop {
        let permits = acquire_claim_permits(&context.semaphore);
        if permits.is_empty() {
            return false;
        }
        match context
            .runtime
            .claim_next_processes(ClaimProcessesRequest {
                worker_id: context.worker_id.clone(),
                scope_filter: scope_filter.clone(),
                process_id_filter: None,
                process_kind_filter: Some(context.process_kind.clone()),
                max_processes: permits.len(),
            })
            .await
        {
            Ok(claims) if claims.is_empty() => return false,
            Ok(claims) => {
                for (claimed, permit) in claims.into_iter().zip(permits) {
                    let process_id = claimed.state.process_id;
                    active.insert(
                        process_id,
                        ClaimedIdentity {
                            process_id,
                            worker_id: claimed.worker_id.clone(),
                            lease_token: claimed.lease_token.clone(),
                        },
                    );
                    spawn_executor(context, claimed, permit, tasks);
                }
            }
            Err(error) => {
                debug!(%error, process_kind = ?context.process_kind, "process claim failed");
                return true;
            }
        }
    }
}

fn acquire_claim_permits(semaphore: &Arc<Semaphore>) -> Vec<OwnedSemaphorePermit> {
    let mut permits = Vec::new();
    for _ in 0..MAX_CLAIMS_PER_DRAIN_BATCH {
        let Ok(permit) = Arc::clone(semaphore).try_acquire_owned() else {
            break;
        };
        permits.push(permit);
    }
    permits
}

fn spawn_executor(
    context: &DrainContext,
    claimed: ClaimedProcess,
    permit: OwnedSemaphorePermit,
    tasks: &mut JoinSet<ProcessId>,
) {
    let runtime = Arc::clone(&context.runtime);
    let executor = Arc::clone(&context.executor);
    let command_tx = context.command_tx.clone();
    let config = context.config.clone();
    let identity = ClaimedIdentity {
        process_id: claimed.state.process_id,
        worker_id: claimed.worker_id.clone(),
        lease_token: claimed.lease_token.clone(),
    };
    let process_kind = claimed.state.process_kind.clone();
    let process_id = claimed.state.process_id;
    let span = tracing::info_span!("process", %process_id, process_kind = ?process_kind);
    tasks.spawn(
        async move {
            let mut heartbeat_tick = interval(config.heartbeat_interval());
            heartbeat_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
            heartbeat_tick.tick().await;
            let execution =
                AssertUnwindSafe(executor.execute_claimed_process(claimed)).catch_unwind();
            tokio::pin!(execution);
            let mut heartbeats = InFlightHeartbeat::default();
            let mut consecutive_failures = 0usize;
            let failure = loop {
                tokio::select! {
                    biased;
                    result = &mut execution => {
                        break match result {
                            Ok(Ok(())) => None,
                            Ok(Err(error)) => Some(error),
                            Err(_) => Some(executor.panic_failure()),
                        };
                    }
                    _ = heartbeat_tick.tick(), if heartbeats.is_idle() => {
                        heartbeats.spawn(
                            Arc::clone(&runtime),
                            identity.process_id,
                            identity.worker_id.clone(),
                            identity.lease_token.clone(),
                            config.heartbeat_interval(),
                        );
                    }
                    Some(result) = heartbeats.join(), if heartbeats.is_running() => {
                        match heartbeat_outcome(result) {
                            HeartbeatOutcome::Succeeded => consecutive_failures = 0,
                            HeartbeatOutcome::Failed | HeartbeatOutcome::TimedOut => {
                                consecutive_failures = consecutive_failures.saturating_add(1);
                            }
                        }
                        if consecutive_failures >= config.max_consecutive_heartbeat_failures() {
                            break Some(executor.heartbeat_failure());
                        }
                    }
                }
            };
            heartbeats.abort();
            if let Some(failure) = failure
                && let Err(error) = record_failure(&runtime, &identity, &failure, &config).await
            {
                debug!(%error, %process_id, "process terminal failure recording exhausted");
                relinquish(&runtime, &identity).await;
            }
            drop(permit);
            let _ = command_tx.send(SupervisorCommand::Drain).await;
            process_id
        }
        .instrument(span),
    );
}

#[derive(Default)]
struct InFlightHeartbeat {
    task: Option<JoinHandle<HeartbeatOutcome>>,
}

impl InFlightHeartbeat {
    fn is_idle(&self) -> bool {
        self.task.is_none()
    }

    fn is_running(&self) -> bool {
        self.task.is_some()
    }

    fn spawn(
        &mut self,
        runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>>,
        process_id: ProcessId,
        worker_id: ProcessWorkerId,
        lease_token: ProcessLeaseToken,
        timeout_after: Duration,
    ) {
        self.task = Some(tokio::spawn(async move {
            let heartbeat = runtime.heartbeat_process(ProcessLeaseRequest {
                process_id,
                worker_id,
                lease_token,
            });
            match tokio::time::timeout(timeout_after, heartbeat).await {
                Ok(Ok(_)) => HeartbeatOutcome::Succeeded,
                Ok(Err(_)) => HeartbeatOutcome::Failed,
                Err(_) => HeartbeatOutcome::TimedOut,
            }
        }));
    }

    async fn join(&mut self) -> Option<Result<HeartbeatOutcome, tokio::task::JoinError>> {
        let result = self.task.as_mut()?.await;
        self.task = None;
        Some(result)
    }

    fn abort(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[derive(Clone, Copy)]
enum HeartbeatOutcome {
    Succeeded,
    Failed,
    TimedOut,
}

fn heartbeat_outcome(result: Result<HeartbeatOutcome, tokio::task::JoinError>) -> HeartbeatOutcome {
    result.unwrap_or(HeartbeatOutcome::Failed)
}

async fn record_failure(
    runtime: &Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>>,
    identity: &ClaimedIdentity,
    executor_failure: &ProcessExecutorFailure,
    config: &ProcessSupervisorConfig,
) -> Result<(), ProcessJournalStoreError> {
    let failure = executor_failure
        .failure()
        .cloned()
        .or_else(|| static_failure("unknown_failure"))
        .ok_or_else(|| {
            ProcessJournalStoreError::InvalidRequest(
                "static unknown process failure category is invalid".to_string(),
            )
        })?;
    for attempt in 1..=config.terminal_failure_record_attempts() {
        match runtime
            .fail_process(FailProcessRequest {
                process_id: identity.process_id,
                worker_id: identity.worker_id.clone(),
                lease_token: identity.lease_token.clone(),
                failure: failure.clone(),
                recovery: executor_failure.recovery(),
                checkpoint_ref: None,
                metadata: None,
            })
            .await
        {
            Ok(_) => return Ok(()),
            Err(error)
                if retryable_store_error(&error)
                    && attempt < config.terminal_failure_record_attempts() =>
            {
                sleep(config.terminal_failure_record_backoff()).await;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn retryable_store_error(error: &ProcessJournalStoreError) -> bool {
    matches!(
        error,
        ProcessJournalStoreError::Filesystem(_)
            | ProcessJournalStoreError::Serialization(_)
            | ProcessJournalStoreError::Deserialization(_)
            | ProcessJournalStoreError::Observer(_)
    )
}

async fn recover_expired_leases(context: &DrainContext) {
    match context
        .runtime
        .recover_expired_process_leases(RecoverExpiredProcessLeasesRequest {
            now: Utc::now(),
            scope_filter: None,
            process_kind_filter: Some(context.process_kind.clone()),
        })
        .await
    {
        Ok(response) => {
            for state in response.recovered {
                debug!(
                    process_id = %state.process_id,
                    process_kind = ?state.process_kind,
                    status = ?state.status,
                    "process supervisor recovered expired lease"
                );
            }
        }
        Err(error) => debug!(%error, "process lease recovery failed"),
    }
}

async fn shutdown_supervisor(
    context: &DrainContext,
    tasks: &mut JoinSet<ProcessId>,
    active: HashMap<ProcessId, ClaimedIdentity>,
) {
    tasks.shutdown().await;
    for identity in active.into_values() {
        relinquish(&context.runtime, &identity).await;
    }
}

async fn relinquish(
    runtime: &Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>>,
    identity: &ClaimedIdentity,
) {
    if let Err(error) = runtime
        .relinquish_process(ProcessLeaseRequest {
            process_id: identity.process_id,
            worker_id: identity.worker_id.clone(),
            lease_token: identity.lease_token.clone(),
        })
        .await
    {
        debug!(%error, process_id = %identity.process_id, "failed to relinquish process");
    }
}

fn static_failure(category: &'static str) -> Option<SanitizedFailure> {
    SanitizedFailure::new(category).ok()
}

fn schedule_retry(command_tx: mpsc::Sender<SupervisorCommand>, delay: Duration) {
    tokio::spawn(async move {
        sleep(delay).await;
        let _ = command_tx.send(SupervisorCommand::RetryDrain).await;
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::{
        GetProcessSnapshotRequest, JournaledProcessSnapshot, ProcessJournalCursor,
        ProcessJournalSource, ProcessLifecycleStatus, ProcessStateTransitionRequest,
        ProcessSubmissionPort, SubmitProcessRequest, SuspendProcessRequest,
        test_support::in_memory_backed_processes_filesystem,
    };
    use ironclaw_filesystem::{FilesystemError, FilesystemOperation, InMemoryBackend};
    use ironclaw_host_api::VirtualPath;
    use ironclaw_host_api::{AgentId, InvocationId, ProjectId, TenantId, ThreadId, UserId};
    use tokio::sync::Notify;

    #[derive(Clone, Copy)]
    enum HeartbeatFault {
        None,
        Error,
        Pending,
    }

    struct FaultRuntime {
        inner: Arc<crate::ProcessJournalStore<InMemoryBackend>>,
        heartbeat: HeartbeatFault,
        claim_errors_remaining: AtomicUsize,
        recover_errors_remaining: AtomicUsize,
        fail_errors_remaining: AtomicUsize,
        fail_attempts: AtomicUsize,
        relinquishes: AtomicUsize,
    }

    impl FaultRuntime {
        fn new(
            inner: Arc<crate::ProcessJournalStore<InMemoryBackend>>,
            heartbeat: HeartbeatFault,
            fail_errors: usize,
        ) -> Self {
            Self {
                inner,
                heartbeat,
                claim_errors_remaining: AtomicUsize::new(0),
                recover_errors_remaining: AtomicUsize::new(0),
                fail_errors_remaining: AtomicUsize::new(fail_errors),
                fail_attempts: AtomicUsize::new(0),
                relinquishes: AtomicUsize::new(0),
            }
        }

        fn with_claim_errors(mut self, errors: usize) -> Self {
            self.claim_errors_remaining = AtomicUsize::new(errors);
            self
        }

        fn with_recover_errors(mut self, errors: usize) -> Self {
            self.recover_errors_remaining = AtomicUsize::new(errors);
            self
        }

        fn injected_error() -> ProcessJournalStoreError {
            ProcessJournalStoreError::Filesystem(FilesystemError::Backend {
                path: VirtualPath::new("/engine/injected/process-supervisor")
                    .unwrap_or_else(|error| panic!("test path must be valid: {error}")),
                operation: FilesystemOperation::WriteFile,
                reason: "injected supervisor persistence fault".to_string(),
            })
        }
    }

    #[async_trait]
    impl ProcessTransitionPort for FaultRuntime {
        type Error = ProcessJournalStoreError;

        async fn claim_next_processes(
            &self,
            request: ClaimProcessesRequest,
        ) -> Result<Vec<ClaimedProcess>, Self::Error> {
            if self
                .claim_errors_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(Self::injected_error());
            }
            self.inner.claim_next_processes(request).await
        }

        async fn heartbeat_process(
            &self,
            request: ProcessLeaseRequest,
        ) -> Result<ProcessJournalCursor, Self::Error> {
            match self.heartbeat {
                HeartbeatFault::None => self.inner.heartbeat_process(request).await,
                HeartbeatFault::Error => Err(Self::injected_error()),
                HeartbeatFault::Pending => std::future::pending().await,
            }
        }

        async fn recover_expired_process_leases(
            &self,
            request: RecoverExpiredProcessLeasesRequest,
        ) -> Result<crate::RecoverExpiredProcessLeasesResponse, Self::Error> {
            if self
                .recover_errors_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(Self::injected_error());
            }
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
            self.inner.complete_process(request).await
        }

        async fn cancel_process(
            &self,
            request: ProcessStateTransitionRequest,
        ) -> Result<JournaledProcessSnapshot, Self::Error> {
            self.inner.cancel_process(request).await
        }

        async fn fail_process(
            &self,
            request: FailProcessRequest,
        ) -> Result<JournaledProcessSnapshot, Self::Error> {
            self.fail_attempts.fetch_add(1, Ordering::SeqCst);
            if self
                .fail_errors_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(Self::injected_error());
            }
            self.inner.fail_process(request).await
        }

        async fn relinquish_process(
            &self,
            request: ProcessLeaseRequest,
        ) -> Result<JournaledProcessSnapshot, Self::Error> {
            self.relinquishes.fetch_add(1, Ordering::SeqCst);
            self.inner.relinquish_process(request).await
        }
    }

    struct CompletingExecutor {
        runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>>,
    }

    #[async_trait]
    impl JournalProcessExecutor for CompletingExecutor {
        async fn execute_claimed_process(
            &self,
            claimed: ClaimedProcess,
        ) -> Result<(), ProcessExecutorFailure> {
            self.runtime
                .complete_process(ProcessStateTransitionRequest {
                    lease: ProcessLeaseRequest {
                        process_id: claimed.state.process_id,
                        worker_id: claimed.worker_id,
                        lease_token: claimed.lease_token,
                    },
                    metadata: None,
                })
                .await
                .map_err(|_| ProcessExecutorFailure::new("completion_failed"))?;
            Ok(())
        }
    }

    struct FailingExecutor;

    #[async_trait]
    impl JournalProcessExecutor for FailingExecutor {
        async fn execute_claimed_process(
            &self,
            _claimed: ClaimedProcess,
        ) -> Result<(), ProcessExecutorFailure> {
            Err(ProcessExecutorFailure::new("executor_rejected"))
        }
    }

    struct PanickingExecutor;

    #[async_trait]
    impl JournalProcessExecutor for PanickingExecutor {
        async fn execute_claimed_process(
            &self,
            _claimed: ClaimedProcess,
        ) -> Result<(), ProcessExecutorFailure> {
            panic!("deterministic executor panic");
        }
    }

    struct DoublePanickingExecutor;

    #[async_trait]
    impl JournalProcessExecutor for DoublePanickingExecutor {
        async fn execute_claimed_process(
            &self,
            _claimed: ClaimedProcess,
        ) -> Result<(), ProcessExecutorFailure> {
            panic!("first deterministic panic");
        }

        fn panic_failure(&self) -> ProcessExecutorFailure {
            panic!("second deterministic panic");
        }
    }

    struct BlockingExecutor {
        started: Arc<Notify>,
        executions: Option<Arc<AtomicUsize>>,
    }

    #[async_trait]
    impl JournalProcessExecutor for BlockingExecutor {
        async fn execute_claimed_process(
            &self,
            _claimed: ClaimedProcess,
        ) -> Result<(), ProcessExecutorFailure> {
            if let Some(executions) = &self.executions {
                executions.fetch_add(1, Ordering::SeqCst);
            }
            self.started.notify_one();
            std::future::pending().await
        }
    }

    struct CountingExecutor {
        runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>>,
        active: AtomicUsize,
        maximum: AtomicUsize,
    }

    #[async_trait]
    impl JournalProcessExecutor for CountingExecutor {
        async fn execute_claimed_process(
            &self,
            claimed: ClaimedProcess,
        ) -> Result<(), ProcessExecutorFailure> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(10)).await;
            let result = self
                .runtime
                .complete_process(ProcessStateTransitionRequest {
                    lease: ProcessLeaseRequest {
                        process_id: claimed.state.process_id,
                        worker_id: claimed.worker_id,
                        lease_token: claimed.lease_token,
                    },
                    metadata: None,
                })
                .await
                .map_err(|_| ProcessExecutorFailure::new("completion_failed"));
            self.active.fetch_sub(1, Ordering::SeqCst);
            result.map(|_| ())
        }
    }

    #[tokio::test]
    async fn wake_executes_and_completes_matching_process_kind() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            store.clone();
        let process_id = submit(&store, ProcessKind::Internal).await;
        let executor = Arc::new(CompletingExecutor {
            runtime: Arc::clone(&runtime),
        });
        let handle =
            ProcessSupervisor::new(runtime, executor, ProcessKind::Internal, fast_config()).start();

        handle
            .wake_notifier()
            .notify_scope(scope())
            .expect("wake supervisor");
        let snapshot = wait_for_status(&store, process_id, ProcessLifecycleStatus::Completed).await;

        assert_eq!(snapshot.status, ProcessLifecycleStatus::Completed);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn executor_failure_is_recorded_by_supervisor() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let process_id = submit(&store, ProcessKind::Internal).await;
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            store.clone();
        let handle = ProcessSupervisor::new(
            runtime,
            Arc::new(FailingExecutor),
            ProcessKind::Internal,
            fast_config(),
        )
        .start();

        let snapshot = wait_for_status(&store, process_id, ProcessLifecycleStatus::Failed).await;

        assert_eq!(
            snapshot.failure.as_ref().map(SanitizedFailure::category),
            Some("executor_rejected")
        );
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn executor_panic_is_converted_to_terminal_failure() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let process_id = submit(&store, ProcessKind::Internal).await;
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            store.clone();
        let handle = ProcessSupervisor::new(
            runtime,
            Arc::new(PanickingExecutor),
            ProcessKind::Internal,
            fast_config(),
        )
        .start();

        let snapshot = wait_for_status(&store, process_id, ProcessLifecycleStatus::Failed).await;
        assert_eq!(
            snapshot.failure.as_ref().map(SanitizedFailure::category),
            Some("process_executor_panic")
        );
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn concurrency_cap_is_never_exceeded_and_queued_work_drains() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let mut process_ids = Vec::new();
        for _ in 0..6 {
            process_ids.push(submit(&store, ProcessKind::Internal).await);
        }
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            store.clone();
        let executor = Arc::new(CountingExecutor {
            runtime: Arc::clone(&runtime),
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
        });
        let handle = ProcessSupervisor::new(
            runtime,
            Arc::clone(&executor) as Arc<dyn JournalProcessExecutor>,
            ProcessKind::Internal,
            fast_config().with_max_concurrent_processes(2),
        )
        .start();

        for process_id in process_ids {
            wait_for_status(&store, process_id, ProcessLifecycleStatus::Completed).await;
        }
        assert!(executor.maximum.load(Ordering::SeqCst) <= 2);
        assert_eq!(executor.active.load(Ordering::SeqCst), 0);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_relinquishes_in_flight_process() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let process_id = submit(&store, ProcessKind::Internal).await;
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            store.clone();
        let started = Arc::new(Notify::new());
        let handle = ProcessSupervisor::new(
            runtime,
            Arc::new(BlockingExecutor {
                started: Arc::clone(&started),
                executions: None,
            }),
            ProcessKind::Internal,
            fast_config(),
        )
        .start();

        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .expect("executor starts");
        handle.shutdown().await;
        let snapshot = store
            .get_process_snapshot(GetProcessSnapshotRequest {
                scope: scope(),
                process_id,
            })
            .await
            .expect("load process");

        assert_eq!(snapshot.status, ProcessLifecycleStatus::Queued);
        assert!(snapshot.lease.is_none());
    }

    #[tokio::test]
    async fn dropping_handle_relinquishes_in_flight_process_without_explicit_shutdown() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let process_id = submit(&store, ProcessKind::Internal).await;
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            store.clone();
        let started = Arc::new(Notify::new());
        let handle = ProcessSupervisor::new(
            runtime,
            Arc::new(BlockingExecutor {
                started: Arc::clone(&started),
                executions: None,
            }),
            ProcessKind::Internal,
            fast_config(),
        )
        .start();

        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .expect("executor starts");
        drop(handle);
        let snapshot = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = store
                    .get_process_snapshot(GetProcessSnapshotRequest {
                        scope: scope(),
                        process_id,
                    })
                    .await
                    .expect("load process");
                if snapshot.status == ProcessLifecycleStatus::Queued && snapshot.lease.is_none() {
                    break snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("drop-triggered shutdown relinquishes process");

        assert_eq!(snapshot.status, ProcessLifecycleStatus::Queued);
        assert!(snapshot.lease.is_none());
    }

    #[tokio::test]
    async fn duplicate_wakes_do_not_execute_the_same_process_twice() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let process_id = submit(&store, ProcessKind::Internal).await;
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            store.clone();
        let executions = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Notify::new());
        let handle = ProcessSupervisor::new(
            runtime,
            Arc::new(BlockingExecutor {
                started: Arc::clone(&started),
                executions: Some(Arc::clone(&executions)),
            }),
            ProcessKind::Internal,
            fast_config(),
        )
        .start();
        let notifier = handle.wake_notifier();

        for _ in 0..16 {
            let _ = notifier.notify_scope(scope());
        }
        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .expect("executor starts");
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            executions.load(Ordering::SeqCst),
            1,
            "duplicate wakes must not duplicate a claimed process execution"
        );

        handle.shutdown().await;
        let snapshot = store
            .get_process_snapshot(GetProcessSnapshotRequest {
                scope: scope(),
                process_id,
            })
            .await
            .expect("load process");
        assert_eq!(snapshot.status, ProcessLifecycleStatus::Queued);
    }

    #[tokio::test]
    async fn blocked_executor_heartbeat_timeouts_fail_once_without_duplicate_execution() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let process_id = submit(&store, ProcessKind::Internal).await;
        let faults = Arc::new(FaultRuntime::new(
            Arc::clone(&store),
            HeartbeatFault::Pending,
            0,
        ));
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            faults.clone();
        let executions = Arc::new(AtomicUsize::new(0));
        let handle = ProcessSupervisor::new(
            runtime,
            Arc::new(BlockingExecutor {
                started: Arc::new(Notify::new()),
                executions: Some(Arc::clone(&executions)),
            }),
            ProcessKind::Internal,
            fast_config()
                .with_heartbeat_interval(Duration::from_millis(5))
                .with_max_consecutive_heartbeat_failures(2),
        )
        .start();

        let snapshot = wait_for_status(&store, process_id, ProcessLifecycleStatus::Failed).await;
        assert_eq!(
            snapshot.failure.as_ref().map(SanitizedFailure::category),
            Some("process_heartbeat_failed")
        );
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn repeated_heartbeat_errors_use_the_same_bounded_failure_path() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let process_id = submit(&store, ProcessKind::Internal).await;
        let faults = Arc::new(FaultRuntime::new(
            Arc::clone(&store),
            HeartbeatFault::Error,
            0,
        ));
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            faults.clone();
        let handle = ProcessSupervisor::new(
            runtime,
            Arc::new(BlockingExecutor {
                started: Arc::new(Notify::new()),
                executions: None,
            }),
            ProcessKind::Internal,
            fast_config()
                .with_heartbeat_interval(Duration::from_millis(5))
                .with_max_consecutive_heartbeat_failures(2),
        )
        .start();

        wait_for_status(&store, process_id, ProcessLifecycleStatus::Failed).await;
        assert_eq!(faults.fail_attempts.load(Ordering::SeqCst), 1);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn terminal_failure_write_retries_then_commits() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let process_id = submit(&store, ProcessKind::Internal).await;
        let faults = Arc::new(FaultRuntime::new(
            Arc::clone(&store),
            HeartbeatFault::None,
            2,
        ));
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            faults.clone();
        let handle = ProcessSupervisor::new(
            runtime,
            Arc::new(FailingExecutor),
            ProcessKind::Internal,
            fast_config()
                .with_terminal_failure_record_attempts(3)
                .with_terminal_failure_record_backoff(Duration::from_millis(1)),
        )
        .start();

        wait_for_status(&store, process_id, ProcessLifecycleStatus::Failed).await;
        assert_eq!(faults.fail_attempts.load(Ordering::SeqCst), 3);
        assert_eq!(faults.relinquishes.load(Ordering::SeqCst), 0);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn terminal_failure_write_exhaustion_relinquishes_the_lease() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        submit(&store, ProcessKind::Internal).await;
        let faults = Arc::new(FaultRuntime::new(
            Arc::clone(&store),
            HeartbeatFault::None,
            usize::MAX,
        ));
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            faults.clone();
        let handle = ProcessSupervisor::new(
            runtime,
            Arc::new(FailingExecutor),
            ProcessKind::Internal,
            fast_config()
                .with_terminal_failure_record_attempts(2)
                .with_terminal_failure_record_backoff(Duration::from_millis(1)),
        )
        .start();

        tokio::time::timeout(Duration::from_secs(2), async {
            while faults.relinquishes.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("supervisor relinquishes after terminal-write exhaustion");
        assert!(faults.fail_attempts.load(Ordering::SeqCst) >= 2);
        handle.shutdown().await;
    }

    #[test]
    fn config_failure_and_wake_surfaces_cover_clamping_and_closed_channels() {
        let config = ProcessSupervisorConfig::default()
            .with_max_concurrent_processes(0)
            .with_poll_interval(Duration::ZERO)
            .with_lease_recovery_interval(Duration::ZERO)
            .with_heartbeat_interval(Duration::ZERO)
            .with_max_consecutive_heartbeat_failures(0)
            .with_terminal_failure_record_attempts(0)
            .with_terminal_failure_record_backoff(Duration::ZERO)
            .with_claim_error_backoff(Duration::ZERO)
            .with_wake_channel_capacity(0);
        assert_eq!(config.max_concurrent_processes(), 1);
        assert_eq!(config.poll_interval(), Duration::from_millis(1));
        assert_eq!(config.lease_recovery_interval(), Duration::from_millis(1));
        assert_eq!(config.heartbeat_interval(), Duration::from_millis(1));
        assert_eq!(config.max_consecutive_heartbeat_failures(), 1);
        assert_eq!(config.terminal_failure_record_attempts(), 1);
        assert_eq!(
            config.terminal_failure_record_backoff(),
            Duration::from_millis(1)
        );
        assert_eq!(config.claim_error_backoff(), Duration::from_millis(1));
        assert_eq!(config.wake_channel_capacity(), 1);

        let sanitized =
            SanitizedFailure::new("explicit_failure").expect("valid sanitized failure category");
        let failure = ProcessExecutorFailure::from_failure(sanitized);
        assert_eq!(failure.failure_category(), "explicit_failure");
        assert!(failure.failure().is_some());
        assert_eq!(
            failure.to_string(),
            "process executor failed: explicit_failure"
        );
        let fallback = ProcessExecutorFailure::new("");
        assert!(fallback.failure().is_none());
        assert_eq!(fallback.failure_category(), "");

        let (notifier, channel) = ProcessWakeNotifier::channel(1);
        assert_eq!(format!("{notifier:?}"), "ProcessWakeNotifier");
        notifier.notify_scope(scope()).expect("first wake fits");
        assert_eq!(
            notifier.notify_scope(scope()),
            Err(ProcessWakeError::DeliveryUnavailable)
        );
        drop(channel);
        assert_eq!(
            notifier.notify_scope(scope()),
            Err(ProcessWakeError::DeliveryUnavailable)
        );

        assert!(static_failure("").is_none());
        assert!(retryable_store_error(&FaultRuntime::injected_error()));
        assert!(!retryable_store_error(
            &ProcessJournalStoreError::InvalidRequest("terminal".to_string())
        ));
    }

    #[tokio::test]
    async fn transient_claim_failure_schedules_one_retry_and_then_executes() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let process_id = submit(&store, ProcessKind::Internal).await;
        let faults = Arc::new(
            FaultRuntime::new(Arc::clone(&store), HeartbeatFault::None, 0).with_claim_errors(1),
        );
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            faults.clone();
        let handle = ProcessSupervisor::new(
            Arc::clone(&runtime),
            Arc::new(CompletingExecutor { runtime }),
            ProcessKind::Internal,
            fast_config().with_claim_error_backoff(Duration::from_millis(1)),
        )
        .start();

        let snapshot = wait_for_status(&store, process_id, ProcessLifecycleStatus::Completed).await;
        assert_eq!(snapshot.status, ProcessLifecycleStatus::Completed);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn successful_heartbeats_reset_failure_counter_before_completion() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let process_id = submit(&store, ProcessKind::Internal).await;
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            store.clone();
        let executor = Arc::new(CountingExecutor {
            runtime: Arc::clone(&runtime),
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
        });
        let handle = ProcessSupervisor::new(
            runtime,
            executor,
            ProcessKind::Internal,
            fast_config()
                .with_heartbeat_interval(Duration::from_millis(1))
                .with_max_consecutive_heartbeat_failures(2),
        )
        .start();

        wait_for_status(&store, process_id, ProcessLifecycleStatus::Completed).await;
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn executor_task_panic_after_panic_mapping_is_joined_and_relinquished_on_shutdown() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let process_id = submit(&store, ProcessKind::Internal).await;
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            store.clone();
        let handle = ProcessSupervisor::new(
            runtime,
            Arc::new(DoublePanickingExecutor),
            ProcessKind::Internal,
            fast_config(),
        )
        .start();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = store
                    .get_process_snapshot(GetProcessSnapshotRequest {
                        scope: scope(),
                        process_id,
                    })
                    .await
                    .expect("load process");
                if snapshot.status == ProcessLifecycleStatus::Running {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("process is claimed before the task panics");
        assert!(!handle.is_stopped());
        tokio::time::sleep(Duration::from_millis(10)).await;
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn lease_recovery_and_relinquish_error_paths_are_non_fatal() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let process_id = submit(&store, ProcessKind::Internal).await;
        let faults = Arc::new(
            FaultRuntime::new(Arc::clone(&store), HeartbeatFault::None, 0).with_recover_errors(1),
        );
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            faults.clone();
        let (command_tx, _command_rx) = mpsc::channel(1);
        let context = DrainContext {
            command_tx,
            runtime: Arc::clone(&runtime),
            executor: Arc::new(FailingExecutor),
            process_kind: ProcessKind::Internal,
            config: fast_config(),
            worker_id: ProcessWorkerId::from_trusted("recovery-worker"),
            semaphore: Arc::new(Semaphore::new(1)),
        };

        recover_expired_leases(&context).await;
        recover_expired_leases(&context).await;
        relinquish(
            &runtime,
            &ClaimedIdentity {
                process_id,
                worker_id: ProcessWorkerId::from_trusted("wrong-worker"),
                lease_token: ProcessLeaseToken::from_trusted("wrong-lease"),
            },
        )
        .await;
        assert_eq!(faults.relinquishes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn detached_handle_reports_stopped_and_shutdown_is_idempotent() {
        let (notifier, _channel) = ProcessWakeNotifier::channel(1);
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        let handle = ProcessSupervisorHandle {
            notifier,
            supervisor: None,
            shutdown_tx,
        };
        assert!(handle.is_stopped());
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn in_flight_heartbeat_abort_cancels_the_running_task() {
        let store = Arc::new(crate::ProcessJournalStore::new(
            in_memory_backed_processes_filesystem(),
        ));
        let runtime: Arc<dyn ProcessTransitionPort<Error = ProcessJournalStoreError>> =
            Arc::new(FaultRuntime::new(store, HeartbeatFault::Pending, 0));
        let mut heartbeat = InFlightHeartbeat::default();
        heartbeat.spawn(
            runtime,
            ProcessId::new(),
            ProcessWorkerId::from_trusted("abort-worker"),
            ProcessLeaseToken::from_trusted("abort-lease"),
            Duration::from_secs(60),
        );
        assert!(heartbeat.is_running());
        heartbeat.abort();
        assert!(heartbeat.is_idle());
    }

    fn fast_config() -> ProcessSupervisorConfig {
        ProcessSupervisorConfig::default()
            .with_poll_interval(Duration::from_millis(5))
            .with_lease_recovery_interval(Duration::from_secs(60))
            .with_heartbeat_interval(Duration::from_secs(60))
    }

    async fn submit(
        store: &crate::ProcessJournalStore<ironclaw_filesystem::InMemoryBackend>,
        process_kind: ProcessKind,
    ) -> ProcessId {
        let process_id = ProcessId::new();
        let resource_scope = scope();
        store
            .submit_process(SubmitProcessRequest {
                process_id,
                process_kind,
                scope: resource_scope.clone(),
                exclusive_within_scope: false,
                operation_id: None,
                owner_user_id: Some(resource_scope.user_id),
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
        process_id
    }

    async fn wait_for_status(
        store: &crate::ProcessJournalStore<ironclaw_filesystem::InMemoryBackend>,
        process_id: ProcessId,
        expected: ProcessLifecycleStatus,
    ) -> JournaledProcessSnapshot {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = store
                    .get_process_snapshot(GetProcessSnapshotRequest {
                        scope: scope(),
                        process_id,
                    })
                    .await
                    .expect("load process");
                if snapshot.status == expected {
                    return snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("process reaches expected status")
    }

    fn scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-supervisor").expect("tenant"),
            user_id: UserId::new("user-supervisor").expect("user"),
            agent_id: Some(AgentId::new("agent-supervisor").expect("agent")),
            project_id: Some(ProjectId::new("project-supervisor").expect("project")),
            mission_id: None,
            thread_id: Some(ThreadId::new("thread-supervisor").expect("thread")),
            invocation_id: InvocationId::new(),
        }
    }
}
