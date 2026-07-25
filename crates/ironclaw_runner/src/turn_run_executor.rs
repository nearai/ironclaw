// arch-exempt: large_file, caller-level failure propagation stays at the turn executor seam, plan #4088
//! Concrete `TurnRunExecutor` for the Reborn planned agent loop.
//!
//! Adapts `RebornLoopDriverHostFactory` + `DriverRegistry` + `LoopExitApplier`
//! to the `TurnRunExecutor` trait consumed by `TurnRunScheduler`.

use std::{
    sync::{Arc, OnceLock},
    time::Instant,
};

use async_trait::async_trait;
use ironclaw_host_api::{GateRecord, GateRef, ResourceScope};
use ironclaw_observability::live_latency_started_at;
use ironclaw_run_state::GateRecordStorePort;
use ironclaw_turns::{
    AgentLoopDriverError, AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, LoopBlocked,
    LoopBlockedKind, LoopExit, TurnStatus,
    run_profile::AgentLoopDriverHost,
    runner::{
        ClaimedTurnRun, RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest,
        TurnRunTransitionPort,
    },
};
use tracing::{debug, error, warn};

/// The loop-facing routing-ref prefix an auth gate carries
/// (`gate:auth-{gate_id}`), minted by `loop_gate_ref("auth", …)` in
/// `ironclaw_loop_host`. The durable `GateRecord::Auth` is keyed by the uuid
/// `gate_id` alone (via [`GateRef::for_auth_gate`]), so the runner strips this
/// prefix to recover the same key the loop-host persisted under.
const AUTH_GATE_LOOP_REF_PREFIX: &str = "gate:auth-";

use crate::{
    after_turn_memory::AfterTurnMemoryRecorder,
    driver_registry::{DriverRegistry, LoopDriverRegistryKey},
    failure_categories::host_stage_unavailable_category,
    loop_exit_applier::LoopExitApplier,
    turn_runner::{HostFactory, sanitized_driver_failure, sanitized_failure},
    turn_scheduler::{TurnRunExecutor, TurnRunExecutorError},
};

/// Upper bound on the best-effort after-turn memory recording that the scheduler
/// worker awaits inline. A slow or hung memory provider must not occupy the
/// worker (and delay unrelated runs) beyond this; on timeout the recording is
/// skipped (the run is already `Completed`). Generous because a network-backed
/// provider performs a thread-history read plus a write.
const AFTER_TURN_MEMORY_RECORD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

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
    /// Durable store the loop-host persisted `GateRecord::Auth` into (§5.2.9).
    ///
    /// After an auth block the executor re-sources the auth gate's
    /// `credential_requirements` from this store, because the flip moved them
    /// off the loop-facing channel onto the host-persisted record — the
    /// loop-facing `LoopBlocked.credential_requirements` arrives empty. `None`
    /// only for helper/test compositions that never wire a run-state
    /// filesystem (they never raise a durable auth gate to render); every
    /// production composition wires the SAME `Arc` it wired into the capability
    /// port's `with_gate_record_store`, so an unwired production path is a bug.
    gate_record_store: Option<Arc<dyn GateRecordStorePort>>,
    /// After-turn interaction recorder (mem0 `add` seam). Optional; production
    /// wires `None` pending #5013 — only compositions that resolve a memory
    /// document-store provider attach it, and a `Completed` run finishes cleanly
    /// without it (the same genuine optionality as `memory_context_service` on
    /// `DefaultPlannedRuntimeParts`).
    // arch-exempt: optional_arc, deferred production wiring, issue #5013
    after_turn_memory_recorder: Option<Arc<AfterTurnMemoryRecorder>>,
}

impl RebornTurnRunExecutor {
    pub fn new(
        loop_exit_applier: Arc<LoopExitApplier>,
        driver_registry: Arc<DriverRegistry>,
        host_factory: Arc<dyn HostFactory>,
        gate_record_store: Option<Arc<dyn GateRecordStorePort>>,
    ) -> Self {
        Self {
            loop_exit_applier,
            driver_registry,
            host_factory,
            gate_record_store,
            after_turn_memory_recorder: None,
        }
    }

