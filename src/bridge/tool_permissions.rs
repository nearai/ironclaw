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
        let explicit = self.explicit_permission_with_names(tool_name, &canonical, &hyphenated);
        let effective = explicit
            .or_else(|| seeded_default_permission(&canonical))
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
}
