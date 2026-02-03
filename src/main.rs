//! NEAR Agent - Main entry point.

use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use near_agent::{
    agent::Agent,
    channels::{ChannelManager, HttpChannel, TuiChannel},
    cli::{Cli, Command, run_tool_command},
    config::Config,
    history::Store,
    llm::create_llm_provider,
    safety::SafetyLayer,
    tools::{
        ToolRegistry,
        wasm::{WasmToolLoader, WasmToolRuntime},
    },
    workspace::Workspace,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Handle non-agent commands first (they don't need TUI/full setup)
    match &cli.command {
        Some(Command::Tool(tool_cmd)) => {
            // Simple logging for CLI commands
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
                )
                .init();

            return run_tool_command(tool_cmd.clone()).await;
        }
        None | Some(Command::Run) => {
            // Continue to run agent
        }
    }

    // Create TUI channel early so we can hook up logging
    // (channel is created but not started until agent.run())
    let tui_channel = TuiChannel::new();
    let tui_log_writer = tui_channel.log_writer();

    // Initialize tracing with both stderr (for pre-TUI output) and TUI writer
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("near_agent=info,tower_http=debug"));

    tracing_subscriber::registry()
        .with(env_filter)
        // TUI layer: sends logs to TUI status line (once TUI is running)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(tui_log_writer)
                .without_time()
                .with_target(false)
                .with_level(true),
        )
        .init();

    tracing::info!("Starting NEAR Agent...");

    // Load configuration
    let config = Config::from_env()?;
    tracing::info!("Loaded configuration for agent: {}", config.agent.name);

    // Initialize database store (optional for testing)
    let store = if cli.no_db {
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
    tracing::info!("Registered {} built-in tools", tools.count());

    // Register memory tools if database is available
    if let Some(ref store) = store {
        let workspace = Arc::new(Workspace::new("default", store.pool()));
        tools.register_memory_tools(workspace);
    }

    // Register builder tool if enabled
    if config.builder.enabled {
        tools
            .register_builder_tool(
                llm.clone(),
                safety.clone(),
                Some(config.builder.to_builder_config()),
            )
            .await;
        tracing::info!("Builder mode enabled");
    }

    // Load installed WASM tools
    if config.wasm.enabled && config.wasm.tools_dir.exists() {
        match WasmToolRuntime::new(config.wasm.to_runtime_config()) {
            Ok(runtime) => {
                let runtime = Arc::new(runtime);
                let loader = WasmToolLoader::new(Arc::clone(&runtime), Arc::clone(&tools));

                match loader.load_from_dir(&config.wasm.tools_dir).await {
                    Ok(results) => {
                        if !results.loaded.is_empty() {
                            tracing::info!(
                                "Loaded {} WASM tools from {}",
                                results.loaded.len(),
                                config.wasm.tools_dir.display()
                            );
                        }
                        for (path, err) in &results.errors {
                            tracing::warn!("Failed to load WASM tool {}: {}", path.display(), err);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to scan WASM tools directory: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to initialize WASM runtime: {}", e);
            }
        }
    }
    tracing::info!(
        "Tool registry initialized with {} total tools",
        tools.count()
    );

    // Initialize channel manager
    let mut channels = ChannelManager::new();

    // Add TUI channel (already created for logging hookup)
    if config.channels.cli.enabled {
        channels.add(Box::new(tui_channel));
        tracing::info!("TUI channel enabled");
    }

    // Add HTTP channel if configured and not CLI-only mode
    if !cli.cli_only {
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
