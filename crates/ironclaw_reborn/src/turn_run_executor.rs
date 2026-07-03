//! Concrete `TurnRunExecutor` for the Reborn planned agent loop.
//!
//! Adapts `RebornLoopDriverHostFactory` + `DriverRegistry` + `LoopExitApplier`
//! to the `TurnRunExecutor` trait consumed by `TurnRunScheduler`.

use std::{
    sync::{Arc, OnceLock},
    time::Instant,
};

use async_trait::async_trait;
use ironclaw_host_runtime::{TurnRunExecutor, TurnRunExecutorError};
use ironclaw_observability::live_latency_started_at;
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

fn trace_executor_latency_ok(
    operation: &'static str,
    claimed: &ClaimedTurnRun,
    started_at: Option<Instant>,
) {
    ironclaw_observability::live_latency_trace_ok!(
        "reborn_turn_executor",
        operation,
        started_at,
        tenant_id = %claimed.state.scope.tenant_id,
        agent_id = claimed.state.scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        project_id = claimed.state.scope.project_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        thread_id = %claimed.state.scope.thread_id,
        owner_user_id = claimed.state.scope.explicit_owner_user_id().map(|id| id.as_str()).unwrap_or(""),
        run_id = %claimed.state.run_id,
        "reborn turn executor operation completed",
    );
}

fn trace_executor_latency_error<E: ?Sized>(
    operation: &'static str,
    claimed: &ClaimedTurnRun,
    started_at: Option<Instant>,
    _error: &E,
) {
    ironclaw_observability::live_latency_trace_error!(
        "reborn_turn_executor",
        operation,
        started_at,
        "executor_error",
        tenant_id = %claimed.state.scope.tenant_id,
        agent_id = claimed.state.scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        project_id = claimed.state.scope.project_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        thread_id = %claimed.state.scope.thread_id,
        owner_user_id = claimed.state.scope.explicit_owner_user_id().map(|id| id.as_str()).unwrap_or(""),
        run_id = %claimed.state.run_id,
        "reborn turn executor operation failed",
    );
}

