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

#[allow(dead_code)]
fn assert_input_drain_strategy_object_safe(_: &dyn InputDrainStrategy) {}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;

    #[test]
    fn drain_strategy_is_object_safe() {
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

        assert_input_drain_strategy_object_safe(&NeverDrain);
    }
}
