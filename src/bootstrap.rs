//! Bootstrap configuration for IronClaw.
//!
//! These are the only settings that MUST live on disk because they're needed
//! before the database connection is established. Everything else lives in the
//! `settings` table in PostgreSQL.
//!
//! File: `~/.ironclaw/bootstrap.json`

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::settings::KeySource;

/// Minimal config needed to connect to the database and decrypt secrets.
///
/// This is the only JSON file IronClaw reads from disk at startup.
/// All other configuration lives in the `settings` table in PostgreSQL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapConfig {
    /// Database connection URL (postgres://...).
    #[serde(default)]
    pub database_url: Option<String>,

    /// Database connection pool size.
    #[serde(default)]
    pub database_pool_size: Option<usize>,

    /// Source for the secrets master key.
    #[serde(default)]
    pub secrets_master_key_source: KeySource,

    /// Whether onboarding wizard has been completed.
    #[serde(default)]
    pub onboard_completed: bool,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            database_url: None,
            database_pool_size: None,
            secrets_master_key_source: KeySource::None,
            onboard_completed: false,
        }
    }
}

impl BootstrapConfig {
    /// Default bootstrap file path: `~/.ironclaw/bootstrap.json`.
    pub fn default_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".ironclaw")
            .join("bootstrap.json")
    }

    /// Legacy settings.json path (for migration detection).
    pub fn legacy_settings_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".ironclaw")
            .join("settings.json")
    }

    /// Load from the default path, falling back to legacy settings.json,
    /// then to defaults if neither exists.
    pub fn load() -> Self {
        let bootstrap_path = Self::default_path();
        if bootstrap_path.exists() {
            return Self::load_from(&bootstrap_path);
        }

        // Fall back to legacy settings.json (extract just the 4 bootstrap fields)
        let legacy_path = Self::legacy_settings_path();
        if legacy_path.exists() {
            return Self::load_from_legacy(&legacy_path);
        }

        Self::default()
    }

    /// Load from a specific path.
    pub fn load_from(path: &PathBuf) -> Self {
        match std::fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Extract bootstrap fields from a legacy settings.json.
    fn load_from_legacy(path: &PathBuf) -> Self {
        match std::fs::read_to_string(path) {
            Ok(data) => {
                // The legacy Settings struct is a superset; serde will ignore extra fields.
                serde_json::from_str(&data).unwrap_or_default()
            }
            Err(_) => Self::default(),
        }
    }

    /// Save to the default path.
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::default_path())
    }

    /// Save to a specific path.
    pub fn save_to(&self, path: &PathBuf) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        std::fs::write(path, json)
    }
}

