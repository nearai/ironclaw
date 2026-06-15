use ironclaw_reborn::failure_categories::{
    MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY, MODEL_CREDITS_EXHAUSTED_CATEGORY,
};

pub fn reborn_failure_summary_for_category(category: Option<&str>) -> &'static str {
    let Some(category) = category else {
        return "The run failed before producing a reply.";
    };

    if let Some(summary) = pinned_failure_summary_for_category(category) {
        return summary;
    }

    match category {
        "driver_not_found" => {
            "The run failed because the configured execution driver was not available."
        }
        "driver_unavailable" => {
            "The run failed because the execution driver was temporarily unavailable."
        }
        "driver_failed" => "The run failed because the execution driver reported an error.",
        "driver_invalid_request" => {
            "The run failed because the execution driver rejected the request."
        }
        "driver_panic" => "The run failed because the execution driver stopped unexpectedly.",
        "host_creation_failed" => "The run failed while preparing the runtime host.",
        "route_snapshot_persistence_failed" => {
            "The run failed while saving the selected model route."
        }
        "heartbeat_failed" => "The run failed after the runner heartbeat could not be recorded.",
        "exit_application_failed" => "The run failed while recording its final result.",
        "lease_expired" => "The run failed because its runner lease expired.",
        "interrupted_unexpectedly" => "The run stopped before it could complete cleanly.",
        "no_progress_detected" => {
            "The run stopped because it repeated the same step without making progress."
        }
        "iteration_limit" => {
            "The run stopped after reaching its iteration limit before producing a reply."
        }
        "unknown_failure" => "The run failed for an unknown reason.",
        _ => "The run failed before producing a reply.",
    }
}

pub(crate) fn pinned_failure_summary_for_category(category: &str) -> Option<&'static str> {
    match category {
        MODEL_CREDITS_EXHAUSTED_CATEGORY => Some(
            "The AI provider account is out of credits. Add credits or switch providers and try again.",
        ),
        MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY => Some(
            "The run failed because model credentials or provider configuration are invalid. Check the selected provider's API key and base URL.",
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::reborn_failure_summary_for_category;

    #[test]
    fn reborn_failure_summary_describes_known_category() {
        assert_eq!(
            reborn_failure_summary_for_category(Some("driver_invalid_request")),
            "The run failed because the execution driver rejected the request."
        );
    }

    #[test]
    fn reborn_failure_summary_describes_iteration_limit() {
        assert_eq!(
            reborn_failure_summary_for_category(Some("iteration_limit")),
            "The run stopped after reaching its iteration limit before producing a reply."
        );
    }

    #[test]
    fn reborn_failure_summary_falls_back_for_unknown_category() {
        assert_eq!(
            reborn_failure_summary_for_category(Some("unexpected_category")),
            "The run failed before producing a reply."
        );
    }
}
