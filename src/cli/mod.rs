//! CLI command handling.
//!
//! Provides subcommands for:
//! - Running the agent (`run`)
//! - Interactive onboarding wizard (`onboard`)
//! - Managing configuration (`config list`, `config get`, `config set`)
//! - Managing WASM tools (`tool install`, `tool list`, `tool remove`)
//! - Managing MCP servers (`mcp add`, `mcp auth`, `mcp list`, `mcp test`)
//! - Querying workspace memory (`memory search`, `memory read`, `memory write`)
//! - Managing routines (`routines list`, `routines create`, `routines edit`, ...)
//! - Managing OS service (`service install`, `service start`, `service stop`)
//! - Listing configured channels (`channels list`)
//! - Active health diagnostics (`doctor`)
//! - Viewing gateway logs (`logs`)
//! - Checking system health (`status`)

pub mod acp;
mod channels;
mod completion;
mod config;
mod doctor;
pub mod fmt;
mod hooks;
#[cfg(feature = "import")]
pub mod import;
mod logs;
mod mcp;
pub mod memory;
mod models;
pub mod oauth_defaults;
mod pairing;
mod registry;
mod routines;
mod service;
mod skills;
pub mod status;
mod tool;

pub use acp::{AcpCommand, run_acp_command};
pub use channels::{ChannelsCommand, run_channels_command};
pub use completion::Completion;
pub use config::{ConfigCommand, run_config_command};
pub use doctor::run_doctor_command;
pub use hooks::{HooksCommand, run_hooks_command};
#[cfg(feature = "import")]
pub use import::{ImportCommand, run_import_command};
pub use logs::{LogsCommand, run_logs_command};
pub use mcp::{McpCommand, run_mcp_command};
pub use memory::MemoryCommand;
pub use memory::run_memory_command_with_db;
pub use models::{ModelsCommand, run_models_command};
pub use pairing::{PairingCommand, run_pairing_command, run_pairing_command_with_store};
pub use registry::{RegistryCommand, run_registry_command};
pub use routines::{RoutinesCommand, run_routines_command};
pub use service::{ServiceCommand, run_service_command};
pub use skills::{SkillsCommand, run_skills_command};
pub use status::run_status_command;
pub use tool::{ToolCommand, run_tool_command};

use std::sync::Arc;

