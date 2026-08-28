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
use ironclaw_approvals::GateRecordStorePort;
use ironclaw_host_api::{gate_record::GateRecord, ids::GateRef, resource::ResourceScope};
use ironclaw_loop_contracts::{
    AgentLoopDriverError, AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest,
    AgentLoopHostError, LoopBlocked, LoopBlockedKind, LoopExit, LoopModelUsage,
};
use ironclaw_loop_host::AwaitEdgeSettler;
use ironclaw_observability::live_latency_started_at;
use ironclaw_processes::ProcessTransitionPort;
use ironclaw_turns::{ActivationProvenance, TurnError, TurnStatus, runner::ClaimedTurnRun};
use tracing::{debug, error, warn};

/// The loop-facing routing-ref prefix an auth gate carries
/// (`gate:auth-{gate_id}`), minted by `loop_gate_ref("auth", …)` in
/// `ironclaw_loop_host`. The durable `GateRecord::Auth` is keyed by the uuid
/// `gate_id` alone (via [`GateRef::for_auth_gate`]), so the runner strips this
/// prefix to recover the same key the loop-host persisted under.
const AUTH_GATE_LOOP_REF_PREFIX: &str = "gate:auth-";

use ironclaw_turns::loop_exit::LoopExitApplier;

use crate::{
    after_turn_hooks::after_turn_hook_context,
    after_turn_memory::AfterTurnMemoryRecorder,
    driver_registry::{DriverRegistry, LoopDriverRegistryKey},
    failure_categories::host_stage_unavailable_category,
    turn_runner::{HostFactory, sanitized_driver_failure, sanitized_failure},
    turn_scheduler::{TurnRunExecutor, TurnRunExecutorError, TurnRunFailureMetadata},
};

/// Upper bound on the best-effort after-turn memory recording that the scheduler
/// worker awaits inline. A slow or hung memory provider must not occupy the
/// worker (and delay unrelated runs) beyond this; on timeout the recording is
/// skipped (the run is already `Completed`). Generous because a network-backed
/// provider performs a thread-history read plus a write.
const AFTER_TURN_MEMORY_RECORD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Outer bound on the whole `after_turn` hook dispatch. The dispatcher owns the
/// real budget: it bounds each hook individually and classifies a hook that
/// overruns as a timeout failure, poisoning that binding. This is only a
/// backstop against a dispatcher-level surprise, so the scheduler worker stays
/// bounded no matter what.
///
/// It must therefore be STRICTLY LARGER than (the dispatcher's per-hook
/// timeout x a plausible hook count), never equal to it: an outer cancel that
/// lands at the same instant as the inner one takes the whole dispatch away
/// mid-classification, so the slow hook is neither recorded nor poisoned and
/// repeats its full budget on every later run. At the recorder's scale (30s)
/// against a 5s per-hook bound, the dispatcher gets to finish and classify.
const AFTER_TURN_HOOK_DISPATCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

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
    DriverNotFound {
        reason: String,
    },
    HostCreationFailed {
        reason: String,
    },
    HostFinalizationFailed {
        // Boxed: `AgentLoopHostError` is the enum's ≥128-byte outlier (clippy
        // `result_large_err`); the `DriverError` variant stays unboxed
        // because five match sites destructure its payload by pattern.
        error: Box<AgentLoopHostError>,
        cumulative_usage: Option<LoopModelUsage>,
    },
    DriverError {
        error: AgentLoopDriverError,
        supplemental_usage: Option<LoopModelUsage>,
    },
}

struct DriverInvocationSuccess {
    exit: LoopExit,
    supplemental_model_usage: Option<LoopModelUsage>,
}

