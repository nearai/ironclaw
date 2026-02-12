//! Configuration management CLI commands.
//!
//! Commands for viewing and modifying settings.
//! Settings are stored in PostgreSQL (env > DB > default).

use clap::Subcommand;

use crate::settings::Settings;

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigCommand {
    /// List all settings and their current values
    List {
        /// Show only settings matching this prefix (e.g., "agent", "heartbeat")
        #[arg(short, long)]
        filter: Option<String>,
    },

    /// Get a specific setting value
    Get {
        /// Setting path (e.g., "agent.max_parallel_jobs")
        path: String,
    },

    /// Set a setting value
    Set {
        /// Setting path (e.g., "agent.max_parallel_jobs")
        path: String,

        /// Value to set
        value: String,
    },

    /// Reset a setting to its default value
    Reset {
        /// Setting path (e.g., "agent.max_parallel_jobs")
        path: String,
    },

    /// Show the settings storage info
    Path,
}

/// Run a config command.
///
/// Connects to the database to read/write settings. Falls back to disk
/// if the database is not available.
pub async fn run_config_command(cmd: ConfigCommand) -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();

    // Try to connect to the DB for settings access
    let store = match connect_store().await {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!(
                "Warning: Could not connect to database ({}), using disk fallback",
                e
            );
            None
        }
    };

    match cmd {
        ConfigCommand::List { filter } => list_settings(store.as_ref(), filter).await,
        ConfigCommand::Get { path } => get_setting(store.as_ref(), &path).await,
        ConfigCommand::Set { path, value } => set_setting(store.as_ref(), &path, &value).await,
        ConfigCommand::Reset { path } => reset_setting(store.as_ref(), &path).await,
        ConfigCommand::Path => show_path(store.is_some()),
    }
}

/// Bootstrap a DB connection for config commands.
async fn connect_store() -> anyhow::Result<crate::history::Store> {
    let config = crate::config::Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let store = crate::history::Store::new(&config.database).await?;
    store.run_migrations().await?;
    Ok(store)
}

const DEFAULT_USER_ID: &str = "default";

/// Load settings: DB if available, else disk.
async fn load_settings(store: Option<&crate::history::Store>) -> Settings {
    if let Some(store) = store {
        match store.get_all_settings(DEFAULT_USER_ID).await {
            Ok(map) if !map.is_empty() => return Settings::from_db_map(&map),
            _ => {}
        }
    }
    Settings::load()
}

/// List all settings.
async fn list_settings(
    store: Option<&crate::history::Store>,
    filter: Option<String>,
) -> anyhow::Result<()> {
    let settings = load_settings(store).await;
    let all = settings.list();

    let max_key_len = all.iter().map(|(k, _)| k.len()).max().unwrap_or(0);

    let source = if store.is_some() { "database" } else { "disk" };
    println!("Settings (source: {}):", source);
    println!();

    for (key, value) in all {
        if let Some(ref f) = filter {
            if !key.starts_with(f) {
                continue;
            }
        }

        let display_value = if value.len() > 60 {
            format!("{}...", &value[..57])
        } else {
            value
        };

        println!("  {:width$}  {}", key, display_value, width = max_key_len);
    }

    Ok(())
}

/// Get a specific setting.
async fn get_setting(store: Option<&crate::history::Store>, path: &str) -> anyhow::Result<()> {
    let settings = load_settings(store).await;

    match settings.get(path) {
        Some(value) => {
            println!("{}", value);
            Ok(())
        }
        None => {
            anyhow::bail!("Setting not found: {}", path);
        }
    }
}

/// Set a setting value.
async fn set_setting(
    store: Option<&crate::history::Store>,
    path: &str,
    value: &str,
) -> anyhow::Result<()> {
    let mut settings = load_settings(store).await;

    settings
        .set(path, value)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Save to DB if available, otherwise disk
    if let Some(store) = store {
        let json_value = match serde_json::from_str::<serde_json::Value>(value) {
            Ok(v) => v,
            Err(_) => serde_json::Value::String(value.to_string()),
        };
        store
            .set_setting(DEFAULT_USER_ID, path, &json_value)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to save to database: {}", e))?;
    } else {
        settings.save()?;
    }

    println!("Set {} = {}", path, value);
    Ok(())
}

/// Reset a setting to default.
async fn reset_setting(store: Option<&crate::history::Store>, path: &str) -> anyhow::Result<()> {
    let default = Settings::default();
    let default_value = default
        .get(path)
        .ok_or_else(|| anyhow::anyhow!("Unknown setting: {}", path))?;

    // Delete from DB (falling back to default) or reset on disk
    if let Some(store) = store {
        store
            .delete_setting(DEFAULT_USER_ID, path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete setting from database: {}", e))?;
    } else {
        let mut settings = Settings::load();
        settings.reset(path).map_err(|e| anyhow::anyhow!("{}", e))?;
        settings.save()?;
    }

    println!("Reset {} to default: {}", path, default_value);
    Ok(())
}

/// Show the settings storage info.
fn show_path(has_db: bool) -> anyhow::Result<()> {
    if has_db {
        println!("Settings stored in: PostgreSQL (settings table)");
        println!(
            "Bootstrap config:   {}",
            crate::bootstrap::BootstrapConfig::default_path().display()
        );
    } else {
        let path = Settings::default_path();
        println!("Settings stored in: {} (disk fallback)", path.display());

        if path.exists() {
            let metadata = std::fs::metadata(&path)?;
            println!("  Size: {} bytes", metadata.len());
            if let Ok(modified) = metadata.modified() {
                use std::time::SystemTime;
                let duration = SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or_default();
                let secs = duration.as_secs();
                if secs < 60 {
                    println!("  Modified: {} seconds ago", secs);
                } else if secs < 3600 {
                    println!("  Modified: {} minutes ago", secs / 60);
                } else if secs < 86400 {
                    println!("  Modified: {} hours ago", secs / 3600);
                } else {
                    println!("  Modified: {} days ago", secs / 86400);
                }
            }
        } else {
            println!("  (does not exist, using defaults)");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_list_settings() {
        // Just verify it doesn't panic
        let settings = Settings::default();
        let list = settings.list();
        assert!(!list.is_empty());
    }

    #[test]
    fn test_get_set_reset() {
        let _dir = tempdir().unwrap();

        let mut settings = Settings::default();

        // Set a value
        settings.set("agent.name", "testbot").unwrap();
        assert_eq!(settings.agent.name, "testbot");

        // Reset to default
        settings.reset("agent.name").unwrap();
        assert_eq!(settings.agent.name, "ironclaw");
    }
}