use clap::{ColorChoice, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "ironclaw")]
#[command(
    about = "Secure personal AI assistant that protects your data and expands its capabilities"
)]
#[command(
    long_about = "IronClaw is a secure AI assistant. Use 'ironclaw <subcommand> --help' for details.\nExamples:\n  ironclaw run  # Start the agent\n  ironclaw config list  # List configs"
)]
#[command(version)]
#[command(color = ColorChoice::Auto)] // Enable auto-color for help (if the terminal supports it)
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Run in interactive CLI mode only (disable other channels)
    #[arg(long, global = true)]
    pub cli_only: bool,

    /// Skip database connection (for testing)
    #[arg(long, global = true)]
    pub no_db: bool,

    /// Single message mode - send one message and exit
    #[arg(short, long, global = true)]
    pub message: Option<String>,

    /// Configuration file path (optional, uses env vars by default)
    #[arg(short, long, global = true)]
    pub config: Option<std::path::PathBuf>,

    /// Skip first-run onboarding check
    #[arg(long, global = true)]
    pub no_onboard: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the agent (default if no subcommand given)
    #[command(
        about = "Run the AI agent",
        long_about = "Starts the IronClaw agent in default mode.\nExample: ironclaw run"
    )]
    Run,

    /// Interactive onboarding wizard
    #[command(
        about = "Run interactive setup wizard",
        long_about = "Guides through initial configuration.\nExamples:\n  ironclaw onboard --skip-auth  # Skip auth step\n  ironclaw onboard --channels-only  # Reconfigure channels\n  ironclaw onboard --provider-only  # Change LLM provider and model"
    )]
    Onboard {
        /// Skip authentication (use existing session)
        #[arg(long)]
        skip_auth: bool,

        /// Reconfigure channels only
        #[arg(long, conflicts_with_all = ["provider_only", "quick", "step"], help = "Deprecated: use --step channels")]
        channels_only: bool,

        /// Reconfigure LLM provider and model only
        #[arg(long, conflicts_with_all = ["channels_only", "quick", "step"], help = "Deprecated: use --step provider")]
        provider_only: bool,

        /// Quick setup: auto-defaults everything except LLM provider and model
        #[arg(long, conflicts_with_all = ["channels_only", "provider_only", "step"])]
        quick: bool,

        /// Run only specific setup steps (comma-separated: provider, channels, model, database, security)
        #[arg(long, value_delimiter = ',', conflicts_with_all = ["channels_only", "provider_only", "quick"])]
        step: Vec<String>,
    },

    /// Manage configuration settings
    #[command(
        subcommand,
        about = "Manage app configs",
        long_about = "Commands for listing, getting, and setting configurations.\nExample: ironclaw config list"
    )]
    Config(ConfigCommand),

    /// Manage WASM tools
    #[command(
        subcommand,
        about = "Manage WASM tools",
        long_about = "Install, list, or remove WASM-based tools.\nExample: ironclaw tool install mytool.wasm"
    )]
    Tool(ToolCommand),

    /// Browse and install extensions from the registry
    #[command(
        subcommand,
        about = "Browse/install extensions",
        long_about = "Interact with extension registry.\nExample: ironclaw registry list"
    )]
    Registry(RegistryCommand),

    /// List and inspect messaging channels
    #[command(
        subcommand,
        about = "Manage channels",
        long_about = "List configured messaging channels.\nExamples:\n  ironclaw channels list\n  ironclaw channels list --verbose\n  ironclaw channels list --json"
    )]
    Channels(ChannelsCommand),

    /// Manage routines (scheduled, event-driven, webhook, manual)
    #[command(
        subcommand,
        alias = "cron",
        about = "Manage routines",
        long_about = "List, create, edit, enable/disable, delete, and view history of routines.\nExamples:\n  ironclaw routines list\n  ironclaw routines create --name daily-digest --schedule '0 0 9 * * *' --prompt 'Summarize today'"
    )]
    Routines(RoutinesCommand),

    /// Manage MCP servers (hosted tool providers)
    #[command(
        subcommand,
        about = "Manage MCP servers",
        long_about = "Add, auth, list, or test MCP servers.\nExample: ironclaw mcp add notion https://mcp.notion.com"
    )]
    Mcp(Box<McpCommand>),

    /// Query and manage workspace memory
    #[command(
        subcommand,
        about = "Manage workspace memory",
        long_about = "Search, read, or write to memory.\nExample: ironclaw memory search 'query'"
    )]
    Memory(MemoryCommand),

    /// DM pairing (approve inbound requests from unknown senders)
    #[command(
        subcommand,
        about = "Manage DM pairing",
        long_about = "Approve or manage pairing requests.\nExamples:\n  ironclaw pairing list telegram\n  ironclaw pairing approve telegram ABC12345"
    )]
    Pairing(PairingCommand),

    /// Manage OS service (launchd / systemd)
    #[command(
        subcommand,
        about = "Manage OS service",
        long_about = "Install, start, or stop service.\nExample: ironclaw service install"
    )]
    Service(ServiceCommand),

    /// Manage SKILL.md-based skills
    #[command(
        subcommand,
        about = "Manage skills",
        long_about = "List, search, and inspect SKILL.md-based skills.\nExamples:\n  ironclaw skills list\n  ironclaw skills search 'writing'\n  ironclaw skills info my-skill"
    )]
    Skills(SkillsCommand),

    /// Manage lifecycle hooks
    #[command(
        subcommand,
        about = "Manage lifecycle hooks",
        long_about = "List and inspect lifecycle hooks (bundled, plugin, workspace).\nExamples:\n  ironclaw hooks list\n  ironclaw hooks list --verbose\n  ironclaw hooks list --json"
    )]
    Hooks(HooksCommand),

    /// Manage LLM providers and models
    #[command(
        subcommand,
        about = "Manage LLM providers and models",
        long_about = "List providers, view current configuration, and set active provider/model.\nExamples:\n  ironclaw models list\n  ironclaw models list openai --verbose\n  ironclaw models status\n  ironclaw models set gpt-4o\n  ironclaw models set-provider anthropic --model claude-sonnet-4-6-20250514"
    )]
    Models(ModelsCommand),

    /// Probe external dependencies and validate configuration
    #[command(
        about = "Run diagnostics",
        long_about = "Checks dependencies and config validity.\nExample: ironclaw doctor"
    )]
    Doctor,

    /// View and manage gateway logs
    #[command(
        about = "View and manage gateway logs",
        long_about = "Tail gateway logs, stream live output, or adjust log level.\nExamples:\n  ironclaw logs                 # Show last 200 lines from gateway.log\n  ironclaw logs --follow        # Stream live logs via SSE\n  ironclaw logs --level         # Show current log level\n  ironclaw logs --level debug   # Set log level to debug"
    )]
    Logs(LogsCommand),

    /// Show system health and diagnostics
    #[command(
        about = "Show system status",
        long_about = "Displays health and diagnostics info.\nExample: ironclaw status"
    )]
    Status,

    /// Generate shell completion scripts
    #[command(
        about = "Generate completions",
        long_about = "Generates shell completion scripts.\nExample: ironclaw completion --shell bash > ironclaw.bash"
    )]
    Completion(Completion),

    /// Import data from other AI systems
    #[cfg(feature = "import")]
    #[command(
        subcommand,
        about = "Import from other AI systems",
        long_about = "Migrate data from other AI assistants like OpenClaw.\nExample: ironclaw import openclaw"
    )]
    Import(ImportCommand),

    /// Authenticate with a provider (re-login)
    #[command(
        about = "Authenticate with a provider",
        long_about = "Re-authenticate with an LLM provider.\nExample: ironclaw login --openai-codex"
    )]
    Login {
        /// Authenticate with OpenAI Codex (ChatGPT subscription)
        #[arg(long)]
        openai_codex: bool,
    },

    /// Run as a sandboxed worker inside a Docker container (internal use).
    /// This is invoked automatically by the orchestrator, not by users directly.
    #[command(hide = true)]
    Worker {
        /// Job ID to execute.
        #[arg(long)]
        job_id: uuid::Uuid,

        /// URL of the orchestrator's internal API.
        #[arg(long, default_value = "http://host.docker.internal:50051")]
        orchestrator_url: String,

        /// Maximum iterations before stopping.
        #[arg(long, env = "IRONCLAW_MAX_ITERATIONS", default_value = "50")]
        max_iterations: u32,
    },

    /// Manage ACP (Agent Client Protocol) agents
    #[command(
        subcommand,
        about = "Manage ACP agents",
        long_about = "Add, list, remove, or test ACP-compliant coding agents.\nExample: ironclaw acp add goose --command goose --arg \"--stdio\""
    )]
    Acp(AcpCommand),

    /// Run as a Claude Code bridge inside a Docker container (internal use).
    /// Spawns the `claude` CLI and streams output back to the orchestrator.
    #[command(hide = true)]
    ClaudeBridge {
        /// Job ID to execute.
        #[arg(long)]
        job_id: uuid::Uuid,

        /// URL of the orchestrator's internal API.
        #[arg(long, default_value = "http://host.docker.internal:50051")]
        orchestrator_url: String,

        /// Maximum agentic turns for Claude Code.
        #[arg(long, default_value = "50")]
        max_turns: u32,

        /// Claude model to use (e.g. "sonnet", "opus").
        #[arg(long, default_value = "sonnet")]
        model: String,
    },

    /// Run as an ACP bridge inside a Docker container (internal use).
    /// Spawns an ACP-compliant agent and streams output back to the orchestrator.
    #[command(hide = true)]
    AcpBridge {
        /// Job ID to execute.
        #[arg(long)]
        job_id: uuid::Uuid,

        /// URL of the orchestrator's internal API.
        #[arg(long, default_value = "http://host.docker.internal:50051")]
        orchestrator_url: String,
    },
}

