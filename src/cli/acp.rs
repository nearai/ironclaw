//! ACP agent management CLI commands.
//!
//! Commands for adding, removing, listing, toggling, and testing ACP agents.
//! Mirrors the MCP server management CLI (`src/cli/mcp.rs`).

use std::collections::HashMap;
use std::sync::Arc;

use clap::{Args, Subcommand};

use crate::config::acp::{self, AcpAgentConfig, AcpAgentsFile};
use crate::db::Database;

/// Arguments for the `acp add` subcommand.
#[derive(Args, Debug, Clone)]
pub struct AcpAddArgs {
    /// Agent name (e.g., "goose", "codex", "gemini")
    pub name: String,

    /// Command to spawn the agent
    #[arg(long)]
    pub command: String,

    /// Command arguments (can be repeated)
    #[arg(long = "arg", num_args = 1..)]
    pub args: Vec<String>,

    /// Environment variables (KEY=VALUE format, can be repeated)
    #[arg(long = "env", value_parser = parse_env_var)]
    pub env: Vec<(String, String)>,

    /// Agent description
    #[arg(long)]
    pub description: Option<String>,
}

fn parse_env_var(s: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = s.splitn(2, '=').collect();
    if parts.len() != 2 {
        return Err(format!("invalid env var format '{s}', expected KEY=VALUE"));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

#[derive(Subcommand, Debug, Clone)]
pub enum AcpCommand {
    /// Add an ACP agent
    Add(Box<AcpAddArgs>),

    /// Remove an ACP agent
    Remove {
        /// Agent name to remove
        name: String,
    },

    /// List configured ACP agents
    List,

    /// Enable or disable an ACP agent
    Toggle {
        /// Agent name to toggle
        name: String,
    },

    /// Test an ACP agent connection (spawn, handshake, report)
    Test {
        /// Agent name to test
        name: String,
    },
}

/// Run an ACP CLI command.
pub async fn run_acp_command(cmd: AcpCommand) -> anyhow::Result<()> {
    match cmd {
        AcpCommand::Add(args) => add_agent(*args).await,
        AcpCommand::Remove { name } => remove_agent(&name).await,
        AcpCommand::List => list_agents().await,
        AcpCommand::Toggle { name } => toggle_agent(&name).await,
        AcpCommand::Test { name } => test_agent(&name).await,
    }
}

async fn add_agent(args: AcpAddArgs) -> anyhow::Result<()> {
    let env: HashMap<String, String> = args.env.into_iter().collect();
    let mut config = AcpAgentConfig::new(&args.name, &args.command, args.args, env);
    if let Some(desc) = args.description {
        config = config.with_description(desc);
    }

    config.validate().map_err(|e| anyhow::anyhow!("{}", e))?;

    let storage = resolve_storage().await;
    let mut agents = load_agents(storage.db.as_deref(), &storage.owner_id).await?;
    let is_update = agents.get(&args.name).is_some();
    agents.upsert(config);
    save_agents(storage.db.as_deref(), &storage.owner_id, &agents).await?;

    if is_update {
        println!("Updated ACP agent '{}'", args.name);
    } else {
        println!("Added ACP agent '{}'", args.name);
    }
    Ok(())
}

async fn remove_agent(name: &str) -> anyhow::Result<()> {
    let storage = resolve_storage().await;
    let mut agents = load_agents(storage.db.as_deref(), &storage.owner_id).await?;

    if !agents.remove(name) {
        anyhow::bail!("ACP agent '{}' not found", name);
    }

    save_agents(storage.db.as_deref(), &storage.owner_id, &agents).await?;
    println!("Removed ACP agent '{}'", name);
    Ok(())
}

async fn list_agents() -> anyhow::Result<()> {
    let storage = resolve_storage().await;
    let agents = load_agents(storage.db.as_deref(), &storage.owner_id).await?;

    if agents.agents.is_empty() {
        println!("No ACP agents configured.");
        println!();
        println!("Add one with:");
        println!("  ironclaw acp add goose --command goose --arg \"--stdio\"");
        return Ok(());
    }

    println!("ACP Agents:");
    println!();
    for agent in &agents.agents {
        let icon = if agent.enabled { "●" } else { "○" };
        let status = if agent.enabled { "enabled" } else { "disabled" };
        println!("  {} {} ({}) [{}]", icon, agent.name, agent.command, status);
        if !agent.args.is_empty() {
            println!("    args: {}", agent.args.join(" "));
        }
        if !agent.env.is_empty() {
            println!("    env: {} variable(s)", agent.env.len());
        }
        if let Some(ref desc) = agent.description {
            println!("    {}", desc);
        }
    }
    Ok(())
}

async fn toggle_agent(name: &str) -> anyhow::Result<()> {
    let storage = resolve_storage().await;
    let mut agents = load_agents(storage.db.as_deref(), &storage.owner_id).await?;

    let agent = agents
        .get_mut(name)
        .ok_or_else(|| anyhow::anyhow!("ACP agent '{}' not found", name))?;

    agent.enabled = !agent.enabled;
    let new_state = if agent.enabled { "enabled" } else { "disabled" };

    save_agents(storage.db.as_deref(), &storage.owner_id, &agents).await?;
    println!("ACP agent '{}' is now {}", name, new_state);
    Ok(())
}

async fn test_agent(name: &str) -> anyhow::Result<()> {
    use agent_client_protocol::{self as acp, Agent as _};
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    use crate::worker::acp_bridge;
    use crate::worker::api::JobEventPayload;

    /// Event sink that prints agent output to stdout during `ironclaw acp test`.
    struct PrintEventSink;

    impl acp_bridge::AcpEventSink for PrintEventSink {
        async fn emit_event(&self, payload: &JobEventPayload) {
            match payload.event_type.as_str() {
                "message" => {
                    if let Some(content) = payload.data["content"].as_str() {
                        println!("    | {}", content);
                    }
                }
                "tool_use" => {
                    if let Some(tool) = payload.data["tool_name"].as_str() {
                        println!("    [tool: {}]", tool);
                    }
                }
                _ => {}
            }
        }
    }

    let storage = resolve_storage().await;
    let agent = crate::config::acp::get_enabled_acp_agent_for_user(
        storage.db.as_deref(),
        &storage.owner_id,
        name,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    println!();
    println!("  Testing ACP agent '{}'...", name);
    println!("  Command: {} {}", agent.command, agent.args.join(" "));

    // Spawn the agent subprocess
    let mut child = tokio::process::Command::new(&agent.command)
        .args(&agent.args)
        .envs(&agent.env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn '{}': {}", agent.command, e))?;

    let child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture agent stdin"))?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture agent stdout"))?;

    // Run ACP handshake inside a LocalSet (!Send futures).
    // Uses IronClawAcpClient with a PrintEventSink so the test exercises
    // the same permission auto-approval and event translation as real jobs.
    let local_set = tokio::task::LocalSet::new();
    let result = local_set
        .run_until(async move {
            let outgoing = child_stdin.compat_write();
            let incoming = child_stdout.compat();

            let client = acp_bridge::IronClawAcpClient::new(PrintEventSink);

            let (conn, handle_io) =
                acp::ClientSideConnection::new(client, outgoing, incoming, |fut| {
                    tokio::task::spawn_local(fut);
                });
            tokio::task::spawn_local(handle_io);

            let handshake = tokio::time::timeout(
                std::time::Duration::from_secs(15),
                conn.initialize(acp_bridge::ironclaw_init_request()),
            )
            .await;

            match handshake {
                Ok(Ok(resp)) => {
                    println!("  \u{2713} ACP handshake successful!");
                    println!();
                    println!("  Agent info:");
                    if let Some(ref info) = resp.agent_info {
                        println!("    Name: {}", info.name);
                        println!("    Version: {}", info.version);
                    }
                    println!("    Protocol: {}", resp.protocol_version);
                    Ok(())
                }
                Ok(Err(e)) => {
                    println!("  \u{2717} ACP handshake failed: {}", e);
                    Err(anyhow::anyhow!("handshake failed: {}", e))
                }
                Err(_) => {
                    println!("  \u{2717} ACP handshake timed out (15s)");
                    Err(anyhow::anyhow!("handshake timed out"))
                }
            }
        })
        .await;

    // Clean up child process
    let _ = child.kill().await;
    println!();
    result
}

// ==================== DB / disk persistence helpers ====================

struct AcpCliStorage {
    db: Option<Arc<dyn Database>>,
    owner_id: String,
}

async fn resolve_storage() -> AcpCliStorage {
    match crate::config::Config::from_env().await {
        Ok(config) => AcpCliStorage {
            db: crate::db::connect_from_config(&config.database)
                .await
                .ok()
                .map(|db| db as Arc<dyn Database>),
            owner_id: config.owner_id,
        },
        Err(_) => AcpCliStorage {
            db: None,
            owner_id: "default".to_string(),
        },
    }
}

async fn load_agents(
    db: Option<&dyn Database>,
    owner_id: &str,
) -> Result<AcpAgentsFile, anyhow::Error> {
    acp::load_acp_agents_for_user(db, owner_id)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))
}

async fn save_agents(
    db: Option<&dyn Database>,
    owner_id: &str,
    agents: &AcpAgentsFile,
) -> Result<(), anyhow::Error> {
    acp::save_acp_agents_for_user(db, owner_id, agents)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_env_var_valid() {
        let (k, v) = parse_env_var("FOO=bar").unwrap();
        assert_eq!(k, "FOO");
        assert_eq!(v, "bar");
    }

    #[test]
    fn test_parse_env_var_with_equals_in_value() {
        let (k, v) = parse_env_var("KEY=val=ue").unwrap();
        assert_eq!(k, "KEY");
        assert_eq!(v, "val=ue");
    }

    #[test]
    fn test_parse_env_var_invalid() {
        let result = parse_env_var("no-equals-sign");
        assert!(result.is_err());
    }

    #[test]
    fn test_acp_command_variants() {
        // Verify all variants exist (compile-time check)
        let _ = AcpCommand::List;
        let _ = AcpCommand::Remove { name: "x".into() };
        let _ = AcpCommand::Toggle { name: "x".into() };
        let _ = AcpCommand::Test { name: "x".into() };
    }
}
