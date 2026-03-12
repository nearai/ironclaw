//! MCP server management CLI commands.
//!
//! Commands for adding, removing, authenticating, and testing MCP servers.

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use clap::{Args, Subcommand};

use crate::config::Config;
use crate::db::Database;
use crate::secrets::SecretsStore;
use crate::tools::mcp::{
    McpClient, McpProcessManager, McpServerConfig, McpSessionManager, OAuthConfig,
    auth::{authorize_mcp_server, is_authenticated},
    config::{self, EffectiveTransport, McpServersFile},
    factory::create_client_from_config,
};

/// Arguments for the `mcp add` subcommand.
#[derive(Args, Debug, Clone)]
pub struct McpAddArgs {
    /// Server name (e.g., "notion", "github")
    pub name: String,

    /// Server URL (e.g., "https://mcp.notion.com") -- required for http transport
    pub url: Option<String>,

    /// Transport type: http (default), stdio, unix
    #[arg(long, default_value = "http")]
    pub transport: String,

    /// Command to run (stdio transport)
    #[arg(long)]
    pub command: Option<String>,

    /// Command arguments (stdio transport, can be repeated)
    #[arg(long = "arg", num_args = 1..)]
    pub cmd_args: Vec<String>,

    /// Environment variables (stdio transport, KEY=VALUE format, can be repeated)
    #[arg(long = "env", value_parser = parse_env_var)]
    pub env: Vec<(String, String)>,

    /// Unix socket path (unix transport)
    #[arg(long)]
    pub socket: Option<String>,

    /// Custom HTTP headers (KEY:VALUE format, can be repeated)
    #[arg(long = "header", value_parser = parse_header)]
    pub headers: Vec<(String, String)>,

    /// OAuth client ID (if authentication is required)
    #[arg(long)]
    pub client_id: Option<String>,

    /// OAuth authorization URL (optional, can be discovered)
    #[arg(long)]
    pub auth_url: Option<String>,

    /// OAuth token URL (optional, can be discovered)
    #[arg(long)]
    pub token_url: Option<String>,

    /// Scopes to request (comma-separated)
    #[arg(long)]
    pub scopes: Option<String>,

