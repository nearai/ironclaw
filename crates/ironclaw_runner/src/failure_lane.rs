//! Failure-lane classification — the run-boundary half of the two-bucket
//! failure model.
//!
//! Every terminal run failure resolves to exactly one [`FailureLane`]:
//! - [`FailureLane::Retriable`] — the run can be re-driven from a durable
//!   checkpoint (the host-derived `retryable` signal: a `Failed` run that has a
//!   resumable checkpoint).
//! - [`FailureLane::Explainable`] — terminal, but carries a specific
//!   user-facing sentence (see [`crate::reborn_failure_summary_for_category`]).
//! - [`FailureLane::Security`] — reserved for the ingress safety/leak refusal
//!   path (`ironclaw_safety`); the run-boundary classifier never produces it,
//!   because injection/leak are caught before the run starts (minimal
//!   security-stop policy).
//!
//! The enforcement test in this module locks the two-bucket invariant: *every*
//! failure category the system can produce maps to a specific explanation
//! (never the generic fallback) AND a definite lane — so a new failure can't
//! ship as an opaque, unexplained dead end.

use serde::{Deserialize, Serialize};

use crate::failure_categories::BUDGET_ACCOUNTING_FAILED_CATEGORY;

/// The lane a terminal run failure (or ingress refusal) belongs to.
///
/// Wire-stable, snake_case. Adding a variant is a wire contract change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureLane {
    /// Re-drivable from the last durable checkpoint (auto or user-initiated).
    Retriable,
    /// Terminal, but carries a specific user-facing explanation.
    Explainable,
    /// Deliberate security stop. Produced only by the ingress safety/leak path,
    /// never by run-failure categorization.
    Security,
}

impl FailureLane {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retriable => "retriable",
            Self::Explainable => "explainable",
            Self::Security => "security",
        }
    }
}

/// Classify a terminal run failure into its lane.
///
/// `retryable` is the host-derived signal (a `Failed` run with a resumable
/// checkpoint). Per the minimal security-stop policy, injection/leak are
/// refused at ingress, so this run-boundary classifier never returns
/// [`FailureLane::Security`]; the `match` on `category` is the seam where a
/// future mid-run safety-abort category would map to `Security`.
pub fn failure_lane(category: &str, retryable: bool) -> FailureLane {
    match category {
        _ if retryable => FailureLane::Retriable,
        _ => FailureLane::Explainable,
    }
}

