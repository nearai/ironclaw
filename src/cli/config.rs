//! Configuration management CLI commands.
//!
//! Commands for viewing and modifying settings.
//! Settings are stored in the database (env > DB > default).

use std::sync::Arc;

use clap::Subcommand;

use crate::config::{Config, ConfigLoadOptions, DatabaseBackend, LlmBackend};
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
    // Try to connect to the DB for settings access
    let db: Option<Arc<dyn crate::db::Database>> = match connect_db().await {
        Ok(d) => Some(d),
        Err(e) => {
            eprintln!(
                "Warning: Could not connect to database ({}), using disk fallback",
                e
            );
            None
        }
    };

    let db_ref = db.as_deref();
    match cmd {
        ConfigCommand::List { filter } => list_settings(db_ref, filter).await,
        ConfigCommand::Get { path } => get_setting(db_ref, &path).await,
        ConfigCommand::Set { path, value } => set_setting(db_ref, &path, &value).await,
        ConfigCommand::Reset { path } => reset_setting(db_ref, &path).await,
        ConfigCommand::Path => show_path(db_ref.is_some()),
    }
}

/// Bootstrap a DB connection for config commands (backend-agnostic).
async fn connect_db() -> anyhow::Result<Arc<dyn crate::db::Database>> {
    let config = crate::config::Config::from_env_with_options(
        ConfigLoadOptions::default()
            .with_keychain_probe(false)
            .allow_incomplete_llm(true),
    )
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;
    crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))
}

const DEFAULT_USER_ID: &str = "default";

#[cfg(feature = "postgres")]
fn mask_database_url(url: &str) -> String {
    // URL format with credentials segment before '@'.
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let credentials_start = scheme_end + 3;

    let Some(at_pos) = url[credentials_start..].find('@') else {
        return url.to_string();
    };
    let at_abs = credentials_start + at_pos;

    let credentials = &url[credentials_start..at_abs];
    let Some(colon_pos) = credentials.find(':') else {
        return url.to_string();
    };

    let scheme = &url[..credentials_start];
    let username = &credentials[..colon_pos];
    let after_at = &url[at_abs..];

    format!("{}{}:****{}", scheme, username, after_at)
}

fn sanitize_settings_for_display(settings: &mut Settings) {
    #[cfg(feature = "postgres")]
    {
        if let Some(url) = settings.database_url.as_deref() {
            settings.database_url = Some(mask_database_url(url));
        }
    }
}

/// Load settings: DB if available, else disk.
async fn load_settings(store: Option<&dyn crate::db::Database>) -> Settings {
    let mut settings = Settings::load();

    if let Some(store) = store
        && let Ok(map) = store.get_all_settings(DEFAULT_USER_ID).await
        && !map.is_empty()
    {
        settings = Settings::from_db_map(&map);
    }

    settings
}