impl std::fmt::Display for DriverInvocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DriverNotFound { reason } => write!(f, "driver not found: {reason}"),
            Self::HostCreationFailed { reason } => write!(f, "host creation failed: {reason}"),
            Self::HostFinalizationFailed { error, .. } => {
                write!(
                    f,
                    "host terminal finalization failed: {}",
                    error.safe_summary
                )
            }
            Self::DriverError { error, .. } => write!(f, "driver error: {error}"),
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
    /// bound memory provider attach it, and a `Completed` run finishes cleanly
    /// without it (the same genuine optionality as `memory_context_service` on
    /// `DefaultPlannedRuntimeParts`).
    // arch-exempt: optional_arc, deferred production wiring, issue #5013
    after_turn_memory_recorder: Option<Arc<AfterTurnMemoryRecorder>>,
    /// Builds the dispatcher fired at the `after_turn` lifecycle point — a
    /// FACTORY, not a dispatcher, because the executor outlives every run.
    /// Hook poison isolation is run-scoped by contract
    /// (`crates/loop/ironclaw_hooks/AGENTS.md`, "poisoned for the rest of the
    /// current turn run"): a panicking or timing-out hook must be barred for
    /// the run it misbehaved in and tried again on the next one. Holding one
    /// process-lifetime dispatcher here would instead bar it until the process
    /// restarts, silently. Optional for the same reason as the recorder above:
    /// only compositions that register lifecycle hooks attach one, and a run
    /// finishes cleanly without it.
    // arch-exempt: optional_arc, lifecycle hooks are opt-in per composition, plan #7770
    after_turn_hook_dispatchers: Option<crate::loop_driver_host::HookDispatcherFactory>,
    /// Run-start sweep target (§4.2): heals background await-edges left
    /// mid-delivery on the claimed run's thread before `invoke_driver` runs.
    /// Required — every production composition wires the same resolver it
    /// registers as the process-commit observer (`runtime.rs`); tests wire a
    /// no-op fake.
    await_edge_settler: Arc<dyn AwaitEdgeSettler>,
}

