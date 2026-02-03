//! NEAR Agent - Main entry point.

use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use near_agent::{
    agent::Agent,
    channels::{ChannelManager, CliChannel, HttpChannel},
    config::Config,
    history::Store,
    llm::create_llm_provider,
    safety::SafetyLayer,
    tools::ToolRegistry,
};

#[derive(Parser, Debug)]
#[command(name = "near-agent")]
#[command(about = "LLM-powered autonomous agent for the NEAR AI marketplace")]
#[command(version)]
struct Args {
    /// Run in interactive CLI mode only (disable other channels)
    #[arg(long)]
    cli_only: bool,

    /// Skip database connection (for testing)
    #[arg(long)]
    no_db: bool,

    /// Configuration file path (optional, uses env vars by default)
    #[arg(short, long)]
    config: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("near_agent=debug,tower_http=debug")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();

    tracing::info!("Starting NEAR Agent...");

    // Load configuration
    let config = Config::from_env()?;
    tracing::info!("Loaded configuration for agent: {}", config.agent.name);

    // Initialize database store (optional for testing)
    let store = if args.no_db {
        tracing::warn!("Running without database connection");
        None
    } else {
        let store = Store::new(&config.database).await?;
        store.run_migrations().await?;
        tracing::info!("Database connected and migrations applied");
        Some(Arc::new(store))
    };

    // Initialize LLM provider
    let llm = create_llm_provider(&config.llm)?;
    tracing::info!("LLM provider initialized: {}", llm.model_name());

    // Initialize safety layer
    let safety = Arc::new(SafetyLayer::new(&config.safety));
    tracing::info!("Safety layer initialized");

    // Initialize tool registry
    let tools = Arc::new(ToolRegistry::new());
    tools.register_builtin_tools();
    tracing::info!("Tool registry initialized with {} tools", tools.count());

    // Initialize channel manager
    let mut channels = ChannelManager::new();

    // Always add CLI channel
    if config.channels.cli.enabled {
        channels.add(Box::new(CliChannel::new()));
        tracing::info!("CLI channel enabled");
    }

    // Add HTTP channel if configured and not CLI-only mode
    if !args.cli_only {
        if let Some(ref http_config) = config.channels.http {
            channels.add(Box::new(HttpChannel::new(http_config.clone())));
            tracing::info!(
                "HTTP channel enabled on {}:{}",
                http_config.host,
                http_config.port
            );
        }

        // TODO: Add Slack and Telegram channels when implemented
        if config.channels.slack.is_some() {
            tracing::warn!("Slack channel configured but not yet implemented");
        }
        if config.channels.telegram.is_some() {
            tracing::warn!("Telegram channel configured but not yet implemented");
        }
    }

    // Create and run the agent
    let agent = Agent::new(config.agent.clone(), store, llm, safety, tools, channels);

    tracing::info!("Agent initialized, starting main loop...");

    // Run the agent (blocks until shutdown)
    agent.run().await?;

    tracing::info!("Agent shutdown complete");
    Ok(())
}