/// Build effective settings by applying env var overrides through the canonical
/// Config resolver, then projecting config values back into a Settings view.
async fn load_effective_settings(
    store: Option<&dyn crate::db::Database>,
) -> anyhow::Result<Settings> {
    let mut effective = load_settings(store).await;

    let config_result = if let Some(store) = store {
        Config::from_db_with_options(
            store,
            DEFAULT_USER_ID,
            ConfigLoadOptions::default()
                .with_keychain_probe(false)
                .allow_missing_database_url(true)
                .allow_incomplete_llm(true),
        )
        .await
    } else {
        Config::from_env_with_options(
            ConfigLoadOptions::default()
                .with_keychain_probe(false)
                .allow_missing_database_url(true)
                .allow_incomplete_llm(true),
        )
        .await
    };

    let Ok(config) = config_result else {
        sanitize_settings_for_display(&mut effective);
        return Ok(effective);
    };

    effective.database_backend = Some(config.database.backend.to_string());
    effective.database_pool_size = Some(config.database.pool_size);
    match config.database.backend {
        DatabaseBackend::Postgres => {
            effective.database_url = None;
            if config.database.url() != "unused://postgres" {
                #[cfg(feature = "postgres")]
                {
                    effective.database_url = Some(mask_database_url(config.database.url()));
                }
                #[cfg(not(feature = "postgres"))]
                {
                    effective.database_url = Some(config.database.url().to_string());
                }
            }
            effective.libsql_path = None;
            effective.libsql_url = None;
        }
        DatabaseBackend::LibSql => {
            effective.database_url = None;
            effective.libsql_path = config
                .database
                .libsql_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned());
            effective.libsql_url = config.database.libsql_url.clone();
        }
    }

    effective.llm_backend = Some(config.llm.backend.to_string());
    effective.selected_model = match config.llm.backend {
        LlmBackend::NearAi => Some(config.llm.nearai.model.clone()),
        LlmBackend::OpenAi => config.llm.openai.as_ref().map(|c| c.model.clone()),
        LlmBackend::Anthropic => config.llm.anthropic.as_ref().map(|c| c.model.clone()),
        LlmBackend::Ollama => config.llm.ollama.as_ref().map(|c| c.model.clone()),
        LlmBackend::OpenAiCompatible => config
            .llm
            .openai_compatible
            .as_ref()
            .map(|c| c.model.clone()),
    };
    effective.ollama_base_url = config.llm.ollama.as_ref().map(|c| c.base_url.clone());
    effective.openai_compatible_base_url = config
        .llm
        .openai_compatible
        .as_ref()
        .map(|c| c.base_url.clone());

    effective.embeddings.enabled = config.embeddings.enabled;
    effective.embeddings.provider = config.embeddings.provider.clone();
    effective.embeddings.model = config.embeddings.model.clone();
    effective.embeddings.dimension = config.embeddings.dimension;

    effective.tunnel.public_url = config.tunnel.public_url.clone();

    effective.channels.http_enabled = config.channels.http.is_some();
    effective.channels.http_host = config.channels.http.as_ref().map(|h| h.host.clone());
    effective.channels.http_port = config.channels.http.as_ref().map(|h| h.port);
    effective.channels.telegram_owner_id = config.channels.telegram_owner_id;
    effective.channels.wasm_channels_enabled = config.channels.wasm_channels_enabled;
    effective.channels.wasm_channels_dir = Some(config.channels.wasm_channels_dir.clone());

    effective.heartbeat.enabled = config.heartbeat.enabled;
    effective.heartbeat.interval_secs = config.heartbeat.interval_secs;
    effective.heartbeat.notify_channel = config.heartbeat.notify_channel.clone();
    effective.heartbeat.notify_user = config.heartbeat.notify_user.clone();

    effective.agent.name = config.agent.name.clone();
    effective.agent.max_parallel_jobs = config.agent.max_parallel_jobs as u32;
    effective.agent.job_timeout_secs = config.agent.job_timeout.as_secs();
    effective.agent.stuck_threshold_secs = config.agent.stuck_threshold.as_secs();
    effective.agent.use_planning = config.agent.use_planning;
    effective.agent.repair_check_interval_secs = config.agent.repair_check_interval.as_secs();
    effective.agent.max_repair_attempts = config.agent.max_repair_attempts;
    effective.agent.session_idle_timeout_secs = config.agent.session_idle_timeout.as_secs();

    effective.wasm.enabled = config.wasm.enabled;
    effective.wasm.tools_dir = Some(config.wasm.tools_dir.clone());
    effective.wasm.default_memory_limit = config.wasm.default_memory_limit;
    effective.wasm.default_timeout_secs = config.wasm.default_timeout_secs;
    effective.wasm.default_fuel_limit = config.wasm.default_fuel_limit;
    effective.wasm.cache_compiled = config.wasm.cache_compiled;
    effective.wasm.cache_dir = config.wasm.cache_dir.clone();

    effective.sandbox.enabled = config.sandbox.enabled;
    effective.sandbox.policy = config.sandbox.policy.clone();
    effective.sandbox.timeout_secs = config.sandbox.timeout_secs;
    effective.sandbox.memory_limit_mb = config.sandbox.memory_limit_mb;
    effective.sandbox.cpu_shares = config.sandbox.cpu_shares;
    effective.sandbox.image = config.sandbox.image.clone();
    effective.sandbox.auto_pull_image = config.sandbox.auto_pull_image;
    effective.sandbox.extra_allowed_domains = config.sandbox.extra_allowed_domains.clone();

    effective.safety.max_output_length = config.safety.max_output_length;
    effective.safety.injection_check_enabled = config.safety.injection_check_enabled;

    effective.builder.enabled = config.builder.enabled;
    effective.builder.build_dir = config.builder.build_dir.clone();
    effective.builder.max_iterations = config.builder.max_iterations;
    effective.builder.timeout_secs = config.builder.timeout_secs;
    effective.builder.auto_register = config.builder.auto_register;

    effective.secrets_master_key_source = config.secrets.source;

    sanitize_settings_for_display(&mut effective);
    Ok(effective)
}