/// One-time migration from disk config files to the database settings table.
///
/// On first boot after upgrade, checks if:
/// 1. `~/.ironclaw/settings.json` exists
/// 2. The DB settings table is empty for this user
///
/// If both conditions hold, migrates settings, MCP servers, and session data
/// to the database, writes `bootstrap.json`, and renames old files to `.migrated`.
pub async fn migrate_disk_to_db(
    store: &crate::history::Store,
    user_id: &str,
) -> Result<(), MigrationError> {
    let legacy_settings_path = BootstrapConfig::legacy_settings_path();
    if !legacy_settings_path.exists() {
        tracing::debug!("No legacy settings.json found, skipping disk-to-DB migration");
        return Ok(());
    }

    // Only migrate if DB is empty for this user
    let has_settings = store.has_settings(user_id).await.map_err(|e| {
        MigrationError::Database(format!("Failed to check existing settings: {}", e))
    })?;
    if has_settings {
        tracing::debug!(
            "DB already has settings for user '{}', skipping migration",
            user_id
        );
        return Ok(());
    }

    tracing::info!("Migrating disk settings to database...");

    // 1. Load and migrate settings.json
    let settings = crate::settings::Settings::load_from(&legacy_settings_path);
    let db_map = settings.to_db_map();
    if !db_map.is_empty() {
        store
            .set_all_settings(user_id, &db_map)
            .await
            .map_err(|e| {
                MigrationError::Database(format!("Failed to write settings to DB: {}", e))
            })?;
        tracing::info!("Migrated {} settings to database", db_map.len());
    }

    // 2. Write bootstrap.json with the 4 essential fields
    let bootstrap = BootstrapConfig {
        database_url: settings.database_url.clone(),
        database_pool_size: settings.database_pool_size,
        secrets_master_key_source: settings.secrets_master_key_source,
        onboard_completed: settings.onboard_completed,
    };
    bootstrap
        .save()
        .map_err(|e| MigrationError::Io(format!("Failed to write bootstrap.json: {}", e)))?;
    tracing::info!("Wrote bootstrap.json");

    // 3. Migrate mcp-servers.json if it exists
    let ironclaw_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw");
    let mcp_path = ironclaw_dir.join("mcp-servers.json");
    if mcp_path.exists() {
        match std::fs::read_to_string(&mcp_path) {
            Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(value) => {
                    store
                        .set_setting(user_id, "mcp_servers", &value)
                        .await
                        .map_err(|e| {
                            MigrationError::Database(format!(
                                "Failed to write MCP servers to DB: {}",
                                e
                            ))
                        })?;
                    tracing::info!("Migrated mcp-servers.json to database");

                    rename_to_migrated(&mcp_path);
                }
                Err(e) => {
                    tracing::warn!("Failed to parse mcp-servers.json: {}", e);
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read mcp-servers.json: {}", e);
            }
        }
    }

    // 4. Migrate session.json if it exists
    let session_path = ironclaw_dir.join("session.json");
    if session_path.exists() {
        match std::fs::read_to_string(&session_path) {
            Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(value) => {
                    store
                        .set_setting(user_id, "nearai.session", &value)
                        .await
                        .map_err(|e| {
                            MigrationError::Database(format!(
                                "Failed to write session to DB: {}",
                                e
                            ))
                        })?;
                    tracing::info!("Migrated session.json to database");

                    rename_to_migrated(&session_path);
                }
                Err(e) => {
                    tracing::warn!("Failed to parse session.json: {}", e);
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read session.json: {}", e);
            }
        }
    }

    // 5. Rename settings.json to .migrated (don't delete, safety net)
    rename_to_migrated(&legacy_settings_path);

    tracing::info!("Disk-to-DB migration complete");
    Ok(())
}

/// Rename a file to `<name>.migrated` as a safety net.
fn rename_to_migrated(path: &PathBuf) {
    let mut migrated = path.as_os_str().to_owned();
    migrated.push(".migrated");
    if let Err(e) = std::fs::rename(path, &migrated) {
        tracing::warn!("Failed to rename {} to .migrated: {}", path.display(), e);
    }
}

/// Errors that can occur during disk-to-DB migration.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("Database error: {0}")]
    Database(String),
    #[error("IO error: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_bootstrap_save_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bootstrap.json");

        let config = BootstrapConfig {
            database_url: Some("postgres://localhost/test".to_string()),
            database_pool_size: Some(5),
            secrets_master_key_source: KeySource::Keychain,
            onboard_completed: true,
        };

        config.save_to(&path).unwrap();

        let loaded = BootstrapConfig::load_from(&path);
        assert_eq!(
            loaded.database_url,
            Some("postgres://localhost/test".to_string())
        );
        assert_eq!(loaded.database_pool_size, Some(5));
        assert_eq!(loaded.secrets_master_key_source, KeySource::Keychain);
        assert!(loaded.onboard_completed);
    }

    #[test]
    fn test_bootstrap_from_legacy_settings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");

        // Write a legacy settings.json with many extra fields
        let legacy = serde_json::json!({
            "database_url": "postgres://localhost/ironclaw",
            "database_pool_size": 10,
            "secrets_master_key_source": "keychain",
            "onboard_completed": true,
            "selected_model": "claude-3-5-sonnet",
            "agent": { "name": "testbot", "max_parallel_jobs": 3 },
            "heartbeat": { "enabled": true }
        });
        std::fs::write(&path, serde_json::to_string_pretty(&legacy).unwrap()).unwrap();

        let config = BootstrapConfig::load_from_legacy(&path);
        assert_eq!(
            config.database_url,
            Some("postgres://localhost/ironclaw".to_string())
        );
        assert_eq!(config.database_pool_size, Some(10));
        assert_eq!(config.secrets_master_key_source, KeySource::Keychain);
        assert!(config.onboard_completed);
    }

    #[test]
    fn test_bootstrap_defaults() {
        let config = BootstrapConfig::default();
        assert!(config.database_url.is_none());
        assert!(config.database_pool_size.is_none());
        assert_eq!(config.secrets_master_key_source, KeySource::None);
        assert!(!config.onboard_completed);
    }
}
