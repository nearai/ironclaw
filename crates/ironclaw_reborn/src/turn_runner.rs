//! Concrete Reborn turn-runner worker composition.
//!
//! This module owns the worker lifecycle that claims queued/resumed turn runs,
//! heartbeats the runner lease, selects a registered loop driver, constructs a
//! per-run `AgentLoopDriverHost`, invokes the driver, and applies the returned
//! `LoopExit` through trusted transition ports.
//!
//! # Architecture boundary
//!
//! `ironclaw_turns` owns `TurnRunTransitionPort`, claim/heartbeat/transition
//! DTOs, state-machine invariants, and the `apply_loop_exit` helper.
//!
//! This module owns the concrete worker loop, driver registry lookup, host
//! factory, readiness/config, and worker lifecycle.

use std::sync::Arc;
use std::time::Duration;

const MIN_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(1);
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(1);

use async_trait::async_trait;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use ironclaw_turns::{
    AgentLoopDriverError, AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, LoopExit,
    ResolvedRunProfile, SanitizedFailure, TurnError, TurnLeaseToken, TurnRunId, TurnRunnerId,
    TurnScope, TurnStatus,
    runner::{
        ClaimRunRequest, ClaimedTurnRun, HeartbeatRequest, RecordRecoveryRequiredRequest,
        TurnRunTransitionPort,
    },
};

use crate::loop_exit_applier::LoopExitApplier;

use crate::driver_registry::{DriverRegistry, LoopDriverRegistryKey};

/// Create a `SanitizedFailure` from a known-valid static category.
///
/// All categories used here are lowercase ASCII with underscores, satisfying
/// validation invariants. Returning `None` is only possible if a static literal
/// is changed to an invalid category.
fn sanitized_failure(category: &'static str) -> Option<SanitizedFailure> {
    match SanitizedFailure::new(category) {
        Ok(failure) => Some(failure),
        Err(error) => {
            error!(category, %error, "invalid static recovery failure category");
            SanitizedFailure::new("unknown_failure").map_or_else(
                |fallback_error| {
                    error!(%fallback_error, "fallback recovery failure category invalid");
                    None
                },
                Some,
            )
        }
    }
}

/// Configuration for the turn-runner worker.
#[derive(Debug, Clone)]
pub struct TurnRunnerWorkerConfig {
    /// How often to send heartbeats for an active run lease.
    pub heartbeat_interval: Duration,

    /// Fallback poll interval when no wake signal arrives.
    pub poll_interval: Duration,

    /// Optional scope filter to restrict which runs this worker claims.
    pub scope_filter: Option<TurnScope>,
}

impl Default for TurnRunnerWorkerConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(10),
            poll_interval: Duration::from_secs(5),
            scope_filter: None,
        }
    }
}

impl TurnRunnerWorkerConfig {
    fn normalized(mut self) -> Self {
        if self.heartbeat_interval.is_zero() {
            self.heartbeat_interval = MIN_HEARTBEAT_INTERVAL;
        }
        if self.poll_interval.is_zero() {
            self.poll_interval = MIN_POLL_INTERVAL;
        }
        self
    }
}

/// Factory trait for constructing a per-run `AgentLoopDriverHost`.
///
/// The host is created once per claimed run and provides the driver with access
/// to model, transcript, checkpoint, input, capabilities, and progress services.
#[async_trait]
pub trait HostFactory: Send + Sync {
    /// Construct a host for the given claimed run.
    ///
    /// The returned host must be valid for the entire duration of the driver
    /// invocation. Errors here result in `RecoveryRequired` for the run.
    async fn create_host(
        &self,
        claimed: &ClaimedTurnRun,
    ) -> Result<
        Box<dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync>,
        HostFactoryError,
    >;
}

/// Error returned when host construction fails.
///
/// TODO(reborn): split retriable/permanent host errors if host-side failure
/// handling needs different recovery policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFactoryError {
    pub reason: String,
}