/// List all settings.
async fn list_settings(
    store: Option<&dyn crate::db::Database>,
    filter: Option<String>,
) -> anyhow::Result<()> {
    let settings = load_effective_settings(store).await;
    let (all, effective): (Vec<(String, String)>, bool) = match settings {
        Ok(s) => (s.list(), true),
        Err(e) => {
            eprintln!(
                "Warning: Failed to resolve full effective config ({}), showing persisted settings only",
                e
            );
            (load_settings(store).await.list(), false)
        }
    };

    let max_key_len = all.iter().map(|(k, _)| k.len()).max().unwrap_or(0);

    let source = if store.is_some() {
        if effective {
            "database (effective: env > db > disk > defaults)"
        } else {
            "database (persisted: db + disk + defaults)"
        }
    } else if effective {
        "disk (effective: env > disk > defaults)"
    } else {
        "disk (persisted: disk + defaults)"
    };
    println!("Settings (source: {}):", source);
    println!();

    for (key, value) in all {
        if let Some(ref f) = filter
            && !key.starts_with(f)
        {
            continue;
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
async fn get_setting(store: Option<&dyn crate::db::Database>, path: &str) -> anyhow::Result<()> {
    let settings = match load_effective_settings(store).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "Warning: Failed to resolve full effective config ({}), showing persisted value only",
                e
            );
            load_settings(store).await
        }
    };

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
    store: Option<&dyn crate::db::Database>,
    path: &str,
    value: &str,
) -> anyhow::Result<()> {
    let mut settings = load_settings(store).await;

    settings
        .set(path, value)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let store = store.ok_or_else(|| {
        anyhow::anyhow!("Database connection required to save settings. Check DATABASE_URL.")
    })?;
    let json_value = match serde_json::from_str::<serde_json::Value>(value) {
        Ok(v) => v,
        Err(_) => serde_json::Value::String(value.to_string()),
    };
    store
        .set_setting(DEFAULT_USER_ID, path, &json_value)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to save to database: {}", e))?;

    println!("Set {} = {}", path, value);
    Ok(())
}

/// Reset a setting to default.
async fn reset_setting(store: Option<&dyn crate::db::Database>, path: &str) -> anyhow::Result<()> {
    let default = Settings::load();
    let default_value = default
        .get(path)
        .ok_or_else(|| anyhow::anyhow!("Unknown setting: {}", path))?;

    let store = store.ok_or_else(|| {
        anyhow::anyhow!("Database connection required to reset settings. Check DATABASE_URL.")
    })?;
    store
        .delete_setting(DEFAULT_USER_ID, path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to delete setting from database: {}", e))?;

    println!("Reset {} to default: {}", path, default_value);
    Ok(())
}

/// Show the settings storage info.
fn show_path(has_db: bool) -> anyhow::Result<()> {
    if has_db {
        println!("Settings stored in: database (settings table)");
    } else {
        println!("Settings stored in: disk fallback (settings.json/defaults)");
    }
    println!(
        "Env config:         {}",
        crate::bootstrap::ironclaw_env_path().display()
    );

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
