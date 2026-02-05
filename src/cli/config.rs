//! Configuration management CLI commands.
//!
//! Commands for viewing and modifying settings.

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

    /// Show the settings file path
    Path,
}

/// Run a config command.
pub fn run_config_command(cmd: ConfigCommand) -> anyhow::Result<()> {
    match cmd {
        ConfigCommand::List { filter } => list_settings(filter),
        ConfigCommand::Get { path } => get_setting(&path),
        ConfigCommand::Set { path, value } => set_setting(&path, &value),
        ConfigCommand::Reset { path } => reset_setting(&path),
        ConfigCommand::Path => show_path(),
    }
}

/// List all settings.
fn list_settings(filter: Option<String>) -> anyhow::Result<()> {
    let settings = Settings::load();
    let all = settings.list();

    // Find the longest key for alignment
    let max_key_len = all.iter().map(|(k, _)| k.len()).max().unwrap_or(0);

    println!("Settings:");
    println!();

    for (key, value) in all {
        // Skip if filter is set and doesn't match
        if let Some(ref f) = filter {
            if !key.starts_with(f) {
                continue;
            }
        }

        // Truncate long values for display
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
fn get_setting(path: &str) -> anyhow::Result<()> {
    let settings = Settings::load();

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
fn set_setting(path: &str, value: &str) -> anyhow::Result<()> {
    let mut settings = Settings::load();

    // Try to set the value
    settings
        .set(path, value)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Save to disk
    settings.save()?;

    println!("Set {} = {}", path, value);
    Ok(())
}

/// Reset a setting to default.
fn reset_setting(path: &str) -> anyhow::Result<()> {
    let mut settings = Settings::load();

    // Get the default value for display
    let default = Settings::default();
    let default_value = default
        .get(path)
        .ok_or_else(|| anyhow::anyhow!("Unknown setting: {}", path))?;

    // Reset it
    settings.reset(path).map_err(|e| anyhow::anyhow!("{}", e))?;

    // Save to disk
    settings.save()?;

    println!("Reset {} to default: {}", path, default_value);
    Ok(())
}

/// Show the settings file path.
fn show_path() -> anyhow::Result<()> {
    let path = Settings::default_path();
    println!("{}", path.display());

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
