//! IronClaw - Main entry point.

use std::sync::Arc;
use std::time::Duration;

use clap::Parser;

use ironclaw::{
    agent::{Agent, AgentDeps},
    app::{AppBuilder, AppBuilderFlags},
    channels::{
        ChannelManager, GatewayChannel, HttpChannel, ReplChannel, SignalChannel, WebhookServer,
        WebhookServerConfig,
        wasm::{WasmChannelRouter, WasmChannelRuntime},
        web::log_layer::LogBroadcaster,
    },
    cli::{
        Cli, Command, run_mcp_command, run_pairing_command, run_service_command,
        run_status_command, run_tool_command,
    },
    config::Config,
    hooks::bootstrap_hooks,
    llm::create_session_manager,
    orchestrator::{ReaperConfig, SandboxReaper},
    pairing::PairingStore,
    tracing_fmt::{init_cli_tracing, init_worker_tracing},
};

#[cfg(any(feature = "postgres", feature = "libsql"))]
use ironclaw::setup::{SetupConfig, SetupWizard};

/// Synchronous entry point. Loads `.env` files before the Tokio runtime
/// starts so that `std::env::set_var` is safe (no worker threads yet).
fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    ironclaw::bootstrap::load_ironclaw_env();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
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
            return run_mcp_command(*mcp_cmd.clone()).await;
        }
        Some(Command::Memory(mem_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_memory_command(mem_cmd).await;
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
            return ironclaw::cli::run_doctor_command().await;
        }
        Some(Command::Status) => {
            init_cli_tracing();
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
            return ironclaw::worker::run_worker(*job_id, orchestrator_url, *max_iterations).await;
        }
        Some(Command::ClaudeBridge {
            job_id,
            orchestrator_url,
            max_turns,
            model,
        }) => {
            init_worker_tracing();
            return ironclaw::worker::run_claude_bridge(
                *job_id,
                orchestrator_url,
                *max_turns,
                model,
            )
            .await;
        }
        Some(Command::Onboard {
            skip_auth,
            channels_only,
            provider_only,
            quick,
        }) => {
            #[cfg(any(feature = "postgres", feature = "libsql"))]
            {
                let config = SetupConfig {
                    skip_auth: *skip_auth,
                    channels_only: *channels_only,
                    provider_only: *provider_only,
                    quick: *quick,
                };
                let mut wizard = SetupWizard::with_config(config);
                wizard.run().await?;
            }
            #[cfg(not(any(feature = "postgres", feature = "libsql")))]
            {
                let _ = (skip_auth, channels_only, provider_only, quick);
                eprintln!("Onboarding wizard requires the 'postgres' or 'libsql' feature.");
            }
            return Ok(());
        }
        None | Some(Command::Run) => {
            // Continue to run agent
        }
    }

    // ── PID lock (prevent multiple instances) ────────────────────────
    let _pid_lock = match ironclaw::bootstrap::PidLock::acquire() {
        Ok(lock) => Some(lock),
        Err(ironclaw::bootstrap::PidLockError::AlreadyRunning { pid }) => {
            anyhow::bail!(
                "Another IronClaw instance is already running (PID {}). \
                 If this is incorrect, remove the stale PID file: {}",
                pid,
                ironclaw::bootstrap::pid_lock_path().display()
            );
        }
        Err(e) => {
            eprintln!("Warning: Could not acquire PID lock: {}", e);
            eprintln!("Continuing without PID lock protection.");
            None
        }
    };

    // ── Agent startup ──────────────────────────────────────────────────

    // Enhanced first-run detection
    #[cfg(any(feature = "postgres", feature = "libsql"))]
    if !cli.no_onboard
        && let Some(reason) = ironclaw::setup::check_onboard_needed()
    {
        println!("Onboarding needed: {}", reason);
        println!();
        let mut wizard = SetupWizard::with_config(SetupConfig {
            quick: true,
            ..Default::default()
        });
        wizard.run().await?;
    }

    // Load initial config from env + disk + optional TOML (before DB is available).
    // Credentials may be missing at this point — that's fine. LlmConfig::resolve()
    // defers gracefully, and AppBuilder::build_all() re-resolves after loading
    // secrets from the encrypted DB.
    let toml_path = cli.config.as_deref();
    let config = match Config::from_env_with_toml(toml_path).await {
        Ok(c) => c,
        Err(ironclaw::error::ConfigError::MissingRequired { key, hint }) => {
            anyhow::bail!(
                "Configuration error: Missing required setting '{}'. {}. \
                 Run 'ironclaw onboard' to configure, or set the required environment variables.",
                key,
                hint
            );
        }
        Err(e) => return Err(e.into()),
    };

    // Initialize session manager before channel setup
    let session = create_session_manager(config.llm.session.clone()).await;

    // Create log broadcaster before tracing init so the WebLogLayer can capture all events.
    let log_broadcaster = Arc::new(LogBroadcaster::new());

    // Initialize tracing with a reloadable EnvFilter so the gateway can switch
    // log levels at runtime without restarting.
    let log_level_handle =
        ironclaw::channels::web::log_layer::init_tracing(Arc::clone(&log_broadcaster));

    tracing::debug!("Starting IronClaw...");
    tracing::debug!("Loaded configuration for agent: {}", config.agent.name);
    tracing::debug!("LLM backend: {}", config.llm.backend);

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

    // ── Tunnel setup ───────────────────────────────────────────────────

    let (config, active_tunnel) = ironclaw::tunnel::start_managed_tunnel(config).await;

    // ── Orchestrator / container job manager ────────────────────────────

    let orch = ironclaw::orchestrator::setup_orchestrator(
        &config,
        &components.llm,
        components.db.as_ref(),
        components.secrets_store.as_ref(),
    )
    .await;
    let container_job_manager = orch.container_job_manager;
    let job_event_tx = orch.job_event_tx;
    let prompt_queue = orch.prompt_queue;
    let docker_status = orch.docker_status;

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
            tracing::debug!("Single message mode");
        } else {
            channel_names.push("repl".to_string());
            tracing::debug!("REPL mode enabled");
        }
    }

    // Collect webhook route fragments; a single WebhookServer hosts them all.
    let mut webhook_routes: Vec<axum::Router> = Vec::new();

    // Load WASM channels and register their webhook routes.
    if config.channels.wasm_channels_enabled && config.channels.wasm_channels_dir.exists() {
        let wasm_result = ironclaw::channels::wasm::setup_wasm_channels(
            &config,
            &components.secrets_store,
            components.extension_manager.as_ref(),
            components.db.as_ref(),
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
        tracing::debug!(
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
        tracing::debug!(
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
    tracing::debug!(
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

    // Lazy scheduler slot — filled after Agent::new creates the Scheduler.
    // Allows CreateJobTool to dispatch local jobs via the Scheduler even though
    // the Scheduler is created after tools are registered (chicken-and-egg).
    let scheduler_slot: ironclaw::tools::builtin::SchedulerSlot =
        Arc::new(tokio::sync::RwLock::new(None));

    // Register job tools (sandbox deps auto-injected when container_job_manager is available)
    components.tools.register_job_tools(
        Arc::clone(&components.context_manager),
        Some(scheduler_slot.clone()),
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
    let mut routine_engine_slot: Option<ironclaw::channels::web::server::RoutineEngineSlot> = None;
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
        gw = gw.with_scheduler(scheduler_slot.clone());
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

        tracing::debug!("Web UI: http://{}:{}/", gw_config.host, gw_config.port);

        // Capture SSE sender and routine engine slot before moving gw into channels.
        // IMPORTANT: This must come after all `with_*` calls since `rebuild_state`
        // creates a new SseManager, which would orphan this sender.
        sse_sender = Some(gw.state().sse.sender());
        routine_engine_slot = Some(Arc::clone(&gw.state().routine_engine));

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
        let active_at_startup: std::collections::HashSet<String> =
            loaded_wasm_channel_names.iter().cloned().collect();
        ext_mgr.set_active_channels(loaded_wasm_channel_names).await;
        ext_mgr
            .set_channel_runtime(
                Arc::clone(&channels),
                rt,
                ps,
                router,
                config.channels.wasm_channel_owner_ids.clone(),
            )
            .await;
        tracing::debug!("Channel runtime wired into extension manager for hot-activation");

        // Auto-activate channels that were active in a previous session.
        let persisted = ext_mgr.load_persisted_active_channels().await;
        for name in &persisted {
            if !active_at_startup.contains(name) {
                match ext_mgr.activate(name).await {
                    Ok(result) => {
                        tracing::debug!(
                            channel = %name,
                            message = %result.message,
                            "Auto-activated persisted channel"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            channel = %name,
                            error = %e,
                            "Failed to auto-activate persisted channel"
                        );
                    }
                }
            }
        }
    }

    // Wire SSE sender into extension manager for broadcasting status events.
    if let Some(ref ext_mgr) = components.extension_manager
        && let Some(ref sender) = sse_sender
    {
        ext_mgr.set_sse_sender(sender.clone()).await;
    }

    // Snapshot memory for trace recording before the agent starts
    if let Some(ref recorder) = components.recording_handle
        && let Some(ref ws) = components.workspace
    {
        recorder.snapshot_memory(ws).await;
    }

    let http_interceptor = components
        .recording_handle
        .as_ref()
        .map(|r| r.http_interceptor());
    // Clone context_manager for the reaper before it's moved into Agent::new()
    let reaper_context_manager = Arc::clone(&components.context_manager);

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
        sse_tx: sse_sender,
        http_interceptor,
        transcription: config
            .transcription
            .create_provider()
            .map(|p| Arc::new(ironclaw::transcription::TranscriptionMiddleware::new(p))),
        document_extraction: Some(Arc::new(
            ironclaw::document_extraction::DocumentExtractionMiddleware::new(),
        )),
    };

    let mut agent = Agent::new(
        config.agent.clone(),
        deps,
        channels,
        Some(config.heartbeat.clone()),
        Some(config.hygiene.clone()),
        Some(config.routines.clone()),
        Some(components.context_manager),
        Some(session_manager),
    );

    // Fill the scheduler slot now that Agent (and its Scheduler) exist.
    *scheduler_slot.write().await = Some(agent.scheduler());

    // Spawn sandbox reaper for orphaned container cleanup
    if let Some(ref jm) = container_job_manager {
        let reaper_jm = Arc::clone(jm);
        let reaper_config = ReaperConfig {
            scan_interval: Duration::from_secs(config.sandbox.reaper_interval_secs),
            orphan_threshold: Duration::from_secs(config.sandbox.orphan_threshold_secs),
            ..ReaperConfig::default()
        };
        let reaper_ctx = Arc::clone(&reaper_context_manager);
        tokio::spawn(async move {
            match SandboxReaper::new(reaper_jm, reaper_ctx, reaper_config).await {
                Ok(reaper) => reaper.run().await,
                Err(e) => tracing::error!("Sandbox reaper failed to initialize: {}", e),
            }
        });
    }

    // Give the agent the routine engine slot so it can expose the engine to the gateway.
    if let Some(slot) = routine_engine_slot {
        agent.set_routine_engine_slot(slot);
    }

    agent.run().await?;

    // ── Shutdown ────────────────────────────────────────────────────────

    // Shut down all stdio MCP server child processes.
    components.mcp_process_manager.shutdown_all().await;

    // Flush LLM trace recording if enabled
    if let Some(ref recorder) = components.recording_handle
        && let Err(e) = recorder.flush().await
    {
        tracing::warn!("Failed to write LLM trace: {}", e);
    }

    if let Some(ref mut server) = webhook_server {
        server.shutdown().await;
    }

    if let Some(tunnel) = active_tunnel {
        tracing::debug!("Stopping {} tunnel...", tunnel.name());
        if let Err(e) = tunnel.stop().await {
            tracing::warn!("Failed to stop tunnel cleanly: {}", e);
        }
    }

    tracing::debug!("Agent shutdown complete");

    Ok(())
}
