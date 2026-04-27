use std::collections::HashMap;

use crate::settings::Settings;
use crate::tools::ToolRegistry;
use crate::tools::permissions::{PermissionState, effective_permission};

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
        let explicit = self.explicit_permission(tool_name);
        let effective = explicit.unwrap_or_else(|| {
            effective_permission(&canonical_tool_name(tool_name), &self.overrides)
        });
        ToolPermissionResolution {
            effective,
            explicit,
        }
    }

    pub(crate) fn explicit_permission(&self, tool_name: &str) -> Option<PermissionState> {
        let canonical = canonical_tool_name(tool_name);
        let hyphenated = canonical.replace('_', "-");

        self.overrides
            .get(tool_name)
            .copied()
            .or_else(|| self.overrides.get(&canonical).copied())
            .or_else(|| self.overrides.get(&hyphenated).copied())
    }
}

pub(crate) fn canonical_tool_name(tool_name: &str) -> String {
    tool_name.replace('-', "_")
}
