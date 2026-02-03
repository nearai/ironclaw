//! NEAR Agent - Main entry point.

use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use near_agent::{
    agent::{Agent, AgentDeps},
    channels::{ChannelManager, HttpChannel, TuiChannel},
    cli::{Cli, Command, run_tool_command},
    config::Config,
    history::Store,
    llm::{SessionConfig, create_llm_provider, create_session_manager},
    safety::SafetyLayer,
    tools::{
        ToolRegistry,
        wasm::{WasmToolLoader, WasmToolRuntime},
    },
    workspace::{EmbeddingProvider, NearAiEmbeddings, OpenAiEmbeddings, Workspace},
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

    // Load configuration first (before any logging setup)
    // so we can do auth before TUI starts
    let _ = dotenvy::dotenv(); // Load .env if present
    let config = Config::from_env()?;

    // Initialize session manager and authenticate BEFORE TUI setup
    // This allows the auth menu to display cleanly without TUI interference
    let session_config = SessionConfig {
        auth_base_url: config.llm.nearai.auth_base_url.clone(),
        session_path: config.llm.nearai.session_path.clone(),
        ..Default::default()
    };
    let session = create_session_manager(session_config).await;

    // Ensure we're authenticated before proceeding (may trigger login flow)
    // This happens before TUI so the menu displays correctly
    session.ensure_authenticated().await?;

    // Now create TUI channel and set up logging
    let tui_channel = TuiChannel::new();
    let tui_log_writer = tui_channel.log_writer();

    // Initialize tracing with TUI writer
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("near_agent=info,tower_http=debug"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(tui_log_writer)
                .without_time()
                .with_target(false)
                .with_level(true),
        )
        .init();

    tracing::info!("Starting NEAR Agent...");
    tracing::info!("Loaded configuration for agent: {}", config.agent.name);
    tracing::info!("NEAR AI session authenticated");

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

    // Initialize LLM provider (clone session so we can reuse it for embeddings)
    let llm = create_llm_provider(&config.llm, session.clone())?;
    tracing::info!("LLM provider initialized: {}", llm.model_name());

    // Initialize safety layer
    let safety = Arc::new(SafetyLayer::new(&config.safety));
    tracing::info!("Safety layer initialized");

    // Initialize tool registry
    let tools = Arc::new(ToolRegistry::new());
    tools.register_builtin_tools();
    tracing::info!("Registered {} built-in tools", tools.count());

    // Create embeddings provider if configured
    let embeddings: Option<Arc<dyn EmbeddingProvider>> = if config.embeddings.enabled {
        match config.embeddings.provider.as_str() {
            "nearai" => {
                tracing::info!(
                    "Embeddings enabled via NEAR AI (model: {})",
                    config.embeddings.model
                );
                Some(Arc::new(
                    NearAiEmbeddings::new(&config.llm.nearai.base_url, session.clone())
                        .with_model(&config.embeddings.model, 1536),
                ))
            }
            _ => {
                // Default to OpenAI for unknown providers
                if let Some(api_key) = config.embeddings.openai_api_key() {
                    tracing::info!(
                        "Embeddings enabled via OpenAI (model: {})",
                        config.embeddings.model
                    );
                    Some(Arc::new(OpenAiEmbeddings::with_model(
                        api_key,
                        &config.embeddings.model,
                        match config.embeddings.model.as_str() {
                            "text-embedding-3-large" => 3072,
                            _ => 1536, // text-embedding-3-small and ada-002
                        },
                    )))
                } else {
                    tracing::warn!("Embeddings configured but OPENAI_API_KEY not set");
                    None
                }
            }
        }
    } else {
        tracing::info!("Embeddings disabled (set OPENAI_API_KEY or EMBEDDING_ENABLED=true)");
        None
    };

    // Register memory tools if database is available
    if let Some(ref store) = store {
        let mut workspace = Workspace::new("default", store.pool());
        if let Some(ref emb) = embeddings {
            workspace = workspace.with_embeddings(emb.clone());
        }
        let workspace = Arc::new(workspace);
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
    }

    // Create workspace for agent (shared with memory tools)
    let workspace = store.as_ref().map(|s| {
        let mut ws = Workspace::new("default", s.pool());
        if let Some(ref emb) = embeddings {
            ws = ws.with_embeddings(emb.clone());
        }
        Arc::new(ws)
    });

    // Backfill embeddings if we just enabled the provider
    if let (Some(ws), Some(_)) = (&workspace, &embeddings) {
        match ws.backfill_embeddings().await {
            Ok(count) if count > 0 => {
                tracing::info!("Backfilled embeddings for {} chunks", count);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("Failed to backfill embeddings: {}", e);
            }
        }
    }

    // Create and run the agent
    let deps = AgentDeps {
        store,
        llm,
        safety,
        tools,
        workspace,
    };
    let agent = Agent::new(
        config.agent.clone(),
        deps,
        channels,
        Some(config.heartbeat.clone()),
    );

    tracing::info!("Agent initialized, starting main loop...");

    // Run the agent (blocks until shutdown)
    agent.run().await?;

    tracing::info!("Agent shutdown complete");
    Ok(())
}
