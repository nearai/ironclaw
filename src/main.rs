//! IronClaw - Main entry point.

use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use ironclaw::{
    agent::{Agent, AgentDeps},
    app::{AppBuilder, AppBuilderFlags},
    channels::{
        ChannelManager, GatewayChannel, HttpChannel, ReplChannel, SignalChannel, WebhookServer,
        WebhookServerConfig,
        wasm::{
            RegisteredEndpoint, SharedWasmChannel, WasmChannelLoader, WasmChannelRouter,
            WasmChannelRuntime, WasmChannelRuntimeConfig, create_wasm_channel_router,
        },
        web::log_layer::LogBroadcaster,
    },
    cli::{
        Cli, Command, run_mcp_command, run_pairing_command, run_service_command,
        run_status_command, run_tool_command,
    },
    config::Config,
    hooks::bootstrap_hooks,
    llm::{SessionConfig, create_session_manager},
    orchestrator::{
        ContainerJobConfig, ContainerJobManager, OrchestratorApi, TokenStore,
        api::OrchestratorState,
    },
    pairing::PairingStore,
    secrets::SecretsStore,
};

#[cfg(any(feature = "postgres", feature = "libsql"))]
use ironclaw::setup::{SetupConfig, SetupWizard};

/// Initialize tracing for simple CLI commands (warn level, no fancy layers).
fn init_cli_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Handle non-agent commands first (they don't need full setup)
    match &cli.command {
        Some(Command::Tool(tool_cmd)) => {
            init_cli_tracing();
            return run_tool_command(tool_cmd.clone()).await;
        }
        Some(Command::Config(config_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_config_command(config_cmd.clone()).await;
        }
        Some(Command::Registry(registry_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_registry_command(registry_cmd.clone()).await;
        }
        Some(Command::Mcp(mcp_cmd)) => {
            init_cli_tracing();
            return run_mcp_command(mcp_cmd.clone()).await;
        }
        Some(Command::Memory(mem_cmd)) => {
            init_cli_tracing();
            return run_memory_command(mem_cmd).await;
        }
        Some(Command::Pairing(pairing_cmd)) => {
            init_cli_tracing();
            return run_pairing_command(pairing_cmd.clone()).map_err(|e| anyhow::anyhow!("{}", e));
        }
        Some(Command::Service(service_cmd)) => {
            init_cli_tracing();
            return run_service_command(service_cmd);
        }
        Some(Command::Doctor) => {
            init_cli_tracing();
            let _ = dotenvy::dotenv();
            ironclaw::bootstrap::load_ironclaw_env();
            return ironclaw::cli::run_doctor_command().await;
        }
        Some(Command::Status) => {
            init_cli_tracing();
            let _ = dotenvy::dotenv();
            ironclaw::bootstrap::load_ironclaw_env();
            return run_status_command().await;
        }
        Some(Command::Completion(completion)) => {
            init_cli_tracing();
            return completion.run();
        }
        Some(Command::Worker {
            job_id,
            orchestrator_url,
            max_iterations,
        }) => {
            init_worker_tracing();
            return run_worker(*job_id, orchestrator_url, *max_iterations).await;
        }
        Some(Command::ClaudeBridge {
            job_id,
            orchestrator_url,
            max_turns,
            model,
        }) => {
            init_worker_tracing();
            return run_claude_bridge(*job_id, orchestrator_url, *max_turns, model).await;
        }
        Some(Command::Onboard {
            skip_auth,
            channels_only,
        }) => {
            let _ = dotenvy::dotenv();
            ironclaw::bootstrap::load_ironclaw_env();

            #[cfg(any(feature = "postgres", feature = "libsql"))]
            {
                let config = SetupConfig {
                    skip_auth: *skip_auth,
                    channels_only: *channels_only,
                };
                let mut wizard = SetupWizard::with_config(config);
                wizard.run().await?;
            }
            #[cfg(not(any(feature = "postgres", feature = "libsql")))]
            {
                let _ = (skip_auth, channels_only);
                eprintln!("Onboarding wizard requires the 'postgres' or 'libsql' feature.");
            }
            return Ok(());
        }
        None | Some(Command::Run) => {
            // Continue to run agent
        }
    }

    // ── Agent startup ──────────────────────────────────────────────────

    // Load .env files early so DATABASE_URL (and any other vars) are
    // available to all subsequent env-based config resolution.
    let _ = dotenvy::dotenv();
    ironclaw::bootstrap::load_ironclaw_env();

    // Enhanced first-run detection
    #[cfg(any(feature = "postgres", feature = "libsql"))]
    if !cli.no_onboard
        && let Some(reason) = check_onboard_needed()
    {
        println!("Onboarding needed: {}", reason);
        println!();
        let mut wizard = SetupWizard::new();
        wizard.run().await?;
    }

    // Load initial config from env + disk + optional TOML (before DB is available)
    let toml_path = cli.config.as_deref();
    let config = match Config::from_env_with_toml(toml_path).await {
        Ok(c) => c,
        Err(ironclaw::error::ConfigError::MissingRequired { key, hint }) => {
            eprintln!("Configuration error: Missing required setting '{}'", key);
            eprintln!("  {}", hint);
            eprintln!();
            eprintln!(
                "Run 'ironclaw onboard' to configure, or set the required environment variables."
            );
            std::process::exit(1);
        }
        Err(e) => return Err(e.into()),
    };

    // Initialize session manager and authenticate before channel setup
    let session_config = SessionConfig {
        auth_base_url: config.llm.nearai.auth_base_url.clone(),
        session_path: config.llm.nearai.session_path.clone(),
    };
    let session = create_session_manager(session_config).await;

    // Create log broadcaster before tracing init so the WebLogLayer can capture all events.
    let log_broadcaster = Arc::new(LogBroadcaster::new());

    // Initialize tracing with a reloadable EnvFilter so the gateway can switch
    // log levels at runtime without restarting.
    let log_level_handle =
        ironclaw::channels::web::log_layer::init_tracing(Arc::clone(&log_broadcaster));

    tracing::info!("Starting IronClaw...");
    tracing::info!("Loaded configuration for agent: {}", config.agent.name);
    tracing::info!("LLM backend: {}", config.llm.backend);

    // ── Phase 1-5: Build all core components via AppBuilder ────────────

    let flags = AppBuilderFlags { no_db: cli.no_db };
    let components = AppBuilder::new(
        config,
        flags,
        toml_path.map(std::path::PathBuf::from),
        session.clone(),
        Arc::clone(&log_broadcaster),
    )
    .build_all()
    .await?;

    let config = components.config;

    // Session-based auth is only needed for NEAR AI backend without an API key.
    if config.llm.backend == ironclaw::config::LlmBackend::NearAi
        && config.llm.nearai.api_key.is_none()
    {
        session.ensure_authenticated().await?;
    }

    // ── Tunnel setup ───────────────────────────────────────────────────

    let (config, active_tunnel) = start_tunnel(config).await;

    // ── Orchestrator / container job manager ────────────────────────────

    // Proactive Docker detection
    let docker_status = if config.sandbox.enabled {
        let detection = ironclaw::sandbox::check_docker().await;
        match detection.status {
            ironclaw::sandbox::DockerStatus::Available => {
                tracing::info!("Docker is available");
            }
            ironclaw::sandbox::DockerStatus::NotInstalled => {
                tracing::warn!(
                    "Docker is not installed -- sandbox disabled for this session. {}",
                    detection.platform.install_hint()
                );
            }
            ironclaw::sandbox::DockerStatus::NotRunning => {
                tracing::warn!(
                    "Docker is installed but not running -- sandbox disabled for this session. {}",
                    detection.platform.start_hint()
                );
            }
            ironclaw::sandbox::DockerStatus::Disabled => {}
        }
        detection.status
    } else {
        ironclaw::sandbox::DockerStatus::Disabled
    };

    let job_event_tx: Option<
        tokio::sync::broadcast::Sender<(uuid::Uuid, ironclaw::channels::web::types::SseEvent)>,
    > = if config.sandbox.enabled && docker_status.is_ok() {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        Some(tx)
    } else {
        None
    };
    let prompt_queue = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::<
        uuid::Uuid,
        std::collections::VecDeque<ironclaw::orchestrator::api::PendingPrompt>,
    >::new()));

    let container_job_manager: Option<Arc<ContainerJobManager>> =
        if config.sandbox.enabled && docker_status.is_ok() {
            let token_store = TokenStore::new();
            let job_config = ContainerJobConfig {
                image: config.sandbox.image.clone(),
                memory_limit_mb: config.sandbox.memory_limit_mb,
                cpu_shares: config.sandbox.cpu_shares,
                orchestrator_port: 50051,
                claude_code_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
                claude_code_oauth_token: ironclaw::config::ClaudeCodeConfig::extract_oauth_token(),
                claude_code_model: config.claude_code.model.clone(),
                claude_code_max_turns: config.claude_code.max_turns,
                claude_code_memory_limit_mb: config.claude_code.memory_limit_mb,
                claude_code_allowed_tools: config.claude_code.allowed_tools.clone(),
            };
            let jm = Arc::new(ContainerJobManager::new(job_config, token_store.clone()));

            // Start the orchestrator internal API in the background
            let orchestrator_state = OrchestratorState {
                llm: components.llm.clone(),
                job_manager: Arc::clone(&jm),
                token_store,
                job_event_tx: job_event_tx.clone(),
                prompt_queue: Arc::clone(&prompt_queue),
                store: components.db.clone(),
                secrets_store: components.secrets_store.clone(),
                user_id: "default".to_string(),
            };

            tokio::spawn(async move {
                if let Err(e) = OrchestratorApi::start(orchestrator_state, 50051).await {
                    tracing::error!("Orchestrator API failed: {}", e);
                }
            });

            if config.claude_code.enabled {
                tracing::info!(
                    "Claude Code sandbox mode available (model: {}, max_turns: {})",
                    config.claude_code.model,
                    config.claude_code.max_turns
                );
            }
            Some(jm)
        } else {
            None
        };

    // ── Channel setup ──────────────────────────────────────────────────

    let channels = ChannelManager::new();
    let mut channel_names: Vec<String> = Vec::new();
    let mut loaded_wasm_channel_names: Vec<String> = Vec::new();
    #[allow(clippy::type_complexity)]
    let mut wasm_channel_runtime_state: Option<(
        Arc<WasmChannelRuntime>,
        Arc<PairingStore>,
        Arc<WasmChannelRouter>,
    )> = None;

    // Create CLI channel
    let repl_channel = if let Some(ref msg) = cli.message {
        Some(ReplChannel::with_message(msg.clone()))
    } else if config.channels.cli.enabled {
        let repl = ReplChannel::new();
        repl.suppress_banner();
        Some(repl)
    } else {
        None
    };

    if let Some(repl) = repl_channel {
        channels.add(Box::new(repl)).await;
        if cli.message.is_some() {
            tracing::info!("Single message mode");
        } else {
            channel_names.push("repl".to_string());
            tracing::info!("REPL mode enabled");
        }
    }

    // Collect webhook route fragments; a single WebhookServer hosts them all.
    let mut webhook_routes: Vec<axum::Router> = Vec::new();

    // Load WASM channels and register their webhook routes.
    if config.channels.wasm_channels_enabled && config.channels.wasm_channels_dir.exists() {
        let wasm_result = setup_wasm_channels(
            &config,
            &components.secrets_store,
            components.extension_manager.as_ref(),
        )
        .await;

        if let Some(result) = wasm_result {
            loaded_wasm_channel_names = result.channel_names;
            wasm_channel_runtime_state = Some((
                result.wasm_channel_runtime,
                result.pairing_store,
                result.wasm_channel_router,
            ));
            for (name, channel) in result.channels {
                channel_names.push(name);
                channels.add(channel).await;
            }
            if let Some(routes) = result.webhook_routes {
                webhook_routes.push(routes);
            }
        }
    }

    // Add Signal channel if configured and not CLI-only mode.
    if !cli.cli_only
        && let Some(ref signal_config) = config.channels.signal
    {
        let signal_channel = SignalChannel::new(signal_config.clone())?;
        channel_names.push("signal".to_string());
        channels.add(Box::new(signal_channel)).await;
        let safe_url = SignalChannel::redact_url(&signal_config.http_url);
        tracing::info!(
            url = %safe_url,
            "Signal channel enabled"
        );
        if signal_config.allow_from.is_empty() {
            tracing::warn!(
                "Signal channel has empty allow_from list - ALL messages will be DENIED."
            );
        }
    }

    // Add HTTP channel if configured and not CLI-only mode.
    let mut webhook_server_addr: Option<std::net::SocketAddr> = None;
    if !cli.cli_only
        && let Some(ref http_config) = config.channels.http
    {
        let http_channel = HttpChannel::new(http_config.clone());
        webhook_routes.push(http_channel.routes());
        let (host, port) = http_channel.addr();
        webhook_server_addr = Some(
            format!("{}:{}", host, port)
                .parse()
                .expect("HttpConfig host:port must be a valid SocketAddr"),
        );
        channel_names.push("http".to_string());
        channels.add(Box::new(http_channel)).await;
        tracing::info!(
            "HTTP channel enabled on {}:{}",
            http_config.host,
            http_config.port
        );
    }

    // Start the unified webhook server if any routes were registered.
    let mut webhook_server = if !webhook_routes.is_empty() {
        let addr =
            webhook_server_addr.unwrap_or_else(|| std::net::SocketAddr::from(([0, 0, 0, 0], 8080)));
        if addr.ip().is_unspecified() {
            tracing::warn!(
                "Webhook server is binding to {} — it will be reachable from all network interfaces. \
                 Set HTTP_HOST=127.0.0.1 to restrict to localhost.",
                addr.ip()
            );
        }
        let mut server = WebhookServer::new(WebhookServerConfig { addr });
        for routes in webhook_routes {
            server.add_routes(routes);
        }
        server.start().await?;
        Some(server)
    } else {
        None
    };

    // Register lifecycle hooks.
    let active_tool_names = components.tools.list().await;

    let hook_bootstrap = bootstrap_hooks(
        &components.hooks,
        components.workspace.as_ref(),
        &config.wasm.tools_dir,
        &config.channels.wasm_channels_dir,
        &active_tool_names,
        &loaded_wasm_channel_names,
        &components.dev_loaded_tool_names,
    )
    .await;
    tracing::info!(
        bundled = hook_bootstrap.bundled_hooks,
        plugin = hook_bootstrap.plugin_hooks,
        workspace = hook_bootstrap.workspace_hooks,
        outbound_webhooks = hook_bootstrap.outbound_webhooks,
        errors = hook_bootstrap.errors,
        "Lifecycle hooks initialized"
    );

    // Create session manager (shared between agent and web gateway)
    let session_manager =
        Arc::new(ironclaw::agent::SessionManager::new().with_hooks(components.hooks.clone()));

    // Register job tools (sandbox deps auto-injected when container_job_manager is available)
    components.tools.register_job_tools(
        Arc::clone(&components.context_manager),
        container_job_manager.clone(),
        components.db.clone(),
        job_event_tx.clone(),
        Some(channels.inject_sender()),
        if config.sandbox.enabled {
            Some(Arc::clone(&prompt_queue))
        } else {
            None
        },
        components.secrets_store.clone(),
    );

    // ── Gateway channel ────────────────────────────────────────────────

    let mut gateway_url: Option<String> = None;
    let mut sse_sender: Option<
        tokio::sync::broadcast::Sender<ironclaw::channels::web::types::SseEvent>,
    > = None;
    let mut gateway_state: Option<std::sync::Arc<ironclaw::channels::web::server::GatewayState>> =
        None;
    if let Some(ref gw_config) = config.channels.gateway {
        let mut gw =
            GatewayChannel::new(gw_config.clone()).with_llm_provider(Arc::clone(&components.llm));
        if let Some(ref ws) = components.workspace {
            gw = gw.with_workspace(Arc::clone(ws));
        }
        gw = gw.with_session_manager(Arc::clone(&session_manager));
        gw = gw.with_log_broadcaster(Arc::clone(&log_broadcaster));
        gw = gw.with_log_level_handle(Arc::clone(&log_level_handle));
        gw = gw.with_tool_registry(Arc::clone(&components.tools));
        if let Some(ref ext_mgr) = components.extension_manager {
            gw = gw.with_extension_manager(Arc::clone(ext_mgr));
        }
        if !components.catalog_entries.is_empty() {
            gw = gw.with_registry_entries(components.catalog_entries.clone());
        }
        if let Some(ref d) = components.db {
            gw = gw.with_store(Arc::clone(d));
        }
        if let Some(ref jm) = container_job_manager {
            gw = gw.with_job_manager(Arc::clone(jm));
        }
        if let Some(ref sr) = components.skill_registry {
            gw = gw.with_skill_registry(Arc::clone(sr));
        }
        if let Some(ref sc) = components.skill_catalog {
            gw = gw.with_skill_catalog(Arc::clone(sc));
        }
        gw = gw.with_cost_guard(Arc::clone(&components.cost_guard));
        if config.sandbox.enabled {
            gw = gw.with_prompt_queue(Arc::clone(&prompt_queue));

            if let Some(ref tx) = job_event_tx {
                let mut rx = tx.subscribe();
                let gw_state = Arc::clone(gw.state());
                tokio::spawn(async move {
                    while let Ok((_job_id, event)) = rx.recv().await {
                        gw_state.sse.broadcast(event);
                    }
                });
            }
        }

        gateway_url = Some(format!(
            "http://{}:{}/?token={}",
            gw_config.host,
            gw_config.port,
            gw.auth_token()
        ));

        tracing::info!("Web UI: http://{}:{}/", gw_config.host, gw_config.port);

        // Capture SSE sender before moving gw into channels.
        // IMPORTANT: This must come after all `with_*` calls since `rebuild_state`
        // creates a new SseManager, which would orphan this sender.
        sse_sender = Some(gw.state().sse.sender());
        gateway_state = Some(Arc::clone(gw.state()));

        channel_names.push("gateway".to_string());
        channels.add(Box::new(gw)).await;
    }

    // ── Boot screen ────────────────────────────────────────────────────

    let boot_tool_count = components.tools.count();
    let boot_llm_model = components.llm.model_name().to_string();
    let boot_cheap_model = components
        .cheap_llm
        .as_ref()
        .map(|c| c.model_name().to_string());

    if config.channels.cli.enabled && cli.message.is_none() {
        let boot_info = ironclaw::boot_screen::BootInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            agent_name: config.agent.name.clone(),
            llm_backend: config.llm.backend.to_string(),
            llm_model: boot_llm_model,
            cheap_model: boot_cheap_model,
            db_backend: if cli.no_db {
                "none".to_string()
            } else {
                config.database.backend.to_string()
            },
            db_connected: !cli.no_db,
            tool_count: boot_tool_count,
            gateway_url,
            embeddings_enabled: config.embeddings.enabled,
            embeddings_provider: if config.embeddings.enabled {
                Some(config.embeddings.provider.clone())
            } else {
                None
            },
            heartbeat_enabled: config.heartbeat.enabled,
            heartbeat_interval_secs: config.heartbeat.interval_secs,
            sandbox_enabled: config.sandbox.enabled,
            docker_status,
            claude_code_enabled: config.claude_code.enabled,
            routines_enabled: config.routines.enabled,
            skills_enabled: config.skills.enabled,
            channels: channel_names,
            tunnel_url: active_tunnel
                .as_ref()
                .and_then(|t| t.public_url())
                .or_else(|| config.tunnel.public_url.clone()),
            tunnel_provider: active_tunnel.as_ref().map(|t| t.name().to_string()),
        };
        ironclaw::boot_screen::print_boot_screen(&boot_info);
    }

    // ── Run the agent ──────────────────────────────────────────────────

    let channels = Arc::new(channels);

    // Register message tool for sending messages to connected channels
    components
        .tools
        .register_message_tools(Arc::clone(&channels))
        .await;

    // Wire up channel runtime for hot-activation of WASM channels.
    if let Some(ref ext_mgr) = components.extension_manager
        && let Some((rt, ps, router)) = wasm_channel_runtime_state.take()
    {
        ext_mgr.set_active_channels(loaded_wasm_channel_names).await;
        ext_mgr
            .set_channel_runtime(
                Arc::clone(&channels),
                rt,
                ps,
                router,
                config.channels.telegram_owner_id,
            )
            .await;
        tracing::info!("Channel runtime wired into extension manager for hot-activation");
    }

    // Wire SSE sender into extension manager for broadcasting status events.
    if let Some(ref ext_mgr) = components.extension_manager
        && let Some(sender) = sse_sender
    {
        ext_mgr.set_sse_sender(sender).await;
    }

    let deps = AgentDeps {
        store: components.db,
        llm: components.llm,
        cheap_llm: components.cheap_llm,
        safety: components.safety,
        tools: components.tools,
        workspace: components.workspace,
        extension_manager: components.extension_manager,
        skill_registry: components.skill_registry,
        skill_catalog: components.skill_catalog,
        skills_config: config.skills.clone(),
        hooks: components.hooks,
        cost_guard: components.cost_guard,
    };

    let agent = Agent::new(
        config.agent.clone(),
        deps,
        channels,
        Some(config.heartbeat.clone()),
        Some(config.hygiene.clone()),
        Some(config.routines.clone()),
        Some(components.context_manager),
        Some(session_manager),
    );

    agent.run().await?;

    // ── Shutdown ────────────────────────────────────────────────────────

    if let Some(ref mut server) = webhook_server {
        server.shutdown().await;
    }

    if let Some(tunnel) = active_tunnel {
        tracing::info!("Stopping {} tunnel...", tunnel.name());
        if let Err(e) = tunnel.stop().await {
            tracing::warn!("Failed to stop tunnel cleanly: {}", e);
        }
    }

    tracing::info!("Agent shutdown complete");

    // Check if a restart was requested via the gateway API.
    if let Some(ref gw_state) = gateway_state
        && gw_state
            .restart_requested
            .load(std::sync::atomic::Ordering::Relaxed)
    {
        eprintln!("Restarting IronClaw (exit code 75)...");
        std::process::exit(75);
    }

    Ok(())
}

