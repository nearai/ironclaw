//! DM pairing CLI commands.
//!
//! Manage pairing requests for channels (Telegram, Slack, etc.).

use clap::Subcommand;

use crate::pairing::PairingStore;

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

/// Run pairing CLI command.
pub fn run_pairing_command(cmd: PairingCommand) -> Result<(), String> {
    run_pairing_command_with_store(&PairingStore::new(), cmd)
}

/// Run pairing CLI command with a given store (for testing).
pub fn run_pairing_command_with_store(
    store: &PairingStore,
    cmd: PairingCommand,
) -> Result<(), String> {
    match cmd {
        PairingCommand::List { channel, json } => run_list(store, &channel, json),
        PairingCommand::Approve { channel, code } => run_approve(store, &channel, &code),
    }
}

fn run_list(store: &PairingStore, channel: &str, json: bool) -> Result<(), String> {
    let requests = store.list_pending(channel).map_err(|e| e.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&requests).map_err(|e| e.to_string())?
        );
        return Ok(());
    }

    if requests.is_empty() {
        println!("No pending {} pairing requests.", channel);
        return Ok(());
    }

    println!("Pairing requests ({}):", requests.len());
    for r in &requests {
        let meta = r
            .meta
            .as_ref()
            .and_then(|m| m.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| format!("{}={}", k, s)))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        println!("  {}  {}  {}  {}", r.code, r.id, meta, r.created_at);
    }

    Ok(())
}

fn run_approve(store: &PairingStore, channel: &str, code: &str) -> Result<(), String> {
    match store.approve(channel, code) {
        Ok(Some(entry)) => {
            println!("Approved {} sender {}.", channel, entry.id);
            Ok(())
        }
        Ok(None) => Err(format!(
            "No pending pairing request found for code: {}",
            code
        )),
        Err(crate::pairing::PairingStoreError::ApproveRateLimited) => Err(
            "Too many failed approve attempts. Wait a few minutes before trying again.".to_string(),
        ),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (PairingStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = PairingStore::with_base_dir(dir.path().to_path_buf());
        (store, dir)
    }

    #[test]
    fn test_list_empty_returns_ok() {
        let (store, _) = test_store();
        let result = run_pairing_command_with_store(
            &store,
            PairingCommand::List {
                channel: "telegram".to_string(),
                json: false,
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_list_json_empty_returns_ok() {
        let (store, _) = test_store();
        let result = run_pairing_command_with_store(
            &store,
            PairingCommand::List {
                channel: "telegram".to_string(),
                json: true,
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_approve_invalid_code_returns_err() {
        let (store, _) = test_store();
        // Create a pending request so the pairing file exists, then approve with wrong code
        store.upsert_request("telegram", "user1", None).unwrap();

        let result = run_pairing_command_with_store(
            &store,
            PairingCommand::Approve {
                channel: "telegram".to_string(),
                code: "BADCODE1".to_string(),
            },
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No pending pairing request"));
    }

    #[test]
    fn test_approve_valid_code_returns_ok() {
        let (store, _) = test_store();
        let r = store.upsert_request("telegram", "user1", None).unwrap();
        assert!(r.created);

        let result = run_pairing_command_with_store(
            &store,
            PairingCommand::Approve {
                channel: "telegram".to_string(),
                code: r.code,
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_list_with_pending_returns_ok() {
        let (store, _) = test_store();
        store.upsert_request("telegram", "user1", None).unwrap();

        let result = run_pairing_command_with_store(
            &store,
            PairingCommand::List {
                channel: "telegram".to_string(),
                json: false,
            },
        );
        assert!(result.is_ok());
    }
}
