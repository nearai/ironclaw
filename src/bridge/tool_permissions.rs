use std::collections::HashMap;

use crate::settings::Settings;
use crate::tools::ToolRegistry;
use crate::tools::permissions::{PermissionState, seeded_default_permission};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ToolPermissionResolution {
    pub(crate) effective: PermissionState,
    pub(crate) explicit: Option<PermissionState>,
}

#[derive(Clone, Default)]
pub(crate) struct ToolPermissionSnapshot {
    overrides: HashMap<String, PermissionState>,
}

impl ToolPermissionSnapshot {
    pub(crate) async fn load(tools: &ToolRegistry, user_id: &str) -> Self {
        let Some(db) = tools.database() else {
            return Self::default();
        };

        match db.get_all_settings(user_id).await {
            Ok(db_map) => Self {
                overrides: Settings::from_db_map(&db_map).tool_permissions,
            },
            Err(error) => {
                tracing::warn!(
                    user_id,
                    error = %error,
                    "Failed to load tool permissions for engine v2"
                );
                Self::default()
            }
        }
    }

    pub(crate) fn resolve_permission(&self, tool_name: &str) -> ToolPermissionResolution {
        let canonical = canonical_tool_name(tool_name);
        let hyphenated = canonical.replace('_', "-");
        let raw_explicit = self.explicit_permission_with_names(tool_name, &canonical, &hyphenated);
        let seeded_default = seeded_default_permission(&canonical);
        // Any DB row is a user-explicit choice. The original #3533 fix
        // here collapsed value-equal-to-seed rows to `explicit = None`
        // so `AGENT_AUTO_APPROVE_TOOLS=true` could bypass them, but
        // value-equality is not provenance — a user who genuinely picks
        // `AskEachTime` for `tool_install` (the seeded default) would
        // see their explicit choice silently bypassed. Provenance is
        // now handled at write time: pre-#3559 `seed_tool_permissions`
        // wrote ghost rows that `cleanup_ghost_seeded_tool_permissions`
        // deletes on first startup, and no new seeded rows are written.
        // See `src/app.rs::cleanup_ghost_seeded_tool_permissions`.
        let effective = raw_explicit
            .or(seeded_default)
            .unwrap_or(PermissionState::AskEachTime);
        ToolPermissionResolution {
            effective,
            explicit: raw_explicit,
        }
    }

    fn explicit_permission_with_names(
        &self,
        tool_name: &str,
        canonical: &str,
        hyphenated: &str,
    ) -> Option<PermissionState> {
        self.overrides
            .get(tool_name)
            .copied()
            .or_else(|| self.overrides.get(canonical).copied())
            .or_else(|| self.overrides.get(hyphenated).copied())
    }
}

pub(crate) fn canonical_tool_name(tool_name: &str) -> String {
    tool_name.replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_http_permission_uses_seeded_default() {
        let snapshot = ToolPermissionSnapshot::default();

        assert_eq!(
            snapshot.resolve_permission("http"),
            ToolPermissionResolution {
                effective: PermissionState::AlwaysAllow,
                explicit: None,
            }
        );
    }

    #[test]
    fn saved_http_permission_wins_over_seeded_default() {
        let snapshot = ToolPermissionSnapshot {
            overrides: HashMap::from([("http".to_string(), PermissionState::AskEachTime)]),
        };

        assert_eq!(
            snapshot.resolve_permission("http"),
            ToolPermissionResolution {
                effective: PermissionState::AskEachTime,
                explicit: Some(PermissionState::AskEachTime),
            }
        );
    }

    #[test]
    fn unknown_tool_defaults_to_ask_each_time() {
        let snapshot = ToolPermissionSnapshot::default();

        assert_eq!(
            snapshot.resolve_permission("unknown_tool"),
            ToolPermissionResolution {
                effective: PermissionState::AskEachTime,
                explicit: None,
            }
        );
    }

    /// #3559 security review: a DB row is always a user-explicit choice.
    /// A user who genuinely picks `AskEachTime` for `tool_install` (which
    /// happens to match the code-level seeded default) must surface as
    /// `explicit = Some(...)` so `effect_adapter::enforce_tool_permission`'s
    /// `is_explicit_ask` check fires and `AGENT_AUTO_APPROVE_TOOLS=true`
    /// does NOT bypass the gate. The pre-#3559 collapse-to-implicit logic
    /// silently dropped this choice; the cleanup migration in
    /// `app::cleanup_ghost_seeded_tool_permissions` now removes the
    /// historical ghost-seeded rows at boot so any surviving DB row is
    /// user-explicit by construction.
    #[test]
    fn user_explicit_value_matching_seeded_default_stays_explicit() {
        let snapshot = ToolPermissionSnapshot {
            overrides: HashMap::from([("tool_install".to_string(), PermissionState::AskEachTime)]),
        };

        assert_eq!(
            snapshot.resolve_permission("tool_install"),
            ToolPermissionResolution {
                effective: PermissionState::AskEachTime,
                explicit: Some(PermissionState::AskEachTime),
            }
        );
    }

    /// A user who explicitly opts out of the seeded default (here:
    /// `tool_install` set to `AlwaysAllow` instead of the seeded
    /// `AskEachTime`) keeps their explicit choice. Same semantics as
    /// `user_explicit_value_matching_seeded_default_stays_explicit`,
    /// but with a value that diverges from the seeded default — the
    /// resolver treats all DB values identically.
    #[test]
    fn user_override_diverging_from_seeded_default_stays_explicit() {
        let snapshot = ToolPermissionSnapshot {
            overrides: HashMap::from([("tool_install".to_string(), PermissionState::AlwaysAllow)]),
        };

        assert_eq!(
            snapshot.resolve_permission("tool_install"),
            ToolPermissionResolution {
                effective: PermissionState::AlwaysAllow,
                explicit: Some(PermissionState::AlwaysAllow),
            }
        );
    }
}
