/// Failure category identifier for model provider credit exhaustion.
/// Exposed for cross-crate consumers that project this category to a user-facing message.
pub const MODEL_CREDITS_EXHAUSTED_CATEGORY: &str = "model_credits_exhausted";

/// Failure category identifier for model provider credential or endpoint configuration failures.
/// Exposed for cross-crate consumers that project this category to a user-facing message.
pub const MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY: &str = "model_credentials_unavailable";

/// Failure category for durable host-side resource accounting outages.
/// This must not be presented as a provider balance or configured-budget outcome.
pub const BUDGET_ACCOUNTING_FAILED_CATEGORY: &str = "budget_accounting_failed";

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

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_agent_loop::executor::HostStage;

    #[test]
    fn host_stage_unavailable_categories_cover_every_host_stage() {
        use HostStage as S;

        let stage_reason_and_category = |stage| match stage {
            S::Prompt => ("prompt", HOST_STAGE_UNAVAILABLE_PROMPT_CATEGORY),
            S::Model => ("model", HOST_STAGE_UNAVAILABLE_MODEL_CATEGORY),
            S::Capability => ("capability", HOST_STAGE_UNAVAILABLE_CAPABILITY_CATEGORY),
            S::Transcript => ("transcript", HOST_STAGE_UNAVAILABLE_TRANSCRIPT_CATEGORY),
            S::Checkpoint => ("checkpoint", HOST_STAGE_UNAVAILABLE_CHECKPOINT_CATEGORY),
            S::Input => ("input", HOST_STAGE_UNAVAILABLE_INPUT_CATEGORY),
        };

        for stage in [
            S::Prompt,
            S::Model,
            S::Capability,
            S::Transcript,
            S::Checkpoint,
            S::Input,
        ] {
            let (reason_prefix, category) = stage_reason_and_category(stage);
            let reason = format!("{reason_prefix}: unavailable");
            assert_eq!(
                host_stage_unavailable_category(&reason),
                category,
                "host stage {stage:?} must map to its unavailable category"
            );
        }
    }

    #[test]
    fn host_stage_unavailable_reason_prefixes_are_classified() {
        for (reason, expected) in [
            (
                "prompt: unavailable",
                HOST_STAGE_UNAVAILABLE_PROMPT_CATEGORY,
            ),
            ("model: unavailable", HOST_STAGE_UNAVAILABLE_MODEL_CATEGORY),
            (
                "capability: unavailable",
                HOST_STAGE_UNAVAILABLE_CAPABILITY_CATEGORY,
            ),
            (
                "transcript: unavailable",
                HOST_STAGE_UNAVAILABLE_TRANSCRIPT_CATEGORY,
            ),
            (
                "checkpoint: unavailable",
                HOST_STAGE_UNAVAILABLE_CHECKPOINT_CATEGORY,
            ),
            ("input: unavailable", HOST_STAGE_UNAVAILABLE_INPUT_CATEGORY),
            (
                "unexpected: unavailable",
                HOST_STAGE_UNAVAILABLE_UNKNOWN_CATEGORY,
            ),
        ] {
            assert_eq!(
                host_stage_unavailable_category(reason),
                expected,
                "{reason}"
            );
        }
    }
}
