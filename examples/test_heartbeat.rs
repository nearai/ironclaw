//! Standalone heartbeat test.
//!
//! Exercises the heartbeat system in isolation: connects to the real
//! database, reads the real HEARTBEAT.md, calls the real LLM, and prints
//! every step so you can see exactly where it breaks.
//!
//! Usage:
//!   cargo run --example test_heartbeat

use std::sync::Arc;

use ironclaw::{
    agent::HeartbeatRunner,
    config::Config,
    history::Store,
    llm::{SessionConfig, create_llm_provider, create_session_manager},
    workspace::Workspace,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env and set up logging
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter("ironclaw=debug")
        .init();

    println!("=== Heartbeat Integration Test ===\n");

    // 1. Load config
    let config = Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("Config: {}", e))?;
    println!("[1/6] Config loaded");
    println!("  heartbeat.enabled = {}", config.heartbeat.enabled);
    println!(
        "  heartbeat.interval_secs = {}",
        config.heartbeat.interval_secs
    );
    println!(
        "  heartbeat.notify_channel = {:?}",
        config.heartbeat.notify_channel
    );
    println!(
        "  heartbeat.notify_user = {:?}",
        config.heartbeat.notify_user
    );

    // 2. Connect to database
    let store = Store::new(&config.database).await?;
    store.run_migrations().await?;
    println!("[2/6] Database connected");

    // 3. Create workspace
    let workspace = Arc::new(Workspace::new("default", store.pool()));
    println!("[3/6] Workspace created");

    // 4. Read HEARTBEAT.md
    let checklist = workspace.heartbeat_checklist().await;
    match &checklist {
        Ok(Some(content)) => {
            let preview: String = content.chars().take(200).collect();
            println!("[4/6] HEARTBEAT.md found ({} chars)", content.len());
            println!("  Preview: {}...", preview);
        }
        Ok(None) => {
            println!("[4/6] HEARTBEAT.md is None (no file, no seed fallback)");
            println!("  Heartbeat will return Skipped.");
        }
        Err(e) => {
            println!("[4/6] HEARTBEAT.md read error: {}", e);
        }
    }

    // Check if the checklist would be considered "effectively empty"
    if let Ok(Some(_)) = checklist {
        println!("  (Will verify via runner below)");
    }

    // 5. Create LLM provider
    let session = create_session_manager(SessionConfig {
        auth_base_url: config.llm.nearai.auth_base_url.clone(),
        session_path: config.llm.nearai.session_path.clone(),
    })
    .await;
    let llm = create_llm_provider(&config.llm, session)?;
    println!("[5/6] LLM provider created (model: {})", llm.model_name());

    // 6. Run heartbeat check
    println!("[6/6] Running check_heartbeat()...\n");

    let hb_config = ironclaw::agent::HeartbeatConfig::default();
    let runner = HeartbeatRunner::new(hb_config, workspace, llm);

    let result = runner.check_heartbeat().await;

    println!("=== Result ===\n");
    match &result {
        ironclaw::agent::HeartbeatResult::Ok => {
            println!("HeartbeatResult::Ok");
            println!("  LLM responded HEARTBEAT_OK, nothing needs attention.");
        }
        ironclaw::agent::HeartbeatResult::NeedsAttention(msg) => {
            println!("HeartbeatResult::NeedsAttention");
            println!("  Message:\n{}", msg);
        }
        ironclaw::agent::HeartbeatResult::Skipped => {
            println!("HeartbeatResult::Skipped");
            println!("  No checklist found, or checklist was effectively empty.");
            println!("  This means the HEARTBEAT.md either:");
            println!("    - Does not exist in the workspace database");
            println!("    - Contains only headers, comments, and empty checkboxes");
        }
        ironclaw::agent::HeartbeatResult::Failed(err) => {
            println!("HeartbeatResult::Failed");
            println!("  Error: {}", err);
        }
    }

    Ok(())
}