// ── Helper functions ────────────────────────────────────────────────────

/// Initialize tracing for worker/bridge processes (info level).
fn init_worker_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("ironclaw=info")),
        )
        .init();
}

/// Run the Memory CLI subcommand.
async fn run_memory_command(mem_cmd: &ironclaw::cli::MemoryCommand) -> anyhow::Result<()> {
    let config = Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let session = create_session_manager(SessionConfig {
        auth_base_url: config.llm.nearai.auth_base_url.clone(),
        session_path: config.llm.nearai.session_path.clone(),
    })
    .await;

    let embeddings = config
        .embeddings
        .create_provider(&config.llm.nearai.base_url, session);

    // Warn if libSQL backend is used with non-1536 embedding dimension.
    if config.database.backend == ironclaw::config::DatabaseBackend::LibSql
        && config.embeddings.enabled
        && config.embeddings.dimension != 1536
    {
        tracing::warn!(
            configured_dimension = config.embeddings.dimension,
            "Embedding dimension {} is not 1536. The libSQL schema uses \
             F32_BLOB(1536) which requires exactly 1536 dimensions. \
             Embedding storage will fail. Use PostgreSQL or set \
             EMBEDDING_DIMENSION=1536.",
            config.embeddings.dimension
        );
    }

    let db: Arc<dyn ironclaw::db::Database> = ironclaw::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    ironclaw::cli::run_memory_command_with_db(mem_cmd.clone(), db, embeddings).await
}

