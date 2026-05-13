//! Model-port invocation with recovery routing (master spec §10).

use ironclaw_turns::{
    LoopFailureKind,
    run_profile::{
        AgentLoopDriverHost, AgentLoopHostError, AgentLoopHostErrorKind, LoopModelRequest,
    },
};

use crate::{
    planner::AgentLoopPlanner,
    state::{CheckpointKind, LoopExecutionState},
    strategies::{ModelErrorClass, ModelErrorSummary, ModelPreference, RecoveryOutcome},
};

use super::util::MAX_RETRIES_PER_CALL;
use super::{
    AgentLoopExecutorError, CancelledKind, CanonicalAgentLoopExecutor, FailureKind, HostStage,
    LoopExit, lifecycle::failure_kind_to_exit,
};

/// Internal routing classification for `LoopModelPort` errors.
pub(super) enum ModelErrorRouting {
    Cancelled,
    StaleSurface,
    Recoverable(ModelErrorClass),
    HostUnavailable,
}

/// Outcome of `invoke_model_with_recovery`.
pub(super) enum ModelStep {
    /// Model returned a usable response; carry forward updated state.
    Response(
        LoopExecutionState,
        ironclaw_turns::run_profile::LoopModelResponse,
    ),
    /// Host reported `StaleSurface`; caller must reload capabilities and
    /// re-issue the iteration without advancing the iteration counter.
    ReloadSurface(LoopExecutionState),
    /// Recovery returned `SkipResult` for a model error. The model call is
    /// dropped, the iteration counter MUST advance on the next loop tick, and
    /// the outer loop's iteration cap / wall-clock cap eventually trips even
    /// if the underlying model port keeps failing. Distinct from
    /// `ReloadSurface`, which restarts the SAME iteration without bumping the
    /// counter (and so would spin forever when recovery always returns
    /// `SkipResult` against a persistent `Unavailable`/`Internal` model
    /// error).
    SkipIteration(LoopExecutionState),
    /// Recovery decided to abort; bubble up the loop exit.
    Exit(LoopExecutionState, LoopExit),
}

impl CanonicalAgentLoopExecutor {
    /// Issue the model call, classifying any host-port error against the
    /// runtime recovery strategy (master spec §10).
    ///
    /// - `Cancelled` from the model port: surfaced as `HostUnavailable` so
    ///   the outer cancellation-observation path runs on the next tick.
    /// - `StaleSurface`: signal the caller to reload capabilities and retry.
    /// - Transient (`Unavailable` / `Internal`): build a `ModelErrorSummary`
    ///   and consult `RecoveryStrategy::on_model_error`; honor `Retry` /
    ///   `SkipResult` / `Abort`.
    /// - Other host errors collapse to `HostUnavailable { Model }`.
    pub(super) async fn invoke_model_with_recovery(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        request: LoopModelRequest,
    ) -> Result<ModelStep, AgentLoopExecutorError> {
        // A logical model call records its failure kind in
        // `recent_failure_kinds` at most once — not once per retry attempt
        // — so an eventually-successful model turn doesn't trip
        // `DefaultStopConditionStrategy::failure_run_threshold` after a few
        // retries. Mirrors the capability retry path, which pushes once at
        // the start of `handle_capability_error` and never inside the
        // inner loop.
        let mut recorded_failure = false;
        for _ in 0..MAX_RETRIES_PER_CALL {
            match host.stream_model(request.clone()).await {
                Ok(response) => return Ok(ModelStep::Response(state, response)),
                Err(error) => match self.classify_model_host_error(&error) {
                    ModelErrorRouting::Cancelled => {
                        // Surface model-port cancellation as
                        // `LoopExit::Cancelled` (Final-checkpointed) so
                        // callers see a normal terminal state, not
                        // infrastructure failure.
                        let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                        let exit = LoopExit::Cancelled(CancelledKind {
                            interrupted_message_refs: checked.assistant_refs.clone(),
                        });
                        return Ok(ModelStep::Exit(checked, exit));
                    }
                    ModelErrorRouting::StaleSurface => {
                        return Ok(ModelStep::ReloadSurface(state));
                    }
                    ModelErrorRouting::Recoverable(class) => {
                        let summary = model_error_summary(class, &error);
                        if !recorded_failure {
                            state.recent_failure_kinds.push(LoopFailureKind::ModelError);
                            recorded_failure = true;
                        }
                        let outcome = planner.recovery().on_model_error(&state, &summary).await;
                        match outcome {
                            RecoveryOutcome::Retry { recovery, alter } => {
                                state.recovery_state = recovery;
                                if matches!(
                                    alter,
                                    Some(crate::strategies::RetryAlteration::AdvanceFallback)
                                ) {
                                    return Ok(ModelStep::Exit(
                                        state,
                                        LoopExit::Failed {
                                            kind: FailureKind::Other(LoopFailureKind::DriverBug),
                                        },
                                    ));
                                }
                                // Honor `Backoff` delay before retry.
                                if let Some(crate::strategies::RetryAlteration::Backoff { delay }) =
                                    alter
                                {
                                    tokio::time::sleep(delay).await;
                                }
                                continue;
                            }
                            RecoveryOutcome::SkipResult { recovery } => {
                                // SkipResult on a model error means "drop
                                // this turn AND advance the iteration".
                                // Routing through `ReloadSurface` instead
                                // would restart the same tick without
                                // bumping `state.iteration`, so a
                                // persistent model failure under a
                                // SkipResult-returning recovery would
                                // spin forever past the iteration and
                                // wall-clock caps. `SkipIteration` is the
                                // explicit monotonic-progress variant.
                                state.recovery_state = recovery;
                                return Ok(ModelStep::SkipIteration(state));
                            }
                            RecoveryOutcome::Abort {
                                recovery,
                                failure_kind,
                            } => {
                                state.recovery_state = recovery;
                                return Ok(ModelStep::Exit(
                                    state,
                                    LoopExit::Failed {
                                        kind: failure_kind_to_exit(failure_kind),
                                    },
                                ));
                            }
                        }
                    }
                    ModelErrorRouting::HostUnavailable => {
                        return Err(AgentLoopExecutorError::HostUnavailable {
                            stage: HostStage::Model,
                        });
                    }
                },
            }
        }
        // Defense-in-depth: a custom recovery strategy returned `Retry` more
        // than `MAX_RETRIES_PER_CALL` times.
        Ok(ModelStep::Exit(
            state,
            LoopExit::Failed {
                kind: FailureKind::Other(LoopFailureKind::DriverBug),
            },
        ))
    }

