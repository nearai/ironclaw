//! Tool tier classification.
//!
//! Maps each action to a privilege tier based on its declared effects and
//! approval requirements. Used by the [`LeasePlanner`] to scope thread-type
//! aware leases and by the [`LeaseGate`] for authorization checks.
//!
//! [`LeasePlanner`]: crate::capability::planner::LeasePlanner
//! [`LeaseGate`]: (future)

use crate::types::capability::{ActionDef, EffectType};

/// Tool actions in the AUTONOMOUS_TOOL_DENYLIST — these are always
/// classified as [`ToolTier::Administrative`] regardless of their
/// declared effects.
pub const AUTONOMOUS_TOOL_DENYLIST: &[&str] = &[
    "routine_create",
    "routine_update",
    "routine_delete",
    "routine_fire",
    "event_emit",
    "create_job",
    "job_prompt",
    "restart",
    "tool_install",
    "tool_auth",
    "tool_activate",
    "tool_remove",
    "tool_upgrade",
    "skill_install",
    "skill_remove",
    "secret_list",
    "secret_delete",
];

/// Returns true if the action name is in the autonomous tool denylist.
pub fn is_autonomous_denylisted(action_name: &str) -> bool {
    AUTONOMOUS_TOOL_DENYLIST.contains(&action_name)
}

/// Privilege tier for a tool action.
///
/// Tiers are totally ordered: `ReadOnly < Stateful < Privileged < Administrative`.
/// The [`LeasePlanner`] uses this ordering to decide which actions to grant
/// for each [`ThreadType`].
///
/// [`ThreadType`]: crate::types::thread::ThreadType
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolTier {
    /// Read-only, no side effects (echo, time, json, memory_search, memory_read).
    ReadOnly,
    /// Creates or reads local state (read_file, list_dir).
    Stateful,
    /// Write operations or external effects (shell, file_write, http, create_job).
    Privileged,
    /// System-level operations that should never run autonomously
    /// (routine_*, tool_install, skill_*, secret_*, restart).
    Administrative,
}

/// Classify a tool action into a [`ToolTier`].
///
/// Classification rules (in priority order):
/// 1. Action name in [`AUTONOMOUS_TOOL_DENYLIST`] → `Administrative`
/// 2. `requires_approval: true` → `Privileged`
/// 3. Only `ReadLocal` / `Compute` effects → `ReadOnly`
/// 4. Everything else → `Stateful`
pub fn classify_tool_tier(action: &ActionDef) -> ToolTier {
    // 1. Denylisted → Administrative
    if is_autonomous_denylisted(&action.name) {
        return ToolTier::Administrative;
    }

    // 2. Requires approval → Privileged
    if action.requires_approval {
        return ToolTier::Privileged;
    }

    // 3. Only read/compute effects → ReadOnly
    let only_read_compute = !action.effects.is_empty()
        && action
            .effects
            .iter()
            .all(|e| matches!(e, EffectType::ReadLocal | EffectType::Compute));
    if only_read_compute {
        return ToolTier::ReadOnly;
    }

    // 4. Default
    ToolTier::Stateful
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::capability::{ActionDef, ModelToolSurface};

    fn action(name: &str, effects: Vec<EffectType>, requires_approval: bool) -> ActionDef {
        ActionDef {
            name: name.into(),
            description: String::new(),
            parameters_schema: serde_json::json!({}),
            effects,
            requires_approval,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
        }
    }

    #[test]
    fn test_denylisted_tool_always_administrative() {
        for &name in AUTONOMOUS_TOOL_DENYLIST {
            let ad = action(name, vec![EffectType::ReadLocal], false);
            assert_eq!(
                classify_tool_tier(&ad),
                ToolTier::Administrative,
                "Expected Administrative for denylisted tool '{name}'"
            );
        }
    }

    #[test]
    fn test_requires_approval_is_privileged() {
        let ad = action("shell", vec![EffectType::WriteLocal], true);
        assert_eq!(classify_tool_tier(&ad), ToolTier::Privileged);
    }

    #[test]
    fn test_read_only_effects() {
        let ad = action(
            "echo",
            vec![EffectType::ReadLocal, EffectType::Compute],
            false,
        );
        assert_eq!(classify_tool_tier(&ad), ToolTier::ReadOnly);
    }

    #[test]
    fn test_read_local_only() {
        let ad = action("memory_search", vec![EffectType::ReadLocal], false);
        assert_eq!(classify_tool_tier(&ad), ToolTier::ReadOnly);
    }

    #[test]
    fn test_write_local_is_stateful() {
        let ad = action("file_write", vec![EffectType::WriteLocal], false);
        assert_eq!(classify_tool_tier(&ad), ToolTier::Stateful);
    }

    #[test]
    fn test_external_effects_stateful() {
        let ad = action("web_fetch", vec![EffectType::ReadExternal], false);
        assert_eq!(classify_tool_tier(&ad), ToolTier::Stateful);
    }

    #[test]
    fn test_no_effects_is_stateful() {
        let ad = action("custom_tool", vec![], false);
        assert_eq!(classify_tool_tier(&ad), ToolTier::Stateful);
    }

    #[test]
    fn test_denylisted_overrides_no_approval() {
        // routine_create doesn't require_approval in its ActionDef,
        // but should still be Administrative because it's denylisted.
        let ad = action("routine_create", vec![EffectType::WriteLocal], false);
        assert_eq!(classify_tool_tier(&ad), ToolTier::Administrative);
    }

    #[test]
    fn test_tier_ordering() {
        assert!(ToolTier::ReadOnly < ToolTier::Stateful);
        assert!(ToolTier::Stateful < ToolTier::Privileged);
        assert!(ToolTier::Privileged < ToolTier::Administrative);
    }
}
