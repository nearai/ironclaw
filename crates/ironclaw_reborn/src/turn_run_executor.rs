//! Concrete `TurnRunExecutor` for the Reborn planned agent loop.
//!
//! Adapts `RebornLoopDriverHostFactory` + `DriverRegistry` + `LoopExitApplier`
//! to the `TurnRunExecutor` trait consumed by `TurnRunScheduler`.

use std::sync::{Arc, OnceLock};

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
            .expect("'unknown_failure' is a valid static executor error category")
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
                // `sanitized` is always Some — sanitized_failure /
                // sanitized_driver_failure fall back to "unknown_failure" before
                // returning None. The unwrap_or_else path is a belt-and-suspenders
                // guard that is never reached in practice.
                let failure =
                    sanitized.unwrap_or_else(|| unknown_failure_error().failure().clone());
                Err(TurnRunExecutorError::new(failure.category())
                    .unwrap_or_else(|_| unknown_failure_error().clone()))
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
            // FIX 3: carry the full Display of the cause, not just the `.reason` field.
            .map_err(|err| DriverInvocationError::HostCreationFailed {
                reason: err.to_string(),
            })?;
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
                // FIX 2: use infallible sanitized_failure — it falls back to
                // "unknown_failure" before returning None, so the run always
                // reaches a terminal state instead of leaking a slot.
                let failure = sanitized_failure("exit_application_failed")
                    .unwrap_or_else(|| unknown_failure_error().failure().clone());
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_host_api::{TenantId, ThreadId};
    use ironclaw_host_runtime::TurnRunExecutor;
    use ironclaw_turns::{
        AcceptedMessageRef, AgentLoopDriverDescriptor, EventCursor, ReplyTargetBindingRef,
        RunProfileVersion, SourceBindingRef, TurnError, TurnId, TurnRunId, TurnRunState, TurnScope,
        TurnStatus,
        run_profile::{CheckpointSchemaId, LoopDriverId},
        runner::{
            ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
            ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
            RecordModelRouteSnapshotRequest, RecoverExpiredLeasesRequest,
            RecoverExpiredLeasesResponse, RelinquishRunRequest, TurnRunTransitionPort,
        },
    };

    use crate::{
        driver_registry::DriverRegistry,
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
}
