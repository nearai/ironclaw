//! Hooks management CLI commands.
//!
//! Lists all discoverable lifecycle hooks from bundled and plugin (WASM
//! capabilities) sources. Plugin discovery uses the same flat-file sidecar
//! layout as the WASM tool/channel loaders (`foo.wasm` + `foo.capabilities.json`).
//!
//! Workspace hooks (`hooks/hooks.json`, `hooks/*.hook.json`) are stored in the
//! database-backed Workspace and require a DB connection to enumerate; this
//! command does not connect to the database, so workspace hooks are omitted.

use std::path::Path;

use clap::Subcommand;

use crate::hooks::bundled::{HookBundleConfig, HookRuleConfig, OutboundWebhookConfig};
use crate::hooks::hook::HookPoint;

const BUNDLED_AUDIT_PRIORITY: u32 = 25;
const DEFAULT_RULE_PRIORITY: u32 = 100;
const DEFAULT_WEBHOOK_PRIORITY: u32 = 300;

#[derive(Subcommand, Debug, Clone)]
pub enum HooksCommand {
    /// List discoverable hooks (bundled + plugin; not filtered by active extensions)
    List {
        /// Show detailed information (hook points, priority, failure mode)
        #[arg(short, long)]
        verbose: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Run the hooks CLI subcommand.
pub async fn run_hooks_command(
    cmd: HooksCommand,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    let config = crate::config::Config::from_env_with_toml(config_path)
        .await
        .map_err(|e| anyhow::anyhow!("{e:#}"))?;

    match cmd {
        HooksCommand::List { verbose, json } => cmd_list(&config, verbose, json).await,
    }
}

/// Discovered hook information for CLI display.
struct HookInfo {
    name: String,
    source: String,
    kind: String,
    points: Vec<HookPoint>,
    priority: u32,
    failure_mode: String,
}

/// Collect all discoverable hooks from bundled and plugin sources.
async fn discover_hooks(config: &crate::config::Config) -> Vec<HookInfo> {
    let mut hooks = Vec::new();

    // 1. Bundled hooks (hardcoded)
    hooks.push(HookInfo {
        name: "builtin.audit_log".to_string(),
        source: "bundled".to_string(),
        kind: "audit".to_string(),
        points: vec![
            HookPoint::BeforeInbound,
            HookPoint::BeforeToolCall,
            HookPoint::BeforeOutbound,
            HookPoint::OnSessionStart,
            HookPoint::OnSessionEnd,
            HookPoint::TransformResponse,
        ],
        priority: BUNDLED_AUDIT_PRIORITY,
        failure_mode: "fail_open".to_string(),
    });

    // 2. Plugin hooks from WASM capabilities sidecar files
    let wasm_tools_dir = &config.wasm.tools_dir;
    let wasm_channels_dir = &config.channels.wasm_channels_dir;

    collect_plugin_hooks(&mut hooks, wasm_tools_dir, "tool").await;
    collect_plugin_hooks(&mut hooks, wasm_channels_dir, "channel").await;

    // Note: workspace hooks (hooks/hooks.json, hooks/*.hook.json) are stored
    // in the database-backed Workspace and require a DB connection to list.

    // Sort by priority then name for stable output
    hooks.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.name.cmp(&b.name)));

    hooks
}

/// Scan a WASM directory for `*.capabilities.json` sidecar files containing hook
/// definitions.
///
/// Uses the same flat-file layout as the real WASM loaders:
/// ```text
/// ~/.ironclaw/tools/
/// ├── slack.wasm
/// ├── slack.capabilities.json   <- hooks section parsed here
/// ├── github.wasm
/// └── github.capabilities.json
/// ```
async fn collect_plugin_hooks(hooks: &mut Vec<HookInfo>, dir: &Path, plugin_type: &str) {
    if !dir.exists() {
        return;
    }

    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(_) => return,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();

        // Match only *.capabilities.json sidecar files (flat layout)
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        if !file_name.ends_with(".capabilities.json") {
            continue;
        }

        // Extract tool/channel name: "slack.capabilities.json" -> "slack"
        let name = match file_name.strip_suffix(".capabilities.json") {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };

        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(_) => continue,
        };

        let value: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Match the same extraction logic as bootstrap: check "hooks" key
        // at root or nested under "capabilities.hooks".
        let hooks_section = value
            .get("hooks")
            .or_else(|| value.get("capabilities").and_then(|c| c.get("hooks")));

        let Some(hooks_value) = hooks_section else {
            continue;
        };

        let bundle = match HookBundleConfig::from_value(hooks_value) {
            Ok(b) => b,
            Err(_) => continue,
        };

        let source = format!("plugin.{plugin_type}:{name}");

        for rule in &bundle.rules {
            hooks.push(hook_info_from_rule(&source, rule));
        }
        for webhook in &bundle.outbound_webhooks {
            hooks.push(hook_info_from_webhook(&source, webhook));
        }
    }
}

