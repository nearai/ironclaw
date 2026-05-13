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
        // #3533: the boot-time seeder writes the seeded default
        // (e.g. `tool_install` → `AskEachTime`) into the DB so the
        // permissions panel can display it. The DB value is then
        // indistinguishable from a user-explicit override, which means
        // `enforce_tool_permission` treats every seeded default as
        // explicit and refuses to honor `AGENT_AUTO_APPROVE_TOOLS=true`.
        // Treat a DB value that matches the seeded default as implicit:
        // a user who genuinely wants to keep the default behaves the
        // same as someone who never touched the setting, and a real
        // explicit override (`AlwaysAllow` or `Disabled`) still surfaces
        // as `explicit = Some(...)`.
        let explicit = match (raw_explicit, seeded_default) {
            (Some(value), Some(default)) if value == default => None,
            (value, _) => value,
        };
        let effective = raw_explicit
            .or(seeded_default)
            .unwrap_or(PermissionState::AskEachTime);
        ToolPermissionResolution {
            effective,
            explicit,
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

    /// Issue #3533 regression: when the boot-time seeder writes the seeded
    /// default for `tool_install` (`AskEachTime`) into the DB, the resolver
    /// must NOT treat that as a user-explicit override — otherwise
    /// `effect_adapter::enforce_tool_permission`'s `is_explicit_ask` check
    /// fires and `AGENT_AUTO_APPROVE_TOOLS=true` is silently neutered.
    #[test]
    fn seeded_default_matching_db_value_is_implicit() {
        let snapshot = ToolPermissionSnapshot {
            overrides: HashMap::from([("tool_install".to_string(), PermissionState::AskEachTime)]),
        };

        assert_eq!(
            snapshot.resolve_permission("tool_install"),
            ToolPermissionResolution {
                effective: PermissionState::AskEachTime,
                // Matches the seeded default → treated as implicit so
                // auto_approve_tools env var can still bypass it.
                explicit: None,
            }
        );
    }

    /// A user who explicitly opts out of the seeded default (here:
    /// `tool_install` set to `AlwaysAllow` instead of the seeded
    /// `AskEachTime`) keeps their explicit choice. Only DB values that
    /// match the seeded default are collapsed to implicit.
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