impl Cli {
    /// Check if we should run the agent (default behavior or explicit `run` command).
    pub fn should_run_agent(&self) -> bool {
        matches!(self.command, None | Some(Command::Run))
    }
}

/// Initialize a secrets store from environment config.
///
/// Shared helper for CLI subcommands (`mcp auth`, `tool auth`, etc.) that need
/// access to encrypted secrets without spinning up the full AppBuilder.
pub async fn init_secrets_store()
-> anyhow::Result<Arc<dyn crate::secrets::SecretsStore + Send + Sync>> {
    let config = crate::config::Config::from_env().await?;
    let master_key = config.secrets.master_key().ok_or_else(|| {
        anyhow::anyhow!(
            "SECRETS_MASTER_KEY not set. Run 'ironclaw onboard' first or set it in .env"
        )
    })?;

    let crypto = Arc::new(crate::secrets::SecretsCrypto::new(master_key.clone())?);

    Ok(crate::db::create_secrets_store(&config.database, crypto).await?)
}

/// Run the Routines CLI subcommand.
pub async fn run_routines_cli(
    routines_cmd: &RoutinesCommand,
    config_path: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let config = crate::config::Config::from_env_with_toml(config_path)
        .await
        .map_err(|e| anyhow::anyhow!("{e:#}"))?;

    let db: Arc<dyn crate::db::Database> = crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("{e:#}"))?;

    let user_id = std::env::var("IRONCLAW_OWNER_ID").unwrap_or_else(|_| "default".to_string());
    run_routines_command(routines_cmd.clone(), db, &user_id).await
}

