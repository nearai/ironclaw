use std::collections::BTreeMap;

use ironclaw_host_api::ids::CapabilityId;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContextStrategyState {}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityStrategyState {}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelStrategyState {
    pub fallback_index: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GoalRefreshStrategyState {
    #[serde(default)]
    pub turns_since_refresh: u32,
}

/// Persistent state owned by `GateHandlingStrategy`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GateStrategyState {}

/// Per-turn pipeline-directive state for `PostCapabilityStage`.
///
/// Unlike sibling `<Domain>StrategyState` types, this slot belongs to a
/// pipeline stage (not a strategy) and tracks two distinct lifecycles:
///
/// - `pending_capability_bytes` is **per-turn**: filled by
///   `push_completed_result` during a capability batch, cleared at the
///   end of every `PostCapabilityStage::process` call (BUG-N1 fix).
/// - `skip_model_this_iteration` is a **one-shot directive**: set by
///   `PostCapabilityStage` when its policy trips, then consumed by the
///   NEXT iteration's `PromptStage` which clears the flag and emits
///   `PromptStep::SkipModel` to short-circuit the model call.
///
/// The distinct naming (`StageState` vs `StrategyState`) marks the
/// category difference: stages own transient one-shot directives;
/// strategies own resumable accounting.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PostCapabilityStageState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pending_capability_bytes: BTreeMap<CapabilityId, u64>,
    #[serde(default)]
    pub skip_model_this_iteration: bool,
}