/// A `TurnRunExecutorError` for the static category `"unknown_failure"`.
///
/// Built once on first access via `OnceLock`. Used as a guaranteed-valid
/// fallback so no production path ever calls `.expect()` or `.unwrap()`.
fn unknown_failure_error() -> &'static TurnRunExecutorError {
    static CELL: OnceLock<TurnRunExecutorError> = OnceLock::new();
    CELL.get_or_init(|| {
        // "unknown_failure" is lowercase ASCII with underscores and satisfies
        // every validation invariant. If this ever fails the binary is
        // fundamentally broken, so a panic here is acceptable at process start.
        TurnRunExecutorError::new("unknown_failure")
            .expect("'unknown_failure' is a valid static executor error category") // safety: compile-time-constant category (lowercase ASCII + underscore) always passes validation; runs once at process start
    })
}

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
        let started_at = live_latency_started_at();
        match self.invoke_driver(&claimed, &transitions).await {
            Ok(exit) => {
                let result = self.apply_exit(&claimed, exit, &transitions).await;
                match result {
                    Ok(()) => {
                        trace_executor_latency_ok("execute_claimed_run", &claimed, started_at);
                        Ok(())
                    }
                    Err(()) => {
                        let error = unknown_failure_error().clone();
                        trace_executor_latency_error(
                            "execute_claimed_run",
                            &claimed,
                            started_at,
                            &error,
                        );
                        Err(error)
                    }
                }
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
                // `sanitized` is always Some — sanitized_failure /
                // sanitized_driver_failure fall back to "unknown_failure" before
                // returning None. The unwrap_or_else path is a belt-and-suspenders
                // guard that is never reached in practice.
                let failure =
                    sanitized.unwrap_or_else(|| unknown_failure_error().failure().clone());
                let error = TurnRunExecutorError::new(failure.category())
                    .unwrap_or_else(|_| unknown_failure_error().clone());
                trace_executor_latency_error("execute_claimed_run", &claimed, started_at, &error);
                Err(error)
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

        let host_started_at = live_latency_started_at();
        let host = match self.host_factory.create_host(claimed).await {
            Ok(host) => {
                trace_executor_latency_ok("create_loop_host", claimed, host_started_at);
                host
            }
            Err(err) => {
                trace_executor_latency_error("create_loop_host", claimed, host_started_at, &err);
                // Use the error's full `Display` (`err.to_string()`) rather than a single
                // field, so whatever context the host factory embedded in its message
                // survives into `reason` (HostFactoryError is a flat message with no
                // `source()` chain of its own).
                return Err(DriverInvocationError::HostCreationFailed {
                    reason: err.to_string(),
                });
            }
        };
        let route_snapshot_started_at = live_latency_started_at();
        if let Err(error) = self
            .persist_model_route_snapshot(claimed, host.as_ref(), transitions)
            .await
        {
            trace_executor_latency_error(
                "persist_model_route_snapshot",
                claimed,
                route_snapshot_started_at,
                &error,
            );
            return Err(error);
        }
        trace_executor_latency_ok(
            "persist_model_route_snapshot",
            claimed,
            route_snapshot_started_at,
        );

        let turn_id = claimed.state.turn_id;
        let run_id = claimed.state.run_id;

        let driver_started_at = live_latency_started_at();
        let driver_operation = if claimed.state.checkpoint_id.is_some() {
            "driver_resume"
        } else {
            "driver_run"
        };
        let driver_result = match (claimed.state.status, claimed.state.checkpoint_id) {
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
        };
        match &driver_result {
            Ok(_) => trace_executor_latency_ok(driver_operation, claimed, driver_started_at),
            Err(error) => {
                trace_executor_latency_error(driver_operation, claimed, driver_started_at, error)
            }
        }
        driver_result
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

    /// Apply a `LoopExit` through the trusted applier.
    ///
    /// Returns:
    /// - `Ok(())` if the run reached a terminal state (either via successful
    ///   exit application or via a successful fallback `record_runner_failure`).
    /// - `Err(())` only when BOTH the exit applier AND the fallback
    ///   `record_runner_failure` fail — a double-failure that leaves the run in
    ///   an unknown state. The caller (`execute_claimed_run`) converts this to a
    ///   `TurnRunExecutorError` so the scheduler can record a terminal failure.
    async fn apply_exit(
        &self,
        claimed: &ClaimedTurnRun,
        exit: LoopExit,
        transitions: &Arc<dyn TurnRunTransitionPort>,
    ) -> Result<(), ()> {
        let started_at = live_latency_started_at();
        let run_id = claimed.state.run_id;
        let runner_id = claimed.runner_id;
        let lease_token = claimed.lease_token;

        match self.loop_exit_applier.apply(claimed, exit).await {
            Ok(state) => {
                trace_executor_latency_ok("apply_loop_exit", claimed, started_at);
                debug!(
                    runner_id = ?runner_id,
                    run_id = ?run_id,
                    status = ?state.status,
                    "loop exit applied successfully"
                );
                Ok(())
            }
            Err(err) => {
                trace_executor_latency_error("apply_loop_exit", claimed, started_at, &err);
                error!(
                    runner_id = ?runner_id,
                    run_id = ?run_id,
                    error = %err,
                    "failed to apply loop exit"
                );
                // Exit application failed: try recording a terminal failure
                // through the transitions port so the run is not stranded.
                // Falls back to "unknown_failure" before returning None so the
                // run always reaches a terminal state if the port cooperates.
                let failure = sanitized_failure("exit_application_failed")
                    .unwrap_or_else(|| unknown_failure_error().failure().clone());
                match transitions
                    .record_runner_failure(RecordRunnerFailureRequest {
                        run_id,
                        runner_id,
                        lease_token,
                        failure,
                    })
                    .await
                {
                    Ok(_) => Ok(()),
                    Err(record_err) => {
                        // Double-failure: both the applier and the fallback
                        // transition failed. Signal the caller so the scheduler
                        // can attempt its own terminal-failure recording.
                        error!(
                            runner_id = ?runner_id,
                            run_id = ?run_id,
                            error = %record_err,
                            "failed to record terminal failure after exit application failure"
                        );
                        Err(())
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_host_api::{TenantId, ThreadId};
    use ironclaw_host_runtime::TurnRunExecutor;
    use ironclaw_turns::{
        AcceptedMessageRef, AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError,
        AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, EventCursor, LoopCompleted,
        LoopCompletionKind, LoopExit, LoopExitId, LoopMessageRef, ReplyTargetBindingRef,
        RunProfileVersion, SourceBindingRef, TurnError, TurnId, TurnRunId, TurnRunState, TurnScope,
        TurnStatus,
        run_profile::{
            AgentLoopDriverHost, AgentLoopHostError, CheckpointSchemaId, LoopDriverId,
            LoopModelRouteSnapshot, LoopRunContext,
        },
        runner::{
            ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
            ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
            RecordModelRouteSnapshotRequest, RecoverExpiredLeasesRequest,
            RecoverExpiredLeasesResponse, RelinquishRunRequest, TurnRunTransitionPort,
        },
    };

    use crate::{
        driver_registry::{DriverKind, DriverRegistry, DriverRequirements},
        loop_exit_applier::{InMemoryLoopExitEvidencePort, LoopExitApplier},
        turn_runner::HostFactoryError,
    };

    use super::RebornTurnRunExecutor;

    // ── Minimal fakes ────────────────────────────────────────────────────────

    /// A `TurnRunTransitionPort` that records which methods were called.
    #[derive(Default)]
    struct RecordingTransitionPort {
        fail_run_calls: Mutex<Vec<FailRunRequest>>,
    }

    impl RecordingTransitionPort {
        fn fail_run_call_count(&self) -> usize {
            self.fail_run_calls.lock().unwrap().len()
        }
    }

    // Helper to build a minimal TurnRunState for a fake response.
    fn fake_run_state() -> TurnRunState {
        TurnRunState {
            scope: TurnScope::new(
                TenantId::new("fake-tenant").expect("valid"),
                None,
                None,
                ThreadId::new("fake-thread").expect("valid"),
            ),
            actor: None,
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Failed,
            accepted_message_ref: AcceptedMessageRef::new("msg:fake").expect("valid"),
            source_binding_ref: SourceBindingRef::new("src:fake").expect("valid"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:fake").expect("valid"),
            resolved_run_profile_id: ironclaw_turns::RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            received_at: chrono::Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: vec![],
            failure: None,
            event_cursor: EventCursor(0),
            product_context: None,
            resume_disposition: None,
        }
    }

    #[async_trait]
    impl TurnRunTransitionPort for RecordingTransitionPort {
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
            Ok(fake_run_state())
        }

        async fn block_run(&self, _request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn complete_run(
            &self,
            _request: CompleteRunRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn cancel_run(
            &self,
            _request: CancelRunCompletionRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
            self.fail_run_calls.lock().unwrap().push(request);
            Ok(fake_run_state())
        }

        async fn relinquish_run(
            &self,
            _request: RelinquishRunRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn apply_validated_loop_exit(
            &self,
            _request: ApplyValidatedLoopExitRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }
    }

    /// A `HostFactory` that always fails.
    struct FailingHostFactory;

    #[async_trait]
    impl crate::turn_runner::HostFactory for FailingHostFactory {
        async fn create_host(
            &self,
            _claimed: &ClaimedTurnRun,
        ) -> Result<
            Box<dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync>,
            HostFactoryError,
        > {
            Err(HostFactoryError::new("induced failure for test"))
        }
    }

    fn test_descriptor() -> AgentLoopDriverDescriptor {
        AgentLoopDriverDescriptor {
            id: LoopDriverId::new("test_loop").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(CheckpointSchemaId::new("test_checkpoint").expect("valid")),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        }
    }

    fn test_claimed_run() -> ClaimedTurnRun {
        use ironclaw_turns::run_profile::*;
        use ironclaw_turns::*;

        let desc = test_descriptor();
        let scope = TurnScope::new(
            TenantId::new("test-tenant").expect("valid"),
            None,
            None,
            ThreadId::new("test-thread").expect("valid"),
        );
        let profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("test_class").expect("valid"),
            profile_id: RunProfileId::default_profile(),
            profile_version: RunProfileVersion::new(1),
            loop_driver: desc.clone(),
            checkpoint_schema_id: CheckpointSchemaId::new("test_checkpoint").expect("valid"),
            checkpoint_schema_version: RunProfileVersion::new(1),
            model_profile_id: ModelProfileId::new("test_model").expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new("test_cap")
                .expect("valid"),
            context_profile_id: ContextProfileId::new("test_ctx").expect("valid"),
            steering_policy: SteeringPolicy {
                allow_steering: false,
                allow_interrupt: true,
                allow_driver_specific_nudges: false,
            },
            cancellation_policy: CancellationPolicy {
                allow_cancel: true,
                require_checkpoint_before_cancel: false,
            },
            checkpoint_policy: CheckpointPolicy {
                require_before_model: false,
                require_before_side_effect: false,
                require_before_block: true,
                max_checkpoint_bytes: 64 * 1024,
                require_final_checkpoint: false,
                allow_no_reply_completion: false,
            },
            resource_budget_policy: ResourceBudgetPolicy {
                tier: ResourceBudgetTier::new("test_tier").expect("valid"),
                max_model_calls: 32,
                max_capability_invocations: 64,
            },
            personal_context_policy: PersonalContextPolicy::Excluded,
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::new("interactive").expect("valid"),
            concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
            resolution_fingerprint: RunProfileFingerprint::new("test-fp-v1").expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        let state = TurnRunState {
            scope,
            actor: None,
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            accepted_message_ref: AcceptedMessageRef::new("msg:test").expect("valid"),
            source_binding_ref: SourceBindingRef::new("src:test").expect("valid"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:test").expect("valid"),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            received_at: chrono::Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: vec![],
            failure: None,
            event_cursor: EventCursor(0),
            product_context: None,
            resume_disposition: None,
        };
        ClaimedTurnRun {
            state,
            resolved_run_profile: profile,
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
        }
    }

    fn make_executor_empty_registry() -> RebornTurnRunExecutor {
        let transitions: Arc<dyn TurnRunTransitionPort> =
            Arc::new(RecordingTransitionPort::default());
        let evidence = Arc::new(InMemoryLoopExitEvidencePort::new());
        let loop_exit_applier = Arc::new(LoopExitApplier::new(transitions, evidence));
        let driver_registry = Arc::new(DriverRegistry::new()); // empty — no drivers registered
        let host_factory = Arc::new(FailingHostFactory);
        RebornTurnRunExecutor::new(loop_exit_applier, driver_registry, host_factory)
    }

    /// When the driver registry has no registered driver, `execute_claimed_run`
    /// must return `Err(TurnRunExecutorError)`. The caller (scheduler) owns
    /// terminal-failure recording; `record_runner_failure` / `fail_run` must NOT
    /// be called from within the executor.
    #[tokio::test]
    async fn driver_not_found_returns_err_without_calling_fail_run() {
        let executor = make_executor_empty_registry();
        let transitions = Arc::new(RecordingTransitionPort::default());

        let result = executor
            .execute_claimed_run(
                test_claimed_run(),
                transitions.clone() as Arc<dyn TurnRunTransitionPort>,
            )
            .await;

        assert!(
            result.is_err(),
            "expected Err(TurnRunExecutorError) for unknown driver, got Ok"
        );
        assert_eq!(
            transitions.fail_run_call_count(),
            0,
            "record_runner_failure / fail_run must NOT be called from execute_claimed_run; \
             the scheduler owns terminal-failure recording"
        );
    }

    /// The `unknown_failure_error()` accessor must return a valid
    /// `TurnRunExecutorError` on first and subsequent calls (OnceLock is
    /// idempotent — same pointer each time).
    #[test]
    fn unknown_failure_error_is_valid_and_idempotent() {
        let first = super::unknown_failure_error();
        let second = super::unknown_failure_error();
        assert_eq!(first.failure_category(), "unknown_failure");
        // Same pointer — OnceLock must not re-initialize.
        assert!(std::ptr::eq(first, second));
    }

    // ── FIX 2: host-creation failure + snapshot-persistence failure tests ─────

    /// A minimal completing driver whose descriptor matches `test_descriptor`.
    struct CompletingDriver {
        descriptor: AgentLoopDriverDescriptor,
    }

    impl CompletingDriver {
        fn new(descriptor: AgentLoopDriverDescriptor) -> Self {
            Self { descriptor }
        }
    }

    #[async_trait]
    impl AgentLoopDriver for CompletingDriver {
        fn descriptor(&self) -> AgentLoopDriverDescriptor {
            self.descriptor.clone()
        }

        async fn run(
            &self,
            _request: AgentLoopDriverRunRequest,
            _host: &(dyn AgentLoopDriverHost + Send + Sync),
        ) -> Result<LoopExit, AgentLoopDriverError> {
            Ok(LoopExit::Completed(LoopCompleted {
                completion_kind: LoopCompletionKind::FinalReply,
                reply_message_refs: vec![LoopMessageRef::new("msg:test").expect("valid")],
                result_refs: vec![],
                final_checkpoint_id: None,
                usage_summary_ref: None,
                exit_id: LoopExitId::new("exit:test").expect("valid"),
            }))
        }

        async fn resume(
            &self,
            _request: AgentLoopDriverResumeRequest,
            _host: &(dyn AgentLoopDriverHost + Send + Sync),
        ) -> Result<LoopExit, AgentLoopDriverError> {
            Ok(LoopExit::Completed(LoopCompleted {
                completion_kind: LoopCompletionKind::FinalReply,
                reply_message_refs: vec![LoopMessageRef::new("msg:test").expect("valid")],
                result_refs: vec![],
                final_checkpoint_id: None,
                usage_summary_ref: None,
                exit_id: LoopExitId::new("exit:test").expect("valid"),
            }))
        }
    }

    /// A `HostFactory` that succeeds and returns a stub host with a model route
    /// snapshot set, so `persist_model_route_snapshot` is triggered.
    struct SucceedingHostFactoryWithSnapshot;

    #[async_trait]
    impl crate::turn_runner::HostFactory for SucceedingHostFactoryWithSnapshot {
        async fn create_host(
            &self,
            claimed: &ClaimedTurnRun,
        ) -> Result<
            Box<dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync>,
            HostFactoryError,
        > {
            let context = LoopRunContext::new(
                claimed.state.scope.clone(),
                claimed.state.turn_id,
                claimed.state.run_id,
                claimed.resolved_run_profile.clone(),
            )
            .with_resolved_model_route(LoopModelRouteSnapshot::new(
                "test_provider",
                "test_model",
                "config:v1",
                "auth:v1",
            ));
            Ok(Box::new(StubDriverHost { context }))
        }
    }

    /// Minimal stub host: only `run_context()` is used by the executor.
    struct StubDriverHost {
        context: LoopRunContext,
    }

    impl ironclaw_turns::run_profile::LoopRunInfoPort for StubDriverHost {
        fn run_context(&self) -> &LoopRunContext {
            &self.context
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopContextPort for StubDriverHost {
        async fn load_loop_context(
            &self,
            _request: ironclaw_turns::run_profile::LoopContextRequest,
        ) -> Result<ironclaw_turns::run_profile::LoopContextBundle, AgentLoopHostError> {
            unimplemented!("stub: not called by executor")
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopPromptPort for StubDriverHost {
        async fn build_prompt_bundle(
            &self,
            _request: ironclaw_turns::run_profile::LoopPromptBundleRequest,
        ) -> Result<ironclaw_turns::run_profile::LoopPromptBundle, AgentLoopHostError> {
            unimplemented!("stub: not called by executor")
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopInputPort for StubDriverHost {
        async fn poll_inputs(
            &self,
            _after: ironclaw_turns::run_profile::LoopInputCursor,
            _limit: usize,
        ) -> Result<ironclaw_turns::run_profile::LoopInputBatch, AgentLoopHostError> {
            unimplemented!("stub: not called by executor")
        }

        async fn ack_inputs(
            &self,
            _tokens: Vec<ironclaw_turns::run_profile::LoopInputAckToken>,
        ) -> Result<(), AgentLoopHostError> {
            unimplemented!("stub: not called by executor")
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopModelPort for StubDriverHost {
        async fn stream_model(
            &self,
            _request: ironclaw_turns::run_profile::LoopModelRequest,
        ) -> Result<ironclaw_turns::run_profile::LoopModelResponse, AgentLoopHostError> {
            unimplemented!("stub: not called by executor")
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopCompactionPort for StubDriverHost {
        async fn compact_loop_context(
            &self,
            _request: ironclaw_turns::run_profile::LoopCompactionRequest,
        ) -> Result<
            ironclaw_turns::run_profile::LoopCompactionOutcome,
            ironclaw_turns::run_profile::LoopCompactionError,
        > {
            unimplemented!("stub: not called by executor")
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopCapabilityPort for StubDriverHost {
        async fn visible_capabilities(
            &self,
            _request: ironclaw_turns::run_profile::VisibleCapabilityRequest,
        ) -> Result<ironclaw_turns::run_profile::VisibleCapabilitySurface, AgentLoopHostError>
        {
            unimplemented!("stub: not called by executor")
        }

        async fn invoke_capability(
            &self,
            _request: ironclaw_turns::run_profile::CapabilityInvocation,
        ) -> Result<ironclaw_turns::run_profile::CapabilityOutcome, AgentLoopHostError> {
            unimplemented!("stub: not called by executor")
        }

        async fn invoke_capability_batch(
            &self,
            _request: ironclaw_turns::run_profile::CapabilityBatchInvocation,
        ) -> Result<ironclaw_turns::run_profile::CapabilityBatchOutcome, AgentLoopHostError>
        {
            unimplemented!("stub: not called by executor")
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopTranscriptPort for StubDriverHost {
        async fn finalize_assistant_message(
            &self,
            _request: ironclaw_turns::run_profile::FinalizeAssistantMessage,
        ) -> Result<LoopMessageRef, AgentLoopHostError> {
            unimplemented!("stub: not called by executor")
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopCheckpointPort for StubDriverHost {
        async fn checkpoint(
            &self,
            _request: ironclaw_turns::run_profile::LoopCheckpointRequest,
        ) -> Result<ironclaw_turns::TurnCheckpointId, AgentLoopHostError> {
            unimplemented!("stub: not called by executor")
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopProgressPort for StubDriverHost {
        async fn emit_loop_progress(
            &self,
            _event: ironclaw_turns::run_profile::LoopProgressEvent,
        ) -> Result<(), AgentLoopHostError> {
            unimplemented!("stub: not called by executor")
        }
    }

    #[async_trait]
    impl ironclaw_turns::run_profile::LoopCancellationPort for StubDriverHost {
        fn observe_cancellation(
            &self,
        ) -> Option<ironclaw_turns::run_profile::LoopCancellationSignal> {
            None
        }

        async fn cancellation_requested(
            &self,
        ) -> ironclaw_turns::run_profile::LoopCancellationSignal {
            std::future::pending().await
        }
    }

    /// A `TurnRunTransitionPort` that returns `Err` from
    /// `record_model_route_snapshot`, and records whether `fail_run` was called.
    #[derive(Default)]
    struct FailingSnapshotTransitionPort {
        fail_run_calls: Mutex<Vec<FailRunRequest>>,
    }

    impl FailingSnapshotTransitionPort {
        fn fail_run_call_count(&self) -> usize {
            self.fail_run_calls.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl TurnRunTransitionPort for FailingSnapshotTransitionPort {
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
            // Simulate a persistence failure.
            Err(TurnError::Unavailable {
                reason: "simulated snapshot persistence error".to_string(),
            })
        }

        async fn block_run(&self, _request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn complete_run(
            &self,
            _request: CompleteRunRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn cancel_run(
            &self,
            _request: CancelRunCompletionRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
            self.fail_run_calls.lock().unwrap().push(request);
            Ok(fake_run_state())
        }

        async fn relinquish_run(
            &self,
            _request: RelinquishRunRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn apply_validated_loop_exit(
            &self,
            _request: ApplyValidatedLoopExitRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }
    }

    fn make_executor_with_driver(
        host_factory: Arc<dyn crate::turn_runner::HostFactory>,
    ) -> RebornTurnRunExecutor {
        let transitions: Arc<dyn TurnRunTransitionPort> =
            Arc::new(RecordingTransitionPort::default());
        let evidence = Arc::new(InMemoryLoopExitEvidencePort::new());
        let loop_exit_applier = Arc::new(LoopExitApplier::new(transitions, evidence));
        // Register a driver matching `test_claimed_run`'s descriptor.
        let mut registry = DriverRegistry::new();
        registry
            .register_driver(
                Arc::new(CompletingDriver::new(test_descriptor())),
                DriverRequirements::all_optional(),
                DriverKind::Production,
            )
            .expect("driver registration must succeed");
        let driver_registry = Arc::new(registry);
        RebornTurnRunExecutor::new(loop_exit_applier, driver_registry, host_factory)
    }

    /// Variant that wires the SAME shared `transitions` port into both the
    /// `LoopExitApplier` and the `execute_claimed_run` call, so tests can
    /// inspect / control the full exit path with a single spy.
    fn make_executor_with_driver_and_shared_transitions(
        host_factory: Arc<dyn crate::turn_runner::HostFactory>,
        transitions: Arc<dyn TurnRunTransitionPort>,
    ) -> RebornTurnRunExecutor {
        let evidence = Arc::new(InMemoryLoopExitEvidencePort::new());
        let loop_exit_applier = Arc::new(LoopExitApplier::new(Arc::clone(&transitions), evidence));
        let mut registry = DriverRegistry::new();
        registry
            .register_driver(
                Arc::new(CompletingDriver::new(test_descriptor())),
                DriverRequirements::all_optional(),
                DriverKind::Production,
            )
            .expect("driver registration must succeed");
        let driver_registry = Arc::new(registry);
        RebornTurnRunExecutor::new(loop_exit_applier, driver_registry, host_factory)
    }

    /// A driver that always returns a caller-supplied `AgentLoopDriverError`.
    ///
    /// Used to exercise the error-category mapping in `execute_claimed_run`
    /// without the overhead of a full host stack.
    struct ErrorReturningDriver {
        descriptor: AgentLoopDriverDescriptor,
        error: AgentLoopDriverError,
    }

    impl ErrorReturningDriver {
        fn new(descriptor: AgentLoopDriverDescriptor, error: AgentLoopDriverError) -> Self {
            Self { descriptor, error }
        }
    }

    #[async_trait]
    impl AgentLoopDriver for ErrorReturningDriver {
        fn descriptor(&self) -> AgentLoopDriverDescriptor {
            self.descriptor.clone()
        }

        async fn run(
            &self,
            _request: AgentLoopDriverRunRequest,
            _host: &(dyn AgentLoopDriverHost + Send + Sync),
        ) -> Result<LoopExit, AgentLoopDriverError> {
            Err(self.error.clone())
        }

        async fn resume(
            &self,
            _request: AgentLoopDriverResumeRequest,
            _host: &(dyn AgentLoopDriverHost + Send + Sync),
        ) -> Result<LoopExit, AgentLoopDriverError> {
            Err(self.error.clone())
        }
    }

    /// Builds an executor whose registered driver always returns the given error.
    fn make_executor_with_failing_driver(error: AgentLoopDriverError) -> RebornTurnRunExecutor {
        let transitions: Arc<dyn TurnRunTransitionPort> =
            Arc::new(RecordingTransitionPort::default());
        let evidence = Arc::new(InMemoryLoopExitEvidencePort::new());
        let loop_exit_applier = Arc::new(LoopExitApplier::new(transitions, evidence));
        let mut registry = DriverRegistry::new();
        registry
            .register_driver(
                Arc::new(ErrorReturningDriver::new(test_descriptor(), error)),
                DriverRequirements::all_optional(),
                DriverKind::Production,
            )
            .expect("driver registration must succeed");
        let driver_registry = Arc::new(registry);
        RebornTurnRunExecutor::new(
            loop_exit_applier,
            driver_registry,
            Arc::new(SucceedingHostFactoryWithSnapshot),
        )
    }

    /// When the driver returns `AgentLoopDriverError::InvalidRequest`, the executor
    /// must return `Err` with category `"driver_invalid_request"`, and when it
    /// returns `AgentLoopDriverError::Unavailable`, the category must be
    /// `"driver_unavailable"`. Both are verified here to confirm the two branches
    /// in the `DriverInvocationError::DriverError` match arm produce distinct,
    /// correctly-named categories.
    #[tokio::test]
    async fn driver_invalid_request_and_unavailable_record_distinct_categories() {
        // ── InvalidRequest → driver_invalid_request ───────────────────────────
        let executor = make_executor_with_failing_driver(AgentLoopDriverError::InvalidRequest {
            reason: "bad input from test".to_string(),
        });
        let transitions = Arc::new(RecordingTransitionPort::default());
        let result = executor
            .execute_claimed_run(
                test_claimed_run(),
                transitions.clone() as Arc<dyn TurnRunTransitionPort>,
            )
            .await;

        let err = result.expect_err("expected Err for InvalidRequest driver error");
        assert_eq!(
            err.failure_category(),
            "driver_invalid_request",
            "InvalidRequest must map to category driver_invalid_request"
        );
        assert_eq!(
            transitions.fail_run_call_count(),
            0,
            "executor must NOT call fail_run; scheduler owns terminal failure recording"
        );

        // ── Unavailable → driver_unavailable ──────────────────────────────────
        let executor = make_executor_with_failing_driver(AgentLoopDriverError::Unavailable {
            reason: "driver temporarily unavailable in test".to_string(),
        });
        let transitions = Arc::new(RecordingTransitionPort::default());
        let result = executor
            .execute_claimed_run(
                test_claimed_run(),
                transitions.clone() as Arc<dyn TurnRunTransitionPort>,
            )
            .await;

        let err = result.expect_err("expected Err for Unavailable driver error");
        assert_eq!(
            err.failure_category(),
            "driver_unavailable",
            "Unavailable must map to category driver_unavailable"
        );
        assert_eq!(
            transitions.fail_run_call_count(),
            0,
            "executor must NOT call fail_run; scheduler owns terminal failure recording"
        );
    }

    /// A `TurnRunTransitionPort` that fails on both `apply_validated_loop_exit`
    /// AND `fail_run` (used by the default `record_runner_failure` impl).
    ///
    /// Used to exercise the double-failure path in `apply_exit`.
    struct DoubleFailingTransitionPort;

    #[async_trait]
    impl TurnRunTransitionPort for DoubleFailingTransitionPort {
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
            Ok(fake_run_state())
        }

        async fn block_run(&self, _request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn complete_run(
            &self,
            _request: CompleteRunRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn cancel_run(
            &self,
            _request: CancelRunCompletionRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn fail_run(&self, _request: FailRunRequest) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: "double-failing: fail_run always returns Err".to_string(),
            })
        }

        async fn relinquish_run(
            &self,
            _request: RelinquishRunRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        /// Fail here so `LoopExitApplier::apply` returns `Err`, triggering the
        /// fallback `record_runner_failure` path in `apply_exit`.
        async fn apply_validated_loop_exit(
            &self,
            _request: ApplyValidatedLoopExitRequest,
        ) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: "double-failing: apply_validated_loop_exit always returns Err".to_string(),
            })
        }
    }

    /// When `HostFactory::create_host` returns `Err`, `execute_claimed_run` must
    /// return `Err(TurnRunExecutorError)` with category `"host_creation_failed"`.
    /// The executor must NOT itself call `fail_run` — that is the scheduler's job.
    #[tokio::test]
    async fn host_creation_failure_returns_err_without_calling_fail_run() {
        let executor = make_executor_with_driver(Arc::new(FailingHostFactory));
        let transitions = Arc::new(RecordingTransitionPort::default());

        let result = executor
            .execute_claimed_run(
                test_claimed_run(),
                transitions.clone() as Arc<dyn TurnRunTransitionPort>,
            )
            .await;

        let err = result.expect_err("expected Err for host creation failure");
        assert_eq!(
            err.failure_category(),
            "host_creation_failed",
            "error category must be host_creation_failed"
        );
        assert_eq!(
            transitions.fail_run_call_count(),
            0,
            "executor must NOT call fail_run; scheduler owns terminal failure recording"
        );
    }

    /// When `persist_model_route_snapshot` fails (the transition port returns
    /// `Err` from `record_model_route_snapshot`), `execute_claimed_run` must
    /// return `Err(TurnRunExecutorError)` with category
    /// `"route_snapshot_persistence_failed"`.
    /// The executor must NOT itself call `fail_run` on this path.
    #[tokio::test]
    async fn model_route_snapshot_persistence_failure_returns_err_without_calling_fail_run() {
        let executor = make_executor_with_driver(Arc::new(SucceedingHostFactoryWithSnapshot));
        let transitions = Arc::new(FailingSnapshotTransitionPort::default());

        let result = executor
            .execute_claimed_run(
                test_claimed_run(),
                transitions.clone() as Arc<dyn TurnRunTransitionPort>,
            )
            .await;

        let err = result.expect_err("expected Err for snapshot persistence failure");
        assert_eq!(
            err.failure_category(),
            "route_snapshot_persistence_failed",
            "error category must be route_snapshot_persistence_failed"
        );
        assert_eq!(
            transitions.fail_run_call_count(),
            0,
            "executor must NOT call fail_run; scheduler owns terminal failure recording"
        );
    }

    /// A `TurnRunTransitionPort` where `apply_validated_loop_exit` always returns
    /// `Err` (causing `LoopExitApplier::apply` to fail), but `fail_run` succeeds
    /// and records the call (the normal recovery path inside `apply_exit`).
    ///
    /// Used to verify that a single exit-application failure — without a
    /// secondary `fail_run` failure — is handled as a successful recovery:
    /// `apply_exit` returns `Ok(())` and `execute_claimed_run` returns `Ok`.
    #[derive(Default)]
    struct FailingApplySucceedingFailRunPort {
        fail_run_calls: Mutex<Vec<FailRunRequest>>,
    }

    impl FailingApplySucceedingFailRunPort {
        fn fail_run_call_count(&self) -> usize {
            self.fail_run_calls.lock().unwrap().len()
        }

        fn fail_run_calls(&self) -> Vec<FailRunRequest> {
            self.fail_run_calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl TurnRunTransitionPort for FailingApplySucceedingFailRunPort {
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
            Ok(fake_run_state())
        }

        async fn block_run(&self, _request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn complete_run(
            &self,
            _request: CompleteRunRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn cancel_run(
            &self,
            _request: CancelRunCompletionRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
            self.fail_run_calls.lock().unwrap().push(request);
            Ok(fake_run_state())
        }

        async fn relinquish_run(
            &self,
            _request: RelinquishRunRequest,
        ) -> Result<TurnRunState, TurnError> {
            Ok(fake_run_state())
        }

        /// Always returns `Err` so that `LoopExitApplier::apply` fails and
        /// `apply_exit` falls through to the `record_runner_failure` recovery arm.
        async fn apply_validated_loop_exit(
            &self,
            _request: ApplyValidatedLoopExitRequest,
        ) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: "induced: apply_validated_loop_exit always returns Err".to_string(),
            })
        }
    }

    /// When `loop_exit_applier.apply` (via `apply_validated_loop_exit`) fails but
    /// the fallback `transitions.record_runner_failure` (via `fail_run`) succeeds,
    /// `apply_exit` must:
    ///   (a) return `Ok(())` — the run is left terminal via the fallback path, so
    ///       the executor considers the run handled and does not bubble an error,
    ///   (b) call `fail_run` exactly once — one recording of the terminal failure.
    ///
    /// Uses `make_executor_with_driver_and_shared_transitions` to wire the same
    /// `FailingApplySucceedingFailRunPort` into both the `LoopExitApplier` (so
    /// `apply_validated_loop_exit` is the one that fails) and the
    /// `execute_claimed_run` call (so `fail_run` on the same port is the spy).
    #[tokio::test]
    async fn apply_exit_failure_recovers_via_fail_run_records_terminal() {
        let transitions = Arc::new(FailingApplySucceedingFailRunPort::default());
        let transitions_arc: Arc<dyn TurnRunTransitionPort> = transitions.clone();
        let executor = make_executor_with_driver_and_shared_transitions(
            Arc::new(SucceedingHostFactoryWithSnapshot),
            Arc::clone(&transitions_arc),
        );

        let claimed = test_claimed_run();
        let claimed_run_id = claimed.state.run_id;

        let result = executor.execute_claimed_run(claimed, transitions_arc).await;

        // (a) Recovery succeeded: apply_exit returns Ok(()) → execute_claimed_run
        //     returns Ok(()).  The run is terminal via the fail_run path.
        assert!(
            result.is_ok(),
            "expected Ok when apply_validated_loop_exit fails but fail_run succeeds; got {result:?}"
        );

        // (b) fail_run was called exactly once with the correct run id.
        let calls = transitions.fail_run_calls();
        assert_eq!(
            transitions.fail_run_call_count(),
            1,
            "fail_run must be called exactly once to record the terminal failure; \
             got {} call(s)",
            calls.len()
        );
        assert_eq!(
            calls[0].run_id, claimed_run_id,
            "fail_run must be called with the claimed run's run_id"
        );
    }

    /// When BOTH `loop_exit_applier.apply` (via `apply_validated_loop_exit`) AND
    /// the fallback `transitions.record_runner_failure` (via `fail_run`) fail,
    /// `execute_claimed_run` must return `Err` so the scheduler can attempt its
    /// own terminal-failure recording.
    ///
    /// Uses `make_executor_with_driver_and_shared_transitions` to wire the same
    /// `DoubleFailingTransitionPort` into both the `LoopExitApplier` and the
    /// `execute_claimed_run` call.
    #[tokio::test]
    async fn double_failure_in_apply_exit_returns_err() {
        // SucceedingHostFactoryWithSnapshot: host creation succeeds, driver runs,
        // returns LoopExit::Completed — so we reach apply_exit.
        // But DoubleFailingTransitionPort makes both apply_validated_loop_exit
        // and fail_run return Err, triggering the double-failure path.
        let transitions: Arc<dyn TurnRunTransitionPort> = Arc::new(DoubleFailingTransitionPort);
        let executor = make_executor_with_driver_and_shared_transitions(
            Arc::new(SucceedingHostFactoryWithSnapshot),
            Arc::clone(&transitions),
        );

        let result = executor
            .execute_claimed_run(test_claimed_run(), transitions)
            .await;

        assert!(
            result.is_err(),
            "expected Err when both loop_exit_applier.apply and record_runner_failure fail"
        );
    }
}
