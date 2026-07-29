//! `RecoveryStrategy` — decides what to do when a capability call OR a model
//! call fails with a (sanitized) error summary.
//!
//! Mutates `recovery_state` (attempt counters, fallback advance bookkeeping).
//! Async because future strategies may consult host state for circuit-breaker
//! counters, route health, etc.
//!
//! Strategies never see raw provider errors, host paths, or secrets;
//! sanitization happens at the host port.

// arch-exempt: large_file, keep recovery policy beside its exhaustive mapping tests until the item-7 conformance-matrix extraction, plan #6284

use async_trait::async_trait;
use ironclaw_host_api::{FailureFate, FailureKind};
use ironclaw_turns::{
    LoopFailureKind, ModelInvalidOutputDetailReason, run_profile::LoopSafeSummary,
};

use crate::state::{
    LoopExecutionState, ModelErrorObservationClass, ModelErrorRecoveryObservation,
    RecoveryAttemptClass, RecoveryStrategyState,
};

/// Decides what to do when a capability call OR a model call fails with a
/// (sanitized) error summary.
///
/// `&self` only — strategies are value-immutable. The new `recovery_state`
/// slot value is carried in the returned [`RecoveryOutcome`]; the executor
/// swaps it into the next whole state.
#[async_trait]
pub(crate) trait RecoveryStrategy: Send + Sync {
    async fn on_capability_error(
        &self,
        state: &LoopExecutionState,
        err: &CapabilityErrorSummary,
    ) -> RecoveryOutcome;

    async fn on_model_error(
        &self,
        state: &LoopExecutionState,
        err: &ModelErrorSummary,
    ) -> RecoveryOutcome;

    /// Ceiling on total model-call attempts the executor allows within a
    /// single model stage before treating continued `Retry` outcomes as a
    /// strategy contract bug and failing the run.
    ///
    /// Implementations MUST return a value large enough that every call-scope
    /// retry budget they can grant reaches their own `Abort` decision — the
    /// executor derives its retry-loop bound from this method, so an
    /// undersized value silently truncates the strategy's budget and loses
    /// its abort diagnostics. The default is a margin above the built-in
    /// per-class budget for strategies that abort quickly.
    fn max_total_model_attempts(&self) -> u32 {
        16
    }
}

/// Compile-time object-safety check.
#[allow(dead_code)]
fn _recovery_strategy_object_safe(_: &dyn RecoveryStrategy) {}

/// Sanitized, strategy-visible error summary text.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(transparent)]
pub(crate) struct SanitizedStrategySummary(String);

impl SanitizedStrategySummary {
    pub(crate) fn new(summary: impl Into<String>) -> Result<Self, String> {
        let summary = summary.into();
        LoopSafeSummary::new(summary.clone()).map(|_| Self(summary))
    }

    pub(crate) fn from_trusted_static(summary: &'static str) -> Self {
        // Invariant: callers pass reviewed hard-coded summaries, so failure
        // here is a programming error in a literal rather than runtime input.
        match Self::new(summary) {
            Ok(summary) => summary,
            Err(reason) => panic!("invalid trusted static strategy summary: {reason}"),
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_inner(self) -> String {
        self.0
    }
}

impl<'de> serde::Deserialize<'de> for SanitizedStrategySummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let summary = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(summary).map_err(serde::de::Error::custom)
    }
}

/// Sanitized capability error — the unified [`FailureKind`] plus a safe
/// summary string. Strategies never see raw
/// provider errors, host paths, or secrets; sanitization happens at the host
/// port boundary before recovery strategy code runs.
///
/// The retired loop-private `CapabilityErrorClass` re-declared the
/// recoverability domain beside the host's; the summary now carries the
/// unified kind and decision sites ask [`FailureKind::fate`] instead.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CapabilityErrorSummary {
    pub(crate) kind: FailureKind,
    pub(crate) safe_summary: SanitizedStrategySummary,
}

/// Sanitized model error — class + safe summary.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ModelErrorSummary {
    pub(crate) class: ModelErrorClass,
    pub(crate) safe_summary: SanitizedStrategySummary,
}

/// Wire-stable model error classification.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModelErrorClass {
    /// Retryable model/provider failure such as timeout or temporary outage.
    Transient,
    /// Prompt/context exceeded the selected model's limits.
    ContextOverflow,
    /// Provider rejected or filtered the content.
    ContentFiltered,
    /// Provider/model output was structurally invalid for the loop contract.
    InvalidOutput,
    /// Model route, credentials, or provider is unavailable.
    Unavailable,
    /// Model gateway failed internally without safe caller detail.
    Internal,
    /// The model request no longer matches the host's current state (stale
    /// capability surface version, mismatched or missing prompt bundle).
    /// Model-fixable by rebuild: an iteration-scoped retry re-derives the
    /// surface and prompt bundle.
    StaleRequest,
    /// The host rejected the model call as unauthorized. Terminal with a
    /// precise credentials category; never silently retried.
    Unauthorized,
    /// The host rejected the model stage's checkpoint interaction. Terminal
    /// with the precise `checkpoint_rejected` category.
    CheckpointRejected,
    /// The host could not persist transcript output for the model stage.
    /// Terminal with the precise `transcript_write_failed` category.
    TranscriptWriteFailed,
}

/// Strategy decision plus the new `recovery_state` slot value.
///
/// Variants:
/// - `Retry` — re-issue (the executor decides whether call-level or
///   iteration-level retry from `scope`; `alter` carries the strategy's
///   prompt/model hint).
/// - `ToolErrorResult` — append a model-visible tool error result and continue
///   the capability batch.
/// - `Abort` — return `LoopExit::Failed { reason_kind: failure_kind }`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub(crate) enum RecoveryOutcome {
    Retry {
        recovery: RecoveryStrategyState,
        scope: RetryScope,
        alter: Option<RetryAlteration>,
    },
    ToolErrorResult {
        recovery: RecoveryStrategyState,
    },
    /// Retry once with a typed, host-authored model-error observation after
    /// the ordinary per-class retry budget has been exhausted.
    ModelErrorObservation {
        recovery: RecoveryStrategyState,
        scope: RetryScope,
        alter: Option<RetryAlteration>,
        observation: ModelErrorRecoveryObservation,
    },
    Abort {
        recovery: RecoveryStrategyState,
        failure_kind: LoopFailureKind,
    },
}

/// Where the executor should apply a retry outcome.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RetryScope {
    /// Retry only the capability call or model call that produced the error.
    Call,
    /// Re-run the current loop iteration after rebuilding iteration context.
    Iteration,
}

