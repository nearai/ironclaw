use async_trait::async_trait;
use ironclaw_turns::run_profile::LoopPromptBundleRequest;

use crate::state::LoopExecutionState;

/// Decides what context the host should materialize for the next model call.
///
/// Pure policy: returns the request value the executor will pass to
/// `LoopPromptPort::build_prompt_bundle`. Does NOT mutate state.
///
/// Inline messages flow through the `inline_messages` field of
/// `LoopPromptBundleRequest`. There is no separate nudge strategy; loop
/// families that need nudges extend their context strategy to populate this
/// field from `state`.
///
/// See `docs/reborn/agent-loop-skeleton.md` section 6.
#[async_trait]
pub(crate) trait ContextStrategy: Send + Sync {
    async fn plan_context_request(&self, state: &LoopExecutionState) -> LoopPromptBundleRequest;
}

#[allow(dead_code)]
fn _assert_object_safe(_: &dyn ContextStrategy) {}
