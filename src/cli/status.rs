//! System health and diagnostics CLI command.
//!
//! Checks database connectivity, session validity, embeddings,
//! WASM runtime, tool count, and channel availability.

use std::path::PathBuf;

use crate::bootstrap::ironclaw_base_dir;
use crate::settings::Settings;

/// Load settings from JSON and TOML config files, matching the runtime
/// priority: TOML overlay > settings.json > defaults.
///
/// This mirrors the loading chain in `Config::from_env_with_toml()` but
/// without resolving the full `Config` (which requires async + secrets).
fn load_settings() -> Settings {
    load_settings_from(&Settings::default_path(), &Settings::default_toml_path())
}

/// Inner implementation with injectable paths (testable).
fn load_settings_from(json_path: &std::path::Path, toml_path: &std::path::Path) -> Settings {
    let mut settings = Settings::load_from(json_path);

    match Settings::load_toml(toml_path) {
        Ok(Some(toml_settings)) => {
            settings.merge_from(&toml_settings);
        }
        Ok(None) => {} // File not found — fine for default path
        Err(e) => {
            eprintln!("Warning: failed to parse {}: {}", toml_path.display(), e);
        }
    }

    settings
}

/// Run the status command, printing system health info.
pub async fn run_status_command() -> anyhow::Result<()> {
    let settings = load_settings();

    println!("IronClaw Status");
    println!("===============\n");

    // Version
    println!(
        "  Version:     {} v{}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    // Database
    print!("  Database:    ");
    let db_backend = std::env::var("DATABASE_BACKEND")
        .ok()
        .unwrap_or_else(|| "postgres".to_string());
    match db_backend.as_str() {
        "libsql" | "turso" | "sqlite" => {
            let path = std::env::var("LIBSQL_PATH")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| crate::config::default_libsql_path());
            if path.exists() {
                let turso = if std::env::var("LIBSQL_URL").is_ok() {
                    " + Turso sync"
                } else {
                    ""
                };
                println!("libSQL ({}{})", path.display(), turso);
            } else {
                println!("libSQL (file missing: {})", path.display());
            }
        }
        _ => {
            if std::env::var("DATABASE_URL").is_ok() {
                match check_database().await {
                    Ok(()) => println!("connected (PostgreSQL)"),
                    Err(e) => println!("error ({})", e),
                }
            } else {
                println!("not configured");
            }
        }
    }

    // Session / Auth
    print!("  Session:     ");
    let session_path = crate::config::llm::default_session_path();
    if session_path.exists() {
        println!("found ({})", session_path.display());
    } else {
        println!("not found (run `ironclaw onboard`)");
    }

    // Secrets (auto-detect from env only; skip keychain probe to avoid
    // triggering macOS system password dialogs on a simple status check)
    print!("  Secrets:     ");
    if std::env::var("SECRETS_MASTER_KEY").is_ok() {
        println!("configured (env)");
    } else {
        // We don't probe the keychain here because get_generic_password()
        // triggers macOS unlock+authorization dialogs, which is bad UX for
        // a read-only status command. If onboarding completed with keychain
        // storage, the key is there; we just can't cheaply verify it.
        println!("env not set (keychain may be configured)");
    }

    // Embeddings
    print!("  Embeddings:  ");
    let emb_enabled = settings.embeddings.enabled
        || std::env::var("OPENAI_API_KEY").is_ok()
        || std::env::var("EMBEDDING_ENABLED")
            .map(|v| v == "true")
            .unwrap_or(false);
    if emb_enabled {
        println!(
            "enabled (provider: {}, model: {})",
            settings.embeddings.provider, settings.embeddings.model
        );
    } else {
        println!("disabled");
    }

    // WASM tools
    print!("  WASM Tools:  ");
    let tools_dir = settings
        .wasm
        .tools_dir
        .clone()
        .unwrap_or_else(default_tools_dir);
    if tools_dir.exists() {
        let count = count_wasm_files(&tools_dir);
        println!("{} installed ({})", count, tools_dir.display());
    } else {
        println!("directory not found ({})", tools_dir.display());
    }

    // WASM channels
    print!("  Channels:    ");
    let channels_dir = settings
        .channels
        .wasm_channels_dir
        .clone()
        .unwrap_or_else(default_channels_dir);
    let mut channel_info = vec!["cli".to_string()];
    if settings.channels.http_enabled {
        channel_info.push(format!(
            "http:{}",
            settings.channels.http_port.unwrap_or(3000)
        ));
    }
    if channels_dir.exists() {
        let wasm_count = count_wasm_files(&channels_dir);
        if wasm_count > 0 {
            channel_info.push(format!("{} wasm", wasm_count));
        }
    }
    println!("{}", channel_info.join(", "));

    // Heartbeat
    print!("  Heartbeat:   ");
    let hb_enabled = settings.heartbeat.enabled
        || std::env::var("HEARTBEAT_ENABLED")
            .map(|v| v == "true")
            .unwrap_or(false);
    if hb_enabled {
        println!("enabled (interval: {}s)", settings.heartbeat.interval_secs);
    } else {
        println!("disabled");
    }

    // MCP servers
    print!("  MCP Servers: ");
    match crate::tools::mcp::config::load_mcp_servers().await {
        Ok(servers) => {
            let enabled = servers.servers.iter().filter(|s| s.enabled).count();
            let total = servers.servers.len();
            println!("{} enabled / {} configured", enabled, total);
        }
        Err(_) => println!("none configured"),
    }

    // Config path
    println!(
        "\n  Config:      {}",
        crate::bootstrap::ironclaw_env_path().display()
    );

    Ok(())
}

