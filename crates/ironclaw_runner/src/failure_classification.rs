//! Run-failure classification facade.
//!
//! This joins the runner-owned failure category, user-facing summary, retry
//! disposition, and lane into one pure classifier. Producers still emit the
//! narrow category string; consumers that need policy should use this module
//! instead of independently recomputing lane and retry behavior.

use crate::{
    failure_lane::{FailureLane, failure_lane},
    failure_summary::ironclaw_failure_summary_for_category,
    retry_disposition::{RetryDisposition, retry_disposition},
};

/// Full policy classification for a terminal run failure category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunFailureClassification {
    pub lane: FailureLane,
    pub retry: RetryDisposition,
    pub user_message: &'static str,
}

/// Classify a terminal run failure category.
///
/// `retryable` is the host-derived signal: the failed run has a resumable
/// checkpoint. `None` or an unknown category remains explainable via the generic
/// failure summary; if a checkpoint exists it is still user-retriable.
pub fn classify_run_failure(category: Option<&str>, retryable: bool) -> RunFailureClassification {
    let category = category.unwrap_or("unknown_failure");
    let retry = retry_disposition(category, retryable);
    RunFailureClassification {
        lane: failure_lane(category, retryable),
        retry,
        user_message: ironclaw_failure_summary_for_category(Some(category)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failure_lane::ALL_RUN_FAILURE_CATEGORIES;

    #[test]
    fn classifier_combines_lane_retry_and_summary_for_every_known_category() {
        for category in ALL_RUN_FAILURE_CATEGORIES {
            let retryable = classify_run_failure(Some(category), true);
            assert_eq!(retryable.lane, FailureLane::Retriable, "{category}");
            assert_ne!(retryable.retry, RetryDisposition::NoRetry, "{category}");
            assert!(
                !retryable.user_message.trim().is_empty(),
                "{category} must carry a user-facing explanation"
            );

            let non_retryable = classify_run_failure(Some(category), false);
            assert_eq!(non_retryable.lane, FailureLane::Explainable, "{category}");
            assert_eq!(non_retryable.retry, RetryDisposition::NoRetry, "{category}");
            assert_eq!(non_retryable.user_message, retryable.user_message);
        }
    }

    #[test]
    fn unknown_category_stays_explainable_and_user_retriable_with_checkpoint() {
        let classification = classify_run_failure(Some("__unknown__"), true);

        assert_eq!(classification.lane, FailureLane::Retriable);
        assert_eq!(classification.retry, RetryDisposition::UserInitiated);
        assert_eq!(
            classification.user_message,
            "The run failed before producing a reply. Retry the run, and contact support if it keeps happening."
        );
    }

    #[test]
    fn missing_category_uses_generic_explainable_summary() {
        let classification = classify_run_failure(None, false);

        assert_eq!(classification.lane, FailureLane::Explainable);
        assert_eq!(classification.retry, RetryDisposition::NoRetry);
        assert_eq!(
            classification.user_message,
            "The run failed before producing a reply. Retry the run, and contact support if it keeps happening."
        );
    }
}
