//! Failure **classification** — deciding which category a failed stage gets.
//!
//! The category identifiers themselves are data and live in
//! `ironclaw_host_api::failure::categories` (moved there by WS1.7 / PROPOSAL
//! §6.1.1, so the product projection can key user-facing copy off them without
//! depending on this crate). What stays here is the part that is *not* data:
//! the stage→category mapping, which is typed on agent-loop vocabulary this
//! crate is allowed to see and `ironclaw_host_api` is not.

use ironclaw_host_api::failure::categories::{
    HOST_STAGE_UNAVAILABLE_CAPABILITY_CATEGORY, HOST_STAGE_UNAVAILABLE_CHECKPOINT_CATEGORY,
    HOST_STAGE_UNAVAILABLE_INPUT_CATEGORY, HOST_STAGE_UNAVAILABLE_MODEL_CATEGORY,
    HOST_STAGE_UNAVAILABLE_PROMPT_CATEGORY, HOST_STAGE_UNAVAILABLE_TRANSCRIPT_CATEGORY,
    HOST_STAGE_UNAVAILABLE_UNKNOWN_CATEGORY,
};

pub(crate) const MODEL_CREDITS_EXHAUSTED_REASON_KIND:
    ironclaw_loop_contracts::AgentLoopHostErrorReasonKind =
    ironclaw_loop_contracts::AgentLoopHostErrorReasonKind::ModelCreditsExhausted;

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
