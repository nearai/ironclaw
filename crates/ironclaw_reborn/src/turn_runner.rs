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
//! DTOs, state-machine invariants, and the trusted `LoopExitApplier`.
//!
//! This module owns the concrete worker loop, driver registry lookup, host
//! factory, readiness/config, and worker lifecycle.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::FutureExt;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use ironclaw_turns::{
    AgentLoopDriverError, AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, LoopExit,
    SanitizedFailure, TurnError, TurnLeaseToken, TurnRunId, TurnRunWake, TurnRunWakeNotifier,
    TurnRunWakeNotifyError, TurnRunnerId, TurnScope, TurnStatus,
    runner::{
        ClaimRunRequest, ClaimedTurnRun, HeartbeatRequest, RecordModelRouteSnapshotRequest,
        RecordRunnerFailureRequest, RecoverExpiredLeasesRequest, RelinquishRunRequest,
        TurnRunTransitionPort,
    },
};

use crate::{
    driver_registry::{DriverRegistry, LoopDriverRegistryKey},
    failure_categories::{
        MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY, MODEL_CREDITS_EXHAUSTED_CATEGORY,
    },
    loop_exit_applier::LoopExitApplier,
};

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
            match SanitizedFailure::new("unknown_failure") {
                Ok(fallback) => Some(fallback),
                Err(fallback_error) => {
                    error!(%fallback_error, "fallback recovery failure category invalid");
                    None
                }
            }
        }
    }
}

