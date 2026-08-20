//! Agent-turn projection over the generic process supervisor.

use std::{error::Error, fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use ironclaw_loop_contracts::LoopModelUsage;
use ironclaw_processes::{
    ClaimedProcess, JournalProcessExecutor, ProcessExecutorFailure, ProcessFailureRecovery,
    ProcessKind, ProcessRuntimePort, ProcessSupervisor, ProcessSupervisorConfig,
    ProcessSupervisorHandle, ProcessWakeChannel, ProcessWakeNotifier,
};
use ironclaw_turns::{
    SanitizedFailure, TurnError, TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError,
    claimed_turn_run_from_process_claim, runner::ClaimedTurnRun,
};

/// Consecutive heartbeat failures a turn run absorbs before the supervisor gives
/// up on it.
///
/// The generic [`ProcessSupervisorConfig`] default is 3. The turn-run maximum
/// remains 8 for callers using shorter intervals, while
/// [`heartbeat_failure_budget_within_lease`] reduces it to the largest budget
/// the configured interval can fit inside the lease TTL. At the shipped
/// 15-second interval, that budget is 3.
///
/// This is a turn-run figure only. The generic default stays at 3 because the
/// capability path heartbeats every 30 seconds, where 8 failures would run far
/// past the lease TTL.
pub const TURN_RUN_MAX_CONSECUTIVE_HEARTBEAT_FAILURES: usize = 8;

/// Largest heartbeat interval whose *first* failure still fits inside the
/// process lease TTL. A failing heartbeat costs one interval of waiting plus
/// one heartbeat timeout, and the supervisor passes the interval as that
/// timeout, so a single attempt costs `2 * interval`. An interval past this
/// bound can let the lease expire before the worker even records its first
/// failure — the worker would be reclaimed while still executing.
pub const MAX_HEARTBEAT_INTERVAL_WITHIN_LEASE: Duration = Duration::from_millis(
    ironclaw_processes::DEFAULT_PROCESS_LEASE_DURATION.as_millis() as u64 / 2,
);

/// Largest failure budget whose abandon window still fits inside the process
/// lease TTL at the given heartbeat interval.
///
/// A failing heartbeat costs one interval of waiting plus one heartbeat timeout,
/// and the supervisor passes the interval as that timeout, so each failure costs
/// `2 * interval`. Staying inside the TTL is what keeps abandonment a *decision
/// the worker makes* — it stops, records a heartbeat failure, and the journal's
/// lease is still live — rather than a lease expiry discovered by a recovery
/// sweep with no idea whether the worker is alive.
fn heartbeat_failure_budget_within_lease(interval: Duration) -> usize {
    let cost_per_failure = heartbeat_interval_within_lease(interval).saturating_mul(2);
    if cost_per_failure.is_zero() {
        return TURN_RUN_MAX_CONSECUTIVE_HEARTBEAT_FAILURES;
    }
    let affordable = usize::try_from(
        ironclaw_processes::DEFAULT_PROCESS_LEASE_DURATION.as_millis()
            / cost_per_failure.as_millis(),
    )
    .unwrap_or(usize::MAX);
    affordable.clamp(1, TURN_RUN_MAX_CONSECUTIVE_HEARTBEAT_FAILURES)
}

/// The requested heartbeat interval, capped so that a *single* failure still
/// fits inside the lease TTL.
///
/// A failure costs `2 * interval` (one interval of waiting plus one heartbeat
/// timeout, which the supervisor sets to the interval), so half the TTL is the
/// widest interval any budget can pay for. Past that, the smallest budget the
/// scheduler can run with — one failure — already outlives the lease, and
/// clamping the *budget* up to 1 would only make the promise untrue: the worker
/// would still be waiting on its first heartbeat when the journal declared its
/// lease expired. Capping the interval instead keeps `budget >= 1` honest for
/// every value an operator can configure, and errs toward heartbeating *more*
/// often than asked, which is the safe direction.
fn heartbeat_interval_within_lease(interval: Duration) -> Duration {
    interval.min(MAX_HEARTBEAT_INTERVAL_WITHIN_LEASE)
}

#[derive(Debug, Clone)]
pub struct TurnRunSchedulerConfig {
    inner: ProcessSupervisorConfig,
}

impl Default for TurnRunSchedulerConfig {
    fn default() -> Self {
        let inner = ProcessSupervisorConfig::default();
        let interval = heartbeat_interval_within_lease(inner.heartbeat_interval());
        let budget = heartbeat_failure_budget_within_lease(interval);
        Self {
            inner: inner
                .with_heartbeat_interval(interval)
                .with_max_consecutive_heartbeat_failures(budget),
        }
    }
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

    /// Set the heartbeat interval, and with it the failure budget the lease TTL
    /// can afford at that interval. A deployment that widens the interval gets a
    /// smaller budget rather than an abandon window that outlives its lease.
    ///
    /// Intervals past [`MAX_HEARTBEAT_INTERVAL_WITHIN_LEASE`] cannot fit even
    /// one heartbeat attempt (wait + timeout) inside the lease TTL, so they are
    /// capped ([`heartbeat_interval_within_lease`]) rather than accepted with a
    /// budget that cannot be honoured. Operator-facing config parsing rejects
    /// such values outright; the cap covers programmatic callers.
    pub fn with_runner_heartbeat_interval(mut self, value: Duration) -> Self {
        self.inner = self
            .inner
            .with_heartbeat_interval(heartbeat_interval_within_lease(value));
        let budget = heartbeat_failure_budget_within_lease(self.inner.heartbeat_interval());
        self.inner = self.inner.with_max_consecutive_heartbeat_failures(budget);
        self
    }

    /// Lower the failure budget. The lease-derived budget for the configured
    /// interval is the ceiling: a caller may be more conservative than the TTL
    /// affords, never less.
    pub fn with_max_consecutive_heartbeat_failures(mut self, maximum: usize) -> Self {
        let ceiling = heartbeat_failure_budget_within_lease(self.inner.heartbeat_interval());
        self.inner = self
            .inner
            .with_max_consecutive_heartbeat_failures(maximum.min(ceiling));
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
    failure_metadata: Option<TurnRunFailureMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnRunFailureMetadata {
    /// The cumulative usage snapshot to preserve when the scheduler has to
    /// terminalize a failed process.  Callers must merge any supplemental
    /// model work into the claimed snapshot before constructing this value.
    model_usage: Option<LoopModelUsage>,
}

impl TurnRunFailureMetadata {
    pub fn from_model_usage(model_usage: LoopModelUsage) -> Self {
        Self::from_optional_model_usage(Some(model_usage))
    }

    pub fn from_optional_model_usage(model_usage: Option<LoopModelUsage>) -> Self {
        Self { model_usage }
    }

    pub fn model_usage(&self) -> Option<LoopModelUsage> {
        self.model_usage
    }
}

impl TurnRunExecutorError {
    pub fn new(failure_category: impl Into<String>) -> Result<Self, String> {
        SanitizedFailure::new(failure_category).map(|failure| Self {
            failure,
            failure_metadata: None,
        })
    }

    pub fn from_failure(failure: SanitizedFailure) -> Self {
        Self {
            failure,
            failure_metadata: None,
        }
    }

    pub fn with_failure_metadata(mut self, metadata: TurnRunFailureMetadata) -> Self {
        self.failure_metadata = Some(metadata);
        self
    }

    pub fn failure_metadata(&self) -> Option<&TurnRunFailureMetadata> {
        self.failure_metadata.as_ref()
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
        let claimed_for_failure_metadata = claimed.clone();
        self.executor
            .execute_claimed_run(claimed, Arc::clone(&self.transitions))
            .await
            .map_err(|error| {
                let recovery = match error.failure_category() {
                    ironclaw_host_api::failure::categories::HOST_STAGE_UNAVAILABLE_INPUT_CATEGORY
                    | ironclaw_host_api::failure::categories::HOST_STAGE_UNAVAILABLE_PROMPT_CATEGORY
                    | ironclaw_host_api::failure::categories::HOST_STAGE_UNAVAILABLE_CAPABILITY_CATEGORY => {
                        ProcessFailureRecovery::RedriveIfCheckpointless
                    }
                    _ => ProcessFailureRecovery::Terminal,
                };
                let mut process_failure =
                    ProcessExecutorFailure::from_failure(error.failure().clone())
                        .with_recovery(recovery);
                if let Some(metadata) = error.failure_metadata() {
                    process_failure = process_failure.with_metadata(
                        ironclaw_turns::process_projection::agent_turn_metadata_from_claimed(
                            &claimed_for_failure_metadata,
                            metadata.model_usage(),
                            None,
                        ),
                    );
                }
                process_failure
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

#[cfg(test)]
mod tests {
    use super::*;

    struct MetadataFailureExecutor {
        usage: LoopModelUsage,
    }

    #[async_trait]
    impl TurnRunExecutor for MetadataFailureExecutor {
        async fn execute_claimed_run(
            &self,
            claimed: ClaimedTurnRun,
            _process_transitions: Arc<
                dyn ironclaw_processes::ProcessTransitionPort<Error = TurnError>,
            >,
        ) -> Result<(), TurnRunExecutorError> {
            assert!(matches!(
                claimed.state.output_contract,
                ironclaw_host_api::output::OutputContract::JsonSchema { .. }
            ));
            Err(TurnRunExecutorError::from_failure(
                SanitizedFailure::new("structured_finalization_failed")
                    .expect("valid failure category"),
            )
            .with_failure_metadata(TurnRunFailureMetadata::from_model_usage(self.usage)))
        }
    }

    #[tokio::test]
    async fn process_executor_failure_projects_complete_agent_turn_metadata() {
        use ironclaw_host_api::{
            ids::{AgentId, ProjectId, TenantId, ThreadId},
            output::OutputContract,
            turn::{TurnId, TurnRunId, TurnScope, TurnStatus},
        };
        use ironclaw_loop_contracts::{
            InMemoryRunProfileResolver, RunProfileResolutionRequest, RunProfileResolver,
        };
        use ironclaw_processes::JournalProcessExecutor;
        use ironclaw_turns::{
            AcceptedMessageRef, EventCursor, TurnLeaseToken, TurnRunState, TurnRunnerId,
            test_support::in_memory_agent_turn_process_system,
        };

        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("profile");
        let output_contract = OutputContract::try_json_schema(
            "failure_output",
            serde_json::json!({"type": "object"}),
        )
        .expect("output contract");
        let scope = TurnScope::new(
            TenantId::new("tenant-scheduler-metadata").expect("tenant"),
            Some(AgentId::new("agent-scheduler-metadata").expect("agent")),
            Some(ProjectId::new("project-scheduler-metadata").expect("project")),
            ThreadId::new("thread-scheduler-metadata").expect("thread"),
        );
        let claimed = ClaimedTurnRun {
            subagent_activation_provenance: None,
            state: TurnRunState {
                scope,
                actor: None,
                turn_id: TurnId::new(),
                run_id: TurnRunId::new(),
                status: TurnStatus::Running,
                accepted_message_ref: AcceptedMessageRef::new("accepted-scheduler-metadata")
                    .expect("accepted message"),
                resolved_run_profile_id: resolved.profile_id.clone(),
                resolved_run_profile_version: resolved.profile_version,
                output_contract: output_contract.clone(),
                allow_steering: false,
                resolved_model_route: None,
                model_usage: None,
                execution_outcome: None,
                received_at: chrono::Utc::now(),
                checkpoint_id: None,
                gate_ref: None,
                blocked_activity_id: None,
                credential_requirements: Vec::new(),
                failure: None,
                event_cursor: EventCursor(1),
                product_context: None,
                resume_disposition: None,
            },
            resolved_run_profile: resolved,
            subagent_depth: 0,
            spawn_tree_descendant_cap: None,
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
        };
        let usage = LoopModelUsage {
            input_tokens: 13,
            output_tokens: 5,
            cache_read_input_tokens: 2,
            cache_creation_input_tokens: 1,
        };
        let process_system = in_memory_agent_turn_process_system();
        let executor = TurnProcessExecutor {
            executor: Arc::new(MetadataFailureExecutor { usage }),
            transitions: process_system.transitions(),
        };

        let error = executor
            .execute_claimed_process(ClaimedProcess::from(&claimed))
            .await
            .expect_err("executor must surface structured finalization failure");
        let metadata: ironclaw_turns::process_projection::AgentTurnProcessStateMetadata =
            serde_json::from_value(
                error
                    .metadata()
                    .expect("agent-turn failure metadata")
                    .get("agent_turn")
                    .expect("agent_turn envelope")
                    .clone(),
            )
            .expect("typed agent-turn metadata");
        assert_eq!(metadata.output_contract, output_contract);
        assert_eq!(metadata.model_usage, Some(usage));
    }

    #[test]
    fn executor_failure_keeps_supplemental_usage_as_typed_metadata() {
        let usage = LoopModelUsage {
            input_tokens: 13,
            output_tokens: 5,
            cache_read_input_tokens: 2,
            cache_creation_input_tokens: 1,
        };
        let failure = TurnRunExecutorError::from_failure(
            SanitizedFailure::new("transcript_write_failed").expect("valid category"),
        )
        .with_failure_metadata(TurnRunFailureMetadata::from_model_usage(usage));

        assert_eq!(
            failure
                .failure_metadata()
                .and_then(TurnRunFailureMetadata::model_usage),
            Some(usage)
        );
    }

    #[test]
    fn resumed_prior_usage_and_finalizer_failure_are_added_once() {
        let prior = LoopModelUsage {
            input_tokens: 100,
            output_tokens: 40,
            cache_read_input_tokens: 10,
            cache_creation_input_tokens: 2,
        };
        let finalizer = LoopModelUsage {
            input_tokens: 13,
            output_tokens: 5,
            cache_read_input_tokens: 2,
            cache_creation_input_tokens: 1,
        };
        let expected = LoopModelUsage {
            input_tokens: 113,
            output_tokens: 45,
            cache_read_input_tokens: 12,
            cache_creation_input_tokens: 3,
        };

        let failure_usage = LoopModelUsage::merge_optional(Some(prior), Some(finalizer));
        assert_eq!(failure_usage, Some(expected));
        let failure_metadata = TurnRunFailureMetadata::from_optional_model_usage(failure_usage);
        assert_eq!(failure_metadata.model_usage(), Some(expected));

        // A replay with no new supplemental call restores the already
        // aggregated snapshot rather than adding it again.
        assert_eq!(
            LoopModelUsage::merge_optional(failure_metadata.model_usage(), None),
            Some(expected)
        );
    }

    /// The failure budget exists to survive a slow store, but a worker that
    /// keeps retrying past its lease TTL is worse than one that gives up: the
    /// journal reclaims the run while the worker is still executing it. Every
    /// heartbeat interval a deployment can configure must keep the abandon
    /// window inside the TTL — and intervals that cannot fit even one attempt
    /// are clamped to [`MAX_HEARTBEAT_INTERVAL_WITHIN_LEASE`] rather than
    /// silently handed a budget of one.
    #[test]
    fn heartbeat_failure_budget_never_outlives_the_process_lease() {
        let intervals = [
            Duration::from_secs(1),
            Duration::from_secs(5),
            Duration::from_secs(10),
            Duration::from_secs(30),
            Duration::from_secs(45),
            Duration::from_secs(60),
            // Wider than the TTL can pay for even once: the interval itself
            // must be capped, not the budget clamped up to a lie.
            Duration::from_secs(120),
            ironclaw_processes::DEFAULT_PROCESS_LEASE_DURATION.saturating_mul(10),
        ];
        for interval in intervals {
            let config = TurnRunSchedulerConfig::default().with_runner_heartbeat_interval(interval);
            let budget = config.max_consecutive_heartbeat_failures();
            // The effective interval is what the supervisor actually waits on,
            // which is the requested one only while the TTL can afford it.
            let effective = config.runner_heartbeat_interval();
            assert!(budget >= 1, "a run must tolerate at least one failure");
            assert!(
                effective <= interval,
                "interval {interval:?} must never be widened, got {effective:?}"
            );
            // The supervisor passes the interval as the per-heartbeat timeout,
            // so a failed heartbeat costs interval + timeout.
            let abandon_window = effective
                .saturating_mul(2)
                .saturating_mul(u32::try_from(budget).expect("budget fits u32"));
            assert!(
                abandon_window <= ironclaw_processes::DEFAULT_PROCESS_LEASE_DURATION,
                "interval {interval:?} (effective {effective:?}) yields budget {budget} and an \
                 abandon window of {abandon_window:?}, past the {:?} lease TTL",
                ironclaw_processes::DEFAULT_PROCESS_LEASE_DURATION
            );
        }
    }

    /// The budget setter is the other way to construct a scheduler config, so
    /// it must not be able to reintroduce an abandon window past the TTL.
    #[test]
    fn an_explicit_failure_budget_cannot_exceed_what_the_lease_affords() {
        let config = TurnRunSchedulerConfig::default()
            .with_runner_heartbeat_interval(Duration::from_secs(30))
            .with_max_consecutive_heartbeat_failures(usize::MAX);
        let budget = config.max_consecutive_heartbeat_failures();
        let abandon_window = config
            .runner_heartbeat_interval()
            .saturating_mul(2)
            .saturating_mul(u32::try_from(budget).expect("budget fits u32"));
        assert!(
            abandon_window <= ironclaw_processes::DEFAULT_PROCESS_LEASE_DURATION,
            "an explicit budget of {budget} yields {abandon_window:?}, past the lease TTL"
        );
    }

    /// An interval past the lease-derived bound must be clamped, never
    /// accepted at a budget of one with an abandon window that outlives the
    /// lease.
    #[test]
    fn unaffordable_heartbeat_intervals_are_clamped_to_the_lease_bound() {
        for interval in [Duration::from_secs(60), Duration::from_secs(120)] {
            let config = TurnRunSchedulerConfig::default().with_runner_heartbeat_interval(interval);
            assert_eq!(
                config.runner_heartbeat_interval(),
                MAX_HEARTBEAT_INTERVAL_WITHIN_LEASE,
                "interval {interval:?} must be clamped to the lease-derived bound"
            );
            let abandon_window = MAX_HEARTBEAT_INTERVAL_WITHIN_LEASE
                .saturating_mul(2)
                .saturating_mul(
                    u32::try_from(config.max_consecutive_heartbeat_failures())
                        .expect("budget fits u32"),
                );
            assert!(
                abandon_window <= ironclaw_processes::DEFAULT_PROCESS_LEASE_DURATION,
                "clamped config must still fit inside the lease TTL, got {abandon_window:?}"
            );
        }
    }

    /// The hosted configuration is the one the pool-starvation incident hit: a
    /// 5-second heartbeat must clear a 30-second Postgres checkout timeout.
    #[test]
    fn hosted_heartbeat_interval_absorbs_one_postgres_checkout_timeout() {
        let config = TurnRunSchedulerConfig::default()
            .with_runner_heartbeat_interval(Duration::from_secs(5));
        assert_eq!(
            config.max_consecutive_heartbeat_failures(),
            TURN_RUN_MAX_CONSECUTIVE_HEARTBEAT_FAILURES
        );
        let tolerated = Duration::from_secs(5).saturating_mul(2).saturating_mul(
            u32::try_from(config.max_consecutive_heartbeat_failures()).expect("budget"),
        );
        assert!(
            tolerated >= Duration::from_secs(30),
            "a run must survive one full connection-checkout stall, tolerated {tolerated:?}"
        );
    }
}