fn hook_info_from_rule(source: &str, rule: &HookRuleConfig) -> HookInfo {
    let scoped_name = format!("{source}::{}", rule.name);
    HookInfo {
        name: scoped_name,
        source: source.to_string(),
        kind: if rule.reject_reason.is_some() {
            "reject".to_string()
        } else {
            "rule".to_string()
        },
        points: rule.points.clone(),
        priority: rule.priority.unwrap_or(DEFAULT_RULE_PRIORITY),
        failure_mode: rule
            .failure_mode
            .as_ref()
            .map(|m| format!("{m:?}"))
            .unwrap_or_else(|| "fail_open".to_string()),
    }
}

fn hook_info_from_webhook(source: &str, webhook: &OutboundWebhookConfig) -> HookInfo {
    let scoped_name = format!("{source}::{}", webhook.name);
    HookInfo {
        name: scoped_name,
        source: source.to_string(),
        kind: "webhook".to_string(),
        points: webhook.points.clone(),
        priority: webhook.priority.unwrap_or(DEFAULT_WEBHOOK_PRIORITY),
        failure_mode: "fail_open".to_string(),
    }
}

/// List all discovered hooks.
async fn cmd_list(config: &crate::config::Config, verbose: bool, json: bool) -> anyhow::Result<()> {
    let hooks = discover_hooks(config).await;

    if json {
        let entries: Vec<serde_json::Value> = hooks
            .iter()
            .map(|h| {
                let mut v = serde_json::json!({
                    "name": h.name,
                    "source": h.source,
                    "kind": h.kind,
                    "priority": h.priority,
                    "points": h.points.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
                });
                if verbose {
                    v["failure_mode"] = serde_json::json!(h.failure_mode);
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

    if hooks.is_empty() {
        println!("No hooks found.");
        return Ok(());
    }

    println!("Discovered {} hook(s):\n", hooks.len());

    for h in &hooks {
        if verbose {
            let points_str: Vec<&str> = h.points.iter().map(|p| p.as_str()).collect();
            println!("  {}", h.name);
            println!("    Source:       {}", h.source);
            println!("    Kind:         {}", h.kind);
            println!("    Priority:     {}", h.priority);
            println!("    Points:       {}", points_str.join(", "));
            println!("    Failure mode: {}", h.failure_mode);
            println!();
        } else {
            let points_str: Vec<&str> = h.points.iter().map(|p| p.as_str()).collect();
            println!(
                "  {:<40} [{:<7}] pri={:<3} {}",
                h.name,
                h.kind,
                h.priority,
                points_str.join(", ")
            );
        }
    }

    if !verbose {
        println!();
        println!(
            "Use --verbose for details. Workspace hooks (DB-stored) are not listed without a database connection."
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn hook_info_from_rule_basic() {
        let rule = HookRuleConfig {
            name: "test-rule".to_string(),
            points: vec![HookPoint::BeforeInbound],
            priority: Some(50),
            failure_mode: None,
            timeout_ms: None,
            when_regex: None,
            reject_reason: None,
            replacements: vec![],
            prepend: None,
            append: None,
        };

        let info = hook_info_from_rule("plugin.tool:my_tool", &rule);
        assert_eq!(info.name, "plugin.tool:my_tool::test-rule");
        assert_eq!(info.source, "plugin.tool:my_tool");
        assert_eq!(info.kind, "rule");
        assert_eq!(info.priority, 50);
    }

    #[test]
    fn hook_info_from_rule_reject() {
        let rule = HookRuleConfig {
            name: "blocker".to_string(),
            points: vec![HookPoint::BeforeInbound, HookPoint::BeforeToolCall],
            priority: None,
            failure_mode: None,
            timeout_ms: None,
            when_regex: Some("bad_pattern".to_string()),
            reject_reason: Some("blocked".to_string()),
            replacements: vec![],
            prepend: None,
            append: None,
        };

        let info = hook_info_from_rule("workspace:hooks/block.hook.json", &rule);
        assert_eq!(info.kind, "reject");
        assert_eq!(info.priority, DEFAULT_RULE_PRIORITY);
    }

    #[test]
    fn hook_info_from_webhook_basic() {
        let webhook = OutboundWebhookConfig {
            name: "notify".to_string(),
            points: vec![HookPoint::BeforeOutbound],
            url: "https://example.com/hook".to_string(),
            headers: Default::default(),
            timeout_ms: None,
            priority: Some(200),
            max_in_flight: None,
        };

        let info = hook_info_from_webhook("plugin.tool:logger", &webhook);
        assert_eq!(info.name, "plugin.tool:logger::notify");
        assert_eq!(info.kind, "webhook");
        assert_eq!(info.priority, 200);
    }

    #[tokio::test]
    async fn discover_plugin_hooks_flat_layout() {
        let dir = tempfile::tempdir().expect("create temp dir");

        // Create a sidecar capabilities file with hooks (flat layout)
        let caps = serde_json::json!({
            "hooks": {
                "rules": [
                    {
                        "name": "redact-keys",
                        "points": ["beforeOutbound"],
                        "replacements": [
                            {"pattern": "sk-[a-zA-Z0-9]+", "replacement": "[REDACTED]"}
                        ]
                    }
                ],
                "outbound_webhooks": [
                    {
                        "name": "log-events",
                        "points": ["beforeInbound"],
                        "url": "https://example.com/events"
                    }
                ]
            }
        });
        let mut f =
            std::fs::File::create(dir.path().join("slack.capabilities.json")).expect("create file");
        f.write_all(serde_json::to_string(&caps).unwrap().as_bytes())
            .expect("write");

        // Also create a .wasm file (not required for discovery, but realistic)
        std::fs::File::create(dir.path().join("slack.wasm")).expect("create wasm");

        // A capabilities file without hooks should be skipped
        let no_hooks = serde_json::json!({"http": {"allowlist": []}});
        let mut f2 = std::fs::File::create(dir.path().join("github.capabilities.json"))
            .expect("create file");
        f2.write_all(serde_json::to_string(&no_hooks).unwrap().as_bytes())
            .expect("write");

        let mut hooks = Vec::new();
        collect_plugin_hooks(&mut hooks, dir.path(), "tool").await;

        assert_eq!(hooks.len(), 2, "should find 1 rule + 1 webhook");
        assert_eq!(hooks[0].name, "plugin.tool:slack::redact-keys");
        assert_eq!(hooks[0].kind, "rule");
        assert_eq!(hooks[1].name, "plugin.tool:slack::log-events");
        assert_eq!(hooks[1].kind, "webhook");
    }

    #[tokio::test]
    async fn discover_plugin_hooks_nested_capabilities() {
        let dir = tempfile::tempdir().expect("create temp dir");

        // Channel-style capabilities with hooks nested under "capabilities"
        let caps = serde_json::json!({
            "type": "channel",
            "capabilities": {
                "hooks": {
                    "rules": [
                        {
                            "name": "filter-spam",
                            "points": ["beforeInbound"],
                            "when_regex": "buy now",
                            "reject_reason": "spam detected"
                        }
                    ]
                }
            }
        });
        let mut f = std::fs::File::create(dir.path().join("telegram.capabilities.json"))
            .expect("create file");
        f.write_all(serde_json::to_string(&caps).unwrap().as_bytes())
            .expect("write");

        let mut hooks = Vec::new();
        collect_plugin_hooks(&mut hooks, dir.path(), "channel").await;

        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].name, "plugin.channel:telegram::filter-spam");
        assert_eq!(hooks[0].kind, "reject");
        assert_eq!(hooks[0].source, "plugin.channel:telegram");
    }

    #[tokio::test]
    async fn discover_plugin_hooks_empty_dir() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let mut hooks = Vec::new();
        collect_plugin_hooks(&mut hooks, dir.path(), "tool").await;
        assert!(hooks.is_empty());
    }

    #[tokio::test]
    async fn discover_plugin_hooks_nonexistent_dir() {
        let mut hooks = Vec::new();
        collect_plugin_hooks(&mut hooks, Path::new("/nonexistent/path"), "tool").await;
        assert!(hooks.is_empty());
    }

    #[tokio::test]
    async fn discover_plugin_hooks_skips_subdirectories() {
        let dir = tempfile::tempdir().expect("create temp dir");

        // Create a subdirectory with capabilities.json inside (old broken layout)
        // This should NOT be discovered — only flat sidecar files are valid.
        let sub = dir.path().join("my_tool");
        std::fs::create_dir_all(&sub).expect("create subdir");
        let caps =
            serde_json::json!({"hooks": {"rules": [{"name": "x", "points": ["beforeInbound"]}]}});
        let mut f = std::fs::File::create(sub.join("capabilities.json")).expect("create file");
        f.write_all(serde_json::to_string(&caps).unwrap().as_bytes())
            .expect("write");

        let mut hooks = Vec::new();
        collect_plugin_hooks(&mut hooks, dir.path(), "tool").await;

        // The subdirectory layout should be ignored
        assert!(
            hooks.is_empty(),
            "subdirectory capabilities.json should not be discovered"
        );
    }
}
