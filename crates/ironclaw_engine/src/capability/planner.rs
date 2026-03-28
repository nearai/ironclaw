//! Lease planning for new threads.
//!
//! Converts capability registry contents plus thread type into explicit
//! capability grants so new threads do not receive implicit wildcard leases.

use crate::capability::registry::CapabilityRegistry;
use crate::types::thread::ThreadType;

/// Explicit grant plan for a single capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityGrantPlan {
    pub capability_name: String,
    pub granted_actions: Vec<String>,
}

/// Plans explicit capability leases for new threads.
#[derive(Debug, Default)]
pub struct LeasePlanner;

impl LeasePlanner {
    pub fn new() -> Self {
        Self
    }

    /// Build the capability grants for a new thread.
    pub fn plan_for_thread(
        &self,
        _thread_type: ThreadType,
        capabilities: &CapabilityRegistry,
    ) -> Vec<CapabilityGrantPlan> {
        capabilities
            .list()
            .into_iter()
            .filter_map(|cap| {
                let granted_actions: Vec<String> = cap
                    .actions
                    .iter()
                    .map(|action| action.name.clone())
                    .collect();
                if granted_actions.is_empty() {
                    None
                } else {
                    Some(CapabilityGrantPlan {
                        capability_name: cap.name.clone(),
                        granted_actions,
                    })
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::capability::{ActionDef, Capability, EffectType};

    fn registry() -> CapabilityRegistry {
        let mut reg = CapabilityRegistry::new();
        reg.register(Capability {
            name: "tools".into(),
            description: "test".into(),
            actions: vec![ActionDef {
                name: "read_file".into(),
                description: "read".into(),
                parameters_schema: serde_json::json!({}),
                effects: vec![EffectType::ReadLocal],
                requires_approval: false,
            }],
            knowledge: vec![],
            policies: vec![],
        });
        reg
    }

    #[test]
    fn foreground_threads_get_explicit_actions() {
        let planner = LeasePlanner::new();
        let plans = planner.plan_for_thread(ThreadType::Foreground, &registry());
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].capability_name, "tools");
        assert_eq!(plans[0].granted_actions, vec!["read_file"]);
    }
}