/// Run the Worker subcommand (inside Docker containers).
async fn run_worker(
    job_id: uuid::Uuid,
    orchestrator_url: &str,
    max_iterations: u32,
) -> anyhow::Result<()> {
    tracing::info!(
        "Starting worker for job {} (orchestrator: {})",
        job_id,
        orchestrator_url
    );

    let config = ironclaw::worker::runtime::WorkerConfig {
        job_id,
        orchestrator_url: orchestrator_url.to_string(),
        max_iterations,
        timeout: std::time::Duration::from_secs(600),
    };

    let runtime = ironclaw::worker::WorkerRuntime::new(config)
        .map_err(|e| anyhow::anyhow!("Worker init failed: {}", e))?;

    runtime
        .run()
        .await
        .map_err(|e| anyhow::anyhow!("Worker failed: {}", e))
}

/// Run the Claude Code bridge subcommand (inside Docker containers).
async fn run_claude_bridge(
    job_id: uuid::Uuid,
    orchestrator_url: &str,
    max_turns: u32,
    model: &str,
) -> anyhow::Result<()> {
    tracing::info!(
        "Starting Claude Code bridge for job {} (orchestrator: {}, model: {})",
        job_id,
        orchestrator_url,
        model
    );

    let config = ironclaw::worker::claude_bridge::ClaudeBridgeConfig {
        job_id,
        orchestrator_url: orchestrator_url.to_string(),
        max_turns,
        model: model.to_string(),
        timeout: std::time::Duration::from_secs(1800),
        allowed_tools: ironclaw::config::ClaudeCodeConfig::from_env().allowed_tools,
    };

    let runtime = ironclaw::worker::ClaudeBridgeRuntime::new(config)
        .map_err(|e| anyhow::anyhow!("Claude bridge init failed: {}", e))?;

    runtime
        .run()
        .await
        .map_err(|e| anyhow::anyhow!("Claude bridge failed: {}", e))
}

