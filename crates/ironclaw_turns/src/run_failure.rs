//! Single-funnel classification for terminal run failures.
//!
//! Every terminal run failure must pass through [`RunFailureReason::classify`]
//! (or the safety-only [`RunFailureReason::security_stop`]) before it can be
//! recorded. `RunFailureReason` has private fields and no public constructor
//! other than those two, so the run-exit contracts that carry it
//! (`TurnRunnerOutcome::Failed`, `FailRunRequest`, `TurnRunExecutorError`, …)
//! cannot receive an unclassified, un-surfaced failure — an unsurfaced terminal
//! error is structurally unrepresentable.
//!
//! Three properties this module guarantees:
//!
//! 1. **Exhaustive classification.** [`RunFailureCategory`] is a wildcard-free
//!    enum; `classify` matches it exhaustively, so a new category cannot be
//!    added without deciding its lane, retry policy, and user message — it fails
//!    to compile otherwise (companion to the capability-classifier keystone).
//! 2. **A non-empty user message always exists.** [`UserMessage`] rejects empty
//!    strings at construction, and every category maps to a fixed, host-authored
//!    sentence — the run boundary can always tell the user *something* concrete.
//! 3. **`SecurityStop` originates only in the safety layer.** `classify` never
//!    yields [`FailureLane::SecurityStop`]; the only path to it is
//!    [`RunFailureReason::security_stop`], gated behind a [`SafetyStopEvidence`]
//!    token that only `ironclaw_safety` can mint.

use crate::ids::TurnRunId;
use crate::status::SanitizedFailure;

/// Which of the three terminal outcomes a run failure resolves to.
///
/// This is the two-bucket end state: a security-related failure stops the run;
/// everything else is user-explainable and, where safe, retriable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureLane {
    /// A security boundary (prompt-injection / real secret leak) halted the run
    /// deliberately. Reachable only from the safety layer via
    /// [`RunFailureReason::security_stop`]; never produced by [`classify`].
    ///
    /// [`classify`]: RunFailureReason::classify
    SecurityStop,
    /// A transient infra/host fault. Safe to re-drive (automatically or by the
    /// user) without the model or user changing anything.
    Retriable,
    /// A failure the user can understand and, depending on [`RetryPolicy`],
    /// possibly retry after changing something (credentials/credits) — or not.
    Explainable,
}

/// What kind of retry, if any, is sensible for a failure.
///
/// Advisory metadata surfaced to the UI/scheduler. It does not itself perform a
/// retry — the actual re-drive is checkpoint-gated `retry_turn` machinery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPolicy {
    /// Retrying re-hits the same deterministic failure; do not offer retry.
    NoRetry,
    /// Transient fault; an automatic or user-initiated re-drive is likely to
    /// succeed with no change.
    RetryTransient,
    /// The user must fix something first (e.g. provider credentials or credits),
    /// then retry.
    RetryAfterUserAction,
}

/// Error constructing a [`UserMessage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserMessageError {
    /// The message was empty (or whitespace-only).
    Empty,
}

impl std::fmt::Display for UserMessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("user message must not be empty"),
        }
    }
}

impl std::error::Error for UserMessageError {}

/// A non-empty, user-facing failure sentence.
///
/// The newtype makes "a terminal failure with no user-facing explanation"
/// unrepresentable: `RunFailureReason` requires one, and it cannot be empty.
/// Deliberately no `From<String>`/`From<&str>` so every construction goes
/// through the validating [`UserMessage::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserMessage(String);

impl UserMessage {
    /// Construct from arbitrary text, rejecting empty/whitespace-only input.
    pub fn new(message: impl Into<String>) -> Result<Self, UserMessageError> {
        let message = message.into();
        if message.trim().is_empty() {
            return Err(UserMessageError::Empty);
        }
        Ok(Self(message))
    }