impl HostFactoryError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for HostFactoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "host factory error: {}", self.reason)
    }
}

impl std::error::Error for HostFactoryError {}

/// Wake signal receiver for the turn-runner worker.
///
/// The worker uses wake-driven execution with fallback polling. Wake delivery
/// is best-effort: safe to duplicate or miss.
#[derive(Debug, Clone)]
pub struct TurnRunnerWakeReceiver {
    notify: Arc<Notify>,
}

impl TurnRunnerWakeReceiver {
    pub fn new() -> (TurnRunnerWakeSender, Self) {
        let notify = Arc::new(Notify::new());
        (
            TurnRunnerWakeSender {
                notify: Arc::clone(&notify),
            },
            Self { notify },
        )
    }

    /// Wait for a wake signal or timeout.
    async fn wait_or_timeout(&self, timeout: Duration) {
        tokio::select! {
            () = self.notify.notified() => {}
            () = tokio::time::sleep(timeout) => {}
        }
    }
}

impl Default for TurnRunnerWakeReceiver {
    fn default() -> Self {
        Self::new().1
    }
}

/// Sender half for wake signals.
///
/// This can be integrated with `TurnRunWakeNotifier` to forward queued-run
/// wakes into the worker.
#[derive(Debug, Clone)]
pub struct TurnRunnerWakeSender {
    notify: Arc<Notify>,
}

impl TurnRunnerWakeSender {
    /// Signal the worker that there may be new work available.
    pub fn wake(&self) {
        self.notify.notify_one();
    }
}

/// The concrete Reborn turn-runner worker.
///
/// Claims one run at a time, heartbeats the lease, invokes the matched driver,
/// and applies the returned `LoopExit` through the trusted transition port.
pub struct TurnRunnerWorker {
    runner_id: TurnRunnerId,
    config: TurnRunnerWorkerConfig,
    transition_port: Arc<dyn TurnRunTransitionPort>,
    driver_registry: Arc<DriverRegistry>,
    host_factory: Arc<dyn HostFactory>,
    wake_receiver: TurnRunnerWakeReceiver,
    loop_exit_applier: Arc<LoopExitApplier>,
}

impl TurnRunnerWorker {
    pub fn new(
        config: TurnRunnerWorkerConfig,
        transition_port: Arc<dyn TurnRunTransitionPort>,
        driver_registry: Arc<DriverRegistry>,
        host_factory: Arc<dyn HostFactory>,
        wake_receiver: TurnRunnerWakeReceiver,
        loop_exit_applier: Arc<LoopExitApplier>,
    ) -> Self {
        let runner_id = TurnRunnerId::new();
        let config = config.normalized();
        debug!(runner_id = ?runner_id, "turn runner worker created");
        Self {
            runner_id,
            config,
            transition_port,
            driver_registry,
            host_factory,
            wake_receiver,
            loop_exit_applier,
        }
    }

    /// Returns the stable runner identity for this worker instance.
    pub fn runner_id(&self) -> TurnRunnerId {
        self.runner_id
    }

    /// Run the worker claim loop until the cancellation token fires.
    ///
    /// This is the main entry point. It loops:
    /// 1. Wait for a wake signal or fallback poll tick
    /// 2. Claim the next available run
    /// 3. If none claimed, continue
    /// 4. Run the claimed run to `LoopExit` / application
    /// 5. Repeat
    ///
    /// Cancellation stops future claim attempts and preempts an in-flight
    /// driver invocation by recording `RecoveryRequired` for the claimed run.
    pub async fn run(&self, cancel: CancellationToken) {
        debug!(runner_id = ?self.runner_id, "turn runner worker started");

        loop {
            tokio::select! {
                () = cancel.cancelled() => {
                    debug!(runner_id = ?self.runner_id, "turn runner worker shutting down");
                    break;
                }
                () = self.wake_receiver.wait_or_timeout(self.config.poll_interval) => {}
            }

            if cancel.is_cancelled() {
                break;
            }

            while !cancel.is_cancelled() {
                match self.try_claim_and_run(cancel.clone()).await {
                    Ok(true) => continue,
                    Ok(false) => break,
                    Err(err) => {
                        warn!(
                            runner_id = ?self.runner_id,
                            error = %err,
                            "claim-and-run cycle failed"
                        );
                        break;
                    }
                }
            }
        }

        debug!(runner_id = ?self.runner_id, "turn runner worker stopped");
    }

