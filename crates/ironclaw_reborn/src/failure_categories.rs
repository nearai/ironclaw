/// Failure category identifier for model provider credit exhaustion.
/// Exposed for cross-crate consumers that project this category to a user-facing message.
pub const MODEL_CREDITS_EXHAUSTED_CATEGORY: &str = "model_credits_exhausted";

/// Failure category identifier for model provider credential or endpoint configuration failures.
/// Exposed for cross-crate consumers that project this category to a user-facing message.
pub const MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY: &str = "model_credentials_unavailable";

pub const HOST_STAGE_UNAVAILABLE_PROMPT_CATEGORY: &str = "host_stage_unavailable_prompt";
pub const HOST_STAGE_UNAVAILABLE_MODEL_CATEGORY: &str = "host_stage_unavailable_model";
pub const HOST_STAGE_UNAVAILABLE_CAPABILITY_CATEGORY: &str = "host_stage_unavailable_capability";
pub const HOST_STAGE_UNAVAILABLE_TRANSCRIPT_CATEGORY: &str = "host_stage_unavailable_transcript";
pub const HOST_STAGE_UNAVAILABLE_CHECKPOINT_CATEGORY: &str = "host_stage_unavailable_checkpoint";
pub const HOST_STAGE_UNAVAILABLE_INPUT_CATEGORY: &str = "host_stage_unavailable_input";
pub const HOST_STAGE_UNAVAILABLE_UNKNOWN_CATEGORY: &str = "host_stage_unavailable_unknown";

pub(crate) const MODEL_CREDITS_EXHAUSTED_REASON_KIND:
    ironclaw_turns::run_profile::AgentLoopHostErrorReasonKind =
    ironclaw_turns::run_profile::AgentLoopHostErrorReasonKind::ModelCreditsExhausted;

pub(crate) fn host_stage_unavailable_category(reason: &str) -> &'static str {
    let stage = reason
        .split_once(':')
        .map(|(stage, _)| stage)
        .unwrap_or(reason)
        .trim();
    match stage.to_ascii_lowercase().as_str() {
        "prompt" => HOST_STAGE_UNAVAILABLE_PROMPT_CATEGORY,
        "model" => HOST_STAGE_UNAVAILABLE_MODEL_CATEGORY,
        "capability" => HOST_STAGE_UNAVAILABLE_CAPABILITY_CATEGORY,
        "transcript" => HOST_STAGE_UNAVAILABLE_TRANSCRIPT_CATEGORY,
        "checkpoint" => HOST_STAGE_UNAVAILABLE_CHECKPOINT_CATEGORY,
        "input" => HOST_STAGE_UNAVAILABLE_INPUT_CATEGORY,
        _ => HOST_STAGE_UNAVAILABLE_UNKNOWN_CATEGORY,
    }
}
