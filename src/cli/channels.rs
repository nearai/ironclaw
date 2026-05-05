//! Channel management CLI commands.
//!
//! Lists configured messaging channels and their status.
//! Enable/disable/status subcommands are deferred pending channel config source
//! unification (see module-level note below).
//!
//! ## Why only `list` for now
//!
//! `enable`/`disable` require modifying channel configuration, but the config
//! source is currently split: built-in channels (cli, http, gateway, signal)
//! are resolved from environment variables in `ChannelsConfig::resolve()`,
//! while `settings.channels.*` fields are not consumed by that path.
//! Until `resolve()` falls back to settings (or the CLI writes `.env`),
//! an `enable`/`disable` command would silently fail to take effect.
//!
//! `status` (runtime health) requires connecting to a running IronClaw instance
//! via IPC or HTTP, which does not exist yet as a CLI control plane.

use std::path::Path;

use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum ChannelsCommand {
    /// List all configured channels
    List {
        /// Show detailed information (host, port, config source)
        #[arg(short, long)]
        verbose: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Install IronClaw into a workspace channel (currently: `slack`).
    ///
    /// Prints the Slack app manifest to upload at api.slack.com/apps,
    /// persists the workspace identity, and prints the slash-command
    /// target URL the installer pastes into the Slack app config.
    ///
    /// Example: `ironclaw channels install slack T0XXXX --base-url https://ironclaw.example.com`
    Install {
        /// Channel name. Currently only `slack` is supported.
        #[arg(required = true)]
        channel: String,

        /// Slack workspace/team id (`T…`) or Enterprise Grid id (`E…`).
        #[arg(required = true)]
        workspace_id: String,

        /// Public HTTPS origin Slack will reach for events, slash commands,
        /// and the OAuth install callback. No trailing slash required.
        #[arg(long)]
        base_url: String,

        /// Emit only the manifest JSON to stdout (for piping into `jq` or
        /// saving). Suppresses the "next steps" banner.
        #[arg(long, default_value_t = false)]
        manifest_only: bool,
    },
}

/// Run the channels CLI subcommand.
pub async fn run_channels_command(
    cmd: ChannelsCommand,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    let config = crate::config::Config::from_env_with_toml(config_path)
        .await
        .map_err(|e| anyhow::anyhow!("{e:#}"))?;

    match cmd {
        ChannelsCommand::List { verbose, json } => cmd_list(&config.channels, verbose, json).await,
        ChannelsCommand::Install {
            channel,
            workspace_id,
            base_url,
            manifest_only,
        } => {
            // Only `slack` lands in this commit. `telegram`/`signal` go through
            // the existing onboard wizard and don't yet have a workspace-scoped
            // install path. Reject other channel names so callers get a clear
            // error rather than a silently-wrong manifest.
            let channel_norm = channel.to_ascii_lowercase();
            if channel_norm != "slack" {
                anyhow::bail!(
                    "install: channel '{channel}' is not supported. Currently only 'slack' \
                     supports `channels install`; other channels are configured via \
                     `ironclaw onboard --step channels`."
                );
            }
            cmd_install_slack(&config, &workspace_id, &base_url, manifest_only).await
        }
    }
}

/// Install the Slack channel for a workspace.
///
/// Workflow:
///   1. Ensure the deployment owner user row exists (FK requirement on
///      `channel_identities.owner_id`).
///   2. Idempotently upsert `(channel='slack', external_id=workspace_id,
///      owner_id=deployment_owner)` into `channel_identities`.
///   3. Generate the Slack app manifest with minimal scopes.
///   4. Print operator-facing instructions: upload manifest, complete OAuth,
///      paste the slash-command URL into the Slack app config.
///
/// `--manifest-only` short-circuits steps (1)-(2) and emits only the JSON,
/// for piping into another tool.
async fn cmd_install_slack(
    config: &crate::config::Config,
    workspace_id: &str,
    base_url: &str,
    manifest_only: bool,
) -> anyhow::Result<()> {
    use crate::channels::slack::{generate_manifest, manifest::slash_command_url};

    let workspace_id = workspace_id.trim();
    if workspace_id.is_empty() {
        anyhow::bail!("install slack: workspace_id is required");
    }
    // Slack team ids start with `T`, Enterprise Grid ids with `E`. Catch the
    // "ABC123" / numeric typo before we write a junk row to the DB.
    let first = workspace_id.chars().next().unwrap_or('?');
    if first != 'T' && first != 'E' {
        anyhow::bail!(
            "install slack: workspace_id '{workspace_id}' does not look like a Slack \
             team id (expected `T…`) or Enterprise Grid id (expected `E…`)"
        );
    }

    let manifest = generate_manifest(base_url);
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| anyhow::anyhow!("failed to serialize manifest: {e}"))?;

    if manifest_only {
        println!("{manifest_json}");
        return Ok(());
    }

    // Persist workspace identity. Mirror the pairing CLI's approach: ensure
    // the deployment-owner user row exists first (FK on channel_identities).
    let db = crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("failed to connect to database: {e}"))?;

    db.get_or_create_user(crate::db::UserRecord {
        id: config.owner_id.clone(),
        role: crate::ownership::UserRole::Owner.as_db_role().to_string(),
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

    let was_new = db
        .upsert_channel_identity("slack", workspace_id, &config.owner_id)
        .await
        .map_err(|e| anyhow::anyhow!("failed to persist workspace identity: {e}"))?;

    let slash_url = slash_command_url(base_url);
    let install_url = format!(
        "{}/auth/slack/install/callback",
        base_url.trim_end_matches('/')
    );

    println!("Slack workspace install for {workspace_id}");
    println!(
        "  identity:    {}",
        if was_new {
            "registered (new)"
        } else {
            "already registered (owner refreshed)"
        }
    );
    println!("  owner:       {}", config.owner_id);
    println!();
    println!("Next steps:");
    println!("  1. Open https://api.slack.com/apps?new_app=1 and choose \"From an app manifest\".");
    println!("  2. Paste the manifest below into the workspace of your choice.");
    println!(
        "  3. After Slack creates the app, install it. Slack will redirect to:\n     {install_url}"
    );
    println!("  4. The bot token will be captured by the redirect handler. Subsequent");
    println!("     follow-up commits on this branch land the slash-command surface and");
    println!("     audit log; this commit ships the install path only.");
    println!();
    println!("Slash-command URL (paste into Slack app config):");
    println!("  {slash_url}");
    println!();
    println!("Manifest JSON:");
    println!("{manifest_json}");

    Ok(())
}

/// Channel entry for display.
struct ChannelInfo {
    name: String,
    kind: &'static str,
    enabled: bool,
    details: Vec<(&'static str, String)>,
}

/// List all configured channels.
async fn cmd_list(
    config: &crate::config::ChannelsConfig,
    verbose: bool,
    json: bool,
) -> anyhow::Result<()> {
    let mut channels = Vec::new();

    // Built-in: CLI
    channels.push(ChannelInfo {
        name: "cli".to_string(),
        kind: "built-in",
        enabled: config.cli.enabled,
        details: vec![],
    });

    // Built-in: Gateway
    if let Some(ref gw) = config.gateway {
        channels.push(ChannelInfo {
            name: "gateway".to_string(),
            kind: "built-in",
            enabled: true,
            details: vec![("host", gw.host.clone()), ("port", gw.port.to_string())],
        });
    } else {
        channels.push(ChannelInfo {
            name: "gateway".to_string(),
            kind: "built-in",
            enabled: false,
            details: vec![],
        });
    }

    // Built-in: HTTP webhook
    if let Some(ref http) = config.http {
        channels.push(ChannelInfo {
            name: "http".to_string(),
            kind: "built-in",
            enabled: true,
            details: vec![("host", http.host.clone()), ("port", http.port.to_string())],
        });
    } else {
        channels.push(ChannelInfo {
            name: "http".to_string(),
            kind: "built-in",
            enabled: false,
            details: vec![],
        });
    }

    // Built-in: Signal
    if let Some(ref sig) = config.signal {
        channels.push(ChannelInfo {
            name: "signal".to_string(),
            kind: "built-in",
            enabled: true,
            details: vec![
                ("http_url", sig.http_url.clone()),
                ("account", sig.account.clone()),
                ("dm_policy", sig.dm_policy.clone()),
                ("group_policy", sig.group_policy.clone()),
            ],
        });
    } else {
        channels.push(ChannelInfo {
            name: "signal".to_string(),
            kind: "built-in",
            enabled: false,
            details: vec![],
        });
    }

    // WASM channels: scan directory
    if config.wasm_channels_enabled {
        let wasm_channels = discover_wasm_channels(&config.wasm_channels_dir).await;
        for name in wasm_channels {
            let owner = config.wasm_channel_owner_ids.get(&name);
            let mut details = vec![];
            if let Some(id) = owner {
                details.push(("owner_id", id.to_string()));
            }
            channels.push(ChannelInfo {
                name,
                kind: "wasm",
                enabled: true,
                details,
            });
        }
    }

    if json {
        let entries: Vec<serde_json::Value> = channels
            .iter()
            .map(|ch| {
                let mut v = serde_json::json!({
                    "name": ch.name,
                    "kind": ch.kind,
                    "enabled": ch.enabled,
                });
                if verbose {
                    let details: serde_json::Map<String, serde_json::Value> = ch
                        .details
                        .iter()
                        .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.clone())))
                        .collect();
                    v["details"] = serde_json::Value::Object(details);
                }
                v
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string())
        );
        return Ok(());
    }

    let enabled_count = channels.iter().filter(|c| c.enabled).count();
    println!(
        "Configured channels ({} enabled, {} total):\n",
        enabled_count,
        channels.len()
    );

    for ch in &channels {
        let status = if ch.enabled { "enabled" } else { "disabled" };
        if verbose {
            println!("  {} [{}] ({})", ch.name, status, ch.kind);
            for (key, val) in &ch.details {
                println!("    {}: {}", key, val);
            }
            if ch.details.is_empty() && ch.enabled {
                println!("    (default config)");
            }
            println!();
        } else {
            let detail_str = if ch.enabled && !ch.details.is_empty() {
                let parts: Vec<String> =
                    ch.details.iter().map(|(k, v)| format!("{k}={v}")).collect();
                format!("  ({})", parts.join(", "))
            } else {
                String::new()
            };
            println!(
                "  {:<16} {:<10} {:<10}{}",
                ch.name, status, ch.kind, detail_str
            );
        }
    }

    if !verbose {
        println!();
        println!("Use --verbose for details.");
        println!();
        println!("Note: enable/disable not yet available. Channel configuration is");
        println!("managed via environment variables. See 'ironclaw onboard --channels-only'.");
    }

    Ok(())
}