/// Reference baseline `RecoveryStrategy`: bounded retry per error class with
/// exponential backoff.
///
/// This strategy:
/// - Turns `PolicyDenied`, `InputInvalid`, and `OperationFailed` into
///   model-visible tool error results without consuming retry budget. The
///   operation-failed class includes ordinary tool failures such as HTTP
///   network errors and output-size limits so the model can explain the
///   failure or choose a different approach.
/// - Aborts immediately on capability `Permanent` errors. A model
///   `ContentFiltered` error gets one typed observation-assisted rephrase
///   attempt before aborting.
/// - Retries capability transient, unavailable, and internal errors up to
///   [`Self::max_attempts_per_class`] times with `Backoff`, then returns a
///   model-visible tool error result.
/// - Retries model invalid-output errors up to the same budget, then gives the
///   model one typed observation-assisted repair attempt before aborting.
/// - Retries model transient, unavailable, and internal errors on the much
///   deeper [`Self::max_model_availability_attempts`] budget with a
///   longer-capped backoff schedule, then aborts the run. Provider outages
///   (5xx storms) routinely outlast a couple of quick retries; a long-running
///   agentic turn must ride them out rather than discard all prior work.
/// - Retries `ContextOverflow` at iteration scope with `ShrinkContext`, then
///   gives the compacted prompt one observation-assisted attempt before aborting.
/// - Retries `StaleRequest` at iteration scope (rebuilding the capability
///   surface and prompt bundle) up to [`Self::max_attempts_per_class`] times,
///   then aborts the run with the precise `model_stale_request` category.
/// - Aborts immediately on `Unauthorized`, `CheckpointRejected`, and
///   `TranscriptWriteFailed` — precise, user-actionable terminal categories
///   that must never be silently retried.
#[derive(Debug, Clone, Copy)]
pub struct DefaultRecoveryStrategy {
    /// Max retries per error class before giving up. Default `2`.
    pub max_attempts_per_class: u32,
    /// Max consecutive retries for availability-class model errors
    /// (transient / unavailable / internal) before aborting the run.
    /// Default `12`, which with [`availability_backoff_for`] rides out
    /// roughly seven minutes of sustained provider failure.
    pub max_model_availability_attempts: u32,
}

impl Default for DefaultRecoveryStrategy {
    fn default() -> Self {
        Self {
            max_attempts_per_class: 2,
            max_model_availability_attempts: 12,
        }
    }
}

#[async_trait]
impl RecoveryStrategy for DefaultRecoveryStrategy {
    async fn on_capability_error(
        &self,
        state: &LoopExecutionState,
        err: &CapabilityErrorSummary,
    ) -> RecoveryOutcome {
        // Wildcard-free by construction: the four `FailureFate` arms are
        // exhaustive, and `FailureKind::fate` is itself wildcard-free over the
        // full kind vocabulary — a new kind refuses to compile until its fate
        // is chosen at the enum, not silently dispositioned here.
        match err.kind.fate() {
            // The model sees the failure as a tool error and routes around it.
            // `Park` (auth-required) reaches this path only as a failure
            // summary — the actual gate parks via `Resolution::Blocked` — so
            // it surfaces model-visibly rather than ending the run.
            FailureFate::ModelVisible | FailureFate::Park => RecoveryOutcome::ToolErrorResult {
                recovery: state.recovery_state.cleared_attempts(),
            },
            // Cancellation is the only terminal kind; abort as before.
            FailureFate::Terminal => RecoveryOutcome::Abort {
                recovery: state.recovery_state.cleared_attempts(),
                failure_kind: capability_error_to_failure_kind(err.kind),
            },
            FailureFate::Retry => {
                let Some(attempt_class) = capability_retry_attempt_class(err.kind) else {
                    // Unreachable: every Retry-fated kind has an attempt class
                    // (pinned by test). Kept as a loud driver bug, not a
                    // silent disposition.
                    return RecoveryOutcome::Abort {
                        recovery: state.recovery_state.cleared_attempts(),
                        failure_kind: LoopFailureKind::DriverBug,
                    };
                };
                retry_or_capability_tool_error(
                    state,
                    attempt_class,
                    self.max_attempts_per_class,
                    RetryScope::Call,
                    |attempts| {
                        Some(RetryAlteration::Backoff {
                            delay_ms: backoff_for(attempts),
                        })
                    },
                )
            }
        }
    }

    async fn on_model_error(
        &self,
        state: &LoopExecutionState,
        err: &ModelErrorSummary,
    ) -> RecoveryOutcome {
        let kind = model_error_to_failure_kind(err.class);
        match err.class {
            // Unauthorized/checkpoint/transcript kinds carry precise
            // user-actionable categories and must not be silently retried.
            ModelErrorClass::Unauthorized
            | ModelErrorClass::CheckpointRejected
            | ModelErrorClass::TranscriptWriteFailed => RecoveryOutcome::Abort {
                recovery: state.recovery_state.cleared_attempts(),
                failure_kind: kind,
            },
            ModelErrorClass::ContentFiltered => observe_once_or_abort(
                state,
                RetryScope::Call,
                ModelErrorRecoveryObservation::content_filtered(),
            ),
            ModelErrorClass::StaleRequest => {
                let Some(attempt_class) = model_retry_attempt_class(err.class) else {
                    return RecoveryOutcome::Abort {
                        recovery: state.recovery_state.cleared_attempts(),
                        failure_kind: LoopFailureKind::DriverBug,
                    };
                };
                // Iteration scope so the executor rebuilds the capability
                // surface and prompt bundle — the stale input — before the
                // next model call. No backoff: the rebuild itself is the fix.
                retry_or_abort(
                    state,
                    attempt_class,
                    self.max_attempts_per_class,
                    kind,
                    RetryScope::Iteration,
                    |_| None,
                )
            }
            ModelErrorClass::ContextOverflow => retry_observe_or_abort(
                state,
                self.max_attempts_per_class,
                RetryScope::Iteration,
                |_| Some(RetryAlteration::ShrinkContext),
                ModelErrorRecoveryObservation::context_overflow(),
            ),
            ModelErrorClass::InvalidOutput => {
                let reason =
                    ModelInvalidOutputDetailReason::from_safe_summary(err.safe_summary.as_str());
                retry_observe_or_abort(
                    state,
                    self.max_attempts_per_class,
                    RetryScope::Call,
                    |_| Some(RetryAlteration::RepairInvalidModelOutput),
                    ModelErrorRecoveryObservation::invalid_output(reason),
                )
            }
            ModelErrorClass::Transient
            | ModelErrorClass::Unavailable
            | ModelErrorClass::Internal => {
                let Some(attempt_class) = model_retry_attempt_class(err.class) else {
                    return RecoveryOutcome::Abort {
                        recovery: state.recovery_state.cleared_attempts(),
                        failure_kind: LoopFailureKind::DriverBug,
                    };
                };
                retry_or_abort(
                    state,
                    attempt_class,
                    self.max_model_availability_attempts,
                    kind,
                    RetryScope::Call,
                    |attempts| {
                        Some(RetryAlteration::Backoff {
                            delay_ms: availability_backoff_for(attempts),
                        })
                    },
                )
            }
        }
    }

    fn max_total_model_attempts(&self) -> u32 {
        // Upper bound on model calls one stage can legitimately issue before
        // some class reaches its own Abort: the initial call, plus call-scope
        // invalid-output retries (`max_attempts_per_class`) and its one
        // observation-assisted repair, plus the one content-filter
        // observation, plus an
        // availability budget for each availability class (transient /
        // unavailable / internal — attempts are tracked per class, so a
        // pathological host can rotate through all three). Context-overflow
        // retries are iteration-scoped and leave the stage, so they don't
        // consume this loop. The final +1 keeps the strategy's Abort — with
        // its failure kind and diagnostics — strictly inside the loop bound.
        3u32.saturating_add(self.max_attempts_per_class)
            .saturating_add(self.max_model_availability_attempts.saturating_mul(3))
    }
}

