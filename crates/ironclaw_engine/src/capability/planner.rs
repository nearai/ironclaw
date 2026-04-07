//! Lease planning for new threads.
//!
//! Converts capability registry contents plus thread type into explicit
//! capability grants. Thread-type-aware: Foreground gets all tiers,
//! Research gets read-only + stateful, Mission excludes administrative tools.

use crate::capability::registry::CapabilityRegistry;
use crate::gate::tool_tier::{ToolTier, classify_tool_tier, is_autonomous_denylisted};
use crate::types::capability::GrantedActions;
use crate::types::thread::ThreadType;

/// Explicit grant plan for a single capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityGrantPlan {
    pub capability_name: String,
    pub granted_actions: GrantedActions,
}

/// Plans explicit capability leases for new threads.
///
/// Uses [`ToolTier`] classification to scope grants by thread type:
/// - **Foreground**: all tiers (interactive approval gates protect Privileged/Admin)
/// - **Research**: `ReadOnly` and `Stateful` only
/// - **Mission**: `ReadOnly`, `Stateful`, and non-denylisted `Privileged`
#[derive(Debug, Default)]
pub struct LeasePlanner;

impl LeasePlanner {
    pub fn new() -> Self {
        Self
    }

    /// Build the capability grants for a new thread.
    pub fn plan_for_thread(
        &self,
        thread_type: ThreadType,
        capabilities: &CapabilityRegistry,
    ) -> Vec<CapabilityGrantPlan> {
        capabilities
            .list()
            .into_iter()
            .filter_map(|cap| {
                let granted_actions: Vec<String> = cap
                    .actions
                    .iter()
                    .filter(|action| {
                        let tier = classify_tool_tier(action);
                        Self::tier_allowed(thread_type, &action.name, tier)
                    })
                    .map(|action| action.name.clone())
                    .collect();
                if granted_actions.is_empty() {
                    None
                } else {
                    Some(CapabilityGrantPlan {
                        capability_name: cap.name.clone(),
                        granted_actions: GrantedActions::Specific(granted_actions),
                    })
                }
            })
            .collect()
    }

    /// Check whether a tool tier is allowed for a given thread type.
    fn tier_allowed(thread_type: ThreadType, action_name: &str, tier: ToolTier) -> bool {
        match thread_type {
            ThreadType::Foreground => {
                // Foreground gets everything — interactive approval gates
                // protect Privileged and Administrative tools.
                true
            }
            ThreadType::Research => {
                // Research threads: read-only and stateful only.
                tier <= ToolTier::Stateful
            }
            ThreadType::Mission => {
                // Mission threads: no Administrative, no denylisted Privileged.
                match tier {
                    ToolTier::ReadOnly | ToolTier::Stateful => true,
                    ToolTier::Privileged => !is_autonomous_denylisted(action_name),
                    ToolTier::Administrative => false,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::capability::{ActionDef, Capability, EffectType, GrantedActions};

    fn action(name: &str, effects: Vec<EffectType>, requires_approval: bool) -> ActionDef {
        ActionDef {
            name: name.into(),
            description: format!("{name} action"),
            parameters_schema: serde_json::json!({}),
            effects,
            requires_approval,
        }
    }

    fn mixed_registry() -> CapabilityRegistry {
        let mut reg = CapabilityRegistry::new();
        reg.register(Capability {
            name: "tools".into(),
            description: "all tools".into(),
            actions: vec![
                action("echo", vec![EffectType::ReadLocal], false), // ReadOnly
                action("read_file", vec![EffectType::ReadLocal], false), // ReadOnly
                action("file_write", vec![EffectType::WriteLocal], false), // Stateful
                action("shell", vec![EffectType::WriteLocal], true), // Privileged
                action("http", vec![EffectType::WriteExternal], true), // Privileged
                action("routine_create", vec![EffectType::WriteLocal], false), // Administrative (denylisted)
                action("tool_install", vec![EffectType::WriteLocal], false), // Administrative (denylisted)
            ],
            knowledge: vec![],
            policies: vec![],
        });
        reg
    }

    fn simple_registry() -> CapabilityRegistry {
        let mut reg = CapabilityRegistry::new();
        reg.register(Capability {
            name: "tools".into(),
            description: "test".into(),
            actions: vec![action("read_file", vec![EffectType::ReadLocal], false)],
            knowledge: vec![],
            policies: vec![],
        });
        reg
    }

    #[test]
    fn foreground_threads_get_explicit_actions() {
        let planner = LeasePlanner::new();
        let plans = planner.plan_for_thread(ThreadType::Foreground, &simple_registry());
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].capability_name, "tools");
        assert_eq!(
            plans[0].granted_actions,
            GrantedActions::Specific(vec!["read_file".into()])
        );
    }

    #[test]
    fn test_foreground_gets_all_tiers() {
        let planner = LeasePlanner::new();
        let plans = planner.plan_for_thread(ThreadType::Foreground, &mixed_registry());
        assert_eq!(plans.len(), 1);
        let actions = plans[0].granted_actions.actions();
        assert_eq!(actions.len(), 7, "Foreground should get all 7 actions");
        assert!(plans[0].granted_actions.covers("routine_create"));
        assert!(plans[0].granted_actions.covers("shell"));
    }

    #[test]
    fn test_research_excludes_privileged_and_admin() {
        let planner = LeasePlanner::new();
        let plans = planner.plan_for_thread(ThreadType::Research, &mixed_registry());
        assert_eq!(plans.len(), 1);
        let actions = plans[0].granted_actions.actions();
        // ReadOnly: echo, read_file. Stateful: file_write.
        assert_eq!(
            actions.len(),
            3,
            "Research should get 3 actions: {:?}",
            actions
        );
        assert!(plans[0].granted_actions.covers("echo"));
        assert!(plans[0].granted_actions.covers("read_file"));
        assert!(plans[0].granted_actions.covers("file_write"));
        assert!(!plans[0].granted_actions.covers("shell"));
        assert!(!plans[0].granted_actions.covers("routine_create"));
    }

    #[test]
    fn test_mission_excludes_administrative() {
        let planner = LeasePlanner::new();
        let plans = planner.plan_for_thread(ThreadType::Mission, &mixed_registry());
        assert_eq!(plans.len(), 1);
        let ga = &plans[0].granted_actions;
        // Includes ReadOnly, Stateful, and non-denylisted Privileged (shell, http).
        // Excludes Administrative (routine_create, tool_install).
        assert!(ga.covers("echo"));
        assert!(ga.covers("shell"));
        assert!(ga.covers("http"));
        assert!(!ga.covers("routine_create"));
        assert!(!ga.covers("tool_install"));
    }

    #[test]
    fn test_mission_excludes_denylisted_privileged() {
        let planner = LeasePlanner::new();
        let plans = planner.plan_for_thread(ThreadType::Mission, &mixed_registry());
        let ga = &plans[0].granted_actions;
        // routine_create and tool_install are in the denylist
        assert!(!ga.covers("routine_create"));
        assert!(!ga.covers("tool_install"));
    }
}
