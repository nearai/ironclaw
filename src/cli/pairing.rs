//! DM pairing CLI commands.
//!
//! Manage pairing requests for channels (Telegram, Slack, etc.).

use clap::Subcommand;

/// Pairing subcommands.
#[derive(Subcommand, Debug, Clone)]
pub enum PairingCommand {
    /// List pending pairing requests
    List {
        /// Channel name (e.g., telegram, slack)
        #[arg(required = true)]
        channel: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Approve a pairing request by code
    Approve {
        /// Channel name (e.g., telegram, slack)
        #[arg(required = true)]
        channel: String,

        /// Pairing code (e.g., ABC12345)
        #[arg(required = true)]
        code: String,
    },
}

/// Run pairing CLI command. Requires a DB connection and an owner_id.
pub async fn run_pairing_command(cmd: PairingCommand) -> Result<(), anyhow::Error> {
    let config = crate::config::Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let db = crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Ensure owner user row exists before approval (FK on channel_identities.owner_id).
    // Best-effort: don't block CLI if the upsert fails (e.g. read-only replica).
    db.get_or_create_user(crate::db::UserRecord {
        id: config.owner_id.clone(),
        role: "admin".to_string(),
        display_name: "Owner".to_string(),
        status: "active".to_string(),
        email: None,
        last_login_at: None,
        created_by: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        metadata: serde_json::Value::Object(Default::default()),
    })
    .await
    .ok();

    let cache = std::sync::Arc::new(crate::ownership::OwnershipCache::new());
    let store = crate::pairing::PairingStore::new(db, cache);
    let owner_id = crate::ownership::OwnerId::from(config.owner_id.clone());

    run_pairing_command_with_store(&store, &owner_id, cmd).await
}

/// Run pairing CLI command with a given store (for testing).
pub async fn run_pairing_command_with_store(
    store: &crate::pairing::PairingStore,
    owner_id: &crate::ownership::OwnerId,
    cmd: PairingCommand,
) -> Result<(), anyhow::Error> {
    match cmd {
        PairingCommand::List { channel, json } => run_list(store, &channel, json).await,
        PairingCommand::Approve { channel, code } => {
            run_approve(store, &channel, &code, owner_id).await
        }
    }
}

async fn run_list(
    store: &crate::pairing::PairingStore,
    channel: &str,
    json: bool,
) -> Result<(), anyhow::Error> {
    let requests: Vec<crate::db::PairingRequestRecord> = store
        .list_pending(channel)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    if json {
        let out: Vec<serde_json::Value> = requests
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "channel": r.channel,
                    "external_id": r.external_id,
                    "code": r.code,
                    "created_at": r.created_at,
                    "expires_at": r.expires_at,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&out).map_err(|e| anyhow::anyhow!("{}", e))?
        );
        return Ok(());
    }

    if requests.is_empty() {
        println!("No pending {} pairing requests.", channel);
        return Ok(());
    }

    println!("Pairing requests ({}):", requests.len());
    for r in &requests {
        println!("  {}  {}  {}", r.code, r.external_id, r.created_at);
    }

    Ok(())
}

async fn run_approve(
    store: &crate::pairing::PairingStore,
    channel: &str,
    code: &str,
    owner_id: &crate::ownership::OwnerId,
) -> Result<(), anyhow::Error> {
    let _ = store
        .approve(channel, code, owner_id)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to approve pairing: {}", e))?;
    println!("Pairing approved.");
    Ok(())
}