fn retry_or_abort(
    state: &LoopExecutionState,
    attempt_class: RecoveryAttemptClass,
    max_attempts_per_class: u32,
    failure_kind: LoopFailureKind,
    scope: RetryScope,
    alteration: impl FnOnce(u32) -> Option<RetryAlteration>,
) -> RecoveryOutcome {
    let attempts = state.recovery_state.attempts_for(attempt_class);
    let next = state
        .recovery_state
        .with_incremented_attempts_for(attempt_class);
    if attempts >= max_attempts_per_class {
        RecoveryOutcome::Abort {
            recovery: next,
            failure_kind,
        }
    } else {
        RecoveryOutcome::Retry {
            recovery: next,
            scope,
            alter: alteration(attempts),
        }
    }
}

fn retry_observe_or_abort(
    state: &LoopExecutionState,
    max_attempts_per_class: u32,
    scope: RetryScope,
    alteration: impl FnOnce(u32) -> Option<RetryAlteration>,
    observation: ModelErrorRecoveryObservation,
) -> RecoveryOutcome {
    let observation_class = observation.class();
    let model_error_class = model_error_class_for_observation(observation_class);
    let Some(attempt_class) = model_retry_attempt_class(model_error_class) else {
        return RecoveryOutcome::Abort {
            recovery: state.recovery_state.cleared_attempts(),
            failure_kind: LoopFailureKind::DriverBug,
        };
    };
    let failure_kind = model_error_to_failure_kind(model_error_class);
    let attempts = state.recovery_state.attempts_for(attempt_class);
    let next = state
        .recovery_state
        .with_incremented_attempts_for(attempt_class);
    if attempts < max_attempts_per_class {
        return RecoveryOutcome::Retry {
            recovery: next,
            scope,
            alter: alteration(attempts),
        };
    }
    if !state
        .recovery_state
        .observation_attempted_for(observation_class)
    {
        return RecoveryOutcome::ModelErrorObservation {
            recovery: next.with_observation_attempted_for(observation_class),
            scope,
            alter: alteration(attempts),
            observation,
        };
    }
    RecoveryOutcome::Abort {
        recovery: next,
        failure_kind,
    }
}

fn observe_once_or_abort(
    state: &LoopExecutionState,
    scope: RetryScope,
    observation: ModelErrorRecoveryObservation,
) -> RecoveryOutcome {
    let observation_class = observation.class();
    let model_error_class = model_error_class_for_observation(observation_class);
    let failure_kind = model_error_to_failure_kind(model_error_class);
    if state
        .recovery_state
        .observation_attempted_for(observation_class)
    {
        return RecoveryOutcome::Abort {
            recovery: state.recovery_state.clone(),
            failure_kind,
        };
    }
    RecoveryOutcome::ModelErrorObservation {
        recovery: state
            .recovery_state
            .with_observation_attempted_for(observation_class),
        scope,
        alter: None,
        observation,
    }
}

fn model_error_class_for_observation(class: ModelErrorObservationClass) -> ModelErrorClass {
    match class {
        ModelErrorObservationClass::ContextOverflow => ModelErrorClass::ContextOverflow,
        ModelErrorObservationClass::InvalidOutput => ModelErrorClass::InvalidOutput,
        ModelErrorObservationClass::ContentFiltered => ModelErrorClass::ContentFiltered,
    }
}

fn retry_or_capability_tool_error(
    state: &LoopExecutionState,
    attempt_class: RecoveryAttemptClass,
    max_attempts_per_class: u32,
    scope: RetryScope,
    alteration: impl FnOnce(u32) -> Option<RetryAlteration>,
) -> RecoveryOutcome {
    let attempts = state.recovery_state.attempts_for(attempt_class);
    let next = state
        .recovery_state
        .with_incremented_attempts_for(attempt_class);
    if attempts >= max_attempts_per_class {
        RecoveryOutcome::ToolErrorResult {
            recovery: next.cleared_attempts(),
        }
    } else {
        RecoveryOutcome::Retry {
            recovery: next,
            scope,
            alter: alteration(attempts),
        }
    }
}

/// Checkpoint-serialized retry-budget bucket for a Retry-fated kind.
/// Wildcard-free: a new kind must deliberately choose a bucket (or `None`)
/// here. `None` for every non-Retry fate — those never consume retry budget.
fn capability_retry_attempt_class(kind: FailureKind) -> Option<RecoveryAttemptClass> {
    match kind {
        FailureKind::Network | FailureKind::Transient => {
            Some(RecoveryAttemptClass::CapabilityTransient)
        }
        FailureKind::Backend | FailureKind::Unavailable => {
            Some(RecoveryAttemptClass::CapabilityUnavailable)
        }
        FailureKind::Internal => Some(RecoveryAttemptClass::CapabilityInternal),
        FailureKind::MethodMissing
        | FailureKind::UndeclaredCapability
        | FailureKind::UnknownCapability
        | FailureKind::UnknownProvider
        | FailureKind::InputEncode
        | FailureKind::OperationFailed
        | FailureKind::OutputTooLarge
        | FailureKind::Resource
        | FailureKind::PolicyDenied
        | FailureKind::NetworkDenied
        | FailureKind::FilesystemDenied
        | FailureKind::SecretDenied
        | FailureKind::Authorization
        | FailureKind::StaleSurface
        | FailureKind::GateDeclined
        | FailureKind::Guest
        | FailureKind::ExitFailure
        | FailureKind::OutputDecode
        | FailureKind::InvalidResult
        | FailureKind::Memory
        | FailureKind::Manifest
        | FailureKind::ExtensionRuntimeMismatch
        | FailureKind::RuntimeMismatch
        | FailureKind::MissingRuntimeBackend
        | FailureKind::UnsupportedRunner
        | FailureKind::MissingRuntime
        | FailureKind::Client
        | FailureKind::Executor
        | FailureKind::Unclassified
        | FailureKind::AuthRequired
        | FailureKind::Cancelled => None,
    }
}

fn model_retry_attempt_class(class: ModelErrorClass) -> Option<RecoveryAttemptClass> {
    match class {
        ModelErrorClass::Transient => Some(RecoveryAttemptClass::ModelTransient),
        ModelErrorClass::ContextOverflow => Some(RecoveryAttemptClass::ModelContextOverflow),
        ModelErrorClass::InvalidOutput => Some(RecoveryAttemptClass::ModelInvalidOutput),
        ModelErrorClass::Unavailable => Some(RecoveryAttemptClass::ModelUnavailable),
        ModelErrorClass::Internal => Some(RecoveryAttemptClass::ModelInternal),
        ModelErrorClass::StaleRequest => Some(RecoveryAttemptClass::ModelStaleRequest),
        ModelErrorClass::ContentFiltered
        | ModelErrorClass::Unauthorized
        | ModelErrorClass::CheckpointRejected
        | ModelErrorClass::TranscriptWriteFailed => None,
    }
}