    /// Server description
    #[arg(long)]
    pub description: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum McpCommand {
    /// Add an MCP server
    Add(Box<McpAddArgs>),

    /// Remove an MCP server
    Remove {
        /// Server name to remove
        name: String,
    },

    /// List configured MCP servers
    List {
        /// Show detailed information
        #[arg(short, long)]
        verbose: bool,
    },

    /// Authenticate with an MCP server (OAuth flow)
    Auth {
        /// Server name to authenticate
        name: String,

        /// User ID for storing the token (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
    },

    /// Test connection to an MCP server
    Test {
        /// Server name to test
        name: String,

        /// User ID for authentication (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
    },

    /// Enable or disable an MCP server
    Toggle {
        /// Server name
        name: String,

        /// Enable the server
        #[arg(long, conflicts_with = "disable")]
        enable: bool,

        /// Disable the server
        #[arg(long, conflicts_with = "enable")]
        disable: bool,
    },
}

fn parse_header(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find(':')
        .ok_or_else(|| format!("invalid header format '{}', expected KEY:VALUE", s))?;
    Ok((s[..pos].trim().to_string(), s[pos + 1..].trim().to_string()))
}

fn parse_env_var(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid env var format '{}', expected KEY=VALUE", s))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

/// Run an MCP command.
pub async fn run_mcp_command(cmd: McpCommand) -> anyhow::Result<()> {
    match cmd {
        McpCommand::Add(args) => add_server(*args).await,
        McpCommand::Remove { name } => remove_server(name).await,
        McpCommand::List { verbose } => list_servers(verbose).await,
        McpCommand::Auth { name, user } => auth_server(name, user).await,
        McpCommand::Test { name, user } => test_server(name, user).await,
        McpCommand::Toggle {
            name,
            enable,
            disable,
        } => toggle_server(name, enable, disable).await,
    }
}

/// Add a new MCP server.
async fn add_server(args: McpAddArgs) -> anyhow::Result<()> {
    let McpAddArgs {
        name,
        url,
        transport,
        command,
        cmd_args,
        env,
        socket,
        headers,
        client_id,
        auth_url,
        token_url,
        scopes,
        description,
    } = args;

    let transport_lower = transport.to_lowercase();

    let mut config = match transport_lower.as_str() {
        "stdio" => {
            let cmd = command
                .clone()
                .ok_or_else(|| anyhow::anyhow!("--command is required for stdio transport"))?;
            let env_map: HashMap<String, String> = env.into_iter().collect();
            McpServerConfig::new_stdio(&name, &cmd, cmd_args.clone(), env_map)
        }
        "unix" => {
            let socket_path = socket
                .clone()
                .ok_or_else(|| anyhow::anyhow!("--socket is required for unix transport"))?;
            McpServerConfig::new_unix(&name, &socket_path)
        }
        "http" => {
            let url_val = url
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("URL is required for http transport"))?;
            McpServerConfig::new(&name, url_val)
        }
        other => {
            anyhow::bail!(
                "Unknown transport type '{}'. Supported: http, stdio, unix",
                other
            );
        }
    };

    // Apply headers if any
    if !headers.is_empty() {
        let headers_map: HashMap<String, String> = headers.into_iter().collect();
        config = config.with_headers(headers_map);
    }

    if let Some(desc) = description {
        config = config.with_description(desc);
    }

    // Track if auth is required
    let requires_auth = client_id.is_some();

    // Set up OAuth if client_id is provided (HTTP transport only)
    if let Some(client_id) = client_id {
        if transport_lower != "http" {
            anyhow::bail!("OAuth authentication is only supported with http transport");
        }

        let mut oauth = OAuthConfig::new(client_id);

        if let (Some(auth), Some(token)) = (auth_url, token_url) {
            oauth = oauth.with_endpoints(auth, token);
        }

        if let Some(scopes_str) = scopes {
            let scope_list: Vec<String> = scopes_str
                .split(',')
                .map(|s| s.trim().to_string())
                .collect();
            oauth = oauth.with_scopes(scope_list);
        }

        config = config.with_oauth(oauth);
    }

    // Validate
    config.validate()?;

    // Save (DB if available, else disk)
    let db = connect_db().await;
    let mut servers = load_servers(db.as_deref()).await?;
    servers.upsert(config);
    save_servers(db.as_deref(), &servers).await?;

    println!();
    println!("  ✓ Added MCP server '{}'", name);

    match transport_lower.as_str() {
        "stdio" => {
            println!(
                "    Transport: stdio (command: {})",
                command.as_deref().unwrap_or("")
            );
        }
        "unix" => {
            println!(
                "    Transport: unix (socket: {})",
                socket.as_deref().unwrap_or("")
            );
        }
        _ => {
            println!("    URL: {}", url.as_deref().unwrap_or(""));
        }
    }

    if requires_auth {
        println!();
        println!("  Run 'ironclaw mcp auth {}' to authenticate.", name);
    }

    println!();

    Ok(())
}

/// Remove an MCP server.
async fn remove_server(name: String) -> anyhow::Result<()> {
    let db = connect_db().await;
    let mut servers = load_servers(db.as_deref()).await?;
    if !servers.remove(&name) {
        anyhow::bail!("Server '{}' not found", name);
    }
    save_servers(db.as_deref(), &servers).await?;

    println!();
    println!("  ✓ Removed MCP server '{}'", name);
    println!();

    Ok(())
}

/// List configured MCP servers.
async fn list_servers(verbose: bool) -> anyhow::Result<()> {
    let db = connect_db().await;
    let servers = load_servers(db.as_deref()).await?;

    if servers.servers.is_empty() {
        println!();
        println!("  No MCP servers configured.");
        println!();
        println!("  Add a server with:");
        println!("    ironclaw mcp add <name> <url> [--client-id <id>]");
        println!();
        return Ok(());
    }

    println!();
    println!("  Configured MCP servers:");
    println!();

    for server in &servers.servers {
        let status = if server.enabled { "●" } else { "○" };
        let auth_status = if server.requires_auth() {
            " (auth required)"
        } else {
            ""
        };

        let effective = server.effective_transport();

        let transport_label = match &effective {
            EffectiveTransport::Http => "http".to_string(),
            EffectiveTransport::Stdio { command, .. } => {
                format!("stdio ({})", command)
            }
            EffectiveTransport::Unix { socket_path } => {
                format!("unix ({})", socket_path)
            }
        };

        if verbose {
            println!("  {} {}{}", status, server.name, auth_status);
            println!("      Transport: {}", transport_label);
            match &effective {
                EffectiveTransport::Http => {
                    println!("      URL: {}", server.url);
                }
                EffectiveTransport::Stdio { command, args, env } => {
                    println!("      Command: {}", command);
                    if !args.is_empty() {
                        println!("      Args: {}", args.join(", "));
                    }
                    if !env.is_empty() {
                        // Only print env var names, not values (may contain secrets).
                        let env_keys: Vec<&str> = env.keys().map(|k| k.as_str()).collect();
                        println!("      Env: {}", env_keys.join(", "));
                    }
                }
                EffectiveTransport::Unix { socket_path } => {
                    println!("      Socket: {}", socket_path);
                }
            }
            if let Some(ref desc) = server.description {
                println!("      Description: {}", desc);
            }
            if let Some(ref oauth) = server.oauth {
                println!("      OAuth Client ID: {}", oauth.client_id);
                if !oauth.scopes.is_empty() {
                    println!("      Scopes: {}", oauth.scopes.join(", "));
                }
            }
            if !server.headers.is_empty() {
                let header_keys: Vec<&String> = server.headers.keys().collect();
                println!(
                    "      Headers: {}",
                    header_keys
                        .iter()
                        .map(|k| k.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            println!();
        } else {
            let display = match &effective {
                EffectiveTransport::Http => server.url.clone(),
                EffectiveTransport::Stdio { command, .. } => command.to_string(),
                EffectiveTransport::Unix { socket_path } => socket_path.to_string(),
            };
            println!(
                "  {} {} - {} [{}]{}",
                status, server.name, display, transport_label, auth_status
            );
        }
    }

    if !verbose {
        println!();
        println!("  Use --verbose for more details.");
    }

    println!();

    Ok(())
}

/// Authenticate with an MCP server.
async fn auth_server(name: String, user_id: String) -> anyhow::Result<()> {
    // Get server config
    let db = connect_db().await;
    let servers = load_servers(db.as_deref()).await?;
    let server = servers
        .get(&name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", name))?;

    // Initialize secrets store
    let secrets = get_secrets_store().await?;

    // Check if already authenticated
    if is_authenticated(&server, &secrets, &user_id).await {
        println!();
        println!("  Server '{}' is already authenticated.", name);
        println!();
        print!("  Re-authenticate? [y/N]: ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            return Ok(());
        }
        println!();
    }

    println!();
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!(
        "║  {:^62}║",
        format!("{} Authentication", name.to_uppercase())
    );
    println!("╚════════════════════════════════════════════════════════════════╝");
    println!();

    // Perform OAuth flow (supports both pre-configured OAuth and DCR)
    match authorize_mcp_server(&server, &secrets, &user_id).await {
        Ok(_token) => {
            println!();
            println!("  ✓ Successfully authenticated with '{}'!", name);
            println!();
            println!("  You can now use tools from this server.");
            println!();
        }
        Err(crate::tools::mcp::auth::AuthError::NotSupported) => {
            println!();
            println!("  ✗ Server does not support OAuth authentication.");
            println!();
            println!("  The server may require a different authentication method,");
            println!("  or you may need to configure OAuth manually:");
            println!();
            println!("    ironclaw mcp remove {}", name);
            println!(
                "    ironclaw mcp add {} {} --client-id YOUR_CLIENT_ID",
                name, server.url
            );
            println!();
        }
        Err(e) => {
            println!();
            println!("  ✗ Authentication failed: {}", e);
            println!();
            return Err(e.into());
        }
    }

    Ok(())
}

/// Test connection to an MCP server.
async fn test_server(name: String, user_id: String) -> anyhow::Result<()> {
    // Get server config
    let db = connect_db().await;
    let servers = load_servers(db.as_deref()).await?;
    let server = servers
        .get(&name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", name))?;

    println!();
    println!("  Testing connection to '{}'...", name);

    // Create client
    let session_manager = Arc::new(McpSessionManager::new());

    // Always check for stored tokens (from either pre-configured OAuth or DCR)
    let secrets = get_secrets_store().await?;
    let has_tokens = is_authenticated(&server, &secrets, &user_id).await;

    let client = if has_tokens {
        // We have stored tokens, use authenticated client
        McpClient::new_authenticated(server.clone(), session_manager.clone(), secrets, user_id)
    } else if server.requires_auth() {
        // OAuth configured but no tokens - need to authenticate
        println!();
        println!(
            "  ✗ Not authenticated. Run 'ironclaw mcp auth {}' first.",
            name
        );
        println!();
        return Ok(());
    } else {
        // Use the factory to dispatch on transport type (HTTP, stdio, unix)
        let process_manager = Arc::new(McpProcessManager::new());
        create_client_from_config(
            server.clone(),
            &session_manager,
            &process_manager,
            None,
            "default",
        )
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?
    };

    // Test connection
    match client.test_connection().await {
        Ok(()) => {
            println!("  ✓ Connection successful!");
            println!();

            // List tools
            match client.list_tools().await {
                Ok(tools) => {
                    println!("  Available tools ({}):", tools.len());
                    for tool in tools {
                        let approval = if tool.requires_approval() {
                            " [approval required]"
                        } else {
                            ""
                        };
                        println!("    • {}{}", tool.name, approval);
                        if !tool.description.is_empty() {
                            // Truncate long descriptions
                            let desc = if tool.description.len() > 60 {
                                format!("{}...", &tool.description[..57])
                            } else {
                                tool.description.clone()
                            };
                            println!("      {}", desc);
                        }
                    }
                }
                Err(e) => {
                    println!("  ✗ Failed to list tools: {}", e);
                }
            }
        }
        Err(e) => {
            let err_str = e.to_string();
            // Check if server requires auth but we don't have valid tokens
            if err_str.contains("401") || err_str.contains("requires authentication") {
                if has_tokens {
                    // We had tokens but they failed - need to re-authenticate
                    println!(
                        "  ✗ Authentication failed (token may be expired). Try re-authenticating:"
                    );
                    println!("    ironclaw mcp auth {}", name);
                } else {
                    // No tokens - server requires auth
                    println!("  ✗ Server requires authentication.");
                    println!();
                    println!("  Run 'ironclaw mcp auth {}' to authenticate.", name);
                }
            } else {
                println!("  ✗ Connection failed: {}", e);
            }
        }
    }

    println!();

    Ok(())
}

/// Toggle server enabled/disabled state.
async fn toggle_server(name: String, enable: bool, disable: bool) -> anyhow::Result<()> {
    let db = connect_db().await;
    let mut servers = load_servers(db.as_deref()).await?;

    let server = servers
        .get_mut(&name)
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", name))?;

    let new_state = if enable {
        true
    } else if disable {
        false
    } else {
        !server.enabled // Toggle if neither specified
    };

    server.enabled = new_state;
    save_servers(db.as_deref(), &servers).await?;

    let status = if new_state { "enabled" } else { "disabled" };
    println!();
    println!("  ✓ Server '{}' is now {}.", name, status);
    println!();

    Ok(())
}

const DEFAULT_USER_ID: &str = "default";

/// Try to connect to the database (backend-agnostic).
async fn connect_db() -> Option<Arc<dyn Database>> {
    let config = Config::from_env().await.ok()?;
    crate::db::connect_from_config(&config.database).await.ok()
}

/// Load MCP servers (DB if available, else disk).
async fn load_servers(db: Option<&dyn Database>) -> Result<McpServersFile, config::ConfigError> {
    if let Some(db) = db {
        config::load_mcp_servers_from_db(db, DEFAULT_USER_ID).await
    } else {
        config::load_mcp_servers().await
    }
}

/// Save MCP servers (DB if available, else disk).
async fn save_servers(
    db: Option<&dyn Database>,
    servers: &McpServersFile,
) -> Result<(), config::ConfigError> {
    if let Some(db) = db {
        config::save_mcp_servers_to_db(db, DEFAULT_USER_ID, servers).await
    } else {
        config::save_mcp_servers(servers).await
    }
}

/// Initialize and return the secrets store.
async fn get_secrets_store() -> anyhow::Result<Arc<dyn SecretsStore + Send + Sync>> {
    crate::cli::init_secrets_store().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_command_parsing() {
        // Just verify the command structure is valid
        use clap::CommandFactory;

        // Create a dummy parent command to test subcommand parsing
        #[derive(clap::Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: McpCommand,
        }

        TestCli::command().debug_assert();
    }

    #[test]
    fn test_parse_header_valid() {
        let result = parse_header("Authorization: Bearer token123").unwrap();
        assert_eq!(result.0, "Authorization");
        assert_eq!(result.1, "Bearer token123");
    }

    #[test]
    fn test_parse_header_no_spaces() {
        let result = parse_header("X-Api-Key:abc123").unwrap();
        assert_eq!(result.0, "X-Api-Key");
        assert_eq!(result.1, "abc123");
    }

    #[test]
    fn test_parse_header_invalid() {
        let result = parse_header("no-colon-here");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid header format"));
    }

    #[test]
    fn test_parse_env_var_valid() {
        let result = parse_env_var("NODE_ENV=production").unwrap();
        assert_eq!(result.0, "NODE_ENV");
        assert_eq!(result.1, "production");
    }

    #[test]
    fn test_parse_env_var_with_equals_in_value() {
        let result = parse_env_var("KEY=value=with=equals").unwrap();
        assert_eq!(result.0, "KEY");
        assert_eq!(result.1, "value=with=equals");
    }

    #[test]
    fn test_parse_env_var_invalid() {
        let result = parse_env_var("no-equals-here");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid env var format"));
    }
}
