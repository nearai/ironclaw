//! Per-user tool permission system.
//!
//! Each tool has a `PermissionState` that controls whether it runs without
//! confirmation, asks the user each time, or is fully disabled.  A static
//! table of tier defaults (`TOOL_RISK_DEFAULTS`) is used as the fallback when
//! no per-user override exists.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

/// How a tool may be invoked by the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    /// Run automatically without asking the user first.
    AlwaysAllow,
    /// Pause and ask the user before every invocation.
    AskEachTime,
    /// Refuse to run the tool at all.
    Disabled,
}

/// Static map of built-in tool names → their default `PermissionState`.
///
/// Tools present here default to `AlwaysAllow` (read-only / low-risk) or
/// `AskEachTime` (write / destructive / network).  Tools **absent** from the
/// map fall back to `AskEachTime` in `effective_permission`.
pub static TOOL_RISK_DEFAULTS: LazyLock<HashMap<&'static str, PermissionState>> =
    LazyLock::new(|| {
        let mut m = HashMap::new();

        // --- AlwaysAllow: informational, low-risk, or safe writes ---
        for name in &[
            "echo",
            "time",
            "json",
            "memory_search",
            "memory_read",
            "memory_write",
            "memory_tree",
            "tool_list",
            "tool_info",
            "tool_search",
            "skill_list",
            "skill_search",
            "list_jobs",
            "job_status",
            "job_events",
            "image_analyze",
            "message",
        ] {
            m.insert(*name, PermissionState::AlwaysAllow);
        }

        // --- AskEachTime: write, destructive, code-execution, or network ---
        for name in &[
            "shell",
            "read_file",
            "write_file",
            "list_dir",
            "apply_patch",
            "http",
            "create_job",
            "event_emit",
            "routine_create",
            "routine_update",
            "cancel_job",
            "job_prompt",
            "routine_delete",
            "routine_fire",
            "tool_install",
            "tool_auth",
            "tool_activate",
            "tool_remove",
            "tool_upgrade",
            "skill_install",
            "skill_remove",
            "secret_list",
            "secret_delete",
            "image_generate",
            "image_edit",
            "restart",
            "build_software",
            "tool_permission_set",
        ] {
            m.insert(*name, PermissionState::AskEachTime);
        }

        m
    });

/// Return the effective `PermissionState` for `tool_name`.
///
/// Lookup order:
/// 1. Per-user `overrides` map (highest priority).
/// 2. `TOOL_RISK_DEFAULTS` static table.
/// 3. `AskEachTime` as the safe fallback for any unknown tool.
pub fn effective_permission(
    tool_name: &str,
    overrides: &HashMap<String, PermissionState>,
) -> PermissionState {
    if let Some(state) = overrides.get(tool_name) {
        return *state;
    }
    if let Some(state) = TOOL_RISK_DEFAULTS.get(tool_name) {
        return *state;
    }
    PermissionState::AskEachTime
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    #[test]
    fn test_effective_permission_user_override_wins() {
        let mut overrides = HashMap::new();
        // Override "shell" (default: AskEachTime) to AlwaysAllow.
        overrides.insert("shell".to_string(), PermissionState::AlwaysAllow);

        assert_eq!(
            effective_permission("shell", &overrides),
            PermissionState::AlwaysAllow,
            "user override should take precedence over tier default"
        );
    }

    #[test]
    fn test_effective_permission_tier_default_fallback() {
        let overrides = HashMap::new();

        // "echo" has AlwaysAllow in the defaults table.
        assert_eq!(
            effective_permission("echo", &overrides),
            PermissionState::AlwaysAllow,
            "tier default should be returned when no user override exists"
        );

        // "shell" has AskEachTime in the defaults table.
        assert_eq!(
            effective_permission("shell", &overrides),
            PermissionState::AskEachTime,
            "tier default should be returned when no user override exists"
        );
    }

    #[test]
    fn test_effective_permission_unknown_tool_defaults_to_ask() {
        let overrides = HashMap::new();

        assert_eq!(
            effective_permission("completely_unknown_tool_xyz", &overrides),
            PermissionState::AskEachTime,
            "unknown tool should default to AskEachTime"
        );
    }

    #[test]
    fn test_settings_serde_roundtrip() {
        let mut settings = Settings::default();
        settings
            .tool_permissions
            .insert("shell".to_string(), PermissionState::AlwaysAllow);
        settings
            .tool_permissions
            .insert("restart".to_string(), PermissionState::Disabled);

        // Round-trip through JSON (the primary serialization format).
        let json = serde_json::to_string(&settings).expect("serialize");
        let loaded: Settings = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(
            loaded.tool_permissions.get("shell"),
            Some(&PermissionState::AlwaysAllow)
        );
        assert_eq!(
            loaded.tool_permissions.get("restart"),
            Some(&PermissionState::Disabled)
        );
        // Tool not in the map should be absent (not defaulted to anything).
        assert!(!loaded.tool_permissions.contains_key("echo"));
    }
}
