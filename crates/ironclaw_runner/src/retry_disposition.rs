//! Retry disposition — the hybrid retry policy for the Retriable lane.
//!
//! [`FailureLane`](crate::FailureLane) answers *"can this be retried at all?"*
//! (Retriable vs Explainable). This module answers the finer *"how?"* — the
//! decided **hybrid** policy:
//!
//! - **Auto** — infra / lease / transient faults re-drive **silently** from the
//!   last checkpoint (bounded; the scheduler owns the attempt budget and, on
//!   exhaustion, downgrades to [`RetryDisposition::UserInitiated`]).
//! - **UserInitiated** — model / provider / config / model-fixable faults stop
//!   with a user-facing explanation + a retry affordance; a silent re-drive of
//!   the identical request would just re-fail until something changes.
//! - **NoRetry** — no resumable checkpoint exists, so the run cannot be
//!   re-driven; it is terminal-but-explainable.
//!
//! This is a pure decision function: the scheduler/auto-redrive layer is its
//! consumer. Keeping the policy here (tested, in lockstep with the failure
//! taxonomy) means the scheduler wiring is mechanical.

use crate::failure_lane::FailureLane;

/// How a terminal run failure should be retried.
///
/// Wire-stable snake_case (so it can be surfaced or logged), though the primary
/// consumer is the server-side auto-redrive scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryDisposition {
    /// Silently re-drive from the last checkpoint, bounded by the scheduler.
    Auto,
    /// Surface to the user with a retry affordance (no silent re-drive).
    UserInitiated,
    /// Not retriable (no resumable checkpoint) — terminal but explainable.
    NoRetry,
}

impl RetryDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::UserInitiated => "user_initiated",
            Self::NoRetry => "no_retry",
        }
    }

    /// The [`FailureLane`] this disposition implies. `Auto`/`UserInitiated` are
    /// both [`FailureLane::Retriable`]; `NoRetry` is [`FailureLane::Explainable`].
    pub fn failure_lane(self) -> FailureLane {
        match self {
            Self::Auto | Self::UserInitiated => FailureLane::Retriable,
            Self::NoRetry => FailureLane::Explainable,
        }
    }
}

/// Categories that re-drive cleanly on a silent retry: transient host / lease /
/// store / provider / tool faults where re-running the *identical* request from
/// the checkpoint is likely to succeed without any change. Conservative by
/// design — anything not clearly transient falls to `UserInitiated`.
fn is_auto_retriable_category(category: &str) -> bool {
    matches!(
        category,
        // Host-stage transient outages
        "host_stage_unavailable_prompt"
            | "host_stage_unavailable_model"
            | "host_stage_unavailable_capability"
            | "host_stage_unavailable_transcript"
            | "host_stage_unavailable_checkpoint"
            | "host_stage_unavailable_input"
            | "host_stage_unavailable_unknown"
            // Lifecycle / store / runner transients
            | "lease_expired"
            | "scheduler_heartbeat_failed"
            | "route_snapshot_persistence_failed"
            | "exit_application_failed"
            | "host_creation_failed"
            | "transcript_write_failed"
            | "checkpoint_unavailable"
            | "context_build_failed"
            // Model provider transients
            | "model_transient"
            | "model_unavailable"
            | "model_internal"
            // Tool / capability transients
            | "capability_transient"
            | "capability_unavailable"
            | "capability_internal"
            // Compaction infra transients
            | "compaction_unavailable"
            | "compaction_inference_failed"
            | "compaction_persistence_failed"
            | "compaction_cancelled"
    )
}

/// Classify how a terminal run failure should be retried, per the hybrid policy.
///
/// `retryable` is the host-derived signal (a `Failed` run with a resumable
/// checkpoint). When absent, the run cannot be re-driven from a checkpoint →
/// [`RetryDisposition::NoRetry`]. (A future from-input retry could make
/// no-checkpoint failures user-retriable; until then they are terminal.)
pub fn retry_disposition(category: &str, retryable: bool) -> RetryDisposition {
    if !retryable {
        return RetryDisposition::NoRetry;
    }
    if is_auto_retriable_category(category) {
        RetryDisposition::Auto
    } else {
        // Model / provider / config / model-fixable faults: a silent re-drive of
        // the identical request would re-fail. Surface with a retry affordance.
        RetryDisposition::UserInitiated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failure_lane::{ALL_RUN_FAILURE_CATEGORIES, failure_lane};

    #[test]
    fn no_resumable_checkpoint_is_no_retry() {
        for category in ALL_RUN_FAILURE_CATEGORIES {
            assert_eq!(
                retry_disposition(category, false),
                RetryDisposition::NoRetry,
                "{category} without a checkpoint cannot be re-driven"
            );
        }
    }

    #[test]
    fn transient_infra_faults_auto_retry() {
        for category in [
            "host_stage_unavailable_model",
            "lease_expired",
            "model_transient",
            "model_unavailable",
            "capability_transient",
            "transcript_write_failed",
            "context_build_failed",
        ] {
            assert_eq!(
                retry_disposition(category, true),
                RetryDisposition::Auto,
                "{category} is a transient infra fault and should auto re-drive"
            );
        }
    }

    #[test]
    fn model_and_config_faults_are_user_initiated() {
        for category in [
            "model_error",
            "model_context_overflow",
            "model_content_filtered",
            "model_credits_exhausted",
            "model_credentials_unavailable",
            "capability_input_invalid",
            "capability_policy_denied",
            "policy_denied",
            "iteration_limit",
            "no_progress_detected",
            "driver_bug",
        ] {
            assert_eq!(
                retry_disposition(category, true),
                RetryDisposition::UserInitiated,
                "{category} needs a change before retry helps — user-initiated"
            );
        }
    }

    /// The disposition must agree with the coarser FailureLane for EVERY known
    /// category: a retryable failure is Retriable (Auto or UserInitiated); a
    /// non-retryable one is Explainable (NoRetry). Locks the two layers in sync.
    #[test]
    fn disposition_is_consistent_with_failure_lane() {
        for category in ALL_RUN_FAILURE_CATEGORIES {
            assert_eq!(
                retry_disposition(category, true).failure_lane(),
                failure_lane(category, true),
                "retryable {category}: disposition lane disagrees with failure_lane"
            );
            assert_eq!(
                retry_disposition(category, false).failure_lane(),
                failure_lane(category, false),
                "non-retryable {category}: disposition lane disagrees with failure_lane"
            );
        }
    }

    #[test]
    fn retry_disposition_round_trips_snake_case() {
        for (disposition, wire) in [
            (RetryDisposition::Auto, "auto"),
            (RetryDisposition::UserInitiated, "user_initiated"),
            (RetryDisposition::NoRetry, "no_retry"),
        ] {
            assert_eq!(disposition.as_str(), wire);
            let value = serde_json::to_value(disposition).expect("serialize");
            assert_eq!(value, serde_json::json!(wire));
            let restored: RetryDisposition = serde_json::from_value(value).expect("deserialize");
            assert_eq!(restored, disposition);
        }
    }
}