    /// Construct from a `&'static str` known to be non-empty at author time.
    /// Panics in debug builds if the invariant is violated; the category message
    /// table is the sole caller and its entries are all non-empty literals.
    fn from_static(message: &'static str) -> Self {
        debug_assert!(
            !message.trim().is_empty(),
            "static user message must not be empty"
        );
        Self(message.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Display for UserMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Provenance of a terminal failure — which recording path produced it.
///
/// Carried alongside the category so a future classifier can nuance lane/retry
/// by source without re-deriving it. Today the category alone determines the
/// classification; `source` is retained for diagnostics and forward flexibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureSource {
    /// A validated `LoopExit::Failed(LoopFailureKind)` from the driver.
    LoopExit,
    /// The driver invocation failed before producing any `LoopExit`.
    DriverInvocation,
    /// A scheduler-internal terminal failure (panic, heartbeat, lease expiry).
    Scheduler,
    /// The loop-exit applier itself failed after the exit was consumed.
    ExitApplication,
}

/// The exhaustive set of terminal run-failure categories.
///
/// Every variant round-trips a wire-stable `snake_case` category string that is
/// what actually persists in `TurnRunState.failure` and projects on the run
/// event. Adding a variant forces [`RunFailureReason::classify`] and
/// [`RunFailureCategory::user_message`] to handle it (no wildcard arm).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunFailureCategory {
    // --- loop-exit path (`LoopFailureKind`) ---
    ModelError,
    ContextBuildFailed,
    CapabilityProtocolError,
    IterationLimit,
    InvalidModelOutput,
    CheckpointRejected,
    CheckpointUnavailable,
    TranscriptWriteFailed,
    DriverBug,
    InterruptedUnexpectedly,
    NoProgressDetected,
    PolicyDenied,
    CompactionUnavailable,
    // --- loop-exit protocol violations ---
    DriverProtocolViolation,
    // --- driver-invocation path ---
    DriverNotFound,
    DriverUnavailable,
    DriverFailed,
    DriverInvalidRequest,
    HostCreationFailed,
    RouteSnapshotPersistenceFailed,
    // --- scheduler-internal path ---
    SchedulerExecutorPanic,
    SchedulerHeartbeatFailed,
    ExitApplicationFailed,
    LeaseExpired,
    // --- model provider categories (user must act) ---
    ModelCreditsExhausted,
    ModelCredentialsUnavailable,
    // --- generic fallback for an unrecognized category string ---
    UnknownFailure,
}

impl RunFailureCategory {
    /// The wire-stable, persisted category string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModelError => "model_error",
            Self::ContextBuildFailed => "context_build_failed",
            Self::CapabilityProtocolError => "capability_protocol_error",
            Self::IterationLimit => "iteration_limit",
            Self::InvalidModelOutput => "invalid_model_output",
            Self::CheckpointRejected => "checkpoint_rejected",
            Self::CheckpointUnavailable => "checkpoint_unavailable",
            Self::TranscriptWriteFailed => "transcript_write_failed",
            Self::DriverBug => "driver_bug",
            Self::InterruptedUnexpectedly => "interrupted_unexpectedly",
            Self::NoProgressDetected => "no_progress_detected",
            Self::PolicyDenied => "policy_denied",
            Self::CompactionUnavailable => "compaction_unavailable",
            Self::DriverProtocolViolation => "driver_protocol_violation",
            Self::DriverNotFound => "driver_not_found",
            Self::DriverUnavailable => "driver_unavailable",
            Self::DriverFailed => "driver_failed",
            Self::DriverInvalidRequest => "driver_invalid_request",
            Self::HostCreationFailed => "host_creation_failed",
            Self::RouteSnapshotPersistenceFailed => "route_snapshot_persistence_failed",
            Self::SchedulerExecutorPanic => "scheduler_executor_panic",
            Self::SchedulerHeartbeatFailed => "scheduler_heartbeat_failed",
            Self::ExitApplicationFailed => "exit_application_failed",
            Self::LeaseExpired => "lease_expired",
            Self::ModelCreditsExhausted => "model_credits_exhausted",
            Self::ModelCredentialsUnavailable => "model_credentials_unavailable",
            Self::UnknownFailure => "unknown_failure",
        }
    }

    /// Parse a persisted/wire category string back into a category.
    ///
    /// An unrecognized string (e.g. a category minted by a newer producer, or a
    /// legacy value no longer emitted) maps to [`RunFailureCategory::UnknownFailure`]
    /// so classification never fails closed. This mirrors the open-set handling
    /// of the capability-failure classifier.
    pub fn from_category_str(category: &str) -> Self {
        match category {
            "model_error" => Self::ModelError,
            "context_build_failed" => Self::ContextBuildFailed,
            "capability_protocol_error" => Self::CapabilityProtocolError,
            "iteration_limit" => Self::IterationLimit,
            "invalid_model_output" => Self::InvalidModelOutput,
            "checkpoint_rejected" => Self::CheckpointRejected,
            "checkpoint_unavailable" => Self::CheckpointUnavailable,
            "transcript_write_failed" => Self::TranscriptWriteFailed,
            "driver_bug" => Self::DriverBug,
            "interrupted_unexpectedly" => Self::InterruptedUnexpectedly,
            "no_progress_detected" => Self::NoProgressDetected,
            "policy_denied" => Self::PolicyDenied,
            "compaction_unavailable" => Self::CompactionUnavailable,
            "driver_protocol_violation" => Self::DriverProtocolViolation,
            "driver_not_found" => Self::DriverNotFound,
            "driver_unavailable" => Self::DriverUnavailable,
            "driver_failed" => Self::DriverFailed,
            "driver_invalid_request" => Self::DriverInvalidRequest,
            "host_creation_failed" => Self::HostCreationFailed,
            "route_snapshot_persistence_failed" => Self::RouteSnapshotPersistenceFailed,
            "scheduler_executor_panic" => Self::SchedulerExecutorPanic,
            "scheduler_heartbeat_failed" => Self::SchedulerHeartbeatFailed,
            "exit_application_failed" => Self::ExitApplicationFailed,
            "lease_expired" => Self::LeaseExpired,
            "model_credits_exhausted" => Self::ModelCreditsExhausted,
            "model_credentials_unavailable" => Self::ModelCredentialsUnavailable,
            _ => Self::UnknownFailure,
        }
    }

    /// The fixed, host-authored user-facing sentence for this category.
    ///
    /// Moved verbatim (per category) from the former
    /// `ironclaw_reborn_composition::failure_summary` table so the run boundary
    /// owns a guaranteed-non-empty baseline message. The projection layer's
    /// model-generated `FailureExplanationProvider` may still enrich this, using
    /// it as the fallback.
    pub fn user_message(self) -> UserMessage {
        let text = match self {
            Self::ModelError => "The run stopped because the model could not complete the request.",
            Self::ContextBuildFailed => {
                "The run failed while preparing the conversation context for the model."
            }
            Self::CapabilityProtocolError => {
                "The run stopped because a tool returned a response it could not process."
            }
            Self::IterationLimit => {
                "The run stopped after reaching its iteration limit before producing a reply."
            }
            Self::InvalidModelOutput => {
                "The run stopped because the model returned a response that could not be parsed."
            }
            Self::CheckpointRejected => "The run failed while saving a progress checkpoint.",
            Self::CheckpointUnavailable => {
                "The run could not resume because its saved progress was unavailable."
            }
            Self::TranscriptWriteFailed => "The run failed while recording its transcript.",
            Self::DriverBug => "The run stopped because of an internal error in the agent runtime.",
            Self::InterruptedUnexpectedly => "The run stopped before it could complete cleanly.",
            Self::NoProgressDetected => {
                "The run stopped because it repeated the same step without making progress."
            }
            Self::PolicyDenied => {
                "The run stopped because an action it attempted was not permitted."
            }
            Self::CompactionUnavailable => {
                "The run stopped because it could not free up context space to continue."
            }
            Self::DriverProtocolViolation => {
                "The run produced an invalid result and stopped before replying."
            }
            Self::DriverNotFound => {
                "The run could not start because the configured agent runtime was unavailable."
            }
            Self::DriverUnavailable => "The run could not start the agent runtime.",
            Self::DriverFailed => {
                "The agent runtime reported an internal error before producing a reply."
            }
            Self::DriverInvalidRequest => {
                "The agent runtime rejected the request before producing a reply."
            }
            Self::HostCreationFailed => "The run failed while preparing the runtime host.",
            Self::RouteSnapshotPersistenceFailed => {
                "The run failed while saving the selected model route."
            }
            Self::SchedulerExecutorPanic => "The agent runtime stopped unexpectedly.",
            Self::SchedulerHeartbeatFailed => {
                "The run failed after the runner heartbeat could not be recorded."
            }
            Self::ExitApplicationFailed => "The run failed while recording its final result.",
            Self::LeaseExpired => "The run failed because its runner lease expired.",
            Self::ModelCreditsExhausted => {
                "The AI provider account is out of credits. Add credits or switch providers and try again."
            }
            Self::ModelCredentialsUnavailable => {
                "The run failed because model credentials or provider configuration are invalid. Check the selected provider's API key and base URL."
            }
            Self::UnknownFailure => "The run failed for an unknown reason.",
        };
        UserMessage::from_static(text)
    }

    /// The recovery lane and retry policy for this category (never `SecurityStop`).
    fn lane_and_policy(self) -> (FailureLane, RetryPolicy) {
        match self {
            // Transient infra/host faults — a re-drive is likely to succeed with
            // no change to the request.
            Self::DriverNotFound
            | Self::DriverUnavailable
            | Self::HostCreationFailed
            | Self::RouteSnapshotPersistenceFailed
            | Self::SchedulerExecutorPanic
            | Self::SchedulerHeartbeatFailed
            | Self::ExitApplicationFailed
            | Self::LeaseExpired
            | Self::CheckpointRejected
            | Self::CheckpointUnavailable
            | Self::TranscriptWriteFailed
            | Self::CompactionUnavailable
            | Self::InterruptedUnexpectedly => {
                (FailureLane::Retriable, RetryPolicy::RetryTransient)
            }

            // The user must fix provider configuration/credits, then retry.
            Self::ModelCreditsExhausted | Self::ModelCredentialsUnavailable => {
                (FailureLane::Explainable, RetryPolicy::RetryAfterUserAction)
            }

            // Deterministic failures — a blind retry re-hits the same wall, so
            // the user gets an explanation but no retry affordance.
            Self::ModelError
            | Self::ContextBuildFailed
            | Self::CapabilityProtocolError
            | Self::IterationLimit
            | Self::InvalidModelOutput
            | Self::DriverBug
            | Self::NoProgressDetected
            | Self::PolicyDenied
            | Self::DriverProtocolViolation
            | Self::DriverFailed
            | Self::DriverInvalidRequest
            | Self::UnknownFailure => (FailureLane::Explainable, RetryPolicy::NoRetry),
        }
    }
}

