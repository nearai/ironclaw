//! Scripted raw-provider seam for Reborn integration tests. Reuses `TraceLlm`'s
//! replay engine (no new provider): builds an in-memory `LlmTrace` from the
//! `RebornScriptedReply` façade and returns a `TraceLlm` to sit at the bottom of
//! the real `ironclaw_llm` decorator chain (design §3.1/§3.3).

use super::reply::RebornScriptedReply;
use crate::support::trace_llm::{LlmTrace, TraceLlm, TraceTurn};

/// Model name surfaced by the scripted provider. Non-empty and not "default" so
/// the Reborn model gateway's model-override resolution accepts it.
pub const SCRIPTED_MODEL_NAME: &str = "scripted/integration-test";

/// Build a `TraceLlm` that replays the given scripted replies in order.
pub fn scripted_trace_llm(replies: impl IntoIterator<Item = RebornScriptedReply>) -> TraceLlm {
    let steps = replies
        .into_iter()
        .map(RebornScriptedReply::into_step)
        .collect();
    let trace = LlmTrace::new(
        SCRIPTED_MODEL_NAME,
        vec![TraceTurn {
            user_input: "(scripted)".to_string(),
            steps,
            expects: Default::default(),
        }],
    );
    TraceLlm::from_trace(trace)
}
