//! Host-factory abstraction and shared failure helpers for the Reborn turn runner.
//!
//! # Architecture boundary
//!
//! `ironclaw_turns` owns `TurnRunTransitionPort`, claim/heartbeat/transition
//! DTOs, state-machine invariants, and the trusted `LoopExitApplier`.
//!
//! This module owns the `HostFactory` trait that constructs a per-run
//! `AgentLoopDriverHost`, and the `sanitized_failure`/`sanitized_driver_failure`
//! helpers that are shared across the executor and composition layers.

use async_trait::async_trait;
use tracing::{debug, error};

use ironclaw_turns::{SanitizedFailure, runner::ClaimedTurnRun};

use crate::failure_categories::{
    BUDGET_ACCOUNTING_FAILED_CATEGORY, MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY,
    MODEL_CREDITS_EXHAUSTED_CATEGORY,
};

/// Create a `SanitizedFailure` from a known-valid static category.
///
/// All categories used here are lowercase ASCII with underscores, satisfying
/// validation invariants. Returning `None` is only possible if a static literal
/// is changed to an invalid category.
pub(crate) fn sanitized_failure(category: &'static str) -> Option<SanitizedFailure> {
    match SanitizedFailure::new(category) {
        Ok(failure) => Some(failure),
        Err(error) => {
            error!(category, %error, "invalid static recovery failure category");
            match SanitizedFailure::new("unknown_failure") {
                Ok(fallback) => Some(fallback),
                Err(fallback_error) => {
                    error!(%fallback_error, "fallback recovery failure category invalid");
                    None
                }
            }
        }
    }
}

pub(crate) fn sanitized_driver_failure(
    reason_kind: &str,
    detail: Option<&str>,
) -> Option<SanitizedFailure> {
    // `interrupted_unexpectedly` is preserved (§5a.5, loop-failure matrix):
    // the planned driver maps an in-flight `Cancelled` executor error to it,
    // and collapsing it to `driver_failed` here erased the original category
    // from the durable failure record.
    let base = if matches!(
        reason_kind,
        MODEL_CREDITS_EXHAUSTED_CATEGORY
            | MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY
            | BUDGET_ACCOUNTING_FAILED_CATEGORY
            | "interrupted_unexpectedly"
    ) {
        match SanitizedFailure::new(reason_kind.to_string()) {
            Ok(failure) => Some(failure),
            Err(error) => {
                debug!(
                    reason_kind,
                    %error,
                    "model failure category failed validation; using generic driver failure"
                );
                sanitized_failure("driver_failed")
            }
        }
    } else {
        sanitized_failure("driver_failed")
    };
    // Carry the secret-scrubbed model-visible detail onto the failure record so
    // it can reach `TurnLifecycleEvent.detail` and the failure explainer.
    base.map(|failure| match detail {
        Some(detail) => failure.with_detail(detail),
        None => failure,
    })
}

/// Factory trait for constructing a per-run `AgentLoopDriverHost`.
///
/// The host is created once per claimed run and provides the driver with access
/// to model, transcript, checkpoint, input, capabilities, and progress services.
#[async_trait]
pub trait HostFactory: Send + Sync {
    /// Construct a host for the given claimed run.
    ///
    /// The returned host must be valid for the entire duration of the driver
    /// invocation. Errors here result in a terminal failed/cancelled transition.
    async fn create_host(
        &self,
        claimed: &ClaimedTurnRun,
    ) -> Result<
        Box<dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync>,
        HostFactoryError,
    >;
}

/// Error returned when host construction fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFactoryError {
    pub reason: String,
}

impl HostFactoryError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for HostFactoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "host factory error: {}", self.reason)
    }
}

impl std::error::Error for HostFactoryError {}

#[cfg(test)]
mod tests {
    use super::sanitized_driver_failure;
    use crate::failure_categories::BUDGET_ACCOUNTING_FAILED_CATEGORY;

    #[test]
    fn sanitized_driver_failure_returns_driver_failed_for_invalid_category() {
        let failure = sanitized_driver_failure("invalid category with spaces", None)
            .expect("driver_failed fallback is valid");

        assert_eq!(failure.category(), "driver_failed");
        assert_eq!(failure.detail(), None);
    }

    #[test]
    fn sanitized_driver_failure_carries_detail_onto_failure_record() {
        let failure = sanitized_driver_failure("driver_failed", Some("HTTP 404 model not found"))
            .expect("driver_failed is valid");

        assert_eq!(failure.category(), "driver_failed");
        assert_eq!(failure.detail(), Some("HTTP 404 model not found"));
    }

    /// §5a.5 (docs/plans/2026-07-03-loop-failure-matrix.md): the planned
    /// driver maps an in-flight `Cancelled` executor error to
    /// `interrupted_unexpectedly`; runner sanitization must preserve that
    /// category instead of overwriting it with the generic `driver_failed`.
    #[test]
    fn sanitized_driver_failure_preserves_interrupted_unexpectedly_category() {
        let failure = sanitized_driver_failure("interrupted_unexpectedly", None)
            .expect("interrupted_unexpectedly is a valid category");

        assert_eq!(failure.category(), "interrupted_unexpectedly");
        assert_eq!(failure.detail(), None);
    }

    #[test]
    fn sanitized_driver_failure_preserves_budget_accounting_category() {
        let failure = sanitized_driver_failure(
            BUDGET_ACCOUNTING_FAILED_CATEGORY,
            Some("resource accounting storage is unavailable"),
        )
        .expect("budget accounting category is valid");

        assert_eq!(failure.category(), BUDGET_ACCOUNTING_FAILED_CATEGORY);
        assert_eq!(
            failure.detail(),
            Some("resource accounting storage is unavailable")
        );
    }
}