/// Opaque proof that a security boundary decided to stop the run.
///
/// The field is private and the only constructor is [`SafetyStopEvidence::from_safety_layer`],
/// so only code that can name that constructor (the `ironclaw_safety` crate and
/// this crate's tests) can mint one. Combined with the private
/// [`RunFailureReason`] fields, this makes [`FailureLane::SecurityStop`]
/// reachable only from the safety layer. The single-origin property is
/// additionally locked by a boundary test.
#[derive(Debug, Clone)]
pub struct SafetyStopEvidence {
    /// Wire-stable category describing the security stop (e.g. a leak-block
    /// category). Kept private; surfaced only through the classified reason.
    category: SanitizedFailure,
}

impl SafetyStopEvidence {
    /// Mint security-stop evidence from the safety layer. The caller is
    /// `ironclaw_safety` at a real block decision (prompt-injection detection or
    /// a confirmed secret leak); `category` is the sanitized block category.
    pub fn from_safety_layer(category: SanitizedFailure) -> Self {
        Self { category }
    }

    fn category(&self) -> SanitizedFailure {
        self.category.clone()
    }
}

/// A classified terminal run failure — the single value every recording path
/// carries.
///
/// All fields are private; the only constructors are [`RunFailureReason::classify`]
/// and [`RunFailureReason::security_stop`]. There is no public struct literal and
/// no `From`, so a `RunFailureReason` cannot exist without having passed one of
/// the two funnels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFailureReason {
    lane: FailureLane,
    retry_policy: RetryPolicy,
    user_message: UserMessage,
    correlation_id: TurnRunId,
    /// The wire-stable category — the only part that persists in
    /// `TurnRunState.failure`; lane/retry/message are recomputable from it.
    category: SanitizedFailure,
}