/// Maps a unified capability [`FailureKind`] to the loop-level failure kind
/// the executor surfaces in `LoopExit::Failed { reason_kind }` for terminal
/// reporting. The single surviving copy of what used to be three byte-identical
/// mappings (executor mapping, recovery abort, exhausted-retry exit).
///
/// Wildcard-free: a new kind must be classified here deliberately. Preserves
/// the retired class semantics — policy-shaped kinds report `PolicyDenied`,
/// model-input-shaped kinds report `ModelError`, everything else reports the
/// capability protocol failure.
pub(crate) fn capability_error_to_failure_kind(kind: FailureKind) -> LoopFailureKind {
    match kind {
        // The model shaped the input wrongly — a model error, not a protocol one.
        FailureKind::InputEncode => LoopFailureKind::ModelError,
        // Policy-shaped denials (including the specific `*Denied` causes and
        // the auth-shaped kinds, which the retired vocabulary coarsened to
        // `Authorization`/`PolicyDenied`).
        FailureKind::PolicyDenied
        | FailureKind::NetworkDenied
        | FailureKind::FilesystemDenied
        | FailureKind::SecretDenied
        | FailureKind::Authorization
        | FailureKind::GateDeclined
        | FailureKind::AuthRequired => LoopFailureKind::PolicyDenied,
        // Every remaining kind maps to the protocol-error failure, enumerated
        // explicitly so a new kind cannot silently inherit this terminal fate.
        FailureKind::MethodMissing
        | FailureKind::UndeclaredCapability
        | FailureKind::UnknownCapability
        | FailureKind::UnknownProvider
        | FailureKind::OperationFailed
        | FailureKind::OutputTooLarge
        | FailureKind::Resource
        | FailureKind::StaleSurface
        | FailureKind::Network
        | FailureKind::Transient
        | FailureKind::Unavailable
        | FailureKind::Backend
        | FailureKind::Internal
        | FailureKind::Guest
        | FailureKind::ExitFailure
        | FailureKind::OutputDecode
        | FailureKind::InvalidResult
        | FailureKind::Memory
        | FailureKind::Manifest
        | FailureKind::ExtensionRuntimeMismatch
        | FailureKind::RuntimeMismatch
        | FailureKind::MissingRuntimeBackend
        | FailureKind::UnsupportedRunner
        | FailureKind::MissingRuntime
        | FailureKind::Client
        | FailureKind::Executor
        | FailureKind::Unclassified
        | FailureKind::Cancelled => LoopFailureKind::CapabilityProtocolError,
    }
}

/// Maps a sanitized model error class to the loop-level failure kind.
pub(crate) fn model_error_to_failure_kind(class: ModelErrorClass) -> LoopFailureKind {
    match class {
        ModelErrorClass::Transient
        | ModelErrorClass::ContextOverflow
        | ModelErrorClass::ContentFiltered
        | ModelErrorClass::Unavailable
        | ModelErrorClass::Internal
        | ModelErrorClass::StaleRequest
        | ModelErrorClass::Unauthorized => LoopFailureKind::ModelError,
        ModelErrorClass::InvalidOutput => LoopFailureKind::InvalidModelOutput,
        ModelErrorClass::CheckpointRejected => LoopFailureKind::CheckpointRejected,
        ModelErrorClass::TranscriptWriteFailed => LoopFailureKind::TranscriptWriteFailed,
    }
}

/// Exponential backoff for retry attempts: `250ms x 2^attempt`, capped at 5s.
///
/// Strictly monotonic in `attempt` until the 5s cap kicks in. The executor
/// honors this as a sleep before re-issuing the call.
fn backoff_for(attempt: u32) -> BackoffDelayMs {
    let shift = attempt.min(5);
    let ms = 250u64.saturating_mul(1u64 << shift);
    BackoffDelayMs(ms.min(5_000))
}

/// Backoff for availability-class model errors: `1s x 2^attempt`, capped at
/// [`BackoffDelayMs::MAX_DELAY_MS`] (60s).
///
/// Paired with `max_model_availability_attempts`, the cumulative schedule
/// (1+2+4+8+16+32+60·k seconds) rides out a multi-minute provider outage
/// instead of aborting the run while the provider recovers.
fn availability_backoff_for(attempt: u32) -> BackoffDelayMs {
    let shift = attempt.min(6);
    let ms = 1_000u64.saturating_mul(1u64 << shift);
    BackoffDelayMs(ms.min(BackoffDelayMs::MAX_DELAY_MS))
}

/// Strategy hint about WHAT to alter on retry. Prompt-shape alteration is
/// supported; model-route swap is reserved for future fallback routing.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "alteration")]
pub(crate) enum RetryAlteration {
    /// Shrink context for the next attempt (e.g. on context-overflow).
    ShrinkContext,
    /// Backoff before retry (executor honors as a sleep).
    Backoff { delay_ms: BackoffDelayMs },
    /// Rebuild the next model prompt with a model-visible invalid-output repair
    /// hint. Used when the provider/model returned an empty or structurally
    /// invalid response for the active loop contract.
    RepairInvalidModelOutput,
    /// Reserved for future `ModelRouteChain` landing. Skeleton executor MUST
    /// reject this alteration with `LoopFailureKind::DriverBug` until the
    /// chain mechanism lands.
    AdvanceFallback,
}

/// Bounded retry backoff delay in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BackoffDelayMs(u64);

impl BackoffDelayMs {
    pub(crate) const MAX_DELAY_MS: u64 = 60_000;

    pub(crate) fn new(delay_ms: u64) -> Result<Self, String> {
        if delay_ms <= Self::MAX_DELAY_MS {
            Ok(Self(delay_ms))
        } else {
            Err(format!(
                "backoff delay {delay_ms}ms exceeds max {}ms",
                Self::MAX_DELAY_MS
            ))
        }
    }

    pub(crate) fn as_u64(self) -> u64 {
        self.0
    }
}

impl serde::Serialize for BackoffDelayMs {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u64(self.0)
    }
}