    /// Attempt one claim-and-run cycle.
    ///
    /// Returns `Ok(true)` when a run was claimed and executed, and `Ok(false)`
    /// when the queue is empty. The main loop uses this to drain available work
    /// after each wake/poll tick without waiting for another interval.
    async fn try_claim_and_run(&self, cancel: CancellationToken) -> Result<bool, TurnRunnerError> {
        let lease_token = TurnLeaseToken::new();
        let request = ClaimRunRequest {
            runner_id: self.runner_id,
            lease_token,
            scope_filter: self.config.scope_filter.clone(),
        };

        let claimed = self
            .transition_port
            .claim_next_run(request)
            .await
            .map_err(TurnRunnerError::ClaimFailed)?;

        let Some(claimed) = claimed else {
            debug!(runner_id = ?self.runner_id, "no runs available to claim");
            return Ok(false);
        };

        let run_id = claimed.state.run_id;
        let status = claimed.state.status;

        debug!(
            runner_id = ?self.runner_id,
            run_id = ?run_id,
            status = ?status,
            "claimed turn run"
        );

        self.execute_claimed_run(claimed, cancel).await;
        Ok(true)
    }

    /// Execute a claimed run: heartbeat, invoke driver, apply exit.
    async fn execute_claimed_run(&self, claimed: ClaimedTurnRun, cancel: CancellationToken) {
        let run_id = claimed.state.run_id;
        let runner_id = claimed.runner_id;
        let lease_token = claimed.lease_token;
        let scope = claimed.state.scope.clone();
        let profile = claimed.resolved_run_profile.clone();

        let heartbeat_cancel = CancellationToken::new();
        let heartbeat = heartbeat_loop(
            Arc::clone(&self.transition_port),
            run_id,
            runner_id,
            lease_token,
            self.config.heartbeat_interval,
            heartbeat_cancel.clone(),
        );
        let driver = self.invoke_driver(&claimed);
        tokio::pin!(heartbeat);
        tokio::pin!(driver);

        // Resolve driver while heartbeating in the same task. If this future is
        // dropped, the heartbeat future is dropped with it, so no detached task
        // can outlive the claimed run.
        let exit_result = tokio::select! {
            result = &mut driver => result,
            heartbeat_result = &mut heartbeat => {
                match heartbeat_result {
                    Ok(()) => {
                        warn!(
                            runner_id = ?runner_id,
                            run_id = ?run_id,
                            "heartbeat loop stopped before driver returned"
                        );
                        Err(DriverInvocationError::HeartbeatStopped)
                    }
                    Err(err) => Err(DriverInvocationError::HeartbeatFailed(err)),
                }
            }
            () = cancel.cancelled() => Err(DriverInvocationError::WorkerCancelled),
        };
        heartbeat_cancel.cancel();

        // Apply the exit or record recovery
        match exit_result {
            Ok(exit) => {
                self.apply_exit(&scope, run_id, runner_id, lease_token, exit, &profile)
                    .await;
            }
            Err(err) => {
                warn!(
                    runner_id = ?runner_id,
                    run_id = ?run_id,
                    error = %err,
                    "driver invocation failed, recording recovery required"
                );
                self.record_recovery(run_id, runner_id, lease_token, &err)
                    .await;
            }
        }
    }

