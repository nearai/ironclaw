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
    }
}

/// Channel entry for display.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChannelInfo {
    name: String,
    kind: &'static str,
    enabled: bool,
    details: Vec<(&'static str, String)>,
}

fn builtin_channels(config: &crate::config::ChannelsConfig) -> Vec<ChannelInfo> {
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

    channels
}

/// List all configured channels.
async fn cmd_list(
    config: &crate::config::ChannelsConfig,
    verbose: bool,
    json: bool,
) -> anyhow::Result<()> {
    let mut channels = builtin_channels(config);

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
        println!("Note: enable/disable not yet available.");
        println!("Built-in channels are managed via environment variables and installable");
        println!("channels are configured through the extensions lifecycle.");
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
    use crate::config::{ChannelsConfig, CliConfig, GatewayConfig, HttpConfig};
    use std::collections::HashMap;
    use std::path::PathBuf;

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

    #[test]
    fn builtin_channels_do_not_include_xmpp() {
        let config = ChannelsConfig {
            cli: CliConfig { enabled: true },
            http: Some(HttpConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
                webhook_secret: None,
                user_id: "owner".to_string(),
            }),
            gateway: Some(GatewayConfig {
                host: "127.0.0.1".to_string(),
                port: 3000,
                auth_token: None,
                user_id: "owner".to_string(),
                workspace_read_scopes: vec![],
                memory_layers: vec![],
                user_tokens: None,
            }),
            signal: None,
            xmpp: None,
            wasm_channels_dir: PathBuf::from("/tmp/channels"),
            wasm_channels_enabled: true,
            wasm_channel_owner_ids: HashMap::new(),
        };

        let channels = builtin_channels(&config);
        assert!(channels.iter().all(|channel| channel.name != "xmpp"));
    }
}
