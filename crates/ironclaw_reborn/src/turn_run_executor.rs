//! Concrete `TurnRunExecutor` for the Reborn planned agent loop.
//!
//! Adapts `RebornLoopDriverHostFactory` + `DriverRegistry` + `LoopExitApplier`
//! to the `TurnRunExecutor` trait consumed by `TurnRunScheduler`.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_runtime::{TurnRunExecutor, TurnRunExecutorError};
use ironclaw_turns::{
    AgentLoopDriverError, AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, LoopExit,
    TurnStatus,
    run_profile::AgentLoopDriverHost,
    runner::{
        ClaimedTurnRun, RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest,
        TurnRunTransitionPort,
    },
};
use tracing::{debug, error};

use crate::{
    driver_registry::{DriverRegistry, LoopDriverRegistryKey},
    loop_exit_applier::LoopExitApplier,
    turn_runner::{HostFactory, sanitized_driver_failure, sanitized_failure},
};

/// Error produced during driver invocation (before `LoopExit` is returned).
///
/// Structurally mirrors the `DriverInvocationError` in `turn_runner.rs` but
/// stripped of the heartbeat/cancel variants that are now owned by the scheduler.
enum DriverInvocationError {
    DriverNotFound { reason: String },
    HostCreationFailed { reason: String },
    RouteSnapshotPersistenceFailed(ironclaw_turns::TurnError),
    DriverError(AgentLoopDriverError),
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
        }
    }
}

/// Concrete `TurnRunExecutor` for the Reborn planned agent loop.
pub struct RebornTurnRunExecutor {
    loop_exit_applier: Arc<LoopExitApplier>,
    driver_registry: Arc<DriverRegistry>,
    host_factory: Arc<dyn HostFactory>,
}

impl RebornTurnRunExecutor {
    pub fn new(
        loop_exit_applier: Arc<LoopExitApplier>,
        driver_registry: Arc<DriverRegistry>,
        host_factory: Arc<dyn HostFactory>,
    ) -> Self {
        Self {
            loop_exit_applier,
            driver_registry,
            host_factory,
        }
    }
}

#[async_trait]
impl TurnRunExecutor for RebornTurnRunExecutor {
    async fn execute_claimed_run(
        &self,
        claimed: ClaimedTurnRun,
        transitions: Arc<dyn TurnRunTransitionPort>,
    ) -> Result<(), TurnRunExecutorError> {
        match self.invoke_driver(&claimed, &transitions).await {
            Ok(exit) => {
                self.apply_exit(&claimed, exit, &transitions).await;
                Ok(())
            }
            Err(err) => {
                let sanitized = match &err {
                    DriverInvocationError::DriverError(AgentLoopDriverError::Failed {
                        reason_kind,
                    }) => sanitized_driver_failure(reason_kind),
                    DriverInvocationError::DriverNotFound { .. } => {
                        sanitized_failure("driver_not_found")
                    }
                    DriverInvocationError::HostCreationFailed { .. } => {
                        sanitized_failure("host_creation_failed")
                    }
                    DriverInvocationError::RouteSnapshotPersistenceFailed(_) => {
                        sanitized_failure("route_snapshot_persistence_failed")
                    }
                    DriverInvocationError::DriverError(AgentLoopDriverError::InvalidRequest {
                        ..
                    }) => sanitized_failure("driver_invalid_request"),
                    DriverInvocationError::DriverError(AgentLoopDriverError::Unavailable {
                        ..
                    }) => sanitized_failure("driver_unavailable"),
                };
                match sanitized {
                    Some(f) => Err(TurnRunExecutorError::new(f.category()).unwrap_or_else(|_| {
                        TurnRunExecutorError::new("unknown_failure")
                            .expect("static category is valid")
                    })),
                    None => Ok(()),
                }
            }
        }
    }
}

impl RebornTurnRunExecutor {
    async fn invoke_driver(
        &self,
        claimed: &ClaimedTurnRun,
        transitions: &Arc<dyn TurnRunTransitionPort>,
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
            run_id = %claimed.state.run_id,
            resolved_run_profile_id = claimed.resolved_run_profile.profile_id.as_str(),
            loop_driver_id = descriptor.id.as_str(),
            loop_driver_version = descriptor.version.as_u64(),
            "reborn executor resolved loop driver"
        );

        let host = self
            .host_factory
            .create_host(claimed)
            .await
            .map_err(|err| DriverInvocationError::HostCreationFailed { reason: err.reason })?;
        self.persist_model_route_snapshot(claimed, host.as_ref(), transitions)
            .await?;

        let turn_id = claimed.state.turn_id;
        let run_id = claimed.state.run_id;

        match (claimed.state.status, claimed.state.checkpoint_id) {
            // Requeued blocked runs keep their checkpoint while returning to
            // `Queued`; checkpoint identity is the resume signal.
            (_, Some(checkpoint_id)) => driver
                .resume(
                    AgentLoopDriverResumeRequest {
                        turn_id,
                        run_id,
                        checkpoint_id,
                        resolved_run_profile: claimed.resolved_run_profile.clone(),
                        resume_disposition: claimed.state.resume_disposition.clone(),
                    },
                    host.as_ref(),
                )
                .await
                .map_err(DriverInvocationError::DriverError),
            (TurnStatus::Queued, _) => driver
                .run(
                    AgentLoopDriverRunRequest {
                        turn_id,
                        run_id,
                        resolved_run_profile: claimed.resolved_run_profile.clone(),
                    },
                    host.as_ref(),
                )
                .await
                .map_err(DriverInvocationError::DriverError),
            // Fallback: treat as new run.
            _ => driver
                .run(
                    AgentLoopDriverRunRequest {
                        turn_id,
                        run_id,
                        resolved_run_profile: claimed.resolved_run_profile.clone(),
                    },
                    host.as_ref(),
                )
                .await
                .map_err(DriverInvocationError::DriverError),
        }
    }

    async fn persist_model_route_snapshot(
        &self,
        claimed: &ClaimedTurnRun,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        transitions: &Arc<dyn TurnRunTransitionPort>,
    ) -> Result<(), DriverInvocationError> {
        let Some(snapshot) = host.run_context().resolved_model_route.clone() else {
            return Ok(());
        };
        if claimed.state.resolved_model_route.as_ref() == Some(&snapshot) {
            return Ok(());
        }
        transitions
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

    async fn apply_exit(
        &self,
        claimed: &ClaimedTurnRun,
        exit: LoopExit,
        transitions: &Arc<dyn TurnRunTransitionPort>,
    ) {
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
                let Some(failure) = sanitized_failure("exit_application_failed") else {
                    return;
                };
                if let Err(record_err) = transitions
                    .record_runner_failure(RecordRunnerFailureRequest {
                        run_id,
                        runner_id,
                        lease_token,
                        failure,
                    })
                    .await
                {
                    debug!(
                        runner_id = ?runner_id,
                        run_id = ?run_id,
                        error = %record_err,
                        "failed to record terminal failure after exit application failure"
                    );
                }
            }
        }
    }
}