    pub(super) fn classify_model_host_error(
        &self,
        error: &AgentLoopHostError,
    ) -> ModelErrorRouting {
        match error.kind {
            AgentLoopHostErrorKind::Cancelled => ModelErrorRouting::Cancelled,
            AgentLoopHostErrorKind::StaleSurface => ModelErrorRouting::StaleSurface,
            AgentLoopHostErrorKind::Unavailable => {
                ModelErrorRouting::Recoverable(ModelErrorClass::Unavailable)
            }
            AgentLoopHostErrorKind::Internal => {
                ModelErrorRouting::Recoverable(ModelErrorClass::Internal)
            }
            AgentLoopHostErrorKind::BudgetExceeded => {
                ModelErrorRouting::Recoverable(ModelErrorClass::ContextOverflow)
            }
            AgentLoopHostErrorKind::Unauthorized
            | AgentLoopHostErrorKind::ScopeMismatch
            | AgentLoopHostErrorKind::InvalidInvocation
            | AgentLoopHostErrorKind::PolicyDenied
            | AgentLoopHostErrorKind::CheckpointRejected
            | AgentLoopHostErrorKind::TranscriptWriteFailed => ModelErrorRouting::HostUnavailable,
        }
    }
}

pub(super) fn model_preference_id(
    preference: ModelPreference,
) -> Result<Option<ironclaw_turns::ModelProfileId>, AgentLoopExecutorError> {
    match preference {
        ModelPreference::Primary => Ok(None),
        ModelPreference::Fallback { .. } => Err(AgentLoopExecutorError::PlannerContract {
            detail: "fallback model preference requires model route chain support",
        }),
    }
}

pub(super) fn model_error_summary(
    class: ModelErrorClass,
    error: &AgentLoopHostError,
) -> ModelErrorSummary {
    ModelErrorSummary {
        class,
        safe_summary: error.safe_summary.clone(),
        diagnostic_ref: error.diagnostic_ref.clone(),
    }
}

/// Synthesize a `Transient` `ModelErrorSummary` representing the
/// "host kept returning `StaleSurface` past the per-tick cap" condition.
/// The classification matches what a transient model port failure would
/// produce, so `RecoveryStrategy::on_model_error` can apply its standard
/// per-class budget without a stale-surface-specific code path.
pub(super) fn synthesize_stale_surface_summary() -> ModelErrorSummary {
    ModelErrorSummary {
        class: ModelErrorClass::Transient,
        safe_summary: "host repeatedly reported stale capability surface".to_string(),
        diagnostic_ref: None,
    }
}
