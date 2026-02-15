//! System health and diagnostics CLI command.
//!
//! Checks database connectivity, session validity, embeddings,
//! WASM runtime, tool count, and channel availability.

use std::path::PathBuf;

use crate::settings::Settings;

/// Run the status command, printing system health info.
pub async fn run_status_command() -> anyhow::Result<()> {
    let settings = Settings::default();

    println!("IronClaw Status");
    println!("===============\n");

    // Version
    println!(
        "  Version:     {} v{}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    // Database
    let db_url_set = std::env::var("DATABASE_URL").is_ok();
    print!("  Database:    ");
    if db_url_set {
        match check_database().await {
            Ok(()) => println!("connected"),
            Err(e) => println!("error ({})", e),
        }
    } else {
        println!("not configured");
    }

    // Session / Auth
    print!("  Session:     ");
    let session_path = crate::llm::session::default_session_path();
    if session_path.exists() {
        println!("found ({})", session_path.display());
    } else {
        println!("not found (run `ironclaw onboard`)");
    }

    // Secrets (auto-detect: env var or keychain)
    print!("  Secrets:     ");
    let has_env_key = std::env::var("SECRETS_MASTER_KEY").is_ok();
    let has_keychain = crate::secrets::keychain::has_master_key().await;
    if has_env_key {
        println!("configured (env)");
    } else if has_keychain {
        println!("configured (keychain)");
    } else {
        println!("not configured");
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

fn default_tools_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("tools")
}

fn default_channels_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("channels")
}
