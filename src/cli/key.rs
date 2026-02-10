//! NEAR key management CLI commands.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Subcommand;
use tokio::fs;

use crate::config::Config;
use crate::history::Store;
use crate::keys::KeyManager;
use crate::keys::policy::{ChainSigRule, FunctionCallRule, PolicyConfig, SignatureDomain};
use crate::keys::types::{
    AccessKeyPermission, NearAccountId, NearNetwork, format_yocto, parse_near_amount,
};
use crate::secrets::{PostgresSecretsStore, SecretsCrypto, SecretsStore};

/// Default policy config path.
fn default_policy_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".ironclaw").join("key_policy.json"))
        .unwrap_or_else(|| PathBuf::from(".ironclaw/key_policy.json"))
}

#[derive(Subcommand, Debug, Clone)]
pub enum KeyCommand {
    /// Generate a new ed25519 keypair
    Generate {
        /// Label for the key (used to reference it later)
        label: String,

        /// NEAR account ID this key belongs to
        #[arg(long)]
        account: String,

        /// Permission level: "full-access" or "function-call"
        #[arg(long, default_value = "function-call")]
        permission: String,

        /// Contract to scope function-call keys to
        #[arg(long)]
        receiver: Option<String>,

        /// Comma-separated method names (empty = all methods on contract)
        #[arg(long)]
        methods: Option<String>,

        /// Allowance in NEAR (e.g., "1.5")
        #[arg(long)]
        allowance: Option<String>,

        /// Network: mainnet, testnet, or RPC URL
        #[arg(long, default_value = "testnet")]
        network: String,
    },

    /// Import an existing secret key
    Import {
        /// Label for the key
        label: String,

        /// NEAR account ID
        #[arg(long)]
        account: String,

        /// Permission level
        #[arg(long, default_value = "function-call")]
        permission: String,

        /// Contract to scope function-call keys to
        #[arg(long)]
        receiver: Option<String>,

        /// Comma-separated method names
        #[arg(long)]
        methods: Option<String>,

        /// Allowance in NEAR
        #[arg(long)]
        allowance: Option<String>,

        /// Network
        #[arg(long, default_value = "testnet")]
        network: String,
    },

    /// List all stored keys
    List {
        /// Show verbose details
        #[arg(short, long)]
        verbose: bool,
    },

    /// Show information about a key
    Info {
        /// Key label
        label: String,
    },

    /// Remove a key
    Remove {
        /// Key label
        label: String,
    },

    /// Export public key (NEVER exports private key)
    Export {
        /// Key label
        label: String,
    },

    /// Manage transaction approval policy
    #[command(subcommand)]
    Policy(PolicyCommand),

    /// Create encrypted backup of all keys
    Backup {
        /// Output file path
        #[arg(long)]
        output: PathBuf,

        /// List keys in a backup without restoring (still needs passphrase)
        #[arg(long)]
        list: bool,
    },

