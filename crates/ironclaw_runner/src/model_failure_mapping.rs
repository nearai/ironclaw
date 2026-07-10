use ironclaw_turns::run_profile::{AgentLoopHostErrorKind, AgentLoopHostErrorReasonKind};

use crate::failure_categories::{
    MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY, MODEL_CREDITS_EXHAUSTED_CATEGORY,
    MODEL_CREDITS_EXHAUSTED_REASON_KIND,
};

pub(crate) fn model_stage_failure_category(
    is_model_stage: bool,
    kind: AgentLoopHostErrorKind,
    reason_kind: Option<AgentLoopHostErrorReasonKind>,
) -> Option<&'static str> {
    if !is_model_stage {
        return None;
    }

    if reason_kind == Some(MODEL_CREDITS_EXHAUSTED_REASON_KIND) {
        return Some(MODEL_CREDITS_EXHAUSTED_CATEGORY);
    }

    (kind == AgentLoopHostErrorKind::CredentialUnavailable)
        .then_some(MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY)
}
