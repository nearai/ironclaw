//! MCP server management CLI commands.
//!
//! Commands for adding, removing, authenticating, and testing MCP servers.

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use clap::{Args, Subcommand};

use crate::config::{Config, LlmConfig};
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

    if config::is_nearai_companion_server_name(&name) {
        anyhow::bail!(
            "Server name '{}' is reserved for the NEAR AI companion MCP server",
            name
        );
    }

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
    let mut servers = load_persisted_servers(db.as_deref()).await?;
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
    if config::is_nearai_companion_server_name(&name) {
        anyhow::bail!(
            "Server '{}' is derived from the active NEAR AI provider and cannot be removed directly",
            name
        );
    }

    let db = connect_db().await;
    let mut servers = load_persisted_servers(db.as_deref()).await?;
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
    let servers = load_servers_with_derived(db.as_deref()).await?;

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
    let servers = load_servers_with_derived(db.as_deref()).await?;
    let server = servers
        .get(&name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", name))?;

    if server.uses_runtime_auth_source() {
        println!();
        println!(
            "  Server '{}' reuses your active NEAR AI authentication and does not support separate MCP OAuth.",
            name
        );
        println!("  Configure NEAR AI auth (API key or session login) instead.");
        println!();
        return Ok(());
    }

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
    let servers = load_servers_with_derived(db.as_deref()).await?;
    let server = servers
        .get(&name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", name))?;

    println!();
    println!("  Testing connection to '{}'...", name);

    // Create client
    let session_manager = Arc::new(McpSessionManager::new());
    let (client, has_tokens) = if server.uses_runtime_auth_source() {
        let process_manager = Arc::new(McpProcessManager::new());
        let llm = resolve_llm_for_cli(as_settings_store(db.as_deref())).await?;
        let nearai_session = crate::llm::create_session_manager(llm.session.clone()).await;
        (
            create_client_from_config(
                server.clone(),
                &session_manager,
                Some(nearai_session),
                llm.nearai.api_key.clone(),
                &process_manager,
                None,
                "default",
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?,
            false,
        )
    } else {
        // Only initialize the secrets store for non-runtime-auth servers that
        // can actually use persisted OAuth/DCR tokens.
        let secrets = get_secrets_store().await?;
        let has_tokens = is_authenticated(&server, &secrets, &user_id).await;

        if has_tokens {
            (
                McpClient::new_authenticated(
                    server.clone(),
                    session_manager.clone(),
                    secrets,
                    user_id,
                ),
                true,
            )
        } else if server.requires_auth() {
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
            (
                create_client_from_config(
                    server.clone(),
                    &session_manager,
                    None,
                    None,
                    &process_manager,
                    None,
                    "default",
                )
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?,
                false,
            )
        }
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
    if config::is_nearai_companion_server_name(&name) {
        anyhow::bail!(
            "Server '{}' is derived from the active NEAR AI provider and cannot be toggled directly",
            name
        );
    }

    let db = connect_db().await;
    let mut servers = load_persisted_servers(db.as_deref()).await?;

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

/// Load only persisted MCP servers (DB if available, else disk).
async fn load_persisted_servers(
    db: Option<&dyn Database>,
) -> Result<McpServersFile, config::ConfigError> {
    Ok(if let Some(db) = db {
        config::load_mcp_servers_from_db(db, DEFAULT_USER_ID).await?
    } else {
        config::load_mcp_servers().await?
    })
}

/// Load MCP servers plus any derived runtime companions.
async fn load_servers_with_derived(
    db: Option<&dyn Database>,
) -> Result<McpServersFile, config::ConfigError> {
    let mut servers = load_persisted_servers(db).await?;

    if let Ok(llm) = resolve_llm_for_cli(as_settings_store(db)).await
        && let Some(companion) = config::derive_nearai_companion_mcp_server_from_llm(&llm)
    {
        servers.insert_if_absent(companion);
    }

    Ok(servers)
}

/// Save MCP servers (DB if available, else disk).
async fn save_servers(
    db: Option<&dyn Database>,
    servers: &McpServersFile,
) -> Result<(), config::ConfigError> {
    let mut persisted = servers.clone();
    persisted
        .servers
        .retain(|server| !config::is_nearai_companion_server_name(&server.name));

    if let Some(db) = db {
        config::save_mcp_servers_to_db(db, DEFAULT_USER_ID, &persisted).await
    } else {
        config::save_mcp_servers(&persisted).await
    }
}

/// Initialize and return the secrets store.
async fn get_secrets_store() -> anyhow::Result<Arc<dyn SecretsStore + Send + Sync>> {
    crate::cli::init_secrets_store().await
}

fn as_settings_store(
    db: Option<&dyn Database>,
) -> Option<&(dyn crate::db::SettingsStore + Sync)> {
    db.map(|db| db as &(dyn crate::db::SettingsStore + Sync))
}

async fn resolve_llm_for_cli(
    store: Option<&(dyn crate::db::SettingsStore + Sync)>,
) -> Result<LlmConfig, crate::error::ConfigError> {
    resolve_llm_for_cli_with_toml(store, None).await
}

async fn resolve_llm_for_cli_with_toml(
    store: Option<&(dyn crate::db::SettingsStore + Sync)>,
    toml_path: Option<&std::path::Path>,
) -> Result<LlmConfig, crate::error::ConfigError> {
    if let Some(store) = store {
        let _ = dotenvy::dotenv();
        crate::bootstrap::load_ironclaw_env();

        let mut settings = match store.get_all_settings(DEFAULT_USER_ID).await {
            Ok(map) => crate::settings::Settings::from_db_map(&map),
            Err(e) => {
                tracing::warn!(
                    "Failed to load CLI settings from DB, falling back to defaults before env/TOML resolution: {}",
                    e
                );
                crate::settings::Settings::default()
            }
        };

        apply_cli_toml_overlay(&mut settings, toml_path)?;
        return LlmConfig::resolve(&settings);
    }

    let settings = crate::config::load_bootstrap_settings(toml_path)?;
    LlmConfig::resolve(&settings)
}

fn apply_cli_toml_overlay(
    settings: &mut crate::settings::Settings,
    explicit_path: Option<&std::path::Path>,
) -> Result<(), crate::error::ConfigError> {
    let path = explicit_path
        .map(std::path::PathBuf::from)
        .unwrap_or_else(crate::settings::Settings::default_toml_path);

    match crate::settings::Settings::load_toml(&path) {
        Ok(Some(toml_settings)) => {
            settings.merge_from(&toml_settings);
        }
        Ok(None) => {
            if explicit_path.is_some() {
                return Err(crate::error::ConfigError::ParseError(format!(
                    "Config file not found: {}",
                    path.display()
                )));
            }
        }
        Err(e) => {
            return Err(crate::error::ConfigError::ParseError(e));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use async_trait::async_trait;

    use crate::error::DatabaseError;
    use crate::history::SettingRow;
    #[cfg(feature = "libsql")]
    use tempfile::NamedTempFile;

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

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_resolve_llm_for_cli_uses_db_backed_selected_model() {
        struct MockSettingsStore {
            settings: HashMap<String, serde_json::Value>,
        }

        #[async_trait]
        impl crate::db::SettingsStore for MockSettingsStore {
            async fn get_setting(
                &self,
                _user_id: &str,
                key: &str,
            ) -> Result<Option<serde_json::Value>, DatabaseError> {
                Ok(self.settings.get(key).cloned())
            }

            async fn get_setting_full(
                &self,
                _user_id: &str,
                _key: &str,
            ) -> Result<Option<SettingRow>, DatabaseError> {
                Ok(None)
            }

            async fn set_setting(
                &self,
                _user_id: &str,
                _key: &str,
                _value: &serde_json::Value,
            ) -> Result<(), DatabaseError> {
                Err(DatabaseError::Query("unused in test".to_string()))
            }

            async fn delete_setting(
                &self,
                _user_id: &str,
                _key: &str,
            ) -> Result<bool, DatabaseError> {
                Err(DatabaseError::Query("unused in test".to_string()))
            }

            async fn list_settings(&self, _user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
                Ok(Vec::new())
            }

            async fn get_all_settings(
                &self,
                _user_id: &str,
            ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
                Ok(self.settings.clone())
            }

            async fn set_all_settings(
                &self,
                _user_id: &str,
                _settings: &HashMap<String, serde_json::Value>,
            ) -> Result<(), DatabaseError> {
                Err(DatabaseError::Query("unused in test".to_string()))
            }

            async fn has_settings(&self, _user_id: &str) -> Result<bool, DatabaseError> {
                Ok(!self.settings.is_empty())
            }
        }

        struct EnvGuard(&'static str, Option<String>);

        impl Drop for EnvGuard {
            fn drop(&mut self) {
                // SAFETY: Protected by ENV_MUTEX for the duration of the test.
                unsafe {
                    match &self.1 {
                        Some(value) => std::env::set_var(self.0, value),
                        None => std::env::remove_var(self.0),
                    }
                }
            }
        }

        let _mutex = crate::config::helpers::ENV_MUTEX.lock().expect("env mutex");
        let prev_backend = std::env::var("LLM_BACKEND").ok();
        let prev_base_url = std::env::var("NEARAI_BASE_URL").ok();
        let prev_auth_url = std::env::var("NEARAI_AUTH_URL").ok();
        let prev_model = std::env::var("NEARAI_MODEL").ok();

        // SAFETY: Protected by ENV_MUTEX for the duration of the test.
        unsafe {
            std::env::set_var("LLM_BACKEND", "");
            std::env::set_var("NEARAI_BASE_URL", "http://127.0.0.1:11434/v1");
            std::env::set_var("NEARAI_AUTH_URL", "http://127.0.0.1:11435");
            std::env::set_var("NEARAI_MODEL", "");
        }

        let _backend_guard = EnvGuard("LLM_BACKEND", prev_backend);
        let _base_url_guard = EnvGuard("NEARAI_BASE_URL", prev_base_url);
        let _auth_url_guard = EnvGuard("NEARAI_AUTH_URL", prev_auth_url);
        let _model_guard = EnvGuard("NEARAI_MODEL", prev_model);

        let empty_toml = NamedTempFile::new().expect("temp toml");
        let store = MockSettingsStore {
            settings: HashMap::from([
                ("llm_backend".to_string(), serde_json::json!("nearai")),
                (
                    "selected_model".to_string(),
                    serde_json::json!("db-backed-nearai-model"),
                ),
            ]),
        };

        let llm = resolve_llm_for_cli_with_toml(Some(&store), Some(empty_toml.path()))
            .await
            .expect("resolve llm");
        assert_eq!(llm.backend, "nearai");
        assert_eq!(llm.nearai.model, "db-backed-nearai-model");

        let companion = config::derive_nearai_companion_mcp_server_from_llm(&llm)
            .expect("derived companion");
        assert_eq!(companion.url, "http://127.0.0.1:11434/mcp");
    }
}
