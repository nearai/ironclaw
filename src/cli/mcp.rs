//! MCP server management CLI commands.
//!
//! Commands for adding, removing, authenticating, and testing MCP servers.

use std::io::Write;
use std::sync::Arc;

use clap::Subcommand;

use crate::config::Config;
use crate::db::Database;
#[cfg(feature = "postgres")]
use crate::secrets::PostgresSecretsStore;
use crate::secrets::{SecretsCrypto, SecretsStore};
use crate::tools::mcp::{
    McpClient, McpServerConfig, McpSessionManager, OAuthConfig,
    auth::{authorize_mcp_server, is_authenticated},
    config::{self, McpServersFile},
};

#[derive(Subcommand, Debug, Clone)]
pub enum McpCommand {
    /// Add an MCP server
    Add {
        /// Server name (e.g., "notion", "github")
        name: String,

        /// Server URL (e.g., "https://mcp.notion.com")
        url: String,

        /// OAuth client ID (if authentication is required)
        #[arg(long)]
        client_id: Option<String>,

        /// OAuth authorization URL (optional, can be discovered)
        #[arg(long)]
        auth_url: Option<String>,

        /// OAuth token URL (optional, can be discovered)
        #[arg(long)]
        token_url: Option<String>,

        /// Scopes to request (comma-separated)
        #[arg(long)]
        scopes: Option<String>,

        /// Server description
        #[arg(long)]
        description: Option<String>,
    },

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

/// Run an MCP command.
pub async fn run_mcp_command(cmd: McpCommand) -> anyhow::Result<()> {
    match cmd {
        McpCommand::Add {
            name,
            url,
            client_id,
            auth_url,
            token_url,
            scopes,
            description,
        } => {
            add_server(
                name,
                url,
                client_id,
                auth_url,
                token_url,
                scopes,
                description,
            )
            .await
        }
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
async fn add_server(
    name: String,
    url: String,
    client_id: Option<String>,
    auth_url: Option<String>,
    token_url: Option<String>,
    scopes: Option<String>,
    description: Option<String>,
) -> anyhow::Result<()> {
    let mut config = McpServerConfig::new(&name, &url);

    if let Some(desc) = description {
        config = config.with_description(desc);
    }

    // Track if auth is required
    let requires_auth = client_id.is_some();

    // Set up OAuth if client_id is provided
    if let Some(client_id) = client_id {
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
    println!("    URL: {}", url);

    if requires_auth {
        println!();
        println!("  Run 'clawyer mcp auth {}' to authenticate.", name);
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
        println!("    clawyer mcp add <name> <url> [--client-id <id>]");
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

        if verbose {
            println!("  {} {}{}", status, server.name, auth_status);
            println!("      URL: {}", server.url);
            if let Some(ref desc) = server.description {
                println!("      Description: {}", desc);
            }
            if let Some(ref oauth) = server.oauth {
                println!("      OAuth Client ID: {}", oauth.client_id);
                if !oauth.scopes.is_empty() {
                    println!("      Scopes: {}", oauth.scopes.join(", "));
                }
            }
            println!();
        } else {
            println!(
                "  {} {} - {}{}",
                status, server.name, server.url, auth_status
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
            println!("    clawyer mcp remove {}", name);
            println!(
                "    clawyer mcp add {} {} --client-id YOUR_CLIENT_ID",
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
        McpClient::new_authenticated(server.clone(), session_manager, secrets, user_id)
    } else if server.requires_auth() {
        // OAuth configured but no tokens - need to authenticate
        println!();
        println!(
            "  ✗ Not authenticated. Run 'clawyer mcp auth {}' first.",
            name
        );
        println!();
        return Ok(());
    } else {
        // No OAuth and no tokens - try unauthenticated
        McpClient::new_with_name(&server.name, &server.url)
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
                    println!("    clawyer mcp auth {}", name);
                } else {
                    // No tokens - server requires auth
                    println!("  ✗ Server requires authentication.");
                    println!();
                    println!("  Run 'clawyer mcp auth {}' to authenticate.", name);
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
    let config = Config::from_env().await?;

    let master_key = config.secrets.master_key().ok_or_else(|| {
        anyhow::anyhow!("SECRETS_MASTER_KEY not set. Run 'clawyer onboard' first or set it in .env")
    })?;

    let crypto = SecretsCrypto::new(master_key.clone())?;

    #[cfg(feature = "postgres")]
    {
        let store = crate::history::Store::new(&config.database).await?;
        store.run_migrations().await?;
        Ok(Arc::new(PostgresSecretsStore::new(
            store.pool(),
            Arc::new(crypto),
        )))
    }

    #[cfg(all(feature = "libsql", not(feature = "postgres")))]
    {
        use crate::db::Database as _;
        use crate::db::libsql::LibSqlBackend;
        use secrecy::ExposeSecret as _;

        let default_path = crate::config::default_libsql_path();
        let db_path = config
            .database
            .libsql_path
            .as_deref()
            .unwrap_or(&default_path);

        let backend = if let Some(ref url) = config.database.libsql_url {
            let token = config.database.libsql_auth_token.as_ref().ok_or_else(|| {
                anyhow::anyhow!("LIBSQL_AUTH_TOKEN is required when LIBSQL_URL is set")
            })?;
            LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?
        } else {
            LibSqlBackend::new_local(db_path)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?
        };
        backend
            .run_migrations()
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        return Ok(Arc::new(crate::secrets::LibSqlSecretsStore::new(
            backend.shared_db(),
            Arc::new(crypto),
        )));
    }

    #[cfg(not(any(feature = "postgres", feature = "libsql")))]
    {
        let _ = crypto;
        anyhow::bail!(
            "No database backend available for secrets. Enable 'postgres' or 'libsql' feature."
        );
    }
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
}