/// Run the Memory CLI subcommand.
pub async fn run_memory_command(mem_cmd: &MemoryCommand) -> anyhow::Result<()> {
    let config = crate::config::Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let session = crate::llm::create_session_manager(config.llm.session.clone()).await;

    let embeddings = config
        .embeddings
        .create_provider(&config.llm.nearai.base_url, session);

    let db: Arc<dyn crate::db::Database> = crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let cache_config = crate::workspace::EmbeddingCacheConfig {
        max_entries: config.embeddings.cache_size,
    };
    run_memory_command_with_db(mem_cmd.clone(), db, embeddings, cache_config).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use insta::assert_snapshot;

    #[test]
    fn test_version() {
        let cmd = Cli::command();
        assert_eq!(
            cmd.get_version().unwrap_or("unknown"),
            env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    #[cfg(feature = "import")]
    fn test_help_output() {
        let mut cmd = Cli::command();
        let help = cmd.render_help().to_string();
        assert_snapshot!(help);
    }

    #[test]
    #[cfg(not(feature = "import"))]
    fn test_help_output_without_import() {
        let mut cmd = Cli::command();
        let help = cmd.render_help().to_string();
        assert_snapshot!(help);
    }

    #[test]
    #[cfg(feature = "import")]
    fn test_long_help_output() {
        let mut cmd = Cli::command();
        let help = cmd.render_long_help().to_string();
        assert_snapshot!(help);
    }

    #[test]
    #[cfg(not(feature = "import"))]
    fn test_long_help_output_without_import() {
        let mut cmd = Cli::command();
        let help = cmd.render_long_help().to_string();
        assert_snapshot!(help);
    }
}