impl RunFailureReason {
    /// Classify a non-security terminal failure into its lane, retry policy, and
    /// user message. This is the single funnel every recording path routes
    /// through; it never yields [`FailureLane::SecurityStop`].
    pub fn classify(
        category: RunFailureCategory,
        _source: FailureSource,
        correlation_id: TurnRunId,
    ) -> Self {
        let (lane, retry_policy) = category.lane_and_policy();
        Self {
            lane,
            retry_policy,
            user_message: category.user_message(),
            correlation_id,
            category: SanitizedFailure::from_trusted_static(category.as_str()),
        }
    }

    /// Classify from a raw persisted/wire category string (recompute on read).
    pub fn from_category_str(
        category: &str,
        source: FailureSource,
        correlation_id: TurnRunId,
    ) -> Self {
        Self::classify(
            RunFailureCategory::from_category_str(category),
            source,
            correlation_id,
        )
    }

    /// The security-stop funnel: the only path to [`FailureLane::SecurityStop`].
    /// Requires [`SafetyStopEvidence`], which only the safety layer can mint.
    pub fn security_stop(evidence: SafetyStopEvidence, correlation_id: TurnRunId) -> Self {
        let category = evidence.category();
        Self {
            lane: FailureLane::SecurityStop,
            retry_policy: RetryPolicy::NoRetry,
            user_message: UserMessage::from_static(
                "The run was stopped by a security check before it could continue.",
            ),
            correlation_id,
            category,
        }
    }

