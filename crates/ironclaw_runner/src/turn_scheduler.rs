//! Agent-turn projection over the generic process supervisor.

use std::{error::Error, fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use ironclaw_processes::{
    ClaimedProcess, JournalProcessExecutor, ProcessExecutorFailure, ProcessFailureRecovery,
    ProcessKind, ProcessRuntimePort, ProcessSupervisor, ProcessSupervisorConfig,
    ProcessSupervisorHandle, ProcessWakeChannel, ProcessWakeNotifier,
};
use ironclaw_turns::{
    SanitizedFailure, TurnError, TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError,
    claimed_turn_run_from_process_claim, runner::ClaimedTurnRun,
};

#[derive(Debug, Clone, Default)]
pub struct TurnRunSchedulerConfig {
    inner: ProcessSupervisorConfig,
}

impl TurnRunSchedulerConfig {
    pub fn max_concurrent_runs(&self) -> usize {
        self.inner.max_concurrent_processes()
    }

    pub fn poll_interval(&self) -> Duration {
        self.inner.poll_interval()
    }

    pub fn lease_recovery_interval(&self) -> Duration {
        self.inner.lease_recovery_interval()
    }

    pub fn runner_heartbeat_interval(&self) -> Duration {
        self.inner.heartbeat_interval()
    }

    pub fn max_consecutive_heartbeat_failures(&self) -> usize {
        self.inner.max_consecutive_heartbeat_failures()
    }

    pub fn terminal_failure_record_attempts(&self) -> usize {
        self.inner.terminal_failure_record_attempts()
    }

    pub fn terminal_failure_record_backoff(&self) -> Duration {
        self.inner.terminal_failure_record_backoff()
    }

    pub fn claim_error_backoff(&self) -> Duration {
        self.inner.claim_error_backoff()
    }

    pub fn wake_channel_capacity(&self) -> usize {
        self.inner.wake_channel_capacity()
    }

    pub fn with_max_concurrent_runs(mut self, maximum: usize) -> Self {
        self.inner = self.inner.with_max_concurrent_processes(maximum);
        self
    }

    pub fn with_poll_interval(mut self, value: Duration) -> Self {
        self.inner = self.inner.with_poll_interval(value);
        self
    }

    pub fn with_lease_recovery_interval(mut self, value: Duration) -> Self {
        self.inner = self.inner.with_lease_recovery_interval(value);
        self
    }

    pub fn with_runner_heartbeat_interval(mut self, value: Duration) -> Self {
        self.inner = self.inner.with_heartbeat_interval(value);
        self
    }

    pub fn with_max_consecutive_heartbeat_failures(mut self, maximum: usize) -> Self {
        self.inner = self.inner.with_max_consecutive_heartbeat_failures(maximum);
        self
    }

    pub fn with_terminal_failure_record_attempts(mut self, maximum: usize) -> Self {
        self.inner = self.inner.with_terminal_failure_record_attempts(maximum);
        self
    }

    pub fn with_terminal_failure_record_backoff(mut self, value: Duration) -> Self {
        self.inner = self.inner.with_terminal_failure_record_backoff(value);
        self
    }

    pub fn with_claim_error_backoff(mut self, value: Duration) -> Self {
        self.inner = self.inner.with_claim_error_backoff(value);
        self
    }

    pub fn with_wake_channel_capacity(mut self, capacity: usize) -> Self {
        self.inner = self.inner.with_wake_channel_capacity(capacity);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRunExecutorError {
    failure: SanitizedFailure,
}

impl TurnRunExecutorError {
    pub fn new(failure_category: impl Into<String>) -> Result<Self, String> {
        SanitizedFailure::new(failure_category).map(|failure| Self { failure })
    }

    pub fn from_failure(failure: SanitizedFailure) -> Self {
        Self { failure }
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
        process_transitions: Arc<dyn ironclaw_processes::ProcessTransitionPort<Error = TurnError>>,
    ) -> Result<(), TurnRunExecutorError>;
}

struct TurnProcessExecutor {
    executor: Arc<dyn TurnRunExecutor>,
    transitions: Arc<dyn ironclaw_processes::ProcessTransitionPort<Error = TurnError>>,
}

#[async_trait]
impl JournalProcessExecutor for TurnProcessExecutor {
    async fn execute_claimed_process(
        &self,
        claimed: ClaimedProcess,
    ) -> Result<(), ProcessExecutorFailure> {
        let claimed = claimed_turn_run_from_process_claim(claimed)
            .map_err(|_| ProcessExecutorFailure::new("process_claim_invalid"))?;
        self.executor
            .execute_claimed_run(claimed, Arc::clone(&self.transitions))
            .await
            .map_err(|error| {
                let recovery = match error.failure_category() {
                    crate::failure_categories::HOST_STAGE_UNAVAILABLE_INPUT_CATEGORY
                    | crate::failure_categories::HOST_STAGE_UNAVAILABLE_PROMPT_CATEGORY
                    | crate::failure_categories::HOST_STAGE_UNAVAILABLE_CAPABILITY_CATEGORY => {
                        ProcessFailureRecovery::RedriveIfCheckpointless
                    }
                    _ => ProcessFailureRecovery::Terminal,
                };
                ProcessExecutorFailure::from_failure(error.failure().clone())
                    .with_recovery(recovery)
            })
    }

    fn panic_failure(&self) -> ProcessExecutorFailure {
        ProcessExecutorFailure::new("scheduler_executor_panic")
    }

    fn heartbeat_failure(&self) -> ProcessExecutorFailure {
        ProcessExecutorFailure::new("scheduler_heartbeat_failed")
    }
}

pub struct TurnRunScheduler {
    runtime: Arc<dyn ProcessRuntimePort>,
    executor: Arc<dyn TurnRunExecutor>,
    config: TurnRunSchedulerConfig,
}

impl TurnRunScheduler {
    pub fn new_with_process_runtime(
        runtime: Arc<dyn ProcessRuntimePort>,
        executor: Arc<dyn TurnRunExecutor>,
        config: TurnRunSchedulerConfig,
    ) -> Self {
        Self {
            runtime,
            executor,
            config,
        }
    }

    pub fn start(self) -> TurnRunSchedulerHandle {
        let (notifier, channel) =
            SchedulerTurnRunWakeNotifier::channel(self.config.wake_channel_capacity());
        self.start_with_channel(notifier, channel)
    }

    pub fn start_with_channel(
        self,
        notifier: Arc<SchedulerTurnRunWakeNotifier>,
        channel: TurnRunWakeChannel,
    ) -> TurnRunSchedulerHandle {
        let transitions = Arc::new(ironclaw_turns::ProcessJournalStoreTurnAdapter::new(
            Arc::clone(&self.runtime),
        ));
        let executor = Arc::new(TurnProcessExecutor {
            executor: self.executor,
            transitions,
        });
        let supervisor = ProcessSupervisor::new(
            self.runtime,
            executor,
            ProcessKind::AgentTurn,
            self.config.inner,
        )
        .start_with_channel(Arc::clone(&notifier.inner), channel.inner);
        TurnRunSchedulerHandle {
            notifier,
            supervisor: Some(supervisor),
        }
    }
}

pub struct TurnRunWakeChannel {
    inner: ProcessWakeChannel,
}

#[derive(Clone)]
pub struct SchedulerTurnRunWakeNotifier {
    inner: Arc<ProcessWakeNotifier>,
}

impl SchedulerTurnRunWakeNotifier {
    pub fn channel(capacity: usize) -> (Arc<Self>, TurnRunWakeChannel) {
        let (notifier, channel) = ProcessWakeNotifier::channel(capacity);
        (
            Arc::new(Self { inner: notifier }),
            TurnRunWakeChannel { inner: channel },
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
        self.inner
            .notify_scope(wake.scope.to_resource_scope())
            .map_err(|_| TurnRunWakeNotifyError::DeliveryUnavailable)
    }
}

pub struct TurnRunSchedulerHandle {
    notifier: Arc<SchedulerTurnRunWakeNotifier>,
    supervisor: Option<ProcessSupervisorHandle>,
}

impl TurnRunSchedulerHandle {
    pub fn wake_notifier(&self) -> Arc<SchedulerTurnRunWakeNotifier> {
        Arc::clone(&self.notifier)
    }

    pub fn is_stopped(&self) -> bool {
        self.supervisor
            .as_ref()
            .is_none_or(ProcessSupervisorHandle::is_stopped)
    }

    pub async fn shutdown(mut self) {
        if let Some(supervisor) = self.supervisor.take() {
            supervisor.shutdown().await;
        }
    }
}