fn sanitized_driver_failure(reason_kind: &str) -> Option<SanitizedFailure> {
    if matches!(
        reason_kind,
        MODEL_CREDITS_EXHAUSTED_CATEGORY | MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY
    ) {
        return match SanitizedFailure::new(reason_kind.to_string()) {
            Ok(failure) => Some(failure),
            Err(error) => {
                debug!(
                    reason_kind,
                    %error,
                    "model failure category failed validation; using generic driver failure"
                );
                sanitized_failure("driver_failed")
            }
        };
    }
    sanitized_failure("driver_failed")
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

/// Factory trait for constructing a per-run `AgentLoopDriverHost`.
///
/// The host is created once per claimed run and provides the driver with access
/// to model, transcript, checkpoint, input, capabilities, and progress services.
#[async_trait]
pub trait HostFactory: Send + Sync {
    /// Construct a host for the given claimed run.
    ///
    /// The returned host must be valid for the entire duration of the driver
    /// invocation. Errors here result in a terminal failed/cancelled transition.
    async fn create_host(
        &self,
        claimed: &ClaimedTurnRun,
    ) -> Result<
        Box<dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync>,
        HostFactoryError,
    >;
}

/// Error returned when host construction fails.
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

impl TurnRunWakeNotifier for TurnRunnerWakeSender {
    fn notify_queued_run(&self, _wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        self.wake();
        Ok(())
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
    loop_exit_applier: Arc<LoopExitApplier>,
    driver_registry: Arc<DriverRegistry>,
    host_factory: Arc<dyn HostFactory>,
    wake_receiver: TurnRunnerWakeReceiver,
}

impl TurnRunnerWorker {
    pub fn new(
        config: TurnRunnerWorkerConfig,
        transition_port: Arc<dyn TurnRunTransitionPort>,
        loop_exit_applier: Arc<LoopExitApplier>,
        driver_registry: Arc<DriverRegistry>,
        host_factory: Arc<dyn HostFactory>,
        wake_receiver: TurnRunnerWakeReceiver,
    ) -> Self {
        let runner_id = TurnRunnerId::new();
        debug!(runner_id = ?runner_id, "turn runner worker created");
        Self {
            runner_id,
            config,
            transition_port,
            loop_exit_applier,
            driver_registry,
            host_factory,
            wake_receiver,
        }
    }

    /// Returns the stable runner identity for this worker instance.
    pub fn runner_id(&self) -> TurnRunnerId {
        self.runner_id
    }

    /// Run the worker claim loop until the cancellation token fires.
    ///
    /// This is the main entry point. It loops:
    /// 1. Sweep expired leases so crashed workers do not strand runs
    /// 2. Claim and run queued work until the queue is empty
    /// 3. Wait for a wake signal or fallback poll tick when no work remains
    /// 4. Repeat until cancelled
    pub async fn run(&self, cancel: CancellationToken) {
        debug!(runner_id = ?self.runner_id, "turn runner worker started");

        loop {
            if cancel.is_cancelled() {
                debug!(runner_id = ?self.runner_id, "turn runner worker shutting down");
                break;
            }

            if let Err(err) = self.recover_expired_leases().await {
                warn!(
                    runner_id = ?self.runner_id,
                    error = %err,
                    "expired lease recovery failed"
                );
            }

            while !cancel.is_cancelled() {
                match self.try_claim_and_run(&cancel).await {
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

            if cancel.is_cancelled() {
                continue;
            }

            tokio::select! {
                () = cancel.cancelled() => {
                    debug!(runner_id = ?self.runner_id, "turn runner worker shutting down");
                    break;
                }
                () = self.wake_receiver.wait_or_timeout(self.config.poll_interval) => {}
            }
        }

        debug!(runner_id = ?self.runner_id, "turn runner worker stopped");
    }

    async fn recover_expired_leases(&self) -> Result<(), TurnError> {
        let response = self
            .transition_port
            .recover_expired_leases(RecoverExpiredLeasesRequest {
                now: chrono::Utc::now(),
                scope_filter: self.config.scope_filter.clone(),
            })
            .await?;
        if !response.recovered.is_empty() {
            debug!(
                runner_id = ?self.runner_id,
                recovered = response.recovered.len(),
                "expired turn-run leases recovered"
            );
        }
        Ok(())
    }

    /// Attempt one claim-and-run cycle.
    async fn try_claim_and_run(&self, cancel: &CancellationToken) -> Result<bool, TurnRunnerError> {
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
            resolved_run_profile_id = claimed.resolved_run_profile.profile_id.as_str(),
            resolved_run_profile_version = claimed.resolved_run_profile.profile_version.as_u64(),
            loop_driver_id = claimed.resolved_run_profile.loop_driver.id.as_str(),
            loop_driver_version = claimed.resolved_run_profile.loop_driver.version.as_u64(),
            "claimed turn run"
        );

        self.execute_claimed_run(claimed, cancel).await;
        Ok(true)
    }

    /// Execute a claimed run: heartbeat, invoke driver, apply exit.
    async fn execute_claimed_run(&self, claimed: ClaimedTurnRun, cancel: &CancellationToken) {
        let run_id = claimed.state.run_id;
        let runner_id = claimed.runner_id;
        let lease_token = claimed.lease_token;

        let exit_result = {
            let heartbeat_cancel = CancellationToken::new();
            let heartbeat = heartbeat_loop(
                Arc::clone(&self.transition_port),
                run_id,
                runner_id,
                lease_token,
                self.config.heartbeat_interval,
                heartbeat_cancel.clone(),
            );
            tokio::pin!(heartbeat);

            // Resolve driver from registry and invoke it. Driver panics indicate
            // unknown partial state, so fail the active run with a sanitized category.
            let driver = AssertUnwindSafe(self.invoke_driver(&claimed)).catch_unwind();
            tokio::pin!(driver);

            let exit_result = tokio::select! {
                result = &mut driver => match result {
                    Ok(result) => result,
                    Err(_) => Err(DriverInvocationError::DriverPanic),
                },
                heartbeat_result = &mut heartbeat => match heartbeat_result {
                    Ok(()) => Err(DriverInvocationError::HeartbeatStopped),
                    Err(err) => Err(DriverInvocationError::HeartbeatFailed(err)),
                },
                () = cancel.cancelled() => Err(DriverInvocationError::WorkerCancelled),
            };

            heartbeat_cancel.cancel();
            exit_result
        };
        // Keep heartbeat scoped above so a losing heartbeat future is dropped
        // before exit application re-enters the same turn-state store.

        // Apply the exit or fail/cancel the claimed run through the compatibility transition.
        match exit_result {
            Ok(exit) => {
                self.apply_exit(&claimed, exit).await;
            }
            Err(err) => {
                warn!(
                    runner_id = ?runner_id,
                    run_id = ?run_id,
                    error = %err,
                    "driver invocation failed, recording terminal failure"
                );
                self.record_terminal_failure(run_id, runner_id, lease_token, &err)
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
        debug!(
            runner_id = ?self.runner_id,
            run_id = %claimed.state.run_id,
            resolved_run_profile_id = claimed.resolved_run_profile.profile_id.as_str(),
            loop_driver_id = descriptor.id.as_str(),
            loop_driver_version = descriptor.version.as_u64(),
            "reborn turn runner resolved loop driver"
        );

        // Create host for this run
        let host = self
            .host_factory
            .create_host(claimed)
            .await
            .map_err(|err| DriverInvocationError::HostCreationFailed { reason: err.reason })?;
        self.persist_model_route_snapshot(claimed, host.as_ref())
            .await?;

        let status = claimed.state.status;
        let turn_id = claimed.state.turn_id;
        let run_id = claimed.state.run_id;

        match (status, claimed.state.checkpoint_id) {
            // Requeued blocked runs keep their checkpoint while returning to
            // `Queued`; checkpoint identity is the resume signal.
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

    /// Persist the host-attached model route before driver invocation can emit side effects.
    async fn persist_model_route_snapshot(
        &self,
        claimed: &ClaimedTurnRun,
        host: &(dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync),
    ) -> Result<(), DriverInvocationError> {
        let Some(snapshot) = host.run_context().resolved_model_route.clone() else {
            return Ok(());
        };
        if claimed.state.resolved_model_route.as_ref() == Some(&snapshot) {
            return Ok(());
        }
        self.transition_port
            .record_model_route_snapshot(RecordModelRouteSnapshotRequest {
                run_id: claimed.state.run_id,
                runner_id: claimed.runner_id,
                lease_token: claimed.lease_token,
                snapshot,
            })
            .await
            .map(|_| ())
            .map_err(DriverInvocationError::RouteSnapshotPersistenceFailed)
    }

    /// Apply a `LoopExit` through the trusted applier.
    async fn apply_exit(&self, claimed: &ClaimedTurnRun, exit: LoopExit) {
        let run_id = claimed.state.run_id;
        let runner_id = claimed.runner_id;
        let lease_token = claimed.lease_token;

        match self.loop_exit_applier.apply(claimed, exit).await {
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
                // If exit application fails, try recording terminal failure through
                // the compatibility transition.
                let Some(failure) = sanitized_failure("exit_application_failed") else {
                    return;
                };
                let failure_request = RecordRunnerFailureRequest {
                    run_id,
                    runner_id,
                    lease_token,
                    failure,
                };
                if let Err(record_err) = self
                    .transition_port
                    .record_runner_failure(failure_request)
                    .await
                {
                    log_runner_failure_record_error(
                        runner_id,
                        run_id,
                        &record_err,
                        "failed to record terminal failure after exit application failure",
                    );
                }
            }
        }
    }

    /// Handle a failed driver invocation.
    ///
    /// Transient worker events (`WorkerCancelled`, `HeartbeatStopped`) relinquish the
    /// lease so another worker can retry.  All other errors record a terminal failure.
    async fn record_terminal_failure(
        &self,
        run_id: TurnRunId,
        runner_id: TurnRunnerId,
        lease_token: TurnLeaseToken,
        error: &DriverInvocationError,
    ) {
        // Errors that warrant relinquish (re-queue) rather than terminal failure.
        let relinquish = matches!(
            error,
            DriverInvocationError::WorkerCancelled | DriverInvocationError::HeartbeatStopped
        );

        if relinquish {
            let request = RelinquishRunRequest {
                run_id,
                runner_id,
                lease_token,
            };
            if let Err(err) = self.transition_port.relinquish_run(request).await {
                log_runner_failure_record_error(
                    runner_id,
                    run_id,
                    &err,
                    "failed to relinquish run",
                );
            }
            return;
        }

        let failure = match error {
            DriverInvocationError::DriverError(AgentLoopDriverError::Failed { reason_kind }) => {
                sanitized_driver_failure(reason_kind)
            }
            other => {
                let category = match other {
                    DriverInvocationError::DriverNotFound { .. } => "driver_not_found",
                    DriverInvocationError::HostCreationFailed { .. } => "host_creation_failed",
                    DriverInvocationError::RouteSnapshotPersistenceFailed(_) => {
                        "route_snapshot_persistence_failed"
                    }
                    DriverInvocationError::DriverError(AgentLoopDriverError::InvalidRequest {
                        ..
                    }) => "driver_invalid_request",
                    DriverInvocationError::DriverError(AgentLoopDriverError::Unavailable {
                        ..
                    }) => "driver_unavailable",
                    // DriverError(Failed) is destructured in the outer arm and dispatched
                    // through sanitized_driver_failure before this branch is reached.
                    DriverInvocationError::DriverError(AgentLoopDriverError::Failed { .. }) => {
                        unreachable!("failed driver errors handled above")
                    }
                    DriverInvocationError::DriverPanic => "driver_panic",
                    DriverInvocationError::HeartbeatFailed(_) => "heartbeat_failed",
                    // WorkerCancelled and HeartbeatStopped handled by relinquish branch above.
                    DriverInvocationError::WorkerCancelled
                    | DriverInvocationError::HeartbeatStopped => {
                        unreachable!("relinquish branch handles these")
                    }
                };
                sanitized_failure(category)
            }
        };

        let Some(failure) = failure else {
            return;
        };
        let request = RecordRunnerFailureRequest {
            run_id,
            runner_id,
            lease_token,
            failure,
        };

        if let Err(err) = self.transition_port.record_runner_failure(request).await {
            log_runner_failure_record_error(
                runner_id,
                run_id,
                &err,
                "failed to record terminal failure",
            );
        }
    }
}

fn log_runner_failure_record_error(
    runner_id: TurnRunnerId,
    run_id: TurnRunId,
    error: &TurnError,
    message: &'static str,
) {
    if runner_failure_rejection_is_expected(error) {
        debug!(
            runner_id = ?runner_id,
            run_id = ?run_id,
            error = %error,
            message
        );
    } else {
        error!(
            runner_id = ?runner_id,
            run_id = ?run_id,
            error = %error,
            message
        );
    }
}

fn runner_failure_rejection_is_expected(error: &TurnError) -> bool {
    matches!(error, TurnError::LeaseMismatch)
        || matches!(
            error,
            TurnError::InvalidTransition {
                from: TurnStatus::Completed
                    | TurnStatus::Cancelled
                    | TurnStatus::Failed
                    | TurnStatus::BlockedApproval
                    | TurnStatus::BlockedAuth
                    | TurnStatus::BlockedResource
                    | TurnStatus::RecoveryRequired,
                to: TurnStatus::Failed,
            }
        )
}

async fn heartbeat_loop(
    port: Arc<dyn TurnRunTransitionPort>,
    run_id: TurnRunId,
    runner_id: TurnRunnerId,
    lease_token: TurnLeaseToken,
    interval: Duration,
    cancel: CancellationToken,
) -> Result<(), TurnError> {
    let interval = if interval.is_zero() {
        Duration::from_millis(1)
    } else {
        interval
    };
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
    RouteSnapshotPersistenceFailed(TurnError),
    DriverError(AgentLoopDriverError),
    DriverPanic,
    HeartbeatFailed(TurnError),
    HeartbeatStopped,
    WorkerCancelled,
}

impl std::fmt::Display for DriverInvocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DriverNotFound { reason } => write!(f, "driver not found: {reason}"),
            Self::HostCreationFailed { reason } => write!(f, "host creation failed: {reason}"),
            Self::RouteSnapshotPersistenceFailed(err) => {
                write!(f, "route snapshot persistence failed: {err}")
            }
            Self::DriverError(err) => write!(f, "driver error: {err}"),
            Self::DriverPanic => write!(f, "driver panicked before returning loop exit"),
            Self::HeartbeatFailed(err) => write!(f, "heartbeat failed: {err}"),
            Self::HeartbeatStopped => write!(f, "heartbeat stopped before driver completed"),
            Self::WorkerCancelled => write!(f, "worker cancelled before driver completed"),
        }
    }
}

#[cfg(test)]
mod tests;
