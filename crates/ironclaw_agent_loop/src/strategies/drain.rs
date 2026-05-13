//! Input-drain strategy contract.

use async_trait::async_trait;

use crate::state::LoopExecutionState;

/// Decides when to drain the host's steering and followup queues.
///
/// This is pure policy: implementations do not mutate strategy state. Async
/// leaves room for future host-backed queue hints or priority checks.
#[async_trait]
pub(crate) trait InputDrainStrategy: Send + Sync {
    /// Called at the start of each tick before prompt construction.
    async fn drain_steering(&self, state: &LoopExecutionState) -> bool;

    /// Called after the loop would otherwise stop, before returning completed.
    async fn drain_followup(&self, state: &LoopExecutionState) -> bool;
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use crate::strategies::{TurnEndKind, TurnSummary};
    use ironclaw_turns::{LoopMessageRef, LoopResultRef};

    use super::*;

    #[test]
    fn drain_strategy_is_object_safe() {
        fn _check(_: &dyn InputDrainStrategy) {}

        struct NeverDrain;

        #[async_trait]
        impl InputDrainStrategy for NeverDrain {
            async fn drain_steering(&self, _: &LoopExecutionState) -> bool {
                false
            }

            async fn drain_followup(&self, _: &LoopExecutionState) -> bool {
                false
            }
        }

        _check(&NeverDrain);
    }

    #[test]
    fn turn_summary_round_trips_through_json() {
        let summary = TurnSummary {
            kind: TurnEndKind::AfterCapabilityBatch,
            assistant_message_ref: Some(LoopMessageRef::new("msg:assistant-1").unwrap()),
            batch_result_refs: vec![
                LoopResultRef::new("result:call-1").unwrap(),
                LoopResultRef::new("result:call-2").unwrap(),
            ],
        };

        let serialized = serde_json::to_string(&summary).unwrap();
        let deserialized = serde_json::from_str::<TurnSummary>(&serialized).unwrap();

        assert_eq!(deserialized, summary);
    }
}
