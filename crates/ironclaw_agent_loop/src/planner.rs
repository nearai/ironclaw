//! Planner facade for loop-family strategy composition.
//!
//! The public trait is intentionally identity-only. The canonical executor
//! consults strategies through the crate-private `AgentLoopPlannerInternal`
//! extension trait so downstream crates can hold a planner object without
//! reaching into its Builtin strategy slots.

use crate::family::{ComponentIdentity, LoopFamilyId};
use crate::strategies::{
    BatchPolicyStrategy, BudgetStrategy, CapabilityStrategy, CompactionStrategy, ContextStrategy,
    GateHandlingStrategy, InputDrainStrategy, ModelStrategy, RecoveryStrategy,
    ReplyAdmissionStrategy, StopConditionStrategy,
};

mod sealed {
    pub trait Sealed {}
}

impl sealed::Sealed for crate::default_planner::DefaultPlanner {}

/// A planner is a Builtin composition of the loop strategies.
///
/// The planner has no `run()` or `tick()` method; loop mechanics live in the
/// executor. Public callers can only observe the family id and content identity
/// used for profile resolution and replay validation.
pub trait AgentLoopPlanner: sealed::Sealed + Send + Sync {
    /// The loop family this planner composes for.
    fn id(&self) -> &LoopFamilyId;

    /// Content-addressed identity for this planner composition.
    fn version(&self) -> &ComponentIdentity;
}

/// Crate-private executor-facing strategy access.
#[allow(dead_code)]
pub(crate) trait AgentLoopPlannerInternal: AgentLoopPlanner {
    fn context(&self) -> &dyn ContextStrategy;
    fn capability(&self) -> &dyn CapabilityStrategy;
    fn model(&self) -> &dyn ModelStrategy;
    fn compaction(&self) -> &dyn CompactionStrategy;
    fn batch(&self) -> &dyn BatchPolicyStrategy;
    fn gate(&self) -> &dyn GateHandlingStrategy;
    fn recovery(&self) -> &dyn RecoveryStrategy;
    fn reply_admission(&self) -> &dyn ReplyAdmissionStrategy;
    fn stop(&self) -> &dyn StopConditionStrategy;
    fn drain(&self) -> &dyn InputDrainStrategy;
    fn budget(&self) -> &dyn BudgetStrategy;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::default_planner::DefaultPlanner;
    use crate::family::LoopFamilyId;

    use super::*;

    #[allow(dead_code)]
    fn _check_object_safe(_: &dyn AgentLoopPlanner) {}

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn default_planner_is_object_safe_as_public_trait() {
        let planner = DefaultPlanner::compose_default();
        let object: &dyn AgentLoopPlanner = &planner;

        assert_eq!(object.id(), &LoopFamilyId::DEFAULT);
        assert_eq!(object.version().id, "default");
    }

    #[test]
    fn arc_dyn_agent_loop_planner_is_send_sync() {
        assert_send_sync::<Arc<dyn AgentLoopPlanner>>();
    }
}
