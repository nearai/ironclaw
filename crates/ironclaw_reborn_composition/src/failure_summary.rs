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
            "The run could not start because the configured agent runtime was unavailable."
        }
        "driver_unavailable" => "The run could not start the agent runtime.",
        "driver_failed" => "The agent runtime reported an internal error before producing a reply.",
        "driver_invalid_request" => {
            "The agent runtime rejected the request before producing a reply."
        }
        "scheduler_executor_panic" => "The agent runtime stopped unexpectedly.",
        "host_creation_failed" => "The run failed while preparing the runtime host.",
        "route_snapshot_persistence_failed" => {
            "The run failed while saving the selected model route."
        }
        "scheduler_heartbeat_failed" => {
            "The run failed after the runner heartbeat could not be recorded."
        }
        "exit_application_failed" => "The run failed while recording its final result.",
        "lease_expired" => "The run failed because its runner lease expired.",
        "interrupted_unexpectedly" => "The run stopped before it could complete cleanly.",
        "no_progress_detected" => {
            "The run stopped because it repeated the same step without making progress."
        }
        "iteration_limit" => {
            "The run stopped after reaching its iteration limit before producing a reply."
        }
        // Categories below come from `LoopFailureKind::as_str()` via the normal
        // loop-exit path (`ironclaw_turns::loop_exit`), not the driver-error
        // path above. They were previously unmapped and degraded to the generic
        // fallback, which masked the real failure (a tool failure surfaced to
        // the user as a vague "driver protocol error").
        "capability_protocol_error" => {
            "The run stopped because a tool returned a response it could not process."
        }
        "model_error" => "The run stopped because the model could not complete the request.",
        "context_build_failed" => {
            "The run failed while preparing the conversation context for the model."
        }
        "invalid_model_output" => {
            "The run stopped because the model returned a response that could not be parsed."
        }
        "checkpoint_rejected" => "The run failed while saving a progress checkpoint.",
        "checkpoint_unavailable" => {
            "The run could not resume because its saved progress was unavailable."
        }
        "transcript_write_failed" => "The run failed while recording its transcript.",
        "driver_bug" => "The run stopped because of an internal error in the agent runtime.",
        "policy_denied" => "The run stopped because an action it attempted was not permitted.",
        "compaction_unavailable" => {
            "The run stopped because it could not free up context space to continue."
        }
        "driver_protocol_violation" => {
            "The run produced an invalid result and stopped before replying."
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
            "The agent runtime rejected the request before producing a reply."
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

    // The scheduler emits `scheduler_heartbeat_failed` / `scheduler_executor_panic`
    // (see `ironclaw_host_runtime::turn_scheduler`), not the previously-matched
    // `heartbeat_failed` / `driver_panic`. These two assertions pin the live
    // mapping to the real producer strings.
    #[test]
    fn reborn_failure_summary_describes_scheduler_heartbeat_failure() {
        assert_eq!(
            reborn_failure_summary_for_category(Some("scheduler_heartbeat_failed")),
            "The run failed after the runner heartbeat could not be recorded."
        );
    }

    #[test]
    fn reborn_failure_summary_describes_scheduler_executor_panic() {
        assert_eq!(
            reborn_failure_summary_for_category(Some("scheduler_executor_panic")),
            "The agent runtime stopped unexpectedly."
        );
    }

    #[test]
    fn reborn_failure_summary_omits_internal_system_tool_language() {
        for category in [
            "driver_not_found",
            "driver_unavailable",
            "driver_failed",
            "driver_invalid_request",
            "scheduler_executor_panic",
        ] {
            let summary = reborn_failure_summary_for_category(Some(category)).to_ascii_lowercase();

            assert!(
                !summary.contains("system tool"),
                "{category} leaked system tool wording"
            );
            assert!(
                !summary.contains("temporarily unavailable"),
                "{category} leaked transient host wording"
            );
            assert!(
                !summary.contains("execution driver"),
                "{category} leaked execution driver wording"
            );
        }
    }

    // Regression guard: categories emitted by `LoopFailureKind::as_str()`
    // through the normal loop-exit path must map to specific, honest summaries
    // instead of degrading to the generic fallback (which the LLM failure
    // explainer then paraphrased into a vague "driver protocol error" that
    // masked the real tool failure).
    #[test]
    fn reborn_failure_summary_describes_capability_protocol_error() {
        assert_eq!(
            reborn_failure_summary_for_category(Some("capability_protocol_error")),
            "The run stopped because a tool returned a response it could not process."
        );
    }

    #[test]
    fn reborn_failure_summary_maps_loop_failure_categories_specifically() {
        let generic = reborn_failure_summary_for_category(None);
        for category in [
            "capability_protocol_error",
            "model_error",
            "context_build_failed",
            "invalid_model_output",
            "checkpoint_rejected",
            "checkpoint_unavailable",
            "transcript_write_failed",
            "driver_bug",
            "policy_denied",
            "compaction_unavailable",
            "driver_protocol_violation",
        ] {
            let summary = reborn_failure_summary_for_category(Some(category));
            assert_ne!(
                summary, generic,
                "{category} still degrades to the generic failure summary"
            );
        }
    }

    // Regression guard: the old, never-produced category strings must no longer
    // be specially cased — they now fall through to the generic summary.
    #[test]
    fn reborn_failure_summary_treats_legacy_dead_categories_as_generic() {
        assert_eq!(
            reborn_failure_summary_for_category(Some("heartbeat_failed")),
            "The run failed before producing a reply."
        );
        assert_eq!(
            reborn_failure_summary_for_category(Some("driver_panic")),
            "The run failed before producing a reply."
        );
    }
}
