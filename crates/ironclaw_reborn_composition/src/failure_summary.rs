use ironclaw_reborn::failure_categories::{
    MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY, MODEL_CREDITS_EXHAUSTED_CATEGORY,
};

pub fn reborn_failure_summary_for_category(category: Option<&str>) -> &'static str {
    let Some(category) = category else {
        return unknown_failure_summary();
    };

    if let Some(summary) = pinned_failure_summary_for_category(category) {
        return summary;
    }

    match category {
        "driver_not_found" => {
            "The run failed because the configured execution driver was not available. Check the runtime configuration and try again."
        }
        "driver_unavailable" => {
            "The run failed because the execution driver was temporarily unavailable. Retry the run."
        }
        "driver_failed" => {
            "The run failed because the execution driver reported an error. Retry the run, and contact support if it happens again."
        }
        "driver_invalid_request" => {
            "The run failed because the execution driver rejected the request. Retry with a supported request."
        }
        "driver_panic" => {
            "The run failed because the execution driver stopped unexpectedly. Retry the run, and contact support if it happens again."
        }
        "host_creation_failed" => {
            "The run failed while preparing the runtime host. Retry the run, and contact support if startup keeps failing."
        }
        "route_snapshot_persistence_failed" => {
            "The run failed while saving the selected model route. Retry the run."
        }
        "heartbeat_failed" => {
            "The run failed after the runner heartbeat could not be recorded. Retry the run."
        }
        "exit_application_failed" => {
            "The run failed while recording its final result. Retry the run, and contact support if results keep failing to save."
        }
        "lease_expired" => "The run failed because its runner lease expired. Retry the run.",
        "model_error" => {
            "The run failed while calling the model. Check the selected model provider and try again."
        }
        "model_transient" => "The run failed after a temporary model error. Retry the run.",
        "model_context_overflow" => {
            "The run failed because the model context was too large. Retry with a shorter request or start a new thread."
        }
        "model_content_filtered" => {
            "The run failed because the model provider filtered the response. Change the request and try again."
        }
        "model_unavailable" => {
            "The run failed because the model provider was unavailable. Check the selected provider and retry the run."
        }
        "model_internal" => {
            "The run failed because the model provider returned an internal error. Retry the run or choose a different provider."
        }
        "context_build_failed" => {
            "The run failed while building the model context. Retry the run, and contact support if it keeps happening."
        }
        "capability_protocol_error" => {
            "The run failed because a capability returned an invalid protocol response. Retry the run, and contact support if it keeps happening."
        }
        "capability_transient" => "The run failed after a temporary tool error. Retry the run.",
        "capability_permanent" => {
            "The run failed because a tool reported a permanent error. Change the request or tool configuration and try again."
        }
        "capability_input_invalid" => {
            "The run failed because a tool rejected its input. Retry with a clearer or narrower request."
        }
        "capability_operation_failed" => {
            "The run failed because a tool operation did not complete. Retry the run, and check the tool integration if it keeps happening."
        }
        "capability_policy_denied" => {
            "The run failed because a tool policy denied the requested action. Change the request or permissions and try again."
        }
        "capability_unavailable" => {
            "The run failed because a required tool was unavailable. Retry the run, and check the tool integration if it keeps happening."
        }
        "capability_internal" => {
            "The run failed because a tool returned an internal error. Retry the run, and check the tool integration if it keeps happening."
        }
        "iteration_limit" => {
            "The run stopped after reaching its iteration limit before producing a reply. Retry with a narrower request or increase the limit."
        }
        "invalid_model_output" => {
            "The run failed because the model returned output the runner could not use. Retry the run or choose a different model."
        }
        "checkpoint_rejected" => {
            "The run failed because its checkpoint was rejected. Retry from the last available checkpoint or start a new run."
        }
        "checkpoint_unavailable" => {
            "The run failed because the checkpoint could not be loaded. Retry the run, and contact support if the checkpoint remains unavailable."
        }
        "transcript_write_failed" => {
            "The run failed while saving transcript output. Retry the run, and contact support if saving still fails."
        }
        "driver_bug" => {
            "The run failed because the execution driver hit an internal bug. Retry the run, and contact support if it happens again."
        }
        "interrupted_unexpectedly" => {
            "The run stopped unexpectedly before it could finish. Retry the run."
        }
        "no_progress_detected" => {
            "The run stopped because it repeated work without making progress. Retry with a clearer instruction or narrower scope."
        }
        "policy_denied" => {
            "The run stopped because a policy denied the requested action. Change the request or permissions and try again."
        }
        "compaction_unavailable" => {
            "The run failed because context compaction was unavailable. Retry with a shorter request or start a new thread."
        }
        "compaction_invalid_cut_point" => {
            "The run failed because context compaction selected an invalid cut point. Retry the run, and contact support if it keeps happening."
        }
        "compaction_unsupported_mode" => {
            "The run failed because the requested context compaction mode is unsupported. Retry with a shorter request or start a new thread."
        }
        "compaction_input_too_large" => {
            "The run failed because context compaction input was too large. Retry with a shorter request or start a new thread."
        }
        "compaction_security_rejected" => {
            "The run failed because context compaction was rejected by a safety check. Change the request and try again."
        }
        "compaction_inference_failed" => {
            "The run failed because context compaction could not complete. Retry with a shorter request or start a new thread."
        }
        "compaction_cancelled" => {
            "The run stopped while context compaction was being cancelled. Retry the run if you still need a response."
        }
        "compaction_persistence_failed" => {
            "The run failed while saving compacted context. Retry the run, and contact support if saving still fails."
        }
        "host_stage_unavailable_prompt" => {
            "The run failed because the host prompt stage was unavailable. Retry the run, and contact support if it keeps happening."
        }
        "host_stage_unavailable_model" => {
            "The run failed because the host model stage was unavailable. Check the model provider and try again."
        }
        "host_stage_unavailable_capability" => {
            "The run failed because the host capability stage was unavailable. Retry the run, and check the tool integration if it keeps happening."
        }
        "host_stage_unavailable_transcript" => {
            "The run failed because the host transcript stage was unavailable. Retry the run, and contact support if saving still fails."
        }
        "host_stage_unavailable_checkpoint" => {
            "The run failed because the host checkpoint stage was unavailable. Retry the run, and contact support if checkpoints remain unavailable."
        }
        "host_stage_unavailable_input" => {
            "The run failed because the host input stage was unavailable. Check the submitted message and try again."
        }
        "host_stage_unavailable_unknown" => {
            "The run failed because a required host stage was unavailable. Retry the run, and contact support if it keeps happening."
        }
        "unknown_failure" => unknown_failure_summary(),
        _ => unknown_failure_summary(),
    }
}