#[cfg(feature = "postgres")]
async fn check_database() -> anyhow::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| anyhow::anyhow!("DATABASE_URL not set"))?;

    let config: deadpool_postgres::Config = deadpool_postgres::Config {
        url: Some(url),
        ..Default::default()
    };
    let pool = crate::db::tls::create_pool(&config, crate::config::SslMode::from_env())
        .map_err(|e| anyhow::anyhow!("pool error: {}", e))?;

    let client = tokio::time::timeout(std::time::Duration::from_secs(5), pool.get())
        .await
        .map_err(|_| anyhow::anyhow!("timeout"))?
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    client
        .execute("SELECT 1", &[])
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(())
}

#[cfg(not(feature = "postgres"))]
async fn check_database() -> anyhow::Result<()> {
    // For non-postgres backends, just report configured
    Ok(())
}

fn count_wasm_files(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "wasm"))
                .count()
        })
        .unwrap_or(0)
}

fn default_tools_dir() -> PathBuf {
    ironclaw_base_dir().join("tools")
}

fn default_channels_dir() -> PathBuf {
    ironclaw_base_dir().join("channels")
}

#[cfg(test)]
mod tests {
    use super::load_settings_from;

    /// Regression test for #354: load_settings_from must read config.toml.
    #[test]
    fn reads_toml_heartbeat_enabled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let json_path = dir.path().join("settings.json");
        let toml_path = dir.path().join("config.toml");

        // No JSON file — only TOML
        std::fs::write(
            &toml_path,
            "[heartbeat]\nenabled = true\ninterval_secs = 600",
        )
        .expect("write toml");

        let settings = load_settings_from(&json_path, &toml_path);
        assert!(settings.heartbeat.enabled);
        assert_eq!(settings.heartbeat.interval_secs, 600);
    }

    /// Without any config files, defaults are returned.
    #[test]
    fn defaults_without_config_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = load_settings_from(
            &dir.path().join("nonexistent.json"),
            &dir.path().join("nonexistent.toml"),
        );
        assert!(!settings.heartbeat.enabled);
    }

    /// settings.json is respected.
    #[test]
    fn reads_json_heartbeat_enabled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let json_path = dir.path().join("settings.json");
        let toml_path = dir.path().join("nonexistent.toml");

        std::fs::write(
            &json_path,
            r#"{"heartbeat":{"enabled":true,"interval_secs":900}}"#,
        )
        .expect("write json");

        let settings = load_settings_from(&json_path, &toml_path);
        assert!(settings.heartbeat.enabled);
        assert_eq!(settings.heartbeat.interval_secs, 900);
    }

    /// TOML overlay wins over JSON settings.
    #[test]
    fn toml_overlay_wins_over_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let json_path = dir.path().join("settings.json");
        let toml_path = dir.path().join("config.toml");

        std::fs::write(
            &json_path,
            r#"{"heartbeat":{"enabled":false,"interval_secs":100}}"#,
        )
        .expect("write json");
        std::fs::write(
            &toml_path,
            "[heartbeat]\nenabled = true\ninterval_secs = 200",
        )
        .expect("write toml");

        let settings = load_settings_from(&json_path, &toml_path);
        assert!(settings.heartbeat.enabled);
        assert_eq!(settings.heartbeat.interval_secs, 200);
    }

    /// Invalid TOML is warned but doesn't crash; falls back to JSON/defaults.
    #[test]
    fn invalid_toml_falls_back_gracefully() {
        let dir = tempfile::tempdir().expect("tempdir");
        let json_path = dir.path().join("settings.json");
        let toml_path = dir.path().join("config.toml");

        std::fs::write(
            &json_path,
            r#"{"heartbeat":{"enabled":true,"interval_secs":500}}"#,
        )
        .expect("write json");
        std::fs::write(&toml_path, "this is not valid toml [[[").expect("write bad toml");

        let settings = load_settings_from(&json_path, &toml_path);
        // Should fall back to JSON values, not crash
        assert!(settings.heartbeat.enabled);
        assert_eq!(settings.heartbeat.interval_secs, 500);
    }
}