/// Discover WASM channel names by scanning the channels directory for `*.wasm` files.
///
/// Matches the real loader's discovery logic (`WasmChannelLoader::load_from_dir`):
/// scans only top-level `*.wasm` files in the directory.
async fn discover_wasm_channels(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(_) => return names,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("wasm")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.push(stem.to_string());
        }
    }

    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn discover_wasm_channels_empty_on_missing_dir() {
        let result = discover_wasm_channels(Path::new("/nonexistent/path")).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn discover_wasm_channels_finds_flat_wasm_files() {
        let tmp = tempfile::tempdir().unwrap();
        // Flat .wasm files — matches real loader (load_from_dir)
        std::fs::File::create(tmp.path().join("slack.wasm")).unwrap();
        std::fs::File::create(tmp.path().join("telegram.wasm")).unwrap();
        // Non-.wasm files should be skipped
        std::fs::File::create(tmp.path().join("readme.txt")).unwrap();
        // Directories should be skipped
        std::fs::create_dir(tmp.path().join("somedir")).unwrap();

        let result = discover_wasm_channels(tmp.path()).await;
        assert_eq!(result, vec!["slack", "telegram"]);
    }

    #[test]
    fn channel_info_struct() {
        let info = ChannelInfo {
            name: "test".to_string(),
            kind: "built-in",
            enabled: true,
            details: vec![("port", "3000".to_string())],
        };
        assert!(info.enabled);
        assert_eq!(info.kind, "built-in");
        assert_eq!(info.details.len(), 1);
    }
}
