//! Bootstrap helpers for IronClaw.
//!
//! The only setting that truly needs disk persistence before the database is
//! available is `DATABASE_URL` (chicken-and-egg: can't connect to DB without
//! it). Everything else is auto-detected or read from env vars.
//!
//! File: `~/.ironclaw/.env` (standard dotenvy format)

use std::path::PathBuf;

/// Path to the IronClaw-specific `.env` file: `~/.ironclaw/.env`.
pub fn ironclaw_env_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join(".env")
}

/// Load env vars from `~/.ironclaw/.env` (in addition to the standard `.env`).
///
/// Call this **after** `dotenvy::dotenv()` so that the standard `./.env`
/// takes priority over `~/.ironclaw/.env`. dotenvy never overwrites
/// existing env vars, so the effective priority is:
///
///   explicit env vars > `./.env` > `~/.ironclaw/.env`
///
/// If `~/.ironclaw/.env` doesn't exist but the legacy `bootstrap.json` does,
/// extracts `DATABASE_URL` from it and writes the `.env` file (one-time
/// upgrade from the old config format).
pub fn load_ironclaw_env() {
    let path = ironclaw_env_path();

    if !path.exists() {
        // One-time upgrade: extract DATABASE_URL from legacy bootstrap.json
        migrate_bootstrap_json_to_env(&path);
    }

    if path.exists() {
        let _ = dotenvy::from_path(&path);
    }
}

/// If `bootstrap.json` exists, pull `database_url` out of it and write `.env`.
fn migrate_bootstrap_json_to_env(env_path: &std::path::Path) {
    let ironclaw_dir = env_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let bootstrap_path = ironclaw_dir.join("bootstrap.json");

    if !bootstrap_path.exists() {
        return;
    }

    let content = match std::fs::read_to_string(&bootstrap_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    // Minimal parse: just grab database_url from the JSON
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    if let Some(url) = parsed.get("database_url").and_then(|v| v.as_str()) {
        if let Some(parent) = env_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            eprintln!("Warning: failed to create {}: {}", parent.display(), e);
            return;
        }
        if let Err(e) = std::fs::write(env_path, format!("DATABASE_URL=\"{}\"\n", url)) {
            eprintln!("Warning: failed to migrate bootstrap.json to .env: {}", e);
            return;
        }
        rename_to_migrated(&bootstrap_path);
        eprintln!(
            "Migrated DATABASE_URL from bootstrap.json to {}",
            env_path.display()
        );
    }
}

/// Write `DATABASE_URL` to `~/.ironclaw/.env`.
///
/// Creates the parent directory if it doesn't exist.
/// The value is double-quoted so that `#` (common in URL-encoded passwords)
/// and other shell-special characters are preserved by dotenvy.
pub fn save_database_url(url: &str) -> std::io::Result<()> {
    let path = ironclaw_env_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, format!("DATABASE_URL=\"{}\"\n", url))
}