impl RebornTurnRunExecutor {
    pub fn new(
        loop_exit_applier: Arc<LoopExitApplier>,
        driver_registry: Arc<DriverRegistry>,
        host_factory: Arc<dyn HostFactory>,
        gate_record_store: Option<Arc<dyn GateRecordStorePort>>,
        await_edge_settler: Arc<dyn AwaitEdgeSettler>,
    ) -> Self {
        Self {
            loop_exit_applier,
            driver_registry,
            host_factory,
            gate_record_store,
            after_turn_memory_recorder: None,
            after_turn_hook_dispatchers: None,
            await_edge_settler,
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

    /// Attach the factory that mints one `after_turn` dispatcher per terminal
    /// run. Wired by compositions that register lifecycle hooks; absent
    /// everywhere else, and a run finishes cleanly without it. A factory
    /// rather than a dispatcher so poison stays run-scoped — see the field.
    pub fn with_after_turn_hook_dispatcher_factory(
        mut self,
        dispatchers: crate::loop_driver_host::HookDispatcherFactory,
    ) -> Self {
        self.after_turn_hook_dispatchers = Some(dispatchers);
        self
    }
}

#[async_trait]
impl TurnRunExecutor for RebornTurnRunExecutor {
    async fn execute_claimed_run(
        &self,
        claimed: ClaimedTurnRun,
        _process_transitions: Arc<dyn ProcessTransitionPort<Error = TurnError>>,
    ) -> Result<(), TurnRunExecutorError> {
        let started_at = live_latency_started_at();
        // Run-start sweep (§4.2), before the driver runs: heal any background
        // await-edges left mid-delivery on this thread. `human_initiated`
        // mirrors the streak-cap admission rule (`activation_streak.rs`) —
        // absent or `Human` provenance is a permitted start; `System` (a
        // background wake) or `ParentAgent` is not, so a streak-capped edge
        // stays parked. A sweep failure must not fail the run start — it is
        // logged and the driver still runs, matching the sweep's own
        // per-edge non-fatal contract.
        let human_initiated = !matches!(
            claimed.subagent_activation_provenance,
            Some(ActivationProvenance::System | ActivationProvenance::ParentAgent)
        );
        if let Err(error) = self
            .await_edge_settler
            .sweep_thread_on_run_start(&claimed.state.scope, human_initiated)
            .await
        {
            debug!(
                run_id = %claimed.state.run_id,
                error = %error,
                "background await-edge run-start sweep failed; continuing without it"
            );
        }
        match self.invoke_driver(&claimed).await {
            Ok(DriverInvocationSuccess {
                exit,
                supplemental_model_usage,
            }) => {
                let result = self
                    .apply_exit(&claimed, exit, supplemental_model_usage)
                    .await;
                match result {
                    Ok(()) => {
                        trace_executor_latency_ok("execute_claimed_run", &claimed, started_at);
                        Ok(())
                    }
                    Err(metadata) => {
                        let error = unknown_failure_error()
                            .clone()
                            .with_failure_metadata(metadata);
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
                    DriverInvocationError::DriverError {
                        error:
                            AgentLoopDriverError::Failed {
                                reason_kind,
                                detail,
                            },
                        ..
                    } => sanitized_driver_failure(reason_kind, detail.as_deref()),
                    DriverInvocationError::DriverNotFound { .. } => {
                        sanitized_failure("driver_not_found")
                    }
                    DriverInvocationError::HostCreationFailed { .. } => {
                        sanitized_failure("host_creation_failed")
                    }
                    DriverInvocationError::HostFinalizationFailed { .. } => {
                        sanitized_failure("structured_finalization_failed")
                    }
                    DriverInvocationError::DriverError {
                        error: AgentLoopDriverError::InvalidRequest { .. },
                        ..
                    } => sanitized_failure("driver_invalid_request"),
                    DriverInvocationError::DriverError {
                        error: AgentLoopDriverError::Unavailable { reason },
                        ..
                    } => sanitized_failure(host_stage_unavailable_category(reason)),
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
                let error = match err {
                    DriverInvocationError::HostFinalizationFailed {
                        cumulative_usage: Some(usage),
                        ..
                    } => TurnRunExecutorError::from_failure(failure).with_failure_metadata(
                        TurnRunFailureMetadata::from_optional_model_usage(Some(usage)),
                    ),
                    DriverInvocationError::DriverError {
                        supplemental_usage: Some(usage),
                        ..
                    } => TurnRunExecutorError::from_failure(failure).with_failure_metadata(
                        TurnRunFailureMetadata::from_optional_model_usage(
                            LoopModelUsage::merge_optional(claimed.state.model_usage, Some(usage)),
                        ),
                    ),
                    _ => TurnRunExecutorError::from_failure(failure),
                };
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
    ) -> Result<DriverInvocationSuccess, DriverInvocationError> {
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
                .map_err(|error| DriverInvocationError::DriverError {
                    error,
                    supplemental_usage: None,
                }),
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
                .map_err(|error| DriverInvocationError::DriverError {
                    error,
                    supplemental_usage: None,
                }),
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
                .map_err(|error| DriverInvocationError::DriverError {
                    error,
                    supplemental_usage: None,
                }),
        };
        match &driver_result {
            Ok(_) => trace_executor_latency_ok(driver_operation, claimed, driver_started_at),
            Err(error) => {
                trace_executor_latency_error(driver_operation, claimed, driver_started_at, error)
            }
        }
        let driver_result = match driver_result {
            Ok(exit) => match host.finalize_terminal_output(&exit).await {
                Ok(()) => Ok(exit),
                Err(error) => Err(DriverInvocationError::HostFinalizationFailed {
                    error: Box::new(error),
                    cumulative_usage: loop_exit_model_usage(&exit).or(claimed.state.model_usage),
                }),
            },
            Err(error) => Err(error),
        };
        // Host-owned terminal finalization is deliberately accounted outside
        // the core loop. Snapshot it while the per-run host is still alive,
        // then attach it to either the returned exit or the typed driver
        // failure metadata before the host is dropped.
        let supplemental_usage = host.supplemental_model_usage();
        drop(host);
        match (driver_result, supplemental_usage) {
            (Ok(exit), supplemental_model_usage) => Ok(DriverInvocationSuccess {
                exit,
                supplemental_model_usage,
            }),
            (Err(error), Some(usage)) => {
                let error = match error {
                    DriverInvocationError::HostFinalizationFailed {
                        error,
                        cumulative_usage,
                    } => DriverInvocationError::HostFinalizationFailed {
                        error,
                        cumulative_usage: LoopModelUsage::merge_optional(
                            cumulative_usage,
                            Some(usage),
                        ),
                    },
                    DriverInvocationError::DriverError {
                        error,
                        supplemental_usage: _,
                    } => DriverInvocationError::DriverError {
                        error,
                        supplemental_usage: Some(usage),
                    },
                    other => other,
                };
                Err(error)
            }
            (Err(error), None) => Err(error),
        }
    }

    /// Apply a `LoopExit` through the trusted applier.
    ///
    /// Returns:
    /// - `Ok(())` if the run reached a terminal state (either via successful
    ///   exit application or via a successful fallback `record_runner_failure`).
    /// - `Err(metadata)` only when BOTH the exit applier AND the fallback
    ///   `record_runner_failure` fail — a double-failure that leaves the run in
    ///   an unknown state. The caller (`execute_claimed_run`) converts this to a
    ///   `TurnRunExecutorError` so the scheduler can record a terminal failure
    ///   without dropping usage already claimed by the exit.
    async fn apply_exit(
        &self,
        claimed: &ClaimedTurnRun,
        mut exit: LoopExit,
        supplemental_model_usage: Option<LoopModelUsage>,
    ) -> Result<(), TurnRunFailureMetadata> {
        let started_at = live_latency_started_at();
        let run_id = claimed.state.run_id;
        let runner_id = claimed.runner_id;
        let exit_model_usage = LoopModelUsage::merge_optional(
            loop_exit_model_usage(&exit).or(claimed.state.model_usage),
            supplemental_model_usage,
        );

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
                .record_exit_failure(claimed, failure_tag, exit_model_usage)
                .await;
        }

        match self
            .loop_exit_applier
            .apply_with_supplemental_model_usage(claimed, exit, supplemental_model_usage)
            .await
        {
            Ok(state) => {
                trace_executor_latency_ok("apply_loop_exit", claimed, started_at);
                debug!(
                    runner_id = ?runner_id,
                    run_id = ?run_id,
                    status = ?state.status,
                    failure = ?state.failure,
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
                // `after_turn` lifecycle hooks (#7276). Same post-terminal,
                // bounded, best-effort discipline as the recorder above: the run
                // is already terminal, so a slow or failing hook must never fail
                // it or hold the scheduler worker. The context derivation decides
                // whether this run may fire the point at all (never an unbound
                // run — otherwise hook-started background work would schedule its
                // own successor forever).
                if let Some(dispatchers) = self.after_turn_hook_dispatchers.as_ref()
                    && let Some(ctx) = after_turn_hook_context(&state)
                {
                    // One FRESH dispatcher for this run. Poison from a hook that
                    // panicked or overran its budget is scoped to the run it
                    // happened in; the next run gets a clean registry and tries
                    // the hook again.
                    let dispatcher = dispatchers();
                    if tokio::time::timeout(
                        AFTER_TURN_HOOK_DISPATCH_TIMEOUT,
                        dispatcher.dispatch_after_turn(ctx),
                    )
                    .await
                    .is_err()
                    {
                        // silent-ok: lifecycle hooks are best-effort post-completion.
                        debug!(
                            run_id = ?run_id,
                            "after-turn hook dispatch timed out; skipping (run already terminal)"
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
                // the port cooperates; the typed error (double-failure) signals
                // the caller so the scheduler can attempt its own recording.
                self.record_exit_failure(claimed, "exit_application_failed", exit_model_usage)
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
    /// Returns typed usage metadata only on the double-failure where the
    /// transition port itself fails, which the caller escalates to the
    /// scheduler.
    async fn record_exit_failure(
        &self,
        claimed: &ClaimedTurnRun,
        failure_tag: &'static str,
        model_usage: Option<LoopModelUsage>,
    ) -> Result<(), TurnRunFailureMetadata> {
        let run_id = claimed.state.run_id;
        let runner_id = claimed.runner_id;
        let failure = sanitized_failure(failure_tag)
            .unwrap_or_else(|| unknown_failure_error().failure().clone());
        match self
            .loop_exit_applier
            .record_runner_failure_with_model_usage(claimed, failure, model_usage)
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
                Err(TurnRunFailureMetadata::from_optional_model_usage(
                    model_usage.or(claimed.state.model_usage),
                ))
            }
        }
    }
}

fn loop_exit_model_usage(exit: &LoopExit) -> Option<LoopModelUsage> {
    match exit {
        LoopExit::Completed(completed) => completed.model_usage,
        LoopExit::Failed(failed) => failed.model_usage,
        LoopExit::Blocked(_) | LoopExit::Cancelled(_) => None,
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
    use super::loop_exit_model_usage;
    use ironclaw_host_api::turn::LoopExitId;
    use ironclaw_loop_contracts::{LoopExit, LoopFailed, LoopFailureKind, LoopModelUsage};

    #[test]
    fn apply_exit_failure_preserves_cumulative_usage_already_on_exit() {
        let claimed = LoopModelUsage {
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
        let exit = LoopExit::Failed(LoopFailed {
            reason_kind: LoopFailureKind::ModelError,
            checkpoint_id: None,
            model_usage: LoopModelUsage::merge_optional(Some(claimed), Some(finalizer)),
            exit_id: LoopExitId::new("exit:apply-failure").expect("valid exit id"),
            explanation_message_refs: Vec::new(),
            safe_summary: None,
        });

        // The exit already owns the cumulative total. Fallback and
        // double-failure paths carry this snapshot through without trying to
        // reconstruct or stringify the consumed LoopExit.
        assert_eq!(loop_exit_model_usage(&exit), Some(expected));
    }
}