pub(crate) fn pinned_failure_summary_for_category(category: &str) -> Option<&'static str> {
    match category {
        MODEL_CREDITS_EXHAUSTED_CATEGORY => Some(
            "The AI provider account is out of credits. Add credits or switch providers and try again.",
        ),
        MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY => Some(
            "The run failed because model credentials or provider configuration are invalid. Check the selected provider's API key and base URL, then try again.",
        ),
        _ => None,
    }
}

fn unknown_failure_summary() -> &'static str {
    "The run failed before producing a reply. Retry the run, and contact support if it keeps happening."
}

#[cfg(test)]
mod tests {
    use super::reborn_failure_summary_for_category;

    #[test]
    fn reborn_failure_summary_describes_known_category() {
        assert_eq!(
            reborn_failure_summary_for_category(Some("driver_invalid_request")),
            "The run failed because the execution driver rejected the request. Retry with a supported request."
        );
    }

    #[test]
    fn reborn_failure_summary_describes_iteration_limit() {
        assert_eq!(
            reborn_failure_summary_for_category(Some("iteration_limit")),
            "The run stopped after reaching its iteration limit before producing a reply. Retry with a narrower request or increase the limit."
        );
    }

    #[test]
    fn reborn_failure_summary_falls_back_for_unknown_category() {
        assert_eq!(
            reborn_failure_summary_for_category(Some("unexpected_category")),
            "The run failed before producing a reply. Retry the run, and contact support if it keeps happening."
        );
    }
}