    /// Attach the after-turn memory recorder. Called by the runtime composition
    /// only when a memory provider is resolved; tests construct one over a real
    /// in-memory provider.
    pub fn with_after_turn_memory_recorder(
        mut self,
        recorder: Arc<AfterTurnMemoryRecorder>,
    ) -> Self {
        self.after_turn_memory_recorder = Some(recorder);
        self
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
                        detail,
                    }) => sanitized_driver_failure(reason_kind, detail.as_deref()),
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
                        reason,
                    }) => sanitized_failure(host_stage_unavailable_category(reason)),
                };
                // `sanitized` is always Some — sanitized_failure /
                // sanitized_driver_failure fall back to "unknown_failure" before
                // returning None. The unwrap_or_else path is a belt-and-suspenders
                // guard that is never reached in practice.
                let failure =
                    sanitized.unwrap_or_else(|| unknown_failure_error().failure().clone());
                // Preserve the full `SanitizedFailure` (category + scrubbed
                // model-visible `detail`) across the host-runtime boundary. The
                // scheduler records `executor_error.failure()`, so this is what
                // carries the real driver-failure cause into
                // `TurnLifecycleEvent.detail` and the failure explainer.
                let error = TurnRunExecutorError::from_failure(failure);
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
        mut exit: LoopExit,
        transitions: &Arc<dyn TurnRunTransitionPort>,
    ) -> Result<(), ()> {
        let started_at = live_latency_started_at();
        let run_id = claimed.state.run_id;
        let runner_id = claimed.runner_id;

        // §5.2.9 render-from-record: an auth block arrives with empty
        // `credential_requirements` (the flip moved them off the loop channel
        // onto the host-persisted `GateRecord::Auth`). Re-source them here,
        // BEFORE the trusted applier persists the `TurnRunRecord`, so the
        // auth-prompt (`channel_delivery`) and resume (`blocked_auth_resume`)
        // read a non-empty requirement set again.
        if let LoopExit::Blocked(blocked) = &mut exit
            && let Err(failure_tag) = self
                .enrich_auth_block_credential_requirements(claimed, blocked)
                .await
        {
            // The auth block's credential requirements could not be re-sourced
            // from the durable record, so applying it would park the run on an
            // unsubmittable (provider-null) auth gate. Record a terminal failure
            // instead — the scheduler surfaces it and the run is failed rather
            // than silently stranded.
            return self
                .record_exit_failure(claimed, transitions, failure_tag)
                .await;
        }

        match self.loop_exit_applier.apply(claimed, exit).await {
            Ok(state) => {
                trace_executor_latency_ok("apply_loop_exit", claimed, started_at);
                debug!(
                    runner_id = ?runner_id,
                    run_id = ?run_id,
                    status = ?state.status,
                    "loop exit applied successfully"
                );
                // After-turn memory recording (mem0 `add` seam): hand the
                // just-finished exchange to the memory provider. This is a
                // post-terminal, best-effort side effect — the run is ALREADY
                // Completed, so the recorder never fails it (every error inside is
                // `debug!`-only, never `info!`/`warn!`).
                if state.status == TurnStatus::Completed
                    && let Some(recorder) = self.after_turn_memory_recorder.as_ref()
                {
                    // Bound this best-effort post-terminal side effect so a slow or
                    // hung memory provider can't occupy the scheduler worker and
                    // delay unrelated runs.
                    if tokio::time::timeout(
                        AFTER_TURN_MEMORY_RECORD_TIMEOUT,
                        recorder.record_completed_run(&state),
                    )
                    .await
                    .is_err()
                    {
                        // silent-ok: after-turn recording is best-effort post-completion;
                        // a timeout must not fail or delay the already-completed run.
                        debug!(
                            run_id = ?run_id,
                            "after-turn memory recording timed out; skipping (run already complete)"
                        );
                    }
                }
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
                // Exit application failed: record a terminal failure through the
                // transitions port so the run is not stranded. Falls back to
                // "unknown_failure" so the run always reaches a terminal state if
                // the port cooperates; `Err(())` (double-failure) signals the
                // caller so the scheduler can attempt its own recording.
                self.record_exit_failure(claimed, transitions, "exit_application_failed")
                    .await
            }
        }
    }

    /// Re-source an auth block's `credential_requirements` from the durable
    /// [`GateRecord::Auth`] the loop-host persisted (§5.2.9 render-from-record).
    ///
    /// The §5.3 Stage 2 flip moved `credential_requirements` off the loop-facing
    /// channel onto the host record, so an auth `LoopBlocked` arrives with them
    /// empty. Without this, `TurnRunRecord.credential_requirements` would be
    /// empty after an auth block — a regression for the auth-prompt view and the
    /// blocked-auth resume path that both read it. Only auth blocks that arrive
    /// empty are touched; a block that already carries requirements (or any
    /// non-auth block) is left as-is.
    ///
    /// Tolerant pre-conditions (a missing store, or a non-auth/non-`gate:auth-`
    /// ref) leave `credential_requirements` empty and log — the same pre-flip
    /// regression, no worse, and not the flip's new failure surface.
    ///
    /// But when the store IS wired and its lookup does not yield the auth record
    /// (a store read fault, a genuinely-absent record, or a wrong-kind record),
    /// returning empty would let `apply_exit` persist an auth block with no
    /// provider/scopes — an unsubmittable gate the user can never action, which
    /// is exactly the regression this render-from-record path exists to prevent.
    /// Those arms return `Err(tag)` so the caller fails the exit (recording a
    /// terminal failure the scheduler surfaces) instead of stranding the run
    /// (`.claude/rules/error-handling.md`).
    async fn enrich_auth_block_credential_requirements(
        &self,
        claimed: &ClaimedTurnRun,
        blocked: &mut LoopBlocked,
    ) -> Result<(), &'static str> {
        if blocked.kind != LoopBlockedKind::Auth || !blocked.credential_requirements.is_empty() {
            return Ok(());
        }
        let run_id = claimed.state.run_id;
        let Some(store) = self.gate_record_store.as_ref() else {
            warn!(
                run_id = %run_id,
                "auth block: no gate record store wired; credential_requirements left empty"
            );
            return Ok(());
        };
        // Recover the durable `GateRecord::Auth` key from the loop routing ref
        // `gate:auth-{gate_id}` — the loop-host persisted under the deterministic
        // (name-based v5) `GateRef::for_auth_gate(gate_id)`, so the same gate id
        // reproduces the identical key here.
        let Some(gate_id) = blocked
            .gate_ref
            .as_str()
            .strip_prefix(AUTH_GATE_LOOP_REF_PREFIX)
        else {
            warn!(
                run_id = %run_id,
                gate_ref = blocked.gate_ref.as_str(),
                "auth block: loop gate ref is not an auth ref; credential_requirements left empty"
            );
            return Ok(());
        };
        let gate_ref = GateRef::for_auth_gate(gate_id);
        let scope = auth_gate_record_read_scope(claimed);
        match store.load(&scope, gate_ref).await {
            Ok(Some(GateRecord::Auth {
                credential_requirements,
                ..
            })) => {
                blocked.credential_requirements = credential_requirements;
                Ok(())
            }
            Ok(Some(other)) => {
                error!(
                    run_id = %run_id,
                    gate_record_kind = other.kind(),
                    "auth block: persisted gate record was not Auth; failing the exit rather than applying an unsubmittable auth block"
                );
                Err("auth_gate_record_wrong_kind")
            }
            Ok(None) => {
                error!(
                    run_id = %run_id,
                    "auth block: no persisted gate record found for auth gate; failing the exit rather than applying an unsubmittable auth block"
                );
                Err("auth_gate_record_missing")
            }
            Err(error) => {
                error!(
                    run_id = %run_id,
                    error = %error,
                    "auth block: gate record store read failed; failing the exit rather than applying an unsubmittable auth block"
                );
                Err("auth_gate_record_read_failed")
            }
        }
    }

    /// Record a terminal failure for a run whose loop exit must not be applied
    /// as-is — either the applier rejected it, or an auth block's credential
    /// requirements could not be re-sourced from the durable record (applying it
    /// would strand the run on an unsubmittable auth gate). Mirrors the applier's
    /// own fallback so both routes leave the run terminal, never stranded.
    /// Returns `Err(())` only on the double-failure where the transition port
    /// itself fails, which the caller escalates to the scheduler.
    async fn record_exit_failure(
        &self,
        claimed: &ClaimedTurnRun,
        transitions: &Arc<dyn TurnRunTransitionPort>,
        failure_tag: &'static str,
    ) -> Result<(), ()> {
        let run_id = claimed.state.run_id;
        let runner_id = claimed.runner_id;
        let lease_token = claimed.lease_token;
        let failure = sanitized_failure(failure_tag)
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
                error!(
                    runner_id = ?runner_id,
                    run_id = ?run_id,
                    error = %record_err,
                    "failed to record terminal failure"
                );
                Err(())
            }
        }
    }
}