/// Start managed tunnel if configured and no static URL is already set.
async fn start_tunnel(
    mut config: ironclaw::config::Config,
) -> (
    ironclaw::config::Config,
    Option<Box<dyn ironclaw::tunnel::Tunnel>>,
) {
    if config.tunnel.public_url.is_some() {
        tracing::info!(
            "Static tunnel URL in use: {}",
            config.tunnel.public_url.as_deref().unwrap_or("?")
        );
        return (config, None);
    }

    let Some(ref provider_config) = config.tunnel.provider else {
        return (config, None);
    };

    let gateway_port = config
        .channels
        .gateway
        .as_ref()
        .map(|g| g.port)
        .unwrap_or(3000);
    let gateway_host = config
        .channels
        .gateway
        .as_ref()
        .map(|g| g.host.as_str())
        .unwrap_or("127.0.0.1");

    match ironclaw::tunnel::create_tunnel(provider_config) {
        Ok(Some(tunnel)) => {
            tracing::info!(
                "Starting {} tunnel on {}:{}...",
                tunnel.name(),
                gateway_host,
                gateway_port
            );
            match tunnel.start(gateway_host, gateway_port).await {
                Ok(url) => {
                    tracing::info!("Tunnel started: {}", url);
                    config.tunnel.public_url = Some(url);
                    (config, Some(tunnel))
                }
                Err(e) => {
                    tracing::error!("Failed to start tunnel: {}", e);
                    (config, None)
                }
            }
        }
        Ok(None) => (config, None),
        Err(e) => {
            tracing::error!("Failed to create tunnel: {}", e);
            (config, None)
        }
    }
}