impl<'de> serde::Deserialize<'de> for BackoffDelayMs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let delay_ms = <u64 as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(delay_ms).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_recovery() -> RecoveryStrategyState {
        RecoveryStrategyState::with_attempts_for(RecoveryAttemptClass::ModelTransient, 2)
    }

    #[test]
    fn sanitized_strategy_summary_serializes_as_string() {
        let summary = SanitizedStrategySummary::new("provider unavailable").expect("valid");
        let value = serde_json::to_value(&summary).expect("serialize");
        assert_eq!(value, serde_json::json!("provider unavailable"));
        let restored: SanitizedStrategySummary =
            serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored.as_str(), "provider unavailable");
    }

    #[test]
    fn sanitized_strategy_summary_rejects_unsafe_dynamic_values() {
        assert!(SanitizedStrategySummary::new("").is_err());
        assert!(SanitizedStrategySummary::new("/Users/alice/.ssh/id_rsa").is_err());
        assert!(SanitizedStrategySummary::new("provider returned sk-live-secret").is_err());
        assert!(SanitizedStrategySummary::new("a".repeat(513)).is_err());
    }

    #[test]
    fn sanitized_strategy_summary_validates_during_deserialization() {
        for unsafe_summary in [
            "",
            "/Users/alice/.ssh/id_rsa",
            "provider returned sk-live-secret",
        ] {
            let result = serde_json::from_value::<SanitizedStrategySummary>(serde_json::json!(
                unsafe_summary
            ));
            assert!(result.is_err(), "accepted unsafe summary: {unsafe_summary}");
        }

        let oversized = "a".repeat(513);
        let result =
            serde_json::from_value::<SanitizedStrategySummary>(serde_json::json!(oversized));
        assert!(result.is_err(), "accepted oversized summary");
    }

    #[test]
    fn model_error_class_round_trips_snake_case() {
        for (variant, wire) in [
            (ModelErrorClass::Transient, "transient"),
            (ModelErrorClass::ContextOverflow, "context_overflow"),
            (ModelErrorClass::ContentFiltered, "content_filtered"),
            (ModelErrorClass::Unavailable, "unavailable"),
            (ModelErrorClass::Internal, "internal"),
            (ModelErrorClass::StaleRequest, "stale_request"),
            (ModelErrorClass::Unauthorized, "unauthorized"),
            (ModelErrorClass::CheckpointRejected, "checkpoint_rejected"),
            (
                ModelErrorClass::TranscriptWriteFailed,
                "transcript_write_failed",
            ),
        ] {
            let value = serde_json::to_value(variant).expect("serialize");
            assert_eq!(value, serde_json::json!(wire));
            let restored: ModelErrorClass = serde_json::from_value(value).expect("deserialize");
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn capability_error_summary_round_trips() {
        let summary = CapabilityErrorSummary {
            kind: FailureKind::Transient,
            safe_summary: SanitizedStrategySummary::new("upstream timed out").expect("valid"),
        };
        let value = serde_json::to_value(&summary).expect("serialize");
        assert_eq!(
            value["safe_summary"],
            serde_json::json!("upstream timed out")
        );
        let restored: CapabilityErrorSummary = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, summary);
        assert_eq!(restored.safe_summary.as_str(), "upstream timed out");
    }

    #[test]
    fn model_error_summary_round_trips() {
        let summary = ModelErrorSummary {
            class: ModelErrorClass::ContextOverflow,
            safe_summary: SanitizedStrategySummary::new("context window exceeded").expect("valid"),
        };
        let value = serde_json::to_value(&summary).expect("serialize");
        assert_eq!(
            value["safe_summary"],
            serde_json::json!("context window exceeded")
        );
        let restored: ModelErrorSummary = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, summary);
        assert_eq!(restored.safe_summary.as_str(), "context window exceeded");
    }

    #[test]
    fn retry_scope_round_trips_snake_case() {
        for (variant, wire) in [
            (RetryScope::Call, "call"),
            (RetryScope::Iteration, "iteration"),
        ] {
            let value = serde_json::to_value(variant).expect("serialize");
            assert_eq!(value, serde_json::json!(wire));
            let restored: RetryScope = serde_json::from_value(value).expect("deserialize");
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn backoff_delay_ms_accepts_bounded_values_and_serializes_as_number() {
        let delay = BackoffDelayMs::new(250).expect("valid");
        assert_eq!(delay.as_u64(), 250);
        let value = serde_json::to_value(delay).expect("serialize");
        assert_eq!(value, serde_json::json!(250));
        let restored: BackoffDelayMs = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, delay);
    }

    #[test]
    fn backoff_delay_ms_rejects_values_above_max() {
        let too_large = BackoffDelayMs::MAX_DELAY_MS + 1;
        assert!(BackoffDelayMs::new(too_large).is_err());
        let result = serde_json::from_value::<BackoffDelayMs>(serde_json::json!(too_large));
        assert!(result.is_err());
    }

    #[test]
    fn retry_alteration_shrink_context_round_trips() {
        let alteration = RetryAlteration::ShrinkContext;
        let value = serde_json::to_value(&alteration).expect("serialize");
        let restored: RetryAlteration = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, alteration);
    }

    #[test]
    fn retry_alteration_backoff_round_trips() {
        let alteration = RetryAlteration::Backoff {
            delay_ms: BackoffDelayMs::new(250).expect("valid"),
        };
        let value = serde_json::to_value(&alteration).expect("serialize");
        assert_eq!(value["delay_ms"], serde_json::json!(250));
        let restored: RetryAlteration = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, alteration);
        match restored {
            RetryAlteration::Backoff { delay_ms } => {
                assert_eq!(delay_ms.as_u64(), 250)
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn retry_alteration_advance_fallback_round_trips() {
        let alteration = RetryAlteration::AdvanceFallback;
        let value = serde_json::to_value(&alteration).expect("serialize");
        let restored: RetryAlteration = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, alteration);
    }

    #[test]
    fn retry_alteration_repair_invalid_model_output_round_trips() {
        let alteration = RetryAlteration::RepairInvalidModelOutput;
        let value = serde_json::to_value(&alteration).expect("serialize");
        assert_eq!(
            value,
            serde_json::json!({"alteration": "repair_invalid_model_output"})
        );
        let restored: RetryAlteration = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, alteration);
    }

    #[test]
    fn recovery_outcome_retry_carries_recovery_slot_and_optional_alteration() {
        let outcome = RecoveryOutcome::Retry {
            recovery: sample_recovery(),
            scope: RetryScope::Call,
            alter: Some(RetryAlteration::ShrinkContext),
        };
        let value = serde_json::to_value(&outcome).expect("serialize");
        let restored: RecoveryOutcome = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, outcome);
        match restored {
            RecoveryOutcome::Retry {
                recovery,
                scope,
                alter,
            } => {
                assert_eq!(recovery, sample_recovery());
                assert_eq!(scope, RetryScope::Call);
                assert_eq!(alter, Some(RetryAlteration::ShrinkContext));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn recovery_outcome_tool_error_result_carries_recovery_slot() {
        let outcome = RecoveryOutcome::ToolErrorResult {
            recovery: sample_recovery(),
        };
        let value = serde_json::to_value(&outcome).expect("serialize");
        let restored: RecoveryOutcome = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, outcome);
        match restored {
            RecoveryOutcome::ToolErrorResult { recovery } => {
                assert_eq!(recovery, sample_recovery())
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn recovery_outcome_abort_carries_recovery_slot_and_failure_kind() {
        let outcome = RecoveryOutcome::Abort {
            recovery: sample_recovery(),
            failure_kind: LoopFailureKind::NoProgressDetected,
        };
        let value = serde_json::to_value(&outcome).expect("serialize");
        let restored: RecoveryOutcome = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, outcome);
        match restored {
            RecoveryOutcome::Abort {
                recovery,
                failure_kind,
            } => {
                assert_eq!(recovery, sample_recovery());
                assert_eq!(failure_kind, LoopFailureKind::NoProgressDetected);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    mod default_recovery_strategy {
        use ironclaw_host_api::{TenantId, ThreadId};
        use ironclaw_turns::{
            AgentLoopDriverDescriptor, RunProfileId, RunProfileVersion, TurnId, TurnRunId,
            TurnScope,
            run_profile::{
                CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy,
                CheckpointSchemaId, ConcurrencyClass, ContextProfileId, LoopDriverId,
                LoopRunContext, ModelProfileId, RedactedRunProfileProvenance, ResolvedRunProfile,
                ResourceBudgetPolicy, ResourceBudgetTier, RunClassId, RunProfileFingerprint,
                RuntimeProfileConstraints, SchedulingClass, SteeringPolicy,
            },
        };

        use super::super::{
            BackoffDelayMs, CapabilityErrorSummary, DefaultRecoveryStrategy, ModelErrorClass,
            ModelErrorSummary, RecoveryOutcome, RecoveryStrategy, RetryAlteration, RetryScope,
            SanitizedStrategySummary, availability_backoff_for, backoff_for,
            capability_error_to_failure_kind, capability_retry_attempt_class,
        };
        use crate::state::{
            LoopExecutionState, ModelErrorRecoveryObservation, RecoveryAttemptClass,
            RecoveryStrategyState,
        };
        use ironclaw_host_api::{FailureFate, FailureKind};
        use ironclaw_turns::LoopFailureKind;

        fn test_run_context() -> LoopRunContext {
            let scope = TurnScope::new(
                TenantId::new("tenant-default-recovery").expect("valid"),
                None,
                None,
                ThreadId::new("thread-default-recovery").expect("valid"),
            );
            let descriptor = AgentLoopDriverDescriptor {
                id: LoopDriverId::new("default_recovery_test_driver").expect("valid"),
                version: RunProfileVersion::new(1),
                checkpoint_schema_id: Some(
                    CheckpointSchemaId::new("default_recovery_test_checkpoint").expect("valid"),
                ),
                checkpoint_schema_version: Some(RunProfileVersion::new(1)),
            };
            let resolved_run_profile = ResolvedRunProfile {
                run_class_id: RunClassId::new("default_recovery_test_class").expect("valid"),
                profile_id: RunProfileId::default_profile(),
                profile_version: RunProfileVersion::new(1),
                loop_driver: descriptor.clone(),
                checkpoint_schema_id: descriptor
                    .checkpoint_schema_id
                    .clone()
                    .expect("descriptor checkpoint id"),
                checkpoint_schema_version: descriptor
                    .checkpoint_schema_version
                    .expect("descriptor checkpoint version"),
                model_profile_id: ModelProfileId::new("default_recovery_test_model")
                    .expect("valid"),
                capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                    "default_recovery_test_capabilities",
                )
                .expect("valid"),
                context_profile_id: ContextProfileId::new("default_recovery_test_context")
                    .expect("valid"),
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
                    tier: ResourceBudgetTier::new("default_recovery_test_tier").expect("valid"),
                    max_model_calls: 32,
                    max_capability_invocations: 64,
                },
                personal_context_policy:
                    ironclaw_turns::run_profile::PersonalContextPolicy::Excluded,
                runtime_constraints: RuntimeProfileConstraints {
                    allow_raw_runtime_backend_selection: false,
                    allow_broad_capability_surface: false,
                },
                runner_pool_id: None,
                scheduling_class: SchedulingClass::new("interactive").expect("valid"),
                concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
                resolution_fingerprint: RunProfileFingerprint::new(
                    "default-recovery-test-fingerprint",
                )
                .expect("valid"),
                provenance: RedactedRunProfileProvenance {
                    sources: vec![],
                    effective_privileges: vec![],
                },
            };
            LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
        }

        fn state_with_no_attempts() -> LoopExecutionState {
            let mut state = LoopExecutionState::initial_for_run(&test_run_context());
            state.recovery_state = RecoveryStrategyState::default();
            state
        }

        fn state_with_attempts_for(
            attempts: u32,
            attempt_class: RecoveryAttemptClass,
        ) -> LoopExecutionState {
            let mut state = LoopExecutionState::initial_for_run(&test_run_context());
            state.recovery_state =
                RecoveryStrategyState::with_attempts_for(attempt_class, attempts);
            state
        }

        fn cap_err(kind: FailureKind) -> CapabilityErrorSummary {
            CapabilityErrorSummary {
                kind,
                safe_summary: SanitizedStrategySummary::from_trusted_static("test"),
            }
        }

        fn model_err(class: ModelErrorClass) -> ModelErrorSummary {
            ModelErrorSummary {
                class,
                safe_summary: SanitizedStrategySummary::from_trusted_static("test"),
            }
        }

        #[test]
        fn default_max_attempts_is_two() {
            let strategy = DefaultRecoveryStrategy::default();
            assert_eq!(strategy.max_attempts_per_class, 2);
            assert_eq!(strategy.max_total_model_attempts(), 41);
        }

        #[tokio::test]
        async fn capability_cancelled_aborts_immediately() {
            // Cancellation is the only terminal capability failure kind; it is
            // normally intercepted upstream as a cancelled exit, but a summary
            // that reaches the strategy must still abort rather than retry.
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_no_attempts();

            let outcome = strategy
                .on_capability_error(&state, &cap_err(FailureKind::Cancelled))
                .await;

            assert!(matches!(
                outcome,
                RecoveryOutcome::Abort {
                    failure_kind: LoopFailureKind::CapabilityProtocolError,
                    ..
                }
            ));
        }

        #[tokio::test]
        async fn capability_input_encode_becomes_tool_error_result() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_no_attempts();

            let outcome = strategy
                .on_capability_error(&state, &cap_err(FailureKind::InputEncode))
                .await;

            match outcome {
                RecoveryOutcome::ToolErrorResult { recovery } => {
                    assert_eq!(recovery, RecoveryStrategyState::default());
                }
                other => panic!("expected ToolErrorResult, got {other:?}"),
            }
            assert_eq!(
                capability_error_to_failure_kind(FailureKind::InputEncode),
                LoopFailureKind::ModelError
            );
        }

        #[tokio::test]
        async fn capability_operation_failed_becomes_tool_error_result() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_no_attempts();

            let outcome = strategy
                .on_capability_error(&state, &cap_err(FailureKind::OperationFailed))
                .await;

            match outcome {
                RecoveryOutcome::ToolErrorResult { recovery } => {
                    assert_eq!(recovery, RecoveryStrategyState::default());
                }
                other => panic!("expected ToolErrorResult, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn capability_policy_denied_becomes_tool_error_result() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_no_attempts();

            let outcome = strategy
                .on_capability_error(&state, &cap_err(FailureKind::PolicyDenied))
                .await;

            match outcome {
                RecoveryOutcome::ToolErrorResult { recovery } => {
                    assert_eq!(recovery, RecoveryStrategyState::default());
                }
                other => panic!("expected ToolErrorResult, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn capability_auth_required_becomes_tool_error_result() {
            // Park-fated kinds reach this path only as failure summaries —
            // the actual gate parks via Resolution::Blocked — so they surface
            // model-visibly instead of retrying or ending the run.
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_no_attempts();

            let outcome = strategy
                .on_capability_error(&state, &cap_err(FailureKind::AuthRequired))
                .await;

            assert!(matches!(outcome, RecoveryOutcome::ToolErrorResult { .. }));
        }

        #[tokio::test]
        async fn capability_transient_retries_then_becomes_tool_error_at_budget() {
            let strategy = DefaultRecoveryStrategy::default();

            for attempts in 0..2 {
                let state =
                    state_with_attempts_for(attempts, RecoveryAttemptClass::CapabilityTransient);
                let outcome = strategy
                    .on_capability_error(&state, &cap_err(FailureKind::Transient))
                    .await;
                assert!(
                    matches!(
                        outcome,
                        RecoveryOutcome::Retry {
                            alter: Some(RetryAlteration::Backoff { .. }),
                            ..
                        }
                    ),
                    "expected retry at attempts={attempts}, got {outcome:?}"
                );
            }

            let state = state_with_attempts_for(2, RecoveryAttemptClass::CapabilityTransient);
            let outcome = strategy
                .on_capability_error(&state, &cap_err(FailureKind::Transient))
                .await;
            assert!(matches!(outcome, RecoveryOutcome::ToolErrorResult { .. }));
        }

        #[tokio::test]
        async fn capability_unavailable_and_internal_become_tool_errors_at_budget() {
            let strategy = DefaultRecoveryStrategy::default();

            for (kind, attempt_class) in [
                (
                    FailureKind::Unavailable,
                    RecoveryAttemptClass::CapabilityUnavailable,
                ),
                (
                    FailureKind::Internal,
                    RecoveryAttemptClass::CapabilityInternal,
                ),
            ] {
                let state = state_with_attempts_for(2, attempt_class);
                let outcome = strategy.on_capability_error(&state, &cap_err(kind)).await;
                assert!(
                    matches!(outcome, RecoveryOutcome::ToolErrorResult { .. }),
                    "{kind:?} at retry budget should become a tool error, got {outcome:?}"
                );
            }
        }

        /// Classification lock over the FULL unified kind vocabulary: every
        /// `FailureKind` has a deliberate capability-recovery outcome derived
        /// from its fate, the terminal set is exactly `{Cancelled}`, and every
        /// Retry-fated kind owns a checkpoint attempt class. This complements
        /// the compile-time guarantee (`fate()` is wildcard-free) by catching
        /// a silent re-bucketing of an existing kind.
        #[tokio::test]
        async fn every_failure_kind_has_a_deliberate_capability_recovery_outcome() {
            let strategy = DefaultRecoveryStrategy::default();
            for &kind in FailureKind::ALL {
                let state = state_with_no_attempts();
                let outcome = strategy.on_capability_error(&state, &cap_err(kind)).await;
                match kind.fate() {
                    FailureFate::Retry => {
                        assert!(
                            matches!(outcome, RecoveryOutcome::Retry { .. }),
                            "{kind:?} is Retry-fated but produced {outcome:?}"
                        );
                        assert!(
                            capability_retry_attempt_class(kind).is_some(),
                            "{kind:?} is Retry-fated but has no attempt class"
                        );
                    }
                    FailureFate::ModelVisible | FailureFate::Park => {
                        assert!(
                            matches!(outcome, RecoveryOutcome::ToolErrorResult { .. }),
                            "{kind:?} must surface model-visibly, got {outcome:?}"
                        );
                    }
                    FailureFate::Terminal => {
                        assert_eq!(
                            kind,
                            FailureKind::Cancelled,
                            "the terminal set must be exactly {{Cancelled}}"
                        );
                        assert!(
                            matches!(outcome, RecoveryOutcome::Abort { .. }),
                            "{kind:?} is Terminal-fated but produced {outcome:?}"
                        );
                    }
                }
                if capability_retry_attempt_class(kind).is_some() {
                    assert_eq!(
                        kind.fate(),
                        FailureFate::Retry,
                        "{kind:?} has an attempt class but is not Retry-fated"
                    );
                }
            }
        }

        #[tokio::test]
        async fn model_context_overflow_retries_then_observes_once_before_abort() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_no_attempts();

            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::ContextOverflow))
                .await;

            match outcome {
                RecoveryOutcome::Retry {
                    recovery,
                    scope,
                    alter,
                } => {
                    assert_eq!(
                        recovery.attempts_for(RecoveryAttemptClass::ModelContextOverflow),
                        1
                    );
                    assert_eq!(scope, RetryScope::Iteration);
                    assert_eq!(alter, Some(RetryAlteration::ShrinkContext));
                }
                other => panic!("expected context overflow retry, got {other:?}"),
            }

            let state = state_with_attempts_for(2, RecoveryAttemptClass::ModelContextOverflow);
            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::ContextOverflow))
                .await;
            let recovery = match outcome {
                RecoveryOutcome::ModelErrorObservation {
                    recovery,
                    scope,
                    alter,
                    observation,
                } => {
                    assert_eq!(scope, RetryScope::Iteration);
                    assert_eq!(alter, Some(RetryAlteration::ShrinkContext));
                    assert_eq!(
                        observation,
                        ModelErrorRecoveryObservation::context_overflow()
                    );
                    recovery
                }
                other => panic!("expected context-overflow observation, got {other:?}"),
            };
            let mut state = state_with_no_attempts();
            state.recovery_state = recovery;
            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::ContextOverflow))
                .await;
            assert!(matches!(
                outcome,
                RecoveryOutcome::Abort {
                    failure_kind: LoopFailureKind::ModelError,
                    ..
                }
            ));
        }

        #[tokio::test]
        async fn model_availability_errors_retry_well_past_the_generic_budget() {
            // A provider 5xx storm must not kill the run after a couple of
            // attempts: availability-class model errors get their own, much
            // deeper retry budget than the generic per-class default.
            let strategy = DefaultRecoveryStrategy::default();

            for (class, attempt_class) in [
                (
                    ModelErrorClass::Transient,
                    RecoveryAttemptClass::ModelTransient,
                ),
                (
                    ModelErrorClass::Unavailable,
                    RecoveryAttemptClass::ModelUnavailable,
                ),
                (
                    ModelErrorClass::Internal,
                    RecoveryAttemptClass::ModelInternal,
                ),
            ] {
                for attempts in 0..strategy.max_model_availability_attempts {
                    let state = state_with_attempts_for(attempts, attempt_class);
                    let outcome = strategy.on_model_error(&state, &model_err(class)).await;
                    assert!(
                        matches!(
                            outcome,
                            RecoveryOutcome::Retry {
                                alter: Some(RetryAlteration::Backoff { .. }),
                                ..
                            }
                        ),
                        "{class:?} at attempts={attempts} should retry, got {outcome:?}"
                    );
                }

                let state = state_with_attempts_for(
                    strategy.max_model_availability_attempts,
                    attempt_class,
                );
                let outcome = strategy.on_model_error(&state, &model_err(class)).await;
                assert!(
                    matches!(outcome, RecoveryOutcome::Abort { .. }),
                    "{class:?} past the availability budget should abort, got {outcome:?}"
                );
            }
        }

        #[tokio::test]
        async fn model_stale_request_retries_at_iteration_scope_then_aborts_at_budget() {
            let strategy = DefaultRecoveryStrategy::default();

            let state = state_with_no_attempts();
            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::StaleRequest))
                .await;
            match outcome {
                RecoveryOutcome::Retry {
                    recovery,
                    scope,
                    alter,
                } => {
                    assert_eq!(
                        recovery.attempts_for(RecoveryAttemptClass::ModelStaleRequest),
                        1
                    );
                    assert_eq!(
                        scope,
                        RetryScope::Iteration,
                        "stale requests must rebuild the iteration (surface + prompt bundle)"
                    );
                    assert_eq!(alter, None);
                }
                other => panic!("expected stale-request iteration retry, got {other:?}"),
            }

            let state = state_with_attempts_for(2, RecoveryAttemptClass::ModelStaleRequest);
            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::StaleRequest))
                .await;
            assert!(matches!(
                outcome,
                RecoveryOutcome::Abort {
                    failure_kind: LoopFailureKind::ModelError,
                    ..
                }
            ));
        }

        #[tokio::test]
        async fn model_precise_terminal_classes_abort_immediately() {
            let strategy = DefaultRecoveryStrategy::default();

            for (class, expected_kind) in [
                (ModelErrorClass::Unauthorized, LoopFailureKind::ModelError),
                (
                    ModelErrorClass::CheckpointRejected,
                    LoopFailureKind::CheckpointRejected,
                ),
                (
                    ModelErrorClass::TranscriptWriteFailed,
                    LoopFailureKind::TranscriptWriteFailed,
                ),
            ] {
                let state = state_with_no_attempts();
                let outcome = strategy.on_model_error(&state, &model_err(class)).await;
                match outcome {
                    RecoveryOutcome::Abort { failure_kind, .. } => {
                        assert_eq!(failure_kind, expected_kind, "failure kind for {class:?}");
                    }
                    other => panic!("{class:?} must abort immediately, got {other:?}"),
                }
            }
        }

        #[tokio::test]
        async fn model_invalid_output_retries_then_observes_once_before_abort() {
            let strategy = DefaultRecoveryStrategy::default();

            let state = state_with_no_attempts();
            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::InvalidOutput))
                .await;
            match outcome {
                RecoveryOutcome::Retry {
                    recovery,
                    scope,
                    alter,
                } => {
                    assert_eq!(
                        recovery.attempts_for(RecoveryAttemptClass::ModelInvalidOutput),
                        1
                    );
                    assert_eq!(scope, RetryScope::Call);
                    assert_eq!(alter, Some(RetryAlteration::RepairInvalidModelOutput));
                }
                other => panic!("expected invalid-output repair retry, got {other:?}"),
            }

            let state = state_with_attempts_for(2, RecoveryAttemptClass::ModelInvalidOutput);
            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::InvalidOutput))
                .await;
            let recovery = match outcome {
                RecoveryOutcome::ModelErrorObservation {
                    recovery,
                    scope,
                    alter,
                    observation,
                } => {
                    assert_eq!(scope, RetryScope::Call);
                    assert_eq!(alter, Some(RetryAlteration::RepairInvalidModelOutput));
                    assert!(
                        observation
                            .model_instruction()
                            .contains("reason=unspecified")
                    );
                    recovery
                }
                other => panic!("expected invalid-output observation, got {other:?}"),
            };
            let mut state = state_with_no_attempts();
            state.recovery_state = recovery;
            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::InvalidOutput))
                .await;
            assert!(matches!(
                outcome,
                RecoveryOutcome::Abort {
                    failure_kind: LoopFailureKind::InvalidModelOutput,
                    ..
                }
            ));
        }

        #[tokio::test]
        async fn model_content_filter_observes_once_before_abort() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_no_attempts();
            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::ContentFiltered))
                .await;
            let recovery = match outcome {
                RecoveryOutcome::ModelErrorObservation {
                    recovery,
                    scope,
                    alter,
                    observation,
                } => {
                    assert_eq!(scope, RetryScope::Call);
                    assert_eq!(alter, None);
                    assert!(recovery.attempts_by_class.is_empty());
                    assert_eq!(
                        observation,
                        ModelErrorRecoveryObservation::content_filtered()
                    );
                    recovery
                }
                other => panic!("expected content-filter observation, got {other:?}"),
            };

            let mut state = state_with_no_attempts();
            state.recovery_state = recovery;
            let outcome = strategy
                .on_model_error(&state, &model_err(ModelErrorClass::ContentFiltered))
                .await;
            assert!(matches!(
                outcome,
                RecoveryOutcome::Abort {
                    failure_kind: LoopFailureKind::ModelError,
                    ..
                }
            ));
        }

        #[tokio::test]
        async fn retry_budget_tracks_each_error_class_independently() {
            let strategy = DefaultRecoveryStrategy::default();
            let mut state = state_with_attempts_for(2, RecoveryAttemptClass::CapabilityTransient);
            state.recovery_state = state
                .recovery_state
                .with_incremented_attempts_for(RecoveryAttemptClass::CapabilityUnavailable);

            let outcome = strategy
                .on_capability_error(&state, &cap_err(FailureKind::Transient))
                .await;

            assert!(matches!(outcome, RecoveryOutcome::ToolErrorResult { .. }));
        }

        #[tokio::test]
        async fn changed_error_class_keeps_prior_attempt_budget() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_attempts_for(2, RecoveryAttemptClass::CapabilityTransient);

            let outcome = strategy
                .on_capability_error(&state, &cap_err(FailureKind::Unavailable))
                .await;

            match outcome {
                RecoveryOutcome::Retry { recovery, .. } => {
                    assert_eq!(
                        recovery.attempts_for(RecoveryAttemptClass::CapabilityTransient),
                        2
                    );
                    assert_eq!(
                        recovery.attempts_for(RecoveryAttemptClass::CapabilityUnavailable),
                        1
                    );
                }
                other => panic!("expected changed class retry, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn non_retry_paths_do_not_poison_later_retry_budget() {
            let strategy = DefaultRecoveryStrategy::default();
            let state = state_with_attempts_for(2, RecoveryAttemptClass::CapabilityTransient);

            let outcome = strategy
                .on_capability_error(&state, &cap_err(FailureKind::PolicyDenied))
                .await;
            let RecoveryOutcome::ToolErrorResult { recovery } = outcome else {
                panic!("expected policy denied tool error result");
            };

            let mut next = LoopExecutionState::initial_for_run(&test_run_context());
            next.recovery_state = recovery;
            let outcome = strategy
                .on_model_error(&next, &model_err(ModelErrorClass::Transient))
                .await;

            assert!(matches!(
                outcome,
                RecoveryOutcome::Retry { recovery, .. }
                    if recovery.attempts_for(RecoveryAttemptClass::ModelTransient) == 1
            ));
        }

        #[test]
        fn backoff_increases_with_attempt_until_cap() {
            let zero = backoff_for(0);
            let one = backoff_for(1);
            let two = backoff_for(2);
            assert!(
                one.as_u64() > zero.as_u64(),
                "expected backoff(1) > backoff(0)"
            );
            assert!(
                two.as_u64() > one.as_u64(),
                "expected backoff(2) > backoff(1)"
            );

            assert!(backoff_for(10).as_u64() <= 5_000);
            assert!(backoff_for(99).as_u64() <= 5_000);
        }

        #[test]
        fn availability_backoff_grows_to_the_max_delay_cap() {
            // Availability retries ride out provider outages: the schedule
            // must keep growing well past the generic 5s cap and settle at
            // BackoffDelayMs::MAX_DELAY_MS so a deep retry budget translates
            // into minutes of ride-out, not seconds.
            let zero = availability_backoff_for(0);
            let one = availability_backoff_for(1);
            assert!(one.as_u64() > zero.as_u64());
            assert!(availability_backoff_for(4).as_u64() > 5_000);
            assert_eq!(
                availability_backoff_for(10).as_u64(),
                BackoffDelayMs::MAX_DELAY_MS
            );
            assert_eq!(
                availability_backoff_for(99).as_u64(),
                BackoffDelayMs::MAX_DELAY_MS
            );
        }

        #[test]
        fn availability_retry_budget_outlasts_a_multi_minute_outage() {
            // The whole point of the deep budget: cumulative sleep across the
            // availability schedule must cover a sustained multi-minute 5xx
            // storm (the observed failure mode was ~5-minute provider
            // outages killing benchmark runs).
            let strategy = DefaultRecoveryStrategy::default();
            let total_ms: u64 = (0..strategy.max_model_availability_attempts)
                .map(|attempt| availability_backoff_for(attempt).as_u64())
                .sum();
            assert!(
                total_ms >= 5 * 60 * 1_000,
                "cumulative availability backoff {total_ms}ms should cover ≥5 minutes"
            );
        }
    }
}
