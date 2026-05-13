//! Stop-condition strategy contract.

use async_trait::async_trait;
use ironclaw_turns::{LoopFailureKind, LoopMessageRef, LoopResultRef};

use crate::state::{LoopExecutionState, StopStrategyState};

/// Decides whether the loop should stop after the current turn finishes.
///
/// Implementations return a new `stop_state` slot value. Async because
/// future strategies may consult host state for milestone tracking.
#[async_trait]
pub(crate) trait StopConditionStrategy: Send + Sync {
    /// Called after a turn completes.
    async fn should_stop_after_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopOutcome;
}

/// Loop-side projection of what just happened in the completed turn.
///
/// This carries refs only. Strategies that need content must read it through
/// host ports so host-side redaction and scope policy remain authoritative.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct TurnSummary {
    pub kind: TurnEndKind,
    pub assistant_message_ref: Option<LoopMessageRef>,
    pub batch_result_refs: Vec<LoopResultRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TurnEndKind {
    /// The model returned a reply and no capability batch executed this turn.
    ReplyOnly,
    /// The model returned capability calls and the listed refs are the
    /// finalized batch outcomes for this turn.
    AfterCapabilityBatch,
}

/// Strategy decision plus the new `stop_state` slot value.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopOutcome {
    Continue {
        stop: StopStrategyState,
    },
    Stop {
        stop: StopStrategyState,
        kind: StopKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopKind {
    /// Strategy is satisfied; the executor maps this to graceful completion.
    GracefulStop,
    /// Safety-net escape for repeated calls or repeated failures.
    NoProgressDetected,
    /// Strategy aborts with an explicit failure kind.
    Aborted(LoopFailureKind),
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde_json::json;

    use super::*;

    #[test]
    fn stop_condition_strategy_is_object_safe() {
        fn _check(_: &dyn StopConditionStrategy) {}

        struct AlwaysContinue;

        #[async_trait]
        impl StopConditionStrategy for AlwaysContinue {
            async fn should_stop_after_turn(
                &self,
                _: &LoopExecutionState,
                _: &TurnSummary,
            ) -> StopOutcome {
                StopOutcome::Continue {
                    stop: StopStrategyState::default(),
                }
            }
        }

        _check(&AlwaysContinue);
    }

    #[test]
    fn stop_outcome_round_trips_through_json() {
        let outcome = StopOutcome::Stop {
            stop: StopStrategyState {
                turns_completed: 3,
                terminate_hints_in_last_batch: 1,
                last_batch_total: 2,
            },
            kind: StopKind::NoProgressDetected,
        };

        let value = serde_json::to_value(&outcome).unwrap();
        // Variant tag must be snake_case on the wire, matching sibling enums.
        assert!(
            value.get("stop").is_some(),
            "expected snake_case `stop` key, got {value}"
        );
        assert!(
            value.get("Stop").is_none(),
            "PascalCase `Stop` key leaked into wire form: {value}"
        );

        let deserialized = serde_json::from_value::<StopOutcome>(value).unwrap();
        assert_eq!(deserialized, outcome);

        let continue_outcome = StopOutcome::Continue {
            stop: StopStrategyState::default(),
        };
        let continue_value = serde_json::to_value(&continue_outcome).unwrap();
        assert!(
            continue_value.get("continue").is_some(),
            "expected snake_case `continue` key, got {continue_value}"
        );
        assert_eq!(
            serde_json::from_value::<StopOutcome>(continue_value).unwrap(),
            continue_outcome
        );
    }

    #[test]
    fn aborted_stop_kind_preserves_policy_denied_variant_tag() {
        let kind = StopKind::Aborted(LoopFailureKind::PolicyDenied);
        let value = serde_json::to_value(kind).unwrap();

        assert_eq!(value, json!({ "aborted": "policy_denied" }));
        assert_eq!(serde_json::from_value::<StopKind>(value).unwrap(), kind);
    }
}
