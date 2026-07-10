use ironclaw_runner::failure_categories::{
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
            "The run could not start because the configured agent runtime was unavailable."
        }
        "driver_unavailable" => "The run could not start the agent runtime.",
        "driver_failed" => "The agent runtime reported an internal error before producing a reply.",
        "driver_invalid_request" => {
            "The agent runtime rejected the request before producing a reply."
        }
        "scheduler_executor_panic" => "The agent runtime stopped unexpectedly.",
        "host_creation_failed" => {
            "The run failed while preparing the runtime host. Retry the run, and contact support if startup keeps failing."
        }
        "route_snapshot_persistence_failed" => {
            "The run failed while saving the selected model route. Retry the run."
        }
        "scheduler_heartbeat_failed" => {
            "The run failed after the runner heartbeat could not be recorded."
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
        "model_invalid_output" => {
            "The run failed because the model returned output the runner could not use. Retry the run or choose a different model."
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
            "The agent runtime reported an internal error. Retry the run, and contact support if it happens again."
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
        "driver_protocol_violation" => {
            "The run produced an invalid result and stopped before replying. Retry the run, and contact support if it keeps happening."
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

pub(crate) fn reborn_failure_summary_for_category_and_detail(
    category: Option<&str>,
    detail: Option<InvalidModelOutputFailureDetail>,
) -> &'static str {
    let Some(category) = category else {
        return unknown_failure_summary();
    };

    if let Some(summary) = pinned_failure_summary_for_category(category) {
        return summary;
    }

    if matches!(category, "model_invalid_output" | "invalid_model_output")
        && let Some(detail) = detail
    {
        return detail.failure_summary();
    }

    reborn_failure_summary_for_category(Some(category))
}

const INVALID_MODEL_OUTPUT_DETAIL_MAX_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvalidModelOutputFailureDetail {
    EmptyAssistantResponse,
    TextualToolCallSyntax,
    OutsideCapabilitySurface,
    ToolUseFinishWithoutToolCalls,
    UnsupportedToolCallsForTextOnlyLoop,
    InvalidReturnedToolName,
    InvalidToolCallArguments,
    MalformedToolCallArguments,
}

impl InvalidModelOutputFailureDetail {
    /// Classify the host-authored model-gateway safe summaries emitted for
    /// invalid model output by `ironclaw_runner::model_gateway`.
    ///
    /// This list intentionally mirrors the runner's fixed `InvalidOutput`
    /// summaries without making those wording details part of `ironclaw_runner`
    /// public API. Keep `invalid_model_output_detail_whitelist_covers_known_runner_summaries`
    /// in sync when adding or renaming one of those upstream safe summaries.
    pub(crate) fn from_failure_category_and_projection_detail(
        category: &str,
        detail: Option<&str>,
    ) -> Option<Self> {
        if !matches!(category, "model_invalid_output" | "invalid_model_output") {
            return None;
        }
        Self::from_projection_detail(detail)
    }

    pub(crate) fn from_projection_detail(detail: Option<&str>) -> Option<Self> {
        let detail = detail?;
        if !is_invalid_model_output_projection_detail_shape(detail) {
            return None;
        }
        match detail {
            "model returned an empty assistant response" => Some(Self::EmptyAssistantResponse),
            "model returned textual tool-call syntax instead of structured tool calls" => {
                Some(Self::TextualToolCallSyntax)
            }
            "model returned a tool call outside the advertised capability surface" => {
                Some(Self::OutsideCapabilitySurface)
            }
            "model returned tool-use finish without tool calls" => {
                Some(Self::ToolUseFinishWithoutToolCalls)
            }
            "model returned unsupported tool calls for a text-only loop" => {
                Some(Self::UnsupportedToolCallsForTextOnlyLoop)
            }
            "model returned an invalid provider tool name" => Some(Self::InvalidReturnedToolName),
            "model returned invalid tool-call arguments" => Some(Self::InvalidToolCallArguments),
            _ if detail.starts_with("failed to parse tool-call arguments JSON:") => {
                Some(Self::MalformedToolCallArguments)
            }
            _ => None,
        }
    }

    fn failure_summary(self) -> &'static str {
        match self {
            Self::EmptyAssistantResponse => {
                "The run failed because the model returned an empty assistant response. Retry the run or choose a different model."
            }
            Self::TextualToolCallSyntax => {
                "The run failed because the model returned a tool call as text instead of structured tool-call data. Retry the run or choose a different model."
            }
            Self::OutsideCapabilitySurface => {
                "The run failed because the model tried to call a tool that was not available in this turn. Retry with a narrower request or choose a different model."
            }
            Self::ToolUseFinishWithoutToolCalls => {
                "The run failed because the model requested tool use without providing structured tool calls. Retry the run or choose a different model."
            }
            Self::UnsupportedToolCallsForTextOnlyLoop => {
                "The run failed because the model tried to call a tool when this turn required a text answer. Retry with a clearer request or choose a different model."
            }
            Self::InvalidReturnedToolName => {
                "The run failed because the model returned an invalid tool name. Retry the run or choose a different model."
            }
            Self::InvalidToolCallArguments => {
                "The run failed because the model returned invalid tool-call arguments. Retry with a clearer or narrower request."
            }
            Self::MalformedToolCallArguments => {
                "The run failed because the model returned malformed tool-call arguments. Retry with a clearer or narrower request."
            }
        }
    }
}

fn is_invalid_model_output_projection_detail_shape(detail: &str) -> bool {
    if detail.is_empty() || detail.len() > INVALID_MODEL_OUTPUT_DETAIL_MAX_BYTES {
        return false;
    }
    if !detail.is_ascii() {
        return false;
    }
    let bytes = detail.as_bytes();
    !bytes[0].is_ascii_whitespace()
        && !bytes[bytes.len() - 1].is_ascii_whitespace()
        && !bytes.iter().any(u8::is_ascii_control)
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
    use super::{
        InvalidModelOutputFailureDetail, reborn_failure_summary_for_category,
        reborn_failure_summary_for_category_and_detail,
    };

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

    #[test]
    fn invalid_model_output_detail_summary_uses_typed_whitelist() {
        let detail = InvalidModelOutputFailureDetail::from_projection_detail(Some(
            "model returned an empty assistant response",
        ));

        assert_eq!(
            detail,
            Some(InvalidModelOutputFailureDetail::EmptyAssistantResponse)
        );
        assert_eq!(
            reborn_failure_summary_for_category_and_detail(Some("model_invalid_output"), detail),
            "The run failed because the model returned an empty assistant response. Retry the run or choose a different model."
        );
    }

    #[test]
    fn invalid_model_output_detail_whitelist_covers_known_runner_summaries() {
        use InvalidModelOutputFailureDetail as Detail;

        for (safe_summary, expected) in [
            (
                "model returned an empty assistant response",
                Detail::EmptyAssistantResponse,
            ),
            (
                "model returned textual tool-call syntax instead of structured tool calls",
                Detail::TextualToolCallSyntax,
            ),
            (
                "model returned a tool call outside the advertised capability surface",
                Detail::OutsideCapabilitySurface,
            ),
            (
                "model returned tool-use finish without tool calls",
                Detail::ToolUseFinishWithoutToolCalls,
            ),
            (
                "model returned unsupported tool calls for a text-only loop",
                Detail::UnsupportedToolCallsForTextOnlyLoop,
            ),
            (
                "model returned an invalid provider tool name",
                Detail::InvalidReturnedToolName,
            ),
            (
                "model returned invalid tool-call arguments",
                Detail::InvalidToolCallArguments,
            ),
            (
                "failed to parse tool-call arguments JSON: expected value at line 1 column 1",
                Detail::MalformedToolCallArguments,
            ),
        ] {
            assert_eq!(
                InvalidModelOutputFailureDetail::from_failure_category_and_projection_detail(
                    "model_invalid_output",
                    Some(safe_summary),
                ),
                Some(expected),
                "{safe_summary:?} should remain mapped to a specific summary"
            );
        }
    }

    #[test]
    fn invalid_model_output_detail_matching_is_category_gated() {
        assert_eq!(
            InvalidModelOutputFailureDetail::from_failure_category_and_projection_detail(
                "model_unavailable",
                Some("model returned an empty assistant response"),
            ),
            None
        );
    }

    #[test]
    fn invalid_model_output_detail_matching_rejects_unvalidated_detail() {
        let oversized = format!(
            "failed to parse tool-call arguments JSON: {}",
            "x".repeat(512)
        );

        for detail in [
            " model returned an empty assistant response",
            "model returned an empty assistant response\n",
            "model returned an empty assistant response\0",
            oversized.as_str(),
        ] {
            assert_eq!(
                InvalidModelOutputFailureDetail::from_projection_detail(Some(detail)),
                None,
                "{detail:?} should not be accepted for projection matching"
            );
        }

        assert_eq!(
            reborn_failure_summary_for_category_and_detail(Some("model_invalid_output"), None),
            reborn_failure_summary_for_category(Some("model_invalid_output"))
        );
    }

    // The scheduler emits `scheduler_heartbeat_failed` / `scheduler_executor_panic`
    // (see `ironclaw_runner::turn_scheduler`), not the previously-matched
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
            "The run failed because a capability returned an invalid protocol response. Retry the run, and contact support if it keeps happening."
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
            "The run failed before producing a reply. Retry the run, and contact support if it keeps happening."
        );
        assert_eq!(
            reborn_failure_summary_for_category(Some("driver_panic")),
            "The run failed before producing a reply. Retry the run, and contact support if it keeps happening."
        );
    }
}