/// Canonical list of every failure category the run boundary can produce.
///
/// Sources: `LoopFailureKind::as_str()` (ironclaw_turns), the reborn driver /
/// scheduler / model / capability / compaction / host-stage categories
/// (`failure_categories.rs` + `planned_driver` / `turn_runner` / recovery
/// mapping), and the pinned provider categories. Keep this in lockstep with
/// `reborn_failure_summary_for_category`: a new producer category MUST be added
/// here (the enforcement test asserts each has a specific explanation + lane).
///
/// `unknown_failure` is deliberately excluded — it IS the generic fallback.
pub const ALL_RUN_FAILURE_CATEGORIES: &[&str] = &[
    // Driver / scheduler / lifecycle (ironclaw_runner planned_driver/turn_runner)
    "driver_not_found",
    "driver_unavailable",
    "driver_failed",
    "driver_invalid_request",
    "scheduler_executor_panic",
    "scheduler_heartbeat_failed",
    "host_creation_failed",
    "route_snapshot_persistence_failed",
    "exit_application_failed",
    "lease_expired",
    // LoopFailureKind (ironclaw_turns)
    "model_error",
    "context_build_failed",
    "capability_protocol_error",
    "iteration_limit",
    "invalid_model_output",
    "checkpoint_rejected",
    "checkpoint_unavailable",
    "transcript_write_failed",
    "driver_bug",
    "interrupted_unexpectedly",
    "no_progress_detected",
    "policy_denied",
    "compaction_unavailable",
    // Model recovery categories
    "model_transient",
    "model_context_overflow",
    "model_content_filtered",
    "model_unavailable",
    "model_internal",
    "model_invalid_output",
    // Capability recovery categories
    "capability_transient",
    "capability_permanent",
    "capability_input_invalid",
    "capability_operation_failed",
    "capability_policy_denied",
    "capability_unavailable",
    "capability_internal",
    // Compaction categories
    "compaction_invalid_cut_point",
    "compaction_unsupported_mode",
    "compaction_input_too_large",
    "compaction_security_rejected",
    "compaction_inference_failed",
    "compaction_cancelled",
    "compaction_persistence_failed",
    // Host-stage-unavailable categories (failure_categories.rs)
    "host_stage_unavailable_prompt",
    "host_stage_unavailable_model",
    "host_stage_unavailable_capability",
    "host_stage_unavailable_transcript",
    "host_stage_unavailable_checkpoint",
    "host_stage_unavailable_input",
    "host_stage_unavailable_unknown",
    // Pinned provider categories (failure_categories.rs)
    "model_credits_exhausted",
    "model_credentials_unavailable",
    BUDGET_ACCOUNTING_FAILED_CATEGORY,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failure_summary::reborn_failure_summary_for_category;

    #[test]
    fn failure_lane_is_retriable_when_retryable_else_explainable() {
        assert_eq!(
            failure_lane("model_error", true),
            FailureLane::Retriable,
            "a failure with a resumable checkpoint is retriable"
        );
        assert_eq!(
            failure_lane("model_error", false),
            FailureLane::Explainable,
            "a failure without a resumable checkpoint is explainable"
        );
    }

    #[test]
    fn failure_lane_never_returns_security_for_run_categories() {
        // SecurityStop is an ingress-only refusal; no run-failure category maps
        // to it. (Verifies the minimal security-stop policy at the run boundary.)
        for category in ALL_RUN_FAILURE_CATEGORIES {
            assert_ne!(
                failure_lane(category, false),
                FailureLane::Security,
                "run-failure category {category} must not be a security stop"
            );
            assert_ne!(failure_lane(category, true), FailureLane::Security);
        }
    }

    #[test]
    fn failure_lane_round_trips_snake_case() {
        for (lane, wire) in [
            (FailureLane::Retriable, "retriable"),
            (FailureLane::Explainable, "explainable"),
            (FailureLane::Security, "security"),
        ] {
            assert_eq!(lane.as_str(), wire);
            let value = serde_json::to_value(lane).expect("serialize");
            assert_eq!(value, serde_json::json!(wire));
            let restored: FailureLane = serde_json::from_value(value).expect("deserialize");
            assert_eq!(restored, lane);
        }
    }

    /// THE two-bucket invariant lock: every failure category the system can
    /// produce resolves to a SPECIFIC user explanation (never the generic
    /// fallback) and a definite lane. A new producer category that forgets its
    /// sentence — or one that regresses to the generic fallback — fails here.
    #[test]
    fn every_failure_category_is_explainable_and_classified() {
        // The generic fallback any unrecognized category collapses to.
        let generic = reborn_failure_summary_for_category(Some("__category_that_does_not_exist__"));

        for category in ALL_RUN_FAILURE_CATEGORIES {
            let sentence = reborn_failure_summary_for_category(Some(category));
            assert_ne!(
                sentence, generic,
                "category {category} has no specific explanation (falls back to the generic summary) \
                 — every run-failure category must be user-explainable"
            );
            assert!(
                !sentence.trim().is_empty(),
                "category {category} produced an empty explanation"
            );

            // Every category is Retriable (with checkpoint) or Explainable
            // (without) — the two-bucket invariant. Never Security.
            assert_eq!(failure_lane(category, true), FailureLane::Retriable);
            assert_eq!(failure_lane(category, false), FailureLane::Explainable);
        }
    }

    /// Guards the canonical list against silent drift: the LoopFailureKind
    /// snake_case categories (the largest source) must all be present.
    #[test]
    fn canonical_list_covers_loop_failure_kinds() {
        for kind in [
            ironclaw_turns::LoopFailureKind::ModelError,
            ironclaw_turns::LoopFailureKind::ContextBuildFailed,
            ironclaw_turns::LoopFailureKind::CapabilityProtocolError,
            ironclaw_turns::LoopFailureKind::IterationLimit,
            ironclaw_turns::LoopFailureKind::InvalidModelOutput,
            ironclaw_turns::LoopFailureKind::CheckpointRejected,
            ironclaw_turns::LoopFailureKind::CheckpointUnavailable,
            ironclaw_turns::LoopFailureKind::TranscriptWriteFailed,
            ironclaw_turns::LoopFailureKind::DriverBug,
            ironclaw_turns::LoopFailureKind::InterruptedUnexpectedly,
            ironclaw_turns::LoopFailureKind::NoProgressDetected,
            ironclaw_turns::LoopFailureKind::PolicyDenied,
            ironclaw_turns::LoopFailureKind::CompactionUnavailable,
        ] {
            assert!(
                ALL_RUN_FAILURE_CATEGORIES.contains(&kind.as_str()),
                "LoopFailureKind {kind:?} ({}) is missing from ALL_RUN_FAILURE_CATEGORIES",
                kind.as_str()
            );
        }
    }
}
