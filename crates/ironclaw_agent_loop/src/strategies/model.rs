use async_trait::async_trait;

use crate::state::LoopExecutionState;

/// Decides which model preference to pass on the next `stream_model` call.
///
/// Pure policy: returns a `ModelPreference` the executor includes in
/// `LoopModelRequest`. Does NOT mutate state.
///
/// The actual model the host calls is bound by `LoopRunContext`'s resolved model
/// route. The strategy's preference is a hint the host may interpret, such as
/// choosing among already-resolved fallbacks. Strategies cannot introduce new
/// routes mid-run.
///
/// See `docs/reborn/agent-loop-skeleton.md` section 6.
#[async_trait]
pub(crate) trait ModelStrategy: Send + Sync {
    async fn preference(&self, state: &LoopExecutionState) -> ModelPreference;
}

#[allow(dead_code)]
fn _assert_object_safe(_: &dyn ModelStrategy) {}

/// Strategy hint to the host about which already-resolved route to use.
///
/// In the skeleton, `Primary` is the only value strategies produce. `Fallback`
/// is reserved for the deferred `ModelRouteChain` follow-up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModelPreference {
    /// Route-chain index 0: the primary route.
    #[default]
    Primary,
    Fallback {
        /// Deferred route-chain index from `ModelStrategyState::fallback_index`.
        /// Valid fallback indexes are nonzero; `1` is the first fallback after
        /// `Primary`.
        index: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::ModelPreference;

    #[test]
    fn default_preference_is_primary() {
        assert_eq!(ModelPreference::default(), ModelPreference::Primary);
    }

    #[test]
    fn preference_round_trips_through_snake_case_json() {
        let primary = serde_json::to_string(&ModelPreference::Primary).expect("serialize primary");
        assert_eq!(primary, "\"primary\"");
        let decoded_primary: ModelPreference =
            serde_json::from_str(&primary).expect("deserialize primary");
        assert_eq!(decoded_primary, ModelPreference::Primary);

        let fallback =
            serde_json::to_string(&ModelPreference::Fallback { index: 2 }).expect("serialize");
        assert_eq!(fallback, "{\"fallback\":{\"index\":2}}");
        let decoded_fallback: ModelPreference =
            serde_json::from_str(&fallback).expect("deserialize fallback");
        assert_eq!(decoded_fallback, ModelPreference::Fallback { index: 2 });
    }
}
