use std::path::PathBuf;

use crate::bootstrap::ironclaw_base_dir;
use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

/// Workspace snapshot configuration.
///
/// Controls periodic export of core workspace documents to a Markdown
/// snapshot file and startup hydration from that snapshot.
#[derive(Debug, Clone)]
pub struct SnapshotConfig {
    /// Whether snapshot is enabled. Env: `SNAPSHOT_ENABLED` (default: false).
    pub enabled: bool,
    /// Minimum hours between snapshot passes. Env: `SNAPSHOT_CADENCE_HOURS` (default: 24).
    pub cadence_hours: u32,
    /// Path template with optional `{user_id}` placeholder.
    /// Env: `SNAPSHOT_PATH`.
    /// Default: `<ironclaw_base_dir>/MEMORY_SNAPSHOT_{user_id}.md`.
    ///
    /// When the template does not contain `{user_id}`, a warning is logged.
    /// This is an intentional single-user compatibility escape hatch —
    /// it does NOT provide multi-user isolation guarantees.
    ///
    /// **Security:** The snapshot file at this path stores sensitive workspace
    /// content (identity files, memory, context documents) in plaintext.
    /// Ensure the path points to a user-controlled directory with restricted
    /// access. On Unix, files are written with 0600 permissions automatically.
    pub path_template: String,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        let base = ironclaw_base_dir();
        Self {
            enabled: false,
            cadence_hours: 24,
            path_template: format!("{}/MEMORY_SNAPSHOT_{{user_id}}.md", base.display()),
        }
    }
}

impl SnapshotConfig {
    /// Resolve from environment variables (no `settings` needed, same as HygieneConfig).
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        let default = Self::default();
        Ok(Self {
            enabled: parse_bool_env("SNAPSHOT_ENABLED", false)?,
            cadence_hours: parse_optional_env("SNAPSHOT_CADENCE_HOURS", 24)?,
            path_template: optional_env("SNAPSHOT_PATH")?.unwrap_or(default.path_template),
        })
    }

    /// Convert to workspace-side config, resolving paths for a specific user.
    pub fn to_workspace_config(&self, user_id: &str) -> crate::workspace::snapshot::SnapshotConfig {
        let safe_id = sanitize_user_id(user_id);

        if !self.path_template.contains("{user_id}") {
            tracing::warn!(
                "SNAPSHOT_PATH does not contain {{user_id}} placeholder; \
                 multi-user environments may experience snapshot file collisions"
            );
        }

        let path_str = self.path_template.replace("{user_id}", &safe_id);
        let snapshot_path = PathBuf::from(path_str);

        // A relative filename such as `snapshot.md` has no meaningful parent;
        // place cadence state next to the current working directory in that case.
        let state_dir = snapshot_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let state_path = state_dir.join(format!("snapshot_state_{safe_id}.json"));

        crate::workspace::snapshot::SnapshotConfig {
            enabled: self.enabled,
            cadence_hours: self.cadence_hours,
            snapshot_path,
            state_path,
        }
    }
}

/// Sanitize user_id for safe use in file paths.
///
/// Only retains `[a-zA-Z0-9_-]`, replaces everything else with `_`.
/// Empty input falls back to `"unknown"`.
fn sanitize_user_id(user_id: &str) -> String {
    if user_id.is_empty() {
        return "unknown".to_string();
    }
    user_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_reasonable() {
        let cfg = SnapshotConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.cadence_hours, 24);
        assert!(cfg.path_template.contains("{user_id}"));
    }

    #[test]
    fn sanitize_normal_user_id() {
        assert_eq!(sanitize_user_id("alice"), "alice");
        assert_eq!(sanitize_user_id("user-123"), "user-123");
        assert_eq!(sanitize_user_id("user_name"), "user_name");
    }

    #[test]
    fn sanitize_dangerous_user_id() {
        assert_eq!(sanitize_user_id("../admin"), "___admin");
        assert_eq!(sanitize_user_id("john doe"), "john_doe");
        assert_eq!(sanitize_user_id("a/b\\c"), "a_b_c");
    }

    #[test]
    fn sanitize_empty_user_id() {
        assert_eq!(sanitize_user_id(""), "unknown");
    }

    #[test]
    fn to_workspace_config_resolves_paths() {
        let cfg = SnapshotConfig {
            enabled: true,
            cadence_hours: 12,
            path_template: "/tmp/test/SNAPSHOT_{user_id}.md".to_string(),
        };
        let ws_cfg = cfg.to_workspace_config("alice");
        assert_eq!(
            ws_cfg.snapshot_path,
            PathBuf::from("/tmp/test/SNAPSHOT_alice.md")
        );
        assert_eq!(
            ws_cfg.state_path,
            PathBuf::from("/tmp/test/snapshot_state_alice.json")
        );
        assert!(ws_cfg.enabled);
        assert_eq!(ws_cfg.cadence_hours, 12);
    }

    #[test]
    fn to_workspace_config_sanitizes_user_id_in_path() {
        let cfg = SnapshotConfig {
            enabled: true,
            cadence_hours: 24,
            path_template: "/tmp/SNAPSHOT_{user_id}.md".to_string(),
        };
        let ws_cfg = cfg.to_workspace_config("../admin");
        assert_eq!(
            ws_cfg.snapshot_path,
            PathBuf::from("/tmp/SNAPSHOT____admin.md")
        );
    }
}