/// Rebuild the resource-owner scope the loop-host persisted the auth
/// [`GateRecord`] under, from the claimed run.
///
/// Must equal the save scope (`visible_request.context.resource_scope` in
/// `ironclaw_loop_host`) on every axis [`same_scope_owner`] compares —
/// tenant/user/agent/project/mission/thread; `invocation_id` is deliberately
/// NOT part of the gate-record key (neither the path nor the owner check use
/// it), so the runner does not need to reproduce it. The save side sets
/// `user_id = explicit thread owner, else the run actor` (its final fallback to
/// a composition-level default user id is unreachable for a claimed run, which
/// always carries an actor); `to_resource_scope` supplies tenant/agent/project/
/// thread and `mission_id = None`.
fn auth_gate_record_read_scope(claimed: &ClaimedTurnRun) -> ResourceScope {
    let mut scope = claimed.state.scope.to_resource_scope();
    if let Some(user_id) = claimed
        .state
        .scope
        .explicit_owner_user_id()
        .or_else(|| claimed.state.actor.as_ref().map(|actor| &actor.user_id))
    {
        scope.user_id = user_id.clone();
    }
    scope
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_host_api::{TenantId, ThreadId};
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
        failure_categories::BUDGET_ACCOUNTING_FAILED_CATEGORY,
        loop_exit_applier::{InMemoryLoopExitEvidencePort, LoopExitApplier},
        turn_runner::HostFactoryError,
    };

    use super::RebornTurnRunExecutor;
    use crate::turn_scheduler::TurnRunExecutor;

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
            model_usage: None,
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
            model_usage: None,
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
        RebornTurnRunExecutor::new(loop_exit_applier, driver_registry, host_factory, None)
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
                model_usage: None,
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
                model_usage: None,
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
            _request: ironclaw_turns::run_profile::LoopRequest,
        ) -> Result<ironclaw_host_api::Resolution, AgentLoopHostError> {
            unimplemented!("stub: not called by executor")
        }

        async fn invoke_capability_batch(
            &self,
            _request: ironclaw_turns::run_profile::LoopRequestBatch,
        ) -> Result<ironclaw_host_api::ResolutionBatch, AgentLoopHostError> {
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
        RebornTurnRunExecutor::new(loop_exit_applier, driver_registry, host_factory, None)
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
        RebornTurnRunExecutor::new(loop_exit_applier, driver_registry, host_factory, None)
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
            None,
        )
    }

    /// When the driver returns `AgentLoopDriverError::InvalidRequest`, the executor
    /// must return `Err` with category `"driver_invalid_request"`, and when it
    /// returns `AgentLoopDriverError::Unavailable`, the category must identify
    /// the unavailable host stage. Both are verified here to confirm the two
    /// branches in the `DriverInvocationError::DriverError` match arm produce
    /// distinct, correctly-named categories.
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

        // ── Unavailable → host_stage_unavailable_model ───────────────────────
        let executor = make_executor_with_failing_driver(AgentLoopDriverError::Unavailable {
            reason: "model: driver temporarily unavailable in test".to_string(),
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
            "host_stage_unavailable_model",
            "Unavailable must map to the host-stage unavailable category"
        );
        assert_eq!(
            transitions.fail_run_call_count(),
            0,
            "executor must NOT call fail_run; scheduler owns terminal failure recording"
        );
    }

    /// A driver `Failed` carrying secret-scrubbed `detail` must have that detail
    /// preserved on the returned `TurnRunExecutorError`. The scheduler records
    /// `error.failure()`, so this Err path is what lets the real scrubbed cause
    /// reach `TurnLifecycleEvent.detail` and the failure explainer. Regression
    /// for the former `TurnRunExecutorError::new(category)` conversion that
    /// dropped `detail` at the host-runtime boundary.
    #[tokio::test]
    async fn driver_failed_preserves_scrubbed_detail_on_executor_error() {
        let executor = make_executor_with_failing_driver(AgentLoopDriverError::Failed {
            reason_kind: "model_error".to_string(),
            detail: Some("provider returned HTTP 500 for /internal/models/route".to_string()),
        });
        let transitions = Arc::new(RecordingTransitionPort::default());
        let result = executor
            .execute_claimed_run(
                test_claimed_run(),
                transitions.clone() as Arc<dyn TurnRunTransitionPort>,
            )
            .await;

        let err = result.expect_err("expected Err for driver Failed");
        assert_eq!(err.failure_category(), "driver_failed");
        assert_eq!(
            err.failure().detail(),
            Some("provider returned HTTP 500 for /internal/models/route"),
            "the scrubbed driver-failure detail must survive onto the executor error, \
             not be dropped at the host-runtime boundary"
        );
        assert_eq!(
            transitions.fail_run_call_count(),
            0,
            "executor must NOT call fail_run; scheduler owns terminal failure recording"
        );
    }

    #[tokio::test]
    async fn driver_failed_preserves_budget_accounting_category_on_executor_error() {
        let executor = make_executor_with_failing_driver(AgentLoopDriverError::Failed {
            reason_kind: BUDGET_ACCOUNTING_FAILED_CATEGORY.to_string(),
            detail: Some("resource accounting storage is unavailable".to_string()),
        });
        let transitions = Arc::new(RecordingTransitionPort::default());

        let error = executor
            .execute_claimed_run(
                test_claimed_run(),
                transitions.clone() as Arc<dyn TurnRunTransitionPort>,
            )
            .await
            .expect_err("accounting failure must remain terminal and typed");

        assert_eq!(error.failure_category(), BUDGET_ACCOUNTING_FAILED_CATEGORY);
        assert_eq!(
            error.failure().detail(),
            Some("resource accounting storage is unavailable")
        );
        assert_eq!(transitions.fail_run_call_count(), 0);
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

    /// #6287 IronLoop regression: an empty auth block whose credential
    /// requirements cannot be re-sourced from the durable record must NOT be
    /// applied — that parks the run on an unsubmittable (provider-null) auth gate
    /// the user can never action. The flip made the durable `GateRecord::Auth`
    /// the ONLY source of `credential_requirements`, so a lookup miss is a real
    /// regression (not a no-worse degrade); `apply_exit` records a terminal
    /// failure instead. Before the fix the miss arms only logged and left the
    /// requirements empty, so `fail_run` would NOT have been called and the
    /// unsubmittable block would have been applied.
    #[tokio::test]
    async fn auth_block_with_unsourceable_requirements_fails_the_exit() {
        use ironclaw_host_api::{GateRecord, GateRef, ResourceScope};
        use ironclaw_run_state::{GateRecordStorePort, RunStateError};
        use ironclaw_turns::{
            LoopBlocked, LoopBlockedKind, LoopCheckpointStateRef, LoopGateRef, TurnCheckpointId,
        };

        // A gate-record store that never yields the auth record (`Ok(None)`).
        struct MissingAuthGateRecordStore;
        #[async_trait]
        impl GateRecordStorePort for MissingAuthGateRecordStore {
            async fn save(
                &self,
                _scope: ResourceScope,
                _gate_ref: GateRef,
                _record: GateRecord,
            ) -> Result<(), RunStateError> {
                Ok(())
            }
            async fn load(
                &self,
                _scope: &ResourceScope,
                _gate_ref: GateRef,
            ) -> Result<Option<GateRecord>, RunStateError> {
                Ok(None)
            }
        }

        let transitions = Arc::new(RecordingTransitionPort::default());
        let transitions_arc: Arc<dyn TurnRunTransitionPort> = transitions.clone();
        let evidence = Arc::new(InMemoryLoopExitEvidencePort::new());
        let loop_exit_applier = Arc::new(LoopExitApplier::new(transitions_arc.clone(), evidence));
        let executor = RebornTurnRunExecutor::new(
            loop_exit_applier,
            Arc::new(DriverRegistry::new()),
            Arc::new(FailingHostFactory),
            Some(Arc::new(MissingAuthGateRecordStore)),
        );

        let claimed = test_claimed_run();
        let run_id = claimed.state.run_id;

        // Auth block arriving with EMPTY credential_requirements (the flip moved
        // them onto the durable `GateRecord::Auth`) and an auth routing ref.
        let exit = LoopExit::Blocked(LoopBlocked {
            kind: LoopBlockedKind::Auth,
            gate_ref: LoopGateRef::new("gate:auth-deadbeef").expect("valid gate ref"),
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            checkpoint_id: TurnCheckpointId::new(),
            state_ref: LoopCheckpointStateRef::new("checkpoint:auth-block-unsourceable")
                .expect("valid"),
            exit_id: LoopExitId::new("exit:test").expect("valid"),
        });

        let result = executor.apply_exit(&claimed, exit, &transitions_arc).await;

        // The exit is failed via a recorded terminal failure (Ok, not the
        // catastrophic double-failure Err), and the block is NOT applied.
        assert!(
            result.is_ok(),
            "apply_exit must record the terminal failure and return Ok; got {result:?}"
        );
        assert_eq!(
            transitions.fail_run_call_count(),
            1,
            "an auth block with unsourceable requirements must be failed via a recorded \
             terminal failure, not applied as an unsubmittable gate"
        );
        assert_eq!(
            transitions.fail_run_calls.lock().unwrap()[0].run_id,
            run_id,
            "the terminal failure must target the claimed run"
        );
    }
}