/// Result of WASM channel setup.
struct WasmChannelSetup {
    channels: Vec<(String, Box<dyn ironclaw::channels::Channel>)>,
    channel_names: Vec<String>,
    webhook_routes: Option<axum::Router>,
    /// Runtime objects needed for hot-activation via ExtensionManager.
    wasm_channel_runtime: Arc<WasmChannelRuntime>,
    pairing_store: Arc<PairingStore>,
    wasm_channel_router: Arc<WasmChannelRouter>,
}

/// Load WASM channels and register their webhook routes.
async fn setup_wasm_channels(
    config: &ironclaw::config::Config,
    secrets_store: &Option<Arc<dyn SecretsStore + Send + Sync>>,
    extension_manager: Option<&Arc<ironclaw::extensions::ExtensionManager>>,
) -> Option<WasmChannelSetup> {
    let runtime = match WasmChannelRuntime::new(WasmChannelRuntimeConfig::default()) {
        Ok(r) => Arc::new(r),
        Err(e) => {
            tracing::warn!("Failed to initialize WASM channel runtime: {}", e);
            return None;
        }
    };

    let pairing_store = Arc::new(PairingStore::new());
    let loader = WasmChannelLoader::new(Arc::clone(&runtime), Arc::clone(&pairing_store));

    let results = match loader
        .load_from_dir(&config.channels.wasm_channels_dir)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Failed to scan WASM channels directory: {}", e);
            return None;
        }
    };

    let wasm_router = Arc::new(WasmChannelRouter::new());
    let mut channels: Vec<(String, Box<dyn ironclaw::channels::Channel>)> = Vec::new();
    let mut channel_names: Vec<String> = Vec::new();

    for loaded in results.loaded {
        let channel_name = loaded.name().to_string();
        channel_names.push(channel_name.clone());
        tracing::info!("Loaded WASM channel: {}", channel_name);

        let secret_name = loaded.webhook_secret_name();

        let webhook_secret = if let Some(secrets) = secrets_store {
            secrets
                .get_decrypted("default", &secret_name)
                .await
                .ok()
                .map(|s| s.expose().to_string())
        } else {
            None
        };

        let secret_header = loaded.webhook_secret_header().map(|s| s.to_string());

        let webhook_path = format!("/webhook/{}", channel_name);
        let endpoints = vec![RegisteredEndpoint {
            channel_name: channel_name.clone(),
            path: webhook_path,
            methods: vec!["POST".to_string()],
            require_secret: webhook_secret.is_some(),
        }];

        let channel_arc = Arc::new(loaded.channel);

        {
            let mut config_updates = std::collections::HashMap::new();

            if let Some(ref tunnel_url) = config.tunnel.public_url {
                config_updates.insert(
                    "tunnel_url".to_string(),
                    serde_json::Value::String(tunnel_url.clone()),
                );
            }

            if let Some(ref secret) = webhook_secret {
                config_updates.insert(
                    "webhook_secret".to_string(),
                    serde_json::Value::String(secret.clone()),
                );
            }

            // Inject owner_id for Telegram so the bot only responds to the bound user.
            if channel_name == "telegram"
                && let Some(owner_id) = config.channels.telegram_owner_id
            {
                config_updates.insert("owner_id".to_string(), serde_json::json!(owner_id));
            }

            if !config_updates.is_empty() {
                channel_arc.update_config(config_updates).await;
                tracing::info!(
                    channel = %channel_name,
                    has_tunnel = config.tunnel.public_url.is_some(),
                    has_webhook_secret = webhook_secret.is_some(),
                    "Injected runtime config into channel"
                );
            }
        }

        tracing::info!(
            channel = %channel_name,
            has_webhook_secret = webhook_secret.is_some(),
            secret_header = ?secret_header,
            "Registering channel with router"
        );

        wasm_router
            .register(
                Arc::clone(&channel_arc),
                endpoints,
                webhook_secret.clone(),
                secret_header,
            )
            .await;
        if let Some(secrets) = secrets_store {
            match inject_channel_credentials(&channel_arc, secrets.as_ref(), &channel_name).await {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!(
                            channel = %channel_name,
                            credentials_injected = count,
                            "Channel credentials injected"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(
                        channel = %channel_name,
                        error = %e,
                        "Failed to inject channel credentials"
                    );
                }
            }
        }

        channels.push((channel_name, Box::new(SharedWasmChannel::new(channel_arc))));
    }

    for (path, err) in &results.errors {
        tracing::warn!("Failed to load WASM channel {}: {}", path.display(), err);
    }

    // Always create webhook routes (even with no channels loaded) so that
    // channels hot-added at runtime can receive webhooks without a restart.
    let webhook_routes = {
        Some(create_wasm_channel_router(
            Arc::clone(&wasm_router),
            extension_manager.map(Arc::clone),
        ))
    };

    Some(WasmChannelSetup {
        channels,
        channel_names,
        webhook_routes,
        wasm_channel_runtime: runtime,
        pairing_store,
        wasm_channel_router: wasm_router,
    })
}

