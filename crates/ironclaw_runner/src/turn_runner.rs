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
    MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY, MODEL_CREDITS_EXHAUSTED_CATEGORY,
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
    let base = if matches!(
        reason_kind,
        MODEL_CREDITS_EXHAUSTED_CATEGORY | MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY
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
}