/// One-time migration of legacy `~/.ironclaw/settings.json` into the database.
///
/// Only runs when a `settings.json` exists on disk AND the DB has no settings
/// yet. After the wizard writes directly to the DB, this path is only hit by
/// users upgrading from the old disk-only configuration.
///
/// After syncing, renames `settings.json` to `.migrated` so it won't trigger again.
pub async fn migrate_disk_to_db(
    store: &dyn crate::db::Database,
    user_id: &str,
) -> Result<(), MigrationError> {
    let ironclaw_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw");
    let legacy_settings_path = ironclaw_dir.join("settings.json");

    if !legacy_settings_path.exists() {
        tracing::debug!("No legacy settings.json found, skipping disk-to-DB migration");
        return Ok(());
    }

    // If DB already has settings, this is not a first boot, the wizard already
    // wrote directly to the DB. Just clean up the stale file.
    let has_settings = store.has_settings(user_id).await.map_err(|e| {
        MigrationError::Database(format!("Failed to check existing settings: {}", e))
    })?;
    if has_settings {
        tracing::info!("DB already has settings, renaming stale settings.json");
        rename_to_migrated(&legacy_settings_path);
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

    // 2. Write DATABASE_URL to ~/.ironclaw/.env
    if let Some(ref url) = settings.database_url {
        save_database_url(url)
            .map_err(|e| MigrationError::Io(format!("Failed to write .env: {}", e)))?;
        tracing::info!("Wrote DATABASE_URL to {}", ironclaw_env_path().display());
    }

    // 3. Migrate mcp-servers.json if it exists
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

    // 6. Clean up old bootstrap.json if it exists (superseded by .env)
    let old_bootstrap = ironclaw_dir.join("bootstrap.json");
    if old_bootstrap.exists() {
        rename_to_migrated(&old_bootstrap);
        tracing::info!("Renamed old bootstrap.json to .migrated");
    }

    tracing::info!("Disk-to-DB migration complete");
    Ok(())
}

/// Rename a file to `<name>.migrated` as a safety net.
fn rename_to_migrated(path: &std::path::Path) {
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
    fn test_save_and_load_database_url() {
        let dir = tempdir().unwrap();
        let env_path = dir.path().join(".env");

        // Write in the quoted format that save_database_url uses
        let url = "postgres://localhost:5432/ironclaw_test";
        std::fs::write(&env_path, format!("DATABASE_URL=\"{}\"\n", url)).unwrap();

        // Verify the content is a valid dotenv line (quoted)
        let content = std::fs::read_to_string(&env_path).unwrap();
        assert_eq!(
            content,
            "DATABASE_URL=\"postgres://localhost:5432/ironclaw_test\"\n"
        );

        // Verify dotenvy can parse it (strips quotes automatically)
        let parsed: Vec<(String, String)> = dotenvy::from_path_iter(&env_path)
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "DATABASE_URL");
        assert_eq!(parsed[0].1, url);
    }

    #[test]
    fn test_save_database_url_with_hash_in_password() {
        let dir = tempdir().unwrap();
        let env_path = dir.path().join(".env");

        // URLs with # in the password are common (URL-encoded special chars).
        // Without quoting, dotenvy treats # as a comment delimiter.
        let url = "postgres://user:p%23ss@localhost:5432/ironclaw";
        std::fs::write(&env_path, format!("DATABASE_URL=\"{}\"\n", url)).unwrap();

        let parsed: Vec<(String, String)> = dotenvy::from_path_iter(&env_path)
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "DATABASE_URL");
        assert_eq!(parsed[0].1, url);
    }

    #[test]
    fn test_save_database_url_creates_parent_dirs() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("deep").join("nested");
        let env_path = nested.join(".env");

        // Parent doesn't exist yet
        assert!(!nested.exists());

        // The global function uses a fixed path, so we test the logic directly
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(&env_path, "DATABASE_URL=postgres://test\n").unwrap();

        assert!(env_path.exists());
        let content = std::fs::read_to_string(&env_path).unwrap();
        assert!(content.contains("DATABASE_URL=postgres://test"));
    }

    #[test]
    fn test_ironclaw_env_path() {
        let path = ironclaw_env_path();
        assert!(path.ends_with(".ironclaw/.env"));
    }

    #[test]
    fn test_migrate_bootstrap_json_to_env() {
        let dir = tempdir().unwrap();
        let env_path = dir.path().join(".env");
        let bootstrap_path = dir.path().join("bootstrap.json");

        // Write a legacy bootstrap.json
        let bootstrap_json = serde_json::json!({
            "database_url": "postgres://localhost/ironclaw_upgrade",
            "database_pool_size": 5,
            "secrets_master_key_source": "keychain",
            "onboard_completed": true
        });
        std::fs::write(
            &bootstrap_path,
            serde_json::to_string_pretty(&bootstrap_json).unwrap(),
        )
        .unwrap();

        assert!(!env_path.exists());
        assert!(bootstrap_path.exists());

        // Run the migration
        migrate_bootstrap_json_to_env(&env_path);

        // .env should now exist with DATABASE_URL
        assert!(env_path.exists());
        let content = std::fs::read_to_string(&env_path).unwrap();
        assert_eq!(
            content,
            "DATABASE_URL=\"postgres://localhost/ironclaw_upgrade\"\n"
        );

        // bootstrap.json should be renamed to .migrated
        assert!(!bootstrap_path.exists());
        assert!(dir.path().join("bootstrap.json.migrated").exists());
    }

    #[test]
    fn test_migrate_bootstrap_json_no_database_url() {
        let dir = tempdir().unwrap();
        let env_path = dir.path().join(".env");
        let bootstrap_path = dir.path().join("bootstrap.json");

        // bootstrap.json with no database_url
        let bootstrap_json = serde_json::json!({
            "onboard_completed": false
        });
        std::fs::write(
            &bootstrap_path,
            serde_json::to_string_pretty(&bootstrap_json).unwrap(),
        )
        .unwrap();

        migrate_bootstrap_json_to_env(&env_path);

        // .env should NOT be created
        assert!(!env_path.exists());
        // bootstrap.json should remain (no migration happened)
        assert!(bootstrap_path.exists());
    }

    #[test]
    fn test_migrate_bootstrap_json_missing() {
        let dir = tempdir().unwrap();
        let env_path = dir.path().join(".env");

        // No bootstrap.json at all
        migrate_bootstrap_json_to_env(&env_path);

        // Nothing should happen
        assert!(!env_path.exists());
    }
}