/// Check if onboarding is needed and return the reason.
#[cfg(any(feature = "postgres", feature = "libsql"))]
fn check_onboard_needed() -> Option<&'static str> {
    let has_db = std::env::var("DATABASE_URL").is_ok()
        || std::env::var("LIBSQL_PATH").is_ok()
        || ironclaw::config::default_libsql_path().exists();

    if !has_db {
        return Some("Database not configured");
    }

    if std::env::var("ONBOARD_COMPLETED")
        .map(|v| v == "true")
        .unwrap_or(false)
    {
        return None;
    }

    if std::env::var("NEARAI_API_KEY").is_err() {
        let session_path = ironclaw::llm::session::default_session_path();
        if !session_path.exists() {
            return Some("First run");
        }
    }

    None
}

/// Inject credentials for a channel based on naming convention.
///
/// Looks for secrets matching the pattern `{channel_name}_*` and injects them
/// as credential placeholders (e.g., `telegram_bot_token` -> `{TELEGRAM_BOT_TOKEN}`).
async fn inject_channel_credentials(
    channel: &Arc<ironclaw::channels::wasm::WasmChannel>,
    secrets: &dyn SecretsStore,
    channel_name: &str,
) -> anyhow::Result<usize> {
    let all_secrets = secrets
        .list("default")
        .await
        .map_err(|e| anyhow::anyhow!("Failed to list secrets: {}", e))?;

    let prefix = format!("{}_", channel_name);
    let mut count = 0;

    for secret_meta in all_secrets {
        if !secret_meta.name.starts_with(&prefix) {
            continue;
        }

        let decrypted = match secrets.get_decrypted("default", &secret_meta.name).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(
                    secret = %secret_meta.name,
                    error = %e,
                    "Failed to decrypt secret for channel credential injection"
                );
                continue;
            }
        };

        let placeholder = secret_meta.name.to_uppercase();

        tracing::debug!(
            channel = %channel_name,
            secret = %secret_meta.name,
            placeholder = %placeholder,
            "Injecting credential"
        );

        channel
            .set_credential(&placeholder, decrypted.expose().to_string())
            .await;
        count += 1;
    }

    Ok(count)
}
