//! System health and diagnostics CLI command.
//!
//! Checks database connectivity, session validity, embeddings,
//! WASM runtime, tool count, and channel availability.

use crate::config::{Config, ConfigLoadOptions, DatabaseBackend};

/// Run the status command, printing system health info.
pub async fn run_status_command() -> anyhow::Result<()> {
    let config = Config::from_env_with_options(
        ConfigLoadOptions::default()
            .with_keychain_probe(false)
            .allow_missing_database_url(true)
            .allow_incomplete_llm(true),
    )
    .await?;

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
    match config.database.backend {
        DatabaseBackend::LibSql => {
            let path = config
                .database
                .libsql_path
                .clone()
                .unwrap_or_else(crate::config::default_libsql_path);
            if path.exists() {
                let turso = if config.database.libsql_url.is_some() {
                    " + Turso sync"
                } else {
                    ""
                };
                println!("libSQL ({}{})", path.display(), turso);
            } else {
                println!("libSQL (file missing: {})", path.display());
            }
        }
        DatabaseBackend::Postgres => {
            if config.database.url() == "unused://postgres" {
                println!("not configured");
            } else {
                match check_database().await {
                    Ok(()) => println!("connected (PostgreSQL)"),
                    Err(e) => println!("error ({})", e),
                }
            }
        }
    }

    // Session / Auth
    print!("  Session:     ");
    let session_path = crate::llm::session::default_session_path();
    if session_path.exists() {
        println!("found ({})", session_path.display());
    } else {
        println!("not found (run `ironclaw onboard`)");
    }

    // Secrets (load config with keychain probing disabled for status UX)
    print!("  Secrets:     ");
    if config.secrets.enabled {
        match config.secrets.source {
            crate::settings::KeySource::Env => println!("configured (env)"),
            crate::settings::KeySource::Keychain => println!("configured (keychain)"),
            crate::settings::KeySource::None => println!("configured"),
        }
    } else {
        println!("env not set (keychain may be configured)");
    }

    // Embeddings
    print!("  Embeddings:  ");
    if config.embeddings.enabled {
        println!(
            "enabled (provider: {}, model: {})",
            config.embeddings.provider, config.embeddings.model
        );
    } else {
        println!("disabled");
    }

    // WASM tools
    print!("  WASM Tools:  ");
    let tools_dir = config.wasm.tools_dir.clone();
    if tools_dir.exists() {
        let count = count_wasm_files(&tools_dir);
        println!("{} installed ({})", count, tools_dir.display());
    } else {
        println!("directory not found ({})", tools_dir.display());
    }

    // Channels
    print!("  Channels:    ");
    let channels_dir = config.channels.wasm_channels_dir.clone();
    let mut channel_info = Vec::new();
    if config.channels.cli.enabled {
        channel_info.push("cli".to_string());
    }
    if let Some(ref http) = config.channels.http {
        channel_info.push(format!("http:{}", http.port));
    }
    if config.channels.gateway.is_some() {
        channel_info.push("gateway".to_string());
    }
    if config.channels.wasm_channels_enabled && channels_dir.exists() {
        let wasm_count = count_wasm_files(&channels_dir);
        if wasm_count > 0 {
            channel_info.push(format!("{} wasm", wasm_count));
        }
    }
    if channel_info.is_empty() {
        channel_info.push("none".to_string());
    }
    println!("{}", channel_info.join(", "));

    // Heartbeat
    print!("  Heartbeat:   ");
    if config.heartbeat.enabled {
        println!("enabled (interval: {}s)", config.heartbeat.interval_secs);
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
    let pool = config
        .create_pool(
            Some(deadpool_postgres::Runtime::Tokio1),
            tokio_postgres::NoTls,
        )
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