    /// Resolve driver from registry and invoke it.
    async fn invoke_driver(
        &self,
        claimed: &ClaimedTurnRun,
    ) -> Result<LoopExit, DriverInvocationError> {
        let descriptor = &claimed.resolved_run_profile.loop_driver;
        let registry_key =
            LoopDriverRegistryKey::from_descriptor(descriptor).map_err(|reason| {
                DriverInvocationError::DriverNotFound {
                    reason: format!("invalid descriptor: {reason}"),
                }
            })?;

        let registered = self.driver_registry.get(&registry_key).ok_or_else(|| {
            DriverInvocationError::DriverNotFound {
                reason: format!("no registered driver for {registry_key}"),
            }
        })?;

        let driver = registered.driver();

        // Create host for this run
        let host = self
            .host_factory
            .create_host(claimed)
            .await
            .map_err(|err| DriverInvocationError::HostCreationFailed { reason: err.reason })?;

        let status = claimed.state.status;
        let turn_id = claimed.state.turn_id;
        let run_id = claimed.state.run_id;

        match (status, claimed.state.checkpoint_id) {
            (TurnStatus::Queued, _) => {
                let request = AgentLoopDriverRunRequest {
                    turn_id,
                    run_id,
                    resolved_run_profile: claimed.resolved_run_profile.clone(),
                };
                driver
                    .run(request, host.as_ref())
                    .await
                    .map_err(DriverInvocationError::DriverError)
            }
            // Resumed runs have a checkpoint
            (_, Some(checkpoint_id)) => {
                let request = AgentLoopDriverResumeRequest {
                    turn_id,
                    run_id,
                    checkpoint_id,
                    resolved_run_profile: claimed.resolved_run_profile.clone(),
                };
                driver
                    .resume(request, host.as_ref())
                    .await
                    .map_err(DriverInvocationError::DriverError)
            }
            // Fallback: treat as new run
            _ => {
                let request = AgentLoopDriverRunRequest {
                    turn_id,
                    run_id,
                    resolved_run_profile: claimed.resolved_run_profile.clone(),
                };
                driver
                    .run(request, host.as_ref())
                    .await
                    .map_err(DriverInvocationError::DriverError)
            }
        }
    }

    /// Apply a `LoopExit` through the trusted `LoopExitApplier`.
    ///
    /// The applier derives evidence from durable stores, computes
    /// `LoopExitValidationPolicy`, and delegates to the existing
    /// `LoopExit::validate()` + `apply_validated_loop_exit()` path.
    async fn apply_exit(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        runner_id: TurnRunnerId,
        lease_token: TurnLeaseToken,
        exit: LoopExit,
        profile: &ResolvedRunProfile,
    ) {
        match self
            .loop_exit_applier
            .apply(scope, run_id, runner_id, lease_token, exit, profile)
            .await
        {
            Ok(state) => {
                debug!(
                    runner_id = ?runner_id,
                    run_id = ?run_id,
                    status = ?state.status,
                    "loop exit applied successfully"
                );
            }
            Err(err) => {
                error!(
                    runner_id = ?runner_id,
                    run_id = ?run_id,
                    error = %err,
                    "failed to apply loop exit"
                );
                // If exit application fails, try recording recovery
                let Some(failure) = sanitized_failure("exit_application_failed") else {
                    return;
                };
                let recovery_request = RecordRecoveryRequiredRequest {
                    run_id,
                    runner_id,
                    lease_token,
                    failure,
                };
                if let Err(recovery_err) = self
                    .transition_port
                    .record_recovery_required(recovery_request)
                    .await
                {
                    error!(
                        runner_id = ?runner_id,
                        run_id = ?run_id,
                        error = %recovery_err,
                        "failed to record recovery after exit application failure"
                    );
                }
            }
        }
    }