    pub fn lane(&self) -> FailureLane {
        self.lane
    }

    pub fn retry_policy(&self) -> RetryPolicy {
        self.retry_policy
    }

    pub fn user_message(&self) -> &UserMessage {
        &self.user_message
    }

    pub fn correlation_id(&self) -> &TurnRunId {
        &self.correlation_id
    }

    /// The wire-stable category to persist in `TurnRunState.failure`.
    pub fn category(&self) -> SanitizedFailure {
        self.category.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every category the enum can hold, for exhaustive iteration in tests. A
    /// new `RunFailureCategory` variant must be added here (and the compiler
    /// forces `as_str`/`user_message`/`lane_and_policy` to handle it), so the
    /// classification of a new failure kind is never skipped.
    const ALL_CATEGORIES: &[RunFailureCategory] = &[
        RunFailureCategory::ModelError,
        RunFailureCategory::ContextBuildFailed,
        RunFailureCategory::CapabilityProtocolError,
        RunFailureCategory::IterationLimit,
        RunFailureCategory::InvalidModelOutput,
        RunFailureCategory::CheckpointRejected,
        RunFailureCategory::CheckpointUnavailable,
        RunFailureCategory::TranscriptWriteFailed,
        RunFailureCategory::DriverBug,
        RunFailureCategory::InterruptedUnexpectedly,
        RunFailureCategory::NoProgressDetected,
        RunFailureCategory::PolicyDenied,
        RunFailureCategory::CompactionUnavailable,
        RunFailureCategory::DriverProtocolViolation,
        RunFailureCategory::DriverNotFound,
        RunFailureCategory::DriverUnavailable,
        RunFailureCategory::DriverFailed,
        RunFailureCategory::DriverInvalidRequest,
        RunFailureCategory::HostCreationFailed,
        RunFailureCategory::RouteSnapshotPersistenceFailed,
        RunFailureCategory::SchedulerExecutorPanic,
        RunFailureCategory::SchedulerHeartbeatFailed,
        RunFailureCategory::ExitApplicationFailed,
        RunFailureCategory::LeaseExpired,
        RunFailureCategory::ModelCreditsExhausted,
        RunFailureCategory::ModelCredentialsUnavailable,
        RunFailureCategory::UnknownFailure,
    ];

    fn run_id() -> TurnRunId {
        TurnRunId::new()
    }

    #[test]
    fn user_message_rejects_empty() {
        assert_eq!(UserMessage::new(""), Err(UserMessageError::Empty));
        assert_eq!(UserMessage::new("   "), Err(UserMessageError::Empty));
        assert!(UserMessage::new("ok").is_ok());
    }

    #[test]
    fn every_category_classifies_without_security_stop_and_with_non_empty_message() {
        for &category in ALL_CATEGORIES {
            for source in [
                FailureSource::LoopExit,
                FailureSource::DriverInvocation,
                FailureSource::Scheduler,
                FailureSource::ExitApplication,
            ] {
                let reason = RunFailureReason::classify(category, source, run_id());
                assert_ne!(
                    reason.lane(),
                    FailureLane::SecurityStop,
                    "classify() must never yield SecurityStop ({category:?})"
                );
                assert!(
                    matches!(
                        reason.lane(),
                        FailureLane::Retriable | FailureLane::Explainable
                    ),
                    "unexpected lane for {category:?}"
                );
                assert!(
                    !reason.user_message().as_str().trim().is_empty(),
                    "empty user message for {category:?}"
                );
                assert_eq!(
                    reason.category().category(),
                    category.as_str(),
                    "category string must survive classification for {category:?}"
                );
            }
        }
    }

    #[test]
    fn category_string_round_trips() {
        for &category in ALL_CATEGORIES {
            assert_eq!(
                RunFailureCategory::from_category_str(category.as_str()),
                category,
                "round-trip failed for {category:?}"
            );
        }
        // Unknown strings degrade to UnknownFailure rather than failing closed.
        assert_eq!(
            RunFailureCategory::from_category_str("some_future_category"),
            RunFailureCategory::UnknownFailure
        );
    }

    #[test]
    fn retriable_lane_and_transient_policy_agree() {
        for &category in ALL_CATEGORIES {
            let reason = RunFailureReason::classify(category, FailureSource::LoopExit, run_id());
            // The only lane paired with a transient retry is Retriable, and
            // Retriable is always RetryTransient — the two encodings can't drift.
            match reason.lane() {
                FailureLane::Retriable => {
                    assert_eq!(
                        reason.retry_policy(),
                        RetryPolicy::RetryTransient,
                        "{category:?}"
                    )
                }
                FailureLane::Explainable => assert_ne!(
                    reason.retry_policy(),
                    RetryPolicy::RetryTransient,
                    "{category:?}"
                ),
                FailureLane::SecurityStop => unreachable!("classify never yields SecurityStop"),
            }
        }
    }

    #[test]
    fn security_stop_is_the_only_path_to_that_lane() {
        let category = SanitizedFailure::from_trusted_static("prompt_injection_blocked");
        let reason = RunFailureReason::security_stop(
            SafetyStopEvidence::from_safety_layer(category),
            run_id(),
        );
        assert_eq!(reason.lane(), FailureLane::SecurityStop);
        assert_eq!(reason.retry_policy(), RetryPolicy::NoRetry);
        assert!(!reason.user_message().as_str().trim().is_empty());
        assert_eq!(reason.category().category(), "prompt_injection_blocked");
    }
}