    /// Restore keys from encrypted backup
    Restore {
        /// Backup file path
        path: PathBuf,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum PolicyCommand {
    /// Show current policy configuration
    Show,

    /// Set auto-approve transfer limit
    SetTransferLimit {
        /// Max NEAR amount for auto-approved transfers (e.g., "1.5")
        amount: String,
    },

    /// Whitelist an account for transfers
    WhitelistAccount {
        /// Account ID to whitelist
        account: String,

        /// Max transfer amount in NEAR
        #[arg(long)]
        max_transfer: Option<String>,
    },

    /// Whitelist a validator for staking
    WhitelistValidator {
        /// Validator account ID
        validator: String,

        /// Max stake amount in NEAR
        #[arg(long)]
        max_stake: Option<String>,
    },

    /// Add a function call rule for a contract
    AddContractRule {
        /// Contract account ID
        contract: String,

        /// Comma-separated method names (empty = all)
        #[arg(long)]
        methods: Option<String>,

        /// Max deposit in NEAR
        #[arg(long, default_value = "0")]
        max_deposit: String,

        /// Auto-approve matching calls
        #[arg(long)]
        auto_approve: bool,
    },

    /// Add a chain signature rule
    AddChainSigRule {
        /// Derivation path pattern (supports * glob)
        path_pattern: String,

        /// Signature domain: secp256k1 or ed25519
        #[arg(long, default_value = "secp256k1")]
        domain: String,

        /// Max payload size in bytes
        #[arg(long, default_value = "4096")]
        max_payload: usize,

        /// Auto-approve matching requests
        #[arg(long)]
        auto_approve: bool,
    },

    /// Set daily cumulative spend limit
    SetDailyLimit {
        /// Max NEAR amount per day
        amount: String,
    },

    /// Set per-transaction auto-approve limit
    SetTxLimit {
        /// Max NEAR amount per transaction
        amount: String,
    },
}

/// Run a key management command.
pub async fn run_key_command(cmd: KeyCommand) -> anyhow::Result<()> {
    match cmd {
        KeyCommand::Generate {
            label,
            account,
            permission,
            receiver,
            methods,
            allowance,
            network,
        } => {
            let manager = create_key_manager().await?;
            let account_id = NearAccountId::new(&account)?;
            let network: NearNetwork = network.parse()?;
            let perm = parse_permission(&permission, receiver, methods, allowance)?;

            let metadata = manager
                .generate_key(&label, &account_id, perm.clone(), network)
                .await?;

            println!("Key generated successfully:");
            println!("  Label:      {}", metadata.label);
            println!("  Account:    {}", metadata.account_id);
            println!("  Public key: {}", metadata.public_key);
            println!("  Permission: {}", perm);
            println!("  Network:    {}", metadata.network);

            if matches!(perm, AccessKeyPermission::FullAccess) {
                println!();
                println!(
                    "  WARNING: This is a FULL ACCESS key for {}.",
                    metadata.account_id
                );
                println!("  If this is the ONLY full-access key for this account and you lose it,");
                println!("  the account becomes permanently inaccessible.");
                println!();
                println!("  Create a backup: ironclaw key backup --output <file>");
            }

            Ok(())
        }

        KeyCommand::Import {
            label,
            account,
            permission,
            receiver,
            methods,
            allowance,
            network,
        } => {
            let manager = create_key_manager().await?;
            let account_id = NearAccountId::new(&account)?;
            let network: NearNetwork = network.parse()?;
            let perm = parse_permission(&permission, receiver, methods, allowance)?;

            // Read secret key from stdin (hidden)
            print!("Paste secret key (ed25519:...): ");
            std::io::stdout().flush()?;
            let secret_key = read_hidden_line()?;
            println!();

            if secret_key.is_empty() {
                anyhow::bail!("No secret key provided");
            }

            let metadata = manager
                .import_key(&label, &account_id, &secret_key, perm.clone(), network)
                .await?;

            println!("Key imported successfully:");
            println!("  Label:      {}", metadata.label);
            println!("  Account:    {}", metadata.account_id);
            println!("  Public key: {}", metadata.public_key);
            println!("  Permission: {}", perm);

            if matches!(perm, AccessKeyPermission::FullAccess) {
                println!();
                println!("  WARNING: Full-access key imported. Back it up!");
                println!("  ironclaw key backup --output <file>");
            }

            Ok(())
        }

        KeyCommand::List { verbose } => {
            let manager = create_key_manager().await?;
            let keys = manager.list_keys().await?;

            if keys.is_empty() {
                println!("No keys stored.");
                println!("Generate one: ironclaw key generate <label> --account <id>");
                return Ok(());
            }

            println!("Stored keys:");
            println!();
            for key in keys {
                if verbose {
                    println!("  {} ({})", key.label, key.network);
                    println!("    Account:    {}", key.account_id);
                    println!("    Public key: {}", key.public_key);
                    println!("    Permission: {}", key.permission);
                    println!(
                        "    Created:    {}",
                        key.created_at.format("%Y-%m-%d %H:%M UTC")
                    );
                    println!();
                } else {
                    println!(
                        "  {} | {} | {} | {}",
                        key.label, key.account_id, key.permission, key.network
                    );
                }
            }

            Ok(())
        }

        KeyCommand::Info { label } => {
            let manager = create_key_manager().await?;
            let key = manager.get_key(&label).await?;

            println!("Key: {}", key.label);
            println!("  Account:    {}", key.account_id);
            println!("  Public key: {}", key.public_key);
            println!("  Permission: {}", key.permission);
            println!("  Network:    {}", key.network);
            println!(
                "  Created:    {}",
                key.created_at.format("%Y-%m-%d %H:%M UTC")
            );

            Ok(())
        }

        KeyCommand::Remove { label } => {
            let manager = create_key_manager().await?;
            manager.remove_key(&label).await?;
            println!("Key '{}' removed.", label);
            Ok(())
        }

        KeyCommand::Export { label } => {
            let manager = create_key_manager().await?;
            let pubkey = manager.export_public_key(&label).await?;
            println!("{}", pubkey.to_near_format());
            Ok(())
        }

        KeyCommand::Policy(policy_cmd) => run_policy_command(policy_cmd).await,

        KeyCommand::Backup { output, list } => {
            if list {
                // List keys in backup
                let data = fs::read(&output).await?;

                print!("Backup passphrase: ");
                std::io::stdout().flush()?;
                let passphrase = read_hidden_line()?;
                println!();

                // We need to decrypt to list, so restore to a temp manager
                // and just display, not actually import
                let plaintext = crate::keys::decrypt_backup(&passphrase, &data)?;
                let backup: serde_json::Value = serde_json::from_slice(&plaintext)?;

                if let Some(keys) = backup.get("keys").and_then(|k| k.as_array()) {
                    println!("Keys in backup ({}):", output.display());
                    for key in keys {
                        let label = key.get("label").and_then(|l| l.as_str()).unwrap_or("?");
                        let account = key
                            .get("account_id")
                            .and_then(|a| a.as_str())
                            .unwrap_or("?");
                        println!("  {} ({})", label, account);
                    }
                }
                return Ok(());
            }

            let manager = create_key_manager().await?;

            print!("Backup passphrase: ");
            std::io::stdout().flush()?;
            let passphrase = read_hidden_line()?;
            println!();

            print!("Confirm passphrase: ");
            std::io::stdout().flush()?;
            let confirm = read_hidden_line()?;
            println!();

            if passphrase != confirm {
                anyhow::bail!("Passphrases do not match");
            }

            if passphrase.len() < 8 {
                anyhow::bail!("Passphrase must be at least 8 characters");
            }

            let backup_data = manager.create_backup(&passphrase).await?;
            fs::write(&output, &backup_data).await?;

            println!(
                "Backup created: {} ({} bytes)",
                output.display(),
                backup_data.len()
            );
            println!("Store this file securely. You'll need the passphrase to restore.");

            Ok(())
        }

        KeyCommand::Restore { path } => {
            let manager = create_key_manager().await?;

            let data = fs::read(&path).await?;

            print!("Backup passphrase: ");
            std::io::stdout().flush()?;
            let passphrase = read_hidden_line()?;
            println!();

            let restored = manager.restore_backup(&data, &passphrase).await?;

            if restored.is_empty() {
                println!("No new keys to restore (all already exist).");
            } else {
                println!("Restored {} keys:", restored.len());
                for label in &restored {
                    println!("  {}", label);
                }
            }

            Ok(())
        }
    }
}

async fn run_policy_command(cmd: PolicyCommand) -> anyhow::Result<()> {
    let policy_path = default_policy_path();

    match cmd {
        PolicyCommand::Show => {
            let policy = load_policy(&policy_path).await?;
            let json = serde_json::to_string_pretty(&policy)?;
            println!("{}", json);
            Ok(())
        }

        PolicyCommand::SetTransferLimit { amount } => {
            let yocto = parse_near_amount(&amount)?;
            let mut policy = load_policy(&policy_path).await?;
            policy.transfer_auto_approve_max_yocto = yocto;
            save_policy(&policy_path, &policy).await?;
            println!("Transfer auto-approve limit set to {}", format_yocto(yocto));
            Ok(())
        }

        PolicyCommand::WhitelistAccount {
            account,
            max_transfer,
        } => {
            let mut policy = load_policy(&policy_path).await?;
            if !policy.transfer_whitelist.contains(&account) {
                policy.transfer_whitelist.push(account.clone());
            }
            if let Some(max) = max_transfer {
                policy.transfer_whitelist_max_yocto = parse_near_amount(&max)?;
            }
            save_policy(&policy_path, &policy).await?;
            println!("Account '{}' added to transfer whitelist", account);
            Ok(())
        }

        PolicyCommand::WhitelistValidator {
            validator,
            max_stake,
        } => {
            let mut policy = load_policy(&policy_path).await?;
            if !policy.stake_validator_whitelist.contains(&validator) {
                policy.stake_validator_whitelist.push(validator.clone());
            }
            if let Some(max) = max_stake {
                policy.stake_auto_approve_max_yocto = parse_near_amount(&max)?;
            }
            save_policy(&policy_path, &policy).await?;
            println!("Validator '{}' added to staking whitelist", validator);
            Ok(())
        }

        PolicyCommand::AddContractRule {
            contract,
            methods,
            max_deposit,
            auto_approve,
        } => {
            let mut policy = load_policy(&policy_path).await?;
            let deposit = parse_near_amount(&max_deposit)?;
            let method_list = methods
                .map(|m| m.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default();

            policy.function_call_rules.push(FunctionCallRule {
                receiver_id: contract.clone(),
                allowed_methods: method_list,
                max_deposit_yocto: deposit,
                max_gas: None,
                auto_approve,
            });
            save_policy(&policy_path, &policy).await?;
            println!(
                "Contract rule added for '{}' (auto_approve={})",
                contract, auto_approve
            );
            Ok(())
        }

        PolicyCommand::AddChainSigRule {
            path_pattern,
            domain,
            max_payload,
            auto_approve,
        } => {
            let domain = match domain.to_lowercase().as_str() {
                "secp256k1" => SignatureDomain::Secp256k1,
                "ed25519" => SignatureDomain::Ed25519,
                other => anyhow::bail!("Unknown domain '{}', expected secp256k1 or ed25519", other),
            };

            let mut policy = load_policy(&policy_path).await?;
            policy.chain_sig_rules.push(ChainSigRule {
                allowed_paths: vec![path_pattern.clone()],
                allowed_domains: vec![domain],
                max_payload_bytes: max_payload,
                auto_approve,
            });
            save_policy(&policy_path, &policy).await?;
            println!(
                "Chain signature rule added for '{}' (auto_approve={})",
                path_pattern, auto_approve
            );
            Ok(())
        }

        PolicyCommand::SetDailyLimit { amount } => {
            let yocto = parse_near_amount(&amount)?;
            let mut policy = load_policy(&policy_path).await?;
            policy.daily_spend_limit_yocto = Some(yocto);
            save_policy(&policy_path, &policy).await?;
            println!("Daily spend limit set to {}", format_yocto(yocto));
            Ok(())
        }

        PolicyCommand::SetTxLimit { amount } => {
            let yocto = parse_near_amount(&amount)?;
            let mut policy = load_policy(&policy_path).await?;
            policy.per_tx_auto_approve_max_yocto = yocto;
            save_policy(&policy_path, &policy).await?;
            println!(
                "Per-transaction auto-approve limit set to {}",
                format_yocto(yocto)
            );
            Ok(())
        }
    }
}

async fn load_policy(path: &PathBuf) -> anyhow::Result<PolicyConfig> {
    if path.exists() {
        let content = fs::read_to_string(path).await?;
        Ok(serde_json::from_str(&content)?)
    } else {
        Ok(PolicyConfig::default())
    }
}

async fn save_policy(path: &PathBuf, policy: &PolicyConfig) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let content = serde_json::to_string_pretty(policy)?;
    fs::write(path, content).await?;
    Ok(())
}

fn parse_permission(
    permission: &str,
    receiver: Option<String>,
    methods: Option<String>,
    allowance: Option<String>,
) -> anyhow::Result<AccessKeyPermission> {
    match permission {
        "full-access" | "FullAccess" => Ok(AccessKeyPermission::FullAccess),
        "function-call" | "FunctionCall" => {
            let receiver_id = receiver
                .ok_or_else(|| anyhow::anyhow!("--receiver required for function-call keys"))?;

            let method_names = methods
                .map(|m| m.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default();

            let allowance_yocto = allowance
                .map(|a| parse_near_amount(&a))
                .transpose()
                .map_err(|e| anyhow::anyhow!("invalid allowance: {}", e))?;

            Ok(AccessKeyPermission::FunctionCall {
                allowance: allowance_yocto,
                receiver_id,
                method_names,
            })
        }
        other => Err(anyhow::anyhow!(
            "unknown permission '{}', expected full-access or function-call",
            other
        )),
    }
}

/// Create a KeyManager with the default secrets store.
async fn create_key_manager() -> anyhow::Result<KeyManager> {
    let config = Config::from_env()?;
    let master_key = config.secrets.master_key().ok_or_else(|| {
        anyhow::anyhow!(
            "SECRETS_MASTER_KEY not set. Run 'ironclaw onboard' first or set it in .env"
        )
    })?;

    let store = Store::new(&config.database).await?;
    store.run_migrations().await?;

    let crypto = SecretsCrypto::new(master_key.clone())?;
    let secrets_store: Arc<dyn SecretsStore + Send + Sync> =
        Arc::new(PostgresSecretsStore::new(store.pool(), Arc::new(crypto)));

    let manager = KeyManager::new(secrets_store, "default".to_string());

    // Load policy if it exists
    let policy_path = default_policy_path();
    if policy_path.exists() {
        let content = fs::read_to_string(&policy_path).await?;
        let policy: PolicyConfig = serde_json::from_str(&content)?;
        Ok(manager.with_policy(policy))
    } else {
        Ok(manager)
    }
}

/// Read a line of input with hidden characters.
fn read_hidden_line() -> anyhow::Result<String> {
    use crossterm::{
        event::{self, Event, KeyCode, KeyModifiers},
        terminal,
    };

    let mut input = String::new();
    terminal::enable_raw_mode()?;

    loop {
        if let Event::Key(key_event) = event::read()? {
            match key_event.code {
                KeyCode::Enter => break,
                KeyCode::Backspace => {
                    if !input.is_empty() {
                        input.pop();
                        print!("\x08 \x08");
                        std::io::stdout().flush()?;
                    }
                }
                KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                    terminal::disable_raw_mode()?;
                    return Err(anyhow::anyhow!("Interrupted"));
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    print!("*");
                    std::io::stdout().flush()?;
                }
                _ => {}
            }
        }
    }

    terminal::disable_raw_mode()?;
    Ok(input)
}

#[cfg(test)]
mod tests {
    use crate::cli::key::parse_permission;
    use crate::keys::types::AccessKeyPermission;

    #[test]
    fn test_parse_full_access() {
        let perm = parse_permission("full-access", None, None, None).unwrap();
        assert!(matches!(perm, AccessKeyPermission::FullAccess));
    }

    #[test]
    fn test_parse_function_call() {
        let perm = parse_permission(
            "function-call",
            Some("contract.near".to_string()),
            Some("deposit,withdraw".to_string()),
            Some("1.5".to_string()),
        )
        .unwrap();

        match perm {
            AccessKeyPermission::FunctionCall {
                receiver_id,
                method_names,
                allowance,
            } => {
                assert_eq!(receiver_id, "contract.near");
                assert_eq!(method_names, vec!["deposit", "withdraw"]);
                assert!(allowance.is_some());
            }
            _ => panic!("expected FunctionCall"),
        }
    }

    #[test]
    fn test_parse_function_call_missing_receiver() {
        let result = parse_permission("function-call", None, None, None);
        assert!(result.is_err());
    }
}