    /// Record recovery required for a failed driver invocation.
    async fn record_recovery(
        &self,
        run_id: TurnRunId,
        runner_id: TurnRunnerId,
        lease_token: TurnLeaseToken,
        error: &DriverInvocationError,
    ) {
        let category = match error {
            DriverInvocationError::DriverNotFound { .. } => "driver_not_found",
            DriverInvocationError::HostCreationFailed { .. } => "host_creation_failed",
            DriverInvocationError::DriverError(AgentLoopDriverError::InvalidRequest { .. }) => {
                "driver_invalid_request"
            }
            DriverInvocationError::DriverError(AgentLoopDriverError::Unavailable { .. }) => {
                "driver_unavailable"
            }
            DriverInvocationError::DriverError(AgentLoopDriverError::Failed { .. }) => {
                "driver_failed"
            }
            DriverInvocationError::HeartbeatStopped => "heartbeat_stopped",
            DriverInvocationError::HeartbeatFailed(_) => "heartbeat_failed",
            DriverInvocationError::WorkerCancelled => "worker_cancelled",
        };

        let Some(failure) = sanitized_failure(category) else {
            return;
        };
        let request = RecordRecoveryRequiredRequest {
            run_id,
            runner_id,
            lease_token,
            failure,
        };

        if let Err(err) = self.transition_port.record_recovery_required(request).await {
            error!(
                runner_id = ?runner_id,
                run_id = ?run_id,
                error = %err,
                "failed to record recovery required"
            );
        }
    }
}

/// Heartbeat loop that runs beside a driver invocation for the duration of a run.
async fn heartbeat_loop(
    port: Arc<dyn TurnRunTransitionPort>,
    run_id: TurnRunId,
    runner_id: TurnRunnerId,
    lease_token: TurnLeaseToken,
    interval: Duration,
    cancel: CancellationToken,
) -> Result<(), TurnError> {
    let mut tick = tokio::time::interval(interval);
    // Skip the first immediate tick
    tick.tick().await;

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                debug!(
                    runner_id = ?runner_id,
                    run_id = ?run_id,
                    "heartbeat loop stopped"
                );
                return Ok(());
            }
            _ = tick.tick() => {
                let request = HeartbeatRequest {
                    run_id,
                    runner_id,
                    lease_token,
                };
                match port.heartbeat(request).await {
                    Ok(_cursor) => {
                        debug!(
                            runner_id = ?runner_id,
                            run_id = ?run_id,
                            "heartbeat sent"
                        );
                    }
                    Err(err) => {
                        warn!(
                            runner_id = ?runner_id,
                            run_id = ?run_id,
                            error = %err,
                            "heartbeat failed"
                        );
                        return Err(err);
                    }
                }
            }
        }
    }
}

/// Internal error type for a single claim-and-run cycle.
#[derive(Debug)]
enum TurnRunnerError {
    ClaimFailed(TurnError),
}

impl std::fmt::Display for TurnRunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClaimFailed(err) => write!(f, "claim failed: {err}"),
        }
    }
}

/// Error during driver invocation (before `LoopExit` is returned).
#[derive(Debug)]
enum DriverInvocationError {
    DriverNotFound { reason: String },
    HostCreationFailed { reason: String },
    DriverError(AgentLoopDriverError),
    HeartbeatStopped,
    HeartbeatFailed(TurnError),
    WorkerCancelled,
}

impl std::fmt::Display for DriverInvocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DriverNotFound { reason } => write!(f, "driver not found: {reason}"),
            Self::HostCreationFailed { reason } => write!(f, "host creation failed: {reason}"),
            Self::DriverError(err) => write!(f, "driver error: {err}"),
            Self::HeartbeatStopped => write!(f, "heartbeat stopped before driver returned"),
            Self::HeartbeatFailed(err) => {
                write!(f, "heartbeat failed before driver returned: {err}")
            }
            Self::WorkerCancelled => write!(f, "worker cancelled before driver returned"),
        }
    }
}

#[cfg(test)]
mod tests;
