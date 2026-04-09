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
    standby::{
        ConfigureCommand, ConfigureFailure, StandbyControl, apply_runtime_config,
        prewarm_runtime_dependencies, resolve_standby_gateway_config, write_persona_files,
    },
    tracing_fmt::{init_cli_tracing, init_worker_tracing},
    webhooks::{self, ToolWebhookState},
};

#[cfg(unix)]
use ironclaw::channels::ChannelSecretUpdater;
#[cfg(any(feature = "postgres", feature = "libsql"))]
use ironclaw::setup::{SetupConfig, SetupWizard};

/// Synchronous entry point. Loads `.env` files before the Tokio runtime
/// starts so that `std::env::set_var` is safe (no worker threads yet).
fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    ironclaw::bootstrap::load_ironclaw_env();

    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main());

    if let Err(ref e) = result {
        format_top_level_error(e);
    }
    result
}

/// Format a top-level error with color and recovery hints.
fn format_top_level_error(err: &anyhow::Error) {
    use ironclaw::cli::fmt;
    let msg = format!("{err:#}");

    eprintln!();
    eprintln!("  {}\u{2717}{} {}", fmt::error(), fmt::reset(), msg);

    // Provide recovery hints for common errors
    let lower = msg.to_ascii_lowercase();
    let hint = if lower.contains("database_url")
        || lower.contains("database") && lower.contains("not set")
    {
        Some("run `ironclaw onboard` or set DATABASE_URL in .env")
    } else if lower.contains("connection refused") || lower.contains("connect error") {
        Some("check that the database server is running")
    } else if lower.contains("session") && lower.contains("not found") {
        Some("run `ironclaw onboard` to set up authentication")
    } else if lower.contains("secrets_master_key") {
        Some("run `ironclaw onboard` or set SECRETS_MASTER_KEY in .env")
    } else if lower.contains("already running") {
        Some("stop the other instance or remove the stale PID file")
    } else if lower.contains("onboard") {
        Some("run `ironclaw onboard` to complete setup")
    } else {
        None
    };

    if let Some(hint_text) = hint {
        eprintln!("  {}hint:{} {}", fmt::dim(), fmt::reset(), hint_text,);
    }
    eprintln!();
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
        Some(Command::Channels(channels_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_channels_command(
                channels_cmd.clone(),
                cli.config.as_deref(),
            )
            .await;
        }
        Some(Command::Routines(routines_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_routines_cli(routines_cmd, cli.config.as_deref()).await;
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
            return run_pairing_command(pairing_cmd.clone()).await;
        }
        Some(Command::Service(service_cmd)) => {
            init_cli_tracing();
            return run_service_command(service_cmd);
        }
        Some(Command::Skills(skills_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_skills_command(skills_cmd.clone(), cli.config.as_deref())
                .await;
        }
        Some(Command::Hooks(hooks_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_hooks_command(hooks_cmd.clone(), cli.config.as_deref())
                .await;
        }
        Some(Command::Logs(logs_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_logs_command(logs_cmd.clone(), cli.config.as_deref()).await;
        }
        Some(Command::Models(models_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_models_command(models_cmd.clone(), cli.config.as_deref())
                .await;
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
        #[cfg(feature = "import")]
        Some(Command::Import(import_cmd)) => {
            init_cli_tracing();
            let config = ironclaw::config::Config::from_env().await?;
            return ironclaw::cli::run_import_command(import_cmd, &config).await;
        }
        Some(Command::Acp(acp_cmd)) => {
            init_cli_tracing();
            return ironclaw::cli::run_acp_command(acp_cmd.clone()).await;
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
        Some(Command::AcpBridge {
            job_id,
            orchestrator_url,
        }) => {
            init_worker_tracing();
            return ironclaw::worker::run_acp_bridge(*job_id, orchestrator_url).await;
        }
        Some(Command::Login { openai_codex }) => {
            init_cli_tracing();
            if *openai_codex {
                // Resolve codex config so OPENAI_CODEX_* env overrides are
                // honoured even when LLM_BACKEND isn't set to openai_codex.
                let codex_config = {
                    let config = Config::from_env()
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e))?;
                    config.llm.openai_codex.unwrap_or_else(|| {
                        use ironclaw::llm::OpenAiCodexConfig;
                        let mut cfg = OpenAiCodexConfig::default();
                        if let Ok(v) = std::env::var("OPENAI_CODEX_AUTH_URL") {
                            cfg.auth_endpoint = v;
                        }
                        if let Ok(v) = std::env::var("OPENAI_CODEX_API_URL") {
                            cfg.api_base_url = v;
                        }
                        if let Ok(v) = std::env::var("OPENAI_CODEX_CLIENT_ID") {
                            cfg.client_id = v;
                        }
                        if let Ok(v) = std::env::var("OPENAI_CODEX_SESSION_PATH") {
                            cfg.session_path = std::path::PathBuf::from(v);
                        }
                        cfg
                    })
                };
                let mgr = ironclaw::llm::OpenAiCodexSessionManager::new(codex_config)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                mgr.device_code_login()
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                println!(
                    "OpenAI Codex authentication complete. Set LLM_BACKEND=openai_codex to use it."
                );
            } else {
                println!("Specify a provider to authenticate with:");
                println!("  ironclaw login --openai-codex   (ChatGPT subscription)");
            }
            return Ok(());
        }
        Some(Command::Onboard {
            skip_auth,
            channels_only,
            provider_only,
            quick,
            step,
        }) => {
            #[cfg(any(feature = "postgres", feature = "libsql"))]
            {
                let config = SetupConfig {
                    skip_auth: *skip_auth,
                    channels_only: *channels_only,
                    provider_only: *provider_only,
                    quick: *quick,
                    steps: step.clone(),
                };
                let mut wizard =
                    SetupWizard::try_with_config_and_toml(config, cli.config.as_deref())?;
                wizard.run().await?;
            }
            #[cfg(not(any(feature = "postgres", feature = "libsql")))]
            {
                let _ = (skip_auth, channels_only, provider_only, quick, step);
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

    let startup_start = std::time::Instant::now();
    let log_broadcaster = Arc::new(LogBroadcaster::new());

    if cli.standby {
        let log_level_handle = ironclaw::channels::web::log_layer::init_tracing(
            Arc::clone(&log_broadcaster),
            false,
        );
        return run_standby(
            &cli,
            startup_start,
            Arc::clone(&log_broadcaster),
            Arc::clone(&log_level_handle),
        )
        .await;
    }

    // ── Agent startup ──────────────────────────────────────────────────

    // Enhanced first-run detection
    #[cfg(any(feature = "postgres", feature = "libsql"))]
    if !cli.no_onboard
        && let Some(reason) = ironclaw::setup::check_onboard_needed()
    {
        println!("Onboarding needed: {}", reason);
        println!();
        let mut wizard = SetupWizard::try_with_config_and_toml(
            SetupConfig {
                quick: true,
                ..Default::default()
            },
            cli.config.as_deref(),
        )?;
        wizard.run().await?;
    }

    // CLI flag overrides for config
    if cli.auto_approve {
        ironclaw::config::set_runtime_env("AGENT_AUTO_APPROVE_TOOLS", "true");
    }
    apply_no_db_config_overrides(&cli);

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
    // Initialize tracing with a reloadable EnvFilter so the gateway can switch
    // log levels at runtime without restarting.
    let suppress_stderr =
        config.channels.tui.is_some() && cli.message.is_none() && cfg!(feature = "tui");
    let log_level_handle = ironclaw::channels::web::log_layer::init_tracing(
        Arc::clone(&log_broadcaster),
        suppress_stderr,
    );

    tracing::debug!("Starting IronClaw...");
    tracing::debug!("Loaded configuration for agent: {}", config.agent.name);
    tracing::debug!("LLM backend: {}", config.llm.backend);

    run_agent_with_config(
        &cli,
        startup_start,
        config,
        toml_path,
        log_broadcaster,
        log_level_handle,
        None,
        false,
    )
    .await
}

fn apply_no_db_config_overrides(cli: &Cli) {
    if !cli.no_db {
        return;
    }

    ironclaw::config::set_runtime_env("DATABASE_BACKEND", "libsql");
    if std::env::var("DATABASE_URL")
        .ok()
        .map(|value| value.trim().is_empty())
        .unwrap_or(true)
    {
        ironclaw::config::set_runtime_env("DATABASE_URL", "unused://libsql");
    }
}

async fn run_standby(
    cli: &Cli,
    startup_start: std::time::Instant,
    log_broadcaster: Arc<LogBroadcaster>,
    log_level_handle: Arc<ironclaw::channels::web::log_layer::LogLevelHandle>,
) -> anyhow::Result<()> {
    let toml_path = cli.config.as_deref();
    let (owner_id, gateway_config) =
        resolve_standby_gateway_config(toml_path).map_err(anyhow::Error::msg)?;
    let (request_tx, mut request_rx) = tokio::sync::mpsc::channel::<ConfigureCommand>(1);

    let mut gateway = GatewayChannel::new(gateway_config.clone(), owner_id);
    let standby_control = StandbyControl::new(gateway.auth_token(), request_tx);
    gateway = gateway.with_standby_control(Arc::clone(&standby_control));
    let bound = gateway.start_server_only().await?;
    standby_control
        .mark_startup_stage("standby.prewarm.start")
        .await;
    let prewarm_start = std::time::Instant::now();
    prewarm_runtime_dependencies(toml_path, cli.no_db)
        .await
        .map_err(anyhow::Error::msg)?;
    ironclaw::bootstrap::log_startup_timing(
        "standby.prewarm_runtime_dependencies",
        prewarm_start.elapsed(),
    );
    standby_control
        .mark_configure_ready("standby.waiting_for_configure")
        .await;

    tracing::info!(
        addr = %bound,
        "Gateway standby mode is ready; waiting for /api/configure"
    );

    let command = request_rx
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("standby configure channel closed unexpectedly"))?;
    standby_control
        .mark_startup_stage("configure.received")
        .await;

    if let Err(message) = apply_runtime_config(&command.request).await {
        standby_control
            .mark_startup_stage("runtime_config.apply_failed")
            .await;
        let _ = command.response_tx.send(Err(ConfigureFailure {
            status: axum::http::StatusCode::BAD_REQUEST,
            message: message.clone(),
        }));
        anyhow::bail!("{message}");
    }
    standby_control
        .mark_startup_stage("runtime_config.applied")
        .await;

    standby_control
        .mark_startup_stage("config.reload.start")
        .await;
    let config_reload_start = std::time::Instant::now();
    let config = match Config::from_env_with_toml(toml_path).await {
        Ok(config) => config,
        Err(error) => {
            ironclaw::bootstrap::log_startup_timing(
                "standby.config_reload",
                config_reload_start.elapsed(),
            );
            standby_control
                .mark_startup_stage("config.reload.failed")
                .await;
            let message = error.to_string();
            let _ = command.response_tx.send(Err(ConfigureFailure {
                status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                message: message.clone(),
            }));
            anyhow::bail!("{message}");
        }
    };
    ironclaw::bootstrap::log_startup_timing("standby.config_reload", config_reload_start.elapsed());
    standby_control
        .mark_startup_stage("config.reload.ready")
        .await;
    tracing::info!(
        agent_id = %command.request.agent_id,
        llm_backend = %config.llm.backend,
        "Standby configure accepted; completing full startup"
    );

    let gateway_state = Arc::clone(gateway.state());
    let persona = command.request.persona.clone();
    standby_control
        .mark_startup_stage("agent.bootstrap.start")
        .await;
    let mut agent_future = std::pin::pin!(run_agent_with_config(
        cli,
        startup_start,
        config,
        toml_path,
        log_broadcaster,
        log_level_handle,
        Some((gateway, persona)),
        true,
    ));
    let mut runtime_ready = std::pin::pin!(standby_control.wait_for_runtime_started());

    let startup_result = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            tokio::select! {
                () = runtime_ready.as_mut() => return Ok(()),
                joined = agent_future.as_mut() => return joined,
            }
        }
    })
    .await;

    match startup_result {
        Ok(Ok(())) => {
            let _ = command.response_tx.send(Ok(()));
        }
        Ok(Err(error)) => {
            standby_control
                .mark_startup_stage("agent.bootstrap.failed")
                .await;
            let _ = command.response_tx.send(Err(ConfigureFailure {
                status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                message: error.to_string(),
            }));
            return Err(error);
        }
        Err(_) => {
            standby_control
                .mark_startup_stage("agent.bootstrap.timeout")
                .await;
            let snapshot = standby_control.startup_snapshot().await;
            let diagnostic = serde_json::json!({
                "lastStage": snapshot.last_stage,
                "runtimeStarted": snapshot.runtime_started,
                "serverStarted": gateway_state.server_started.load(std::sync::atomic::Ordering::Relaxed),
                "hasMsgTx": gateway_state.msg_tx.read().await.is_some(),
                "hasSessionManager": gateway_state.session_manager().is_some(),
                "hasLlmProvider": gateway_state.llm_provider().is_some(),
                "hasWorkspace": gateway_state.workspace().is_some(),
                "activeConfig": gateway_state.active_config_snapshot(),
            });
            tracing::error!(diagnostic = %diagnostic, "standby startup timed out");
            let _ = command.response_tx.send(Err(ConfigureFailure {
                status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                message: format!(
                    "timed out waiting for configured gateway startup (last stage: {})",
                    snapshot.last_stage
                ),
            }));
            anyhow::bail!(
                "timed out waiting for configured gateway startup (last stage: {})",
                snapshot.last_stage
            );
        }
    }

    agent_future.as_mut().await
}

async fn run_agent_with_config(
    cli: &Cli,
    startup_start: std::time::Instant,
    config: Config,
    toml_path: Option<&std::path::Path>,
    log_broadcaster: Arc<LogBroadcaster>,
    log_level_handle: Arc<ironclaw::channels::web::log_layer::LogLevelHandle>,
    mut prestarted_gateway: Option<(GatewayChannel, ironclaw::standby::TidePoolConfigurePersona)>,
    standby_db_prewarmed: bool,
) -> anyhow::Result<()> {
    let standby_control = prestarted_gateway
        .as_ref()
        .and_then(|(gateway, _)| gateway.state().standby_control.clone());

    if let Some(control) = standby_control.as_ref() {
        control
            .mark_startup_stage("agent.session_manager.start")
            .await;
    }
    // Initialize session manager before channel setup
    let session = create_session_manager(config.llm.session.clone()).await;
    if let Some(control) = standby_control.as_ref() {
        control
            .mark_startup_stage("agent.session_manager.ready")
            .await;
        control.mark_startup_stage("app_builder.start").await;
    }

    // ── Phase 1-5: Build all core components via AppBuilder ────────────

    let flags = AppBuilderFlags {
        no_db: cli.no_db,
        skip_db_migrations: standby_db_prewarmed,
    };
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

    if let Some((_, ref persona)) = prestarted_gateway
        && let Some(ref workspace) = components.workspace
    {
        write_persona_files(workspace, persona)
            .await
            .map_err(anyhow::Error::msg)?;
    }

    if let Some(ref workspace) = components.workspace {
        match workspace.system_prompt().await {
            Ok(prompt) if !prompt.is_empty() => {
                tracing::info!(
                    user_id = %workspace.user_id(),
                    prompt_len = prompt.len(),
                    prompt_fingerprint = %ironclaw::workspace::prompt_fingerprint(&prompt),
                    "Workspace system prompt snapshot ready after startup configuration"
                );
            }
            Ok(_) => {
                tracing::debug!(
                    user_id = %workspace.user_id(),
                    "Workspace system prompt absent after startup configuration"
                );
            }
            Err(error) => {
                tracing::warn!(
                    user_id = %workspace.user_id(),
                    "Failed to load workspace system prompt after startup configuration: {}",
                    error
                );
            }
        }
    }
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

    // Derive user-facing warning from docker_status for channel notification
    let docker_user_warning: Option<String> = match docker_status {
        ironclaw::sandbox::DockerStatus::NotInstalled => Some(
            "Sandbox is enabled but Docker is not installed -- \
             full_job routines will fail until Docker is available."
                .to_string(),
        ),
        ironclaw::sandbox::DockerStatus::NotRunning => Some(
            "Sandbox is enabled but Docker is not running -- \
             full_job routines will fail until Docker is started."
                .to_string(),
        ),
        _ => None,
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

    // Create CLI channel (REPL or TUI — mutually exclusive, both claim stdin)
    let tui_mode = config.channels.tui.is_some();

    #[cfg(feature = "tui")]
    if tui_mode && cli.message.is_none() {
        let tool_names = components.tools.list().await;
        let tool_categories = ironclaw::channels::tui::group_tools_by_prefix(tool_names);

        let skill_categories = if let Some(ref registry) = components.skill_registry {
            let registry = registry.read().unwrap_or_else(|e| e.into_inner());
            let skill_data: Vec<(String, Vec<String>)> = registry
                .skills()
                .iter()
                .map(|s| (s.manifest.name.clone(), s.manifest.activation.tags.clone()))
                .collect();
            ironclaw::channels::tui::group_skills_by_tag(&skill_data)
        } else {
            Vec::new()
        };

        let workspace_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::new());
        let workspace_path = workspace_root.display().to_string();
        let layout = if let Some(ref tui_config) = config.channels.tui {
            ironclaw::channels::tui::resolve_tui_layout(tui_config, &workspace_root)
        } else {
            ironclaw_tui::TuiLayout::default()
        };

        let (memory_count, identity_files) = if let Some(ref ws) = components.workspace {
            let count = ws.list_all().await.map(|docs| docs.len()).unwrap_or(0);
            let identity_names = ["AGENTS.md", "SOUL.md", "USER.md", "IDENTITY.md"];
            let mut found = Vec::new();
            for name in &identity_names {
                if ws.read(name).await.is_ok() {
                    found.push((*name).to_string());
                }
            }
            (count, found)
        } else {
            (0, Vec::new())
        };

        let current_model = components.llm.model_name().to_string();
        let context_window =
            match tokio::time::timeout(Duration::from_secs(5), components.llm.model_metadata())
                .await
            {
                Ok(Ok(metadata)) => metadata.context_length.map(u64::from),
                Ok(Err(e)) => {
                    tracing::debug!(
                        "TUI context metadata unavailable: could not fetch model metadata: {}",
                        e
                    );
                    None
                }
                Err(_) => {
                    tracing::debug!("TUI context metadata unavailable: model metadata timed out");
                    None
                }
            };
        let available_models = match tokio::time::timeout(
            Duration::from_secs(5),
            components.llm.list_models(),
        )
        .await
        {
            Ok(Ok(mut models)) if !models.is_empty() => {
                if let Some(pos) = models.iter().position(|m| m == &current_model) {
                    if pos != 0 {
                        let current = models.remove(pos);
                        models.insert(0, current);
                    }
                } else {
                    models.insert(0, current_model.clone());
                }
                models
            }
            Ok(Ok(_)) => Vec::new(),
            Ok(Err(e)) => {
                tracing::debug!("TUI model picker unavailable: could not list models: {}", e);
                Vec::new()
            }
            Err(_) => {
                tracing::debug!("TUI model picker unavailable: model discovery timed out");
                Vec::new()
            }
        };

        let tui_channel = ironclaw::channels::TuiChannel::new(
            config.owner_id.clone(),
            env!("CARGO_PKG_VERSION"),
            current_model,
        )
        .with_context_window(context_window.unwrap_or(128_000))
        .with_layout(layout)
        .with_log_broadcaster(Arc::clone(&log_broadcaster))
        .with_tools(tool_categories)
        .with_skills(skill_categories)
        .with_workspace_path(workspace_path)
        .with_memory_count(memory_count)
        .with_identity_files(identity_files)
        .with_available_models(available_models);

        channels.add(Box::new(tui_channel)).await;
        channel_names.push("tui".to_string());
        tracing::debug!("TUI mode enabled");
    }

    #[cfg(not(feature = "tui"))]
    if tui_mode {
        tracing::warn!(
            "CLI_MODE=tui requested but the 'tui' feature is not enabled. Falling back to REPL."
        );
    }

    let use_repl = !tui_mode || cfg!(not(feature = "tui"));
    let repl_channel = if let Some(ref msg) = cli.message {
        Some(ReplChannel::with_message_for_user(
            config.owner_id.clone(),
            msg.clone(),
        ))
    } else if use_repl && config.channels.cli.enabled {
        let repl = ReplChannel::with_user_id(config.owner_id.clone());
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

    // Shared routine engine slot for gateway + generic webhook ingress.
    let shared_routine_engine_slot: ironclaw::channels::web::server::RoutineEngineSlot =
        Arc::new(tokio::sync::RwLock::new(None));

    // Collect webhook route fragments; a single WebhookServer hosts them all.
    let mut webhook_routes: Vec<axum::Router> = Vec::new();

    webhook_routes.push(webhooks::routes(ToolWebhookState {
        tools: Arc::clone(&components.tools),
        routine_engine: Arc::clone(&shared_routine_engine_slot),
        user_id: config.owner_id.clone(),
        secrets_store: components.secrets_store.clone(),
    }));

    // Load WASM channels and register their webhook routes.
    // Ensure the channels directory exists so the WASM runtime initializes even when
    // no channels are installed yet — hot-activation needs the runtime to be available.
    if config.channels.wasm_channels_enabled
        && let Err(e) = std::fs::create_dir_all(&config.channels.wasm_channels_dir)
    {
        tracing::warn!(
            path = %config.channels.wasm_channels_dir.display(),
            error = %e,
            "Failed to create WASM channels directory"
        );
    }
    if config.channels.wasm_channels_enabled && config.channels.wasm_channels_dir.exists() {
        let wasm_result = ironclaw::channels::wasm::setup_wasm_channels(
            &config,
            &components.secrets_store,
            components.extension_manager.as_ref(),
            components.db.as_ref(),
            &channel_names,
            Arc::clone(&components.ownership_cache),
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
        let signal_channel = SignalChannel::new(
            signal_config.clone(),
            components.db.clone(),
            Arc::clone(&components.ownership_cache),
        )?;
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

    // Add DingTalk channel if configured and not CLI-only mode.
    if !cli.cli_only
        && let Some(ref dingtalk_config) = config.channels.dingtalk
    {
        if dingtalk_config.enabled {
            let dingtalk_channel = ironclaw::channels::DingTalkChannel::new(dingtalk_config.clone())
                .map_err(|e| anyhow::anyhow!("DingTalk channel init failed: {e}"))?;
            channel_names.push("dingtalk".to_string());
            channels.add(Box::new(dingtalk_channel)).await;
            tracing::info!(
                client_id = %dingtalk_config.client_id,
                "DingTalk channel enabled (Stream mode)"
            );
        }
    }

    // Add HTTP channel if configured and not CLI-only mode.
    let mut webhook_server_addr: Option<std::net::SocketAddr> = None;
    #[cfg(unix)]
    let mut http_channel_state: Option<Arc<ironclaw::channels::HttpChannelState>> = None;
    if !cli.cli_only
        && let Some(ref http_config) = config.channels.http
    {
        let http_channel = HttpChannel::new(http_config.clone());
        #[cfg(unix)]
        {
            http_channel_state = Some(http_channel.shared_state());
        }
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
    let webhook_server: Option<Arc<tokio::sync::Mutex<WebhookServer>>> = if !webhook_routes
        .is_empty()
    {
        let addr = webhook_server_addr
            .unwrap_or_else(|| std::net::SocketAddr::from(([127, 0, 0, 1], 8080)));
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
        Some(Arc::new(tokio::sync::Mutex::new(server)))
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

    // Reuse the shared agent session manager prepared by AppBuilder.
    let session_manager = Arc::clone(&components.agent_session_manager);

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
    let mut sse_manager: Option<std::sync::Arc<ironclaw::channels::web::sse::SseManager>> = None;
    if let Some(ref gw_config) = config.channels.gateway {
        let mut gw = GatewayChannel::new(gw_config.clone(), config.owner_id.clone());
        gw = gw.with_llm_provider(Arc::clone(&components.llm));
        if let Some(ref ws) = components.workspace {
            gw = gw.with_workspace(Arc::clone(ws));
        }
        // Create per-user workspace pool for multi-user mode.
        if let Some(ref db) = components.db {
            let emb_cache_config = ironclaw::workspace::EmbeddingCacheConfig {
                max_entries: config.embeddings.cache_size,
            };
            let pool = Arc::new(ironclaw::channels::web::server::WorkspacePool::new(
                Arc::clone(db),
                components.embeddings.clone(),
                emb_cache_config,
                config.search.clone(),
                config.workspace.clone(),
            ));
            gw = gw.with_workspace_pool(pool);
        }
        gw = gw.with_session_manager(Arc::clone(&session_manager));
        gw = gw.with_log_broadcaster(Arc::clone(&log_broadcaster));
        gw = gw.with_log_level_handle(Arc::clone(&log_level_handle));
        gw = gw.with_tool_registry(Arc::clone(&components.tools));
        if let Some(ref ext_mgr) = components.extension_manager {
            // Enable gateway mode so MCP OAuth returns auth URLs to the frontend
            // instead of calling open::that() on the server.
            let gw_base = config
                .tunnel
                .public_url
                .clone()
                .unwrap_or_else(|| format!("http://{}:{}", gw_config.host, gw_config.port));
            ext_mgr.enable_gateway_mode(gw_base).await;
            gw = gw.with_extension_manager(Arc::clone(ext_mgr));
        }
        if !components.catalog_entries.is_empty() {
            gw = gw.with_registry_entries(components.catalog_entries.clone());
        }
        if let Some(ref d) = components.db {
            gw = gw.with_store(Arc::clone(d));
            gw = gw.with_db_auth(Arc::clone(d));
            let pairing_store = Arc::new(ironclaw::pairing::PairingStore::new(
                Arc::clone(d),
                Arc::clone(&components.ownership_cache),
            ));
            gw = gw.with_pairing_store(pairing_store);
            if let Some(ref ss) = components.secrets_store {
                gw = gw.with_secrets_store(Arc::clone(ss));
            }

            // Bootstrap: create the first admin user from single-user config
            // so the owner appears in the Users admin panel immediately.
            if let Ok(false) = d.has_any_users().await {
                let now = chrono::Utc::now();
                let user = ironclaw::db::UserRecord {
                    id: config.owner_id.clone(),
                    email: None,
                    display_name: config.owner_id.clone(),
                    status: "active".to_string(),
                    role: "admin".to_string(),
                    created_at: now,
                    updated_at: now,
                    last_login_at: None,
                    created_by: None,
                    metadata: serde_json::json!({"source": "bootstrap"}),
                };
                // Create admin user + bootstrap token atomically.
                let auth_token = gw.auth_token();
                if auth_token.is_empty() {
                    if let Err(e) = d.create_user(&user).await {
                        tracing::warn!("Failed to bootstrap admin user: {}", e);
                    }
                } else {
                    use ironclaw::channels::web::auth::hash_token;
                    let hash = hash_token(auth_token);
                    let prefix = if auth_token.len() >= 8 {
                        &auth_token[..8]
                    } else {
                        auth_token
                    };
                    if let Err(e) = d
                        .create_user_with_token(&user, "bootstrap", &hash, prefix, None)
                        .await
                    {
                        tracing::warn!("Failed to bootstrap admin user: {}", e);
                    } else {
                        tracing::debug!(
                            user_id = config.owner_id,
                            "Bootstrapped admin user from gateway config"
                        );
                    }
                }
            }
        }
        if let Some(ref ss) = components.secrets_store {
            gw = gw.with_secrets_store(Arc::clone(ss));
        }
        if let Some(ref jm) = container_job_manager {
            gw = gw.with_job_manager(Arc::clone(jm));
        }
        gw = gw.with_scheduler(scheduler_slot.clone());
        gw = gw.with_routine_engine_slot(Arc::clone(&shared_routine_engine_slot));
        if let Some(ref sr) = components.skill_registry {
            gw = gw.with_skill_registry(Arc::clone(sr));
        }
        if let Some(ref sc) = components.skill_catalog {
            gw = gw.with_skill_catalog(Arc::clone(sc));
        }
        gw = gw.with_cost_guard(Arc::clone(&components.cost_guard));
        gw = gw.with_oauth(config.oauth.clone(), gw_config.port);
        {
            let active_model = components.llm.model_name().to_string();
            let mut enabled = channel_names.clone();
            enabled.push("gateway".into());
            gw = gw.with_active_config(ironclaw::channels::web::server::ActiveConfigSnapshot {
                llm_backend: config.llm.backend.to_string(),
                llm_model: active_model,
                enabled_channels: enabled,
            });
        }
        if config.sandbox.enabled {
            gw = gw.with_prompt_queue(Arc::clone(&prompt_queue));

            if let Some(ref tx) = job_event_tx {
                let mut rx = tx.subscribe();
                let gw_state = Arc::clone(gw.state());
                tokio::spawn(async move {
                    while let Ok((_job_id, user_id, event)) = rx.recv().await {
                        if user_id.is_empty() {
                            gw_state.sse.broadcast(event);
                        } else {
                            gw_state.sse.broadcast_for_user(&user_id, event);
                        }
                    }
                });
            }
        }

        // Persist auto-generated auth token so it survives restarts.
        // Gateway auth is env-only, so write to bootstrap `.env` rather than DB
        // settings and opportunistically remove any legacy DB copy.
        if gw_config.auth_token.is_none() {
            let token_to_persist = gw.auth_token().to_string();
            tokio::spawn(async move {
                if let Err(e) = ironclaw::bootstrap::upsert_bootstrap_var(
                    "GATEWAY_AUTH_TOKEN",
                    &token_to_persist,
                ) {
                    tracing::warn!("Failed to persist auto-generated gateway auth token: {e}");
                } else {
                    tracing::debug!("Persisted auto-generated gateway auth token to bootstrap env");
                }
            });

            if let Some(ref db) = components.db {
                let db = db.clone();
                tokio::spawn(async move {
                    match db
                        .delete_setting("default", "channels.gateway_auth_token")
                        .await
                    {
                        Ok(true) => {
                            tracing::debug!("Removed legacy gateway auth token from DB settings");
                        }
                        Ok(false) => {}
                        Err(e) => {
                            tracing::warn!(
                                "Failed to remove legacy gateway auth token from DB settings: {e}"
                            );
                        }
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
        sse_manager = Some(Arc::clone(&gw.state().sse));
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
            acp_enabled: config.acp.enabled,
            routines_enabled: config.routines.enabled,
            skills_enabled: config.skills.enabled,
            channels: channel_names,
            tunnel_url: active_tunnel
                .as_ref()
                .and_then(|t| t.public_url())
                .or_else(|| config.tunnel.public_url.clone()),
            tunnel_provider: active_tunnel.as_ref().map(|t| t.name().to_string()),
            startup_elapsed: Some(startup_start.elapsed()),
        };
        ironclaw::boot_screen::print_boot_screen(&boot_info);
    }

    // ── Run the agent ──────────────────────────────────────────────────

    let channels = Arc::new(channels);

    // Register message tool for sending messages to connected channels
    components
        .tools
        .register_message_tools(Arc::clone(&channels), components.extension_manager.clone())
        .await;

    // Default user ID for extension operations (single-user mode).
    let ext_user_id = config.owner_id.clone();

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

        // Auto-activate WASM channels that were active in a previous session.
        // Relay channels are handled separately below via restore_relay_channels().
        let persisted = ext_mgr.load_persisted_active_channels(&ext_user_id).await;
        for name in &persisted {
            if active_at_startup.contains(name)
                || ext_mgr.is_relay_channel(name, &ext_user_id).await
            {
                continue;
            }
            match ext_mgr.activate(name, &ext_user_id).await {
                Ok(result) => {
                    tracing::debug!(
                        channel = %name,
                        message = %result.message,
                        "Auto-activated persisted WASM channel"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        channel = %name,
                        error = %e,
                        "Failed to auto-activate persisted WASM channel"
                    );
                }
            }
        }
    }

    // Ensure the relay channel manager is always set (even without WASM runtime),
    // then restore any persisted relay channels.
    if let Some(ref ext_mgr) = components.extension_manager {
        ext_mgr
            .set_relay_channel_manager(Arc::clone(&channels))
            .await;
        ext_mgr.restore_relay_channels(&ext_user_id).await;
    }

    // Wire SSE sender into extension manager for broadcasting status events.
    if let Some(ref ext_mgr) = components.extension_manager
        && let Some(ref sse) = sse_manager
    {
        ext_mgr.set_sse_sender(Arc::clone(sse)).await;
    }

    // Wire SSE into plan_update tool for live plan progress broadcasting.
    if let Some(ref sse) = sse_manager {
        components.tools.register_plan_tools(Some(Arc::clone(sse)));
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

    // Capture db reference for SIGHUP handler before it's moved into AgentDeps (Unix only)
    #[cfg(unix)]
    let sighup_settings_store: Option<Arc<dyn ironclaw::db::SettingsStore>> = components
        .db
        .as_ref()
        .map(|db| Arc::clone(db) as Arc<dyn ironclaw::db::SettingsStore>);

    // Capture db reference for WAL checkpoint on shutdown
    let shutdown_db = components.db.as_ref().map(Arc::clone);

    let deps = AgentDeps {
        owner_id: config.owner_id.clone(),
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
        sse_tx: sse_manager,
        http_interceptor,
        transcription: config.transcription.create_provider().map(|p| {
            Arc::new(ironclaw::llm::transcription::TranscriptionMiddleware::new(
                p,
            ))
        }),
        document_extraction: Some(Arc::new(
            ironclaw::document_extraction::DocumentExtractionMiddleware::new(),
        )),
        sandbox_readiness: if !config.sandbox.enabled {
            ironclaw::agent::routine_engine::SandboxReadiness::DisabledByConfig
        } else if docker_status.is_ok() {
            ironclaw::agent::routine_engine::SandboxReadiness::Available
        } else {
            ironclaw::agent::routine_engine::SandboxReadiness::DockerUnavailable
        },
        builder: components.builder,
        llm_backend: config.llm.backend.clone(),
        tenant_rates: Arc::new(ironclaw::tenant::TenantRateRegistry::new(
            config.agent.max_llm_concurrent_per_user.unwrap_or(4),
            config.agent.max_jobs_concurrent_per_user.unwrap_or(3),
        )),
    };

    let channels_for_warnings = Arc::clone(&channels);
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
    agent.set_routine_engine_slot(shared_routine_engine_slot);

    // Prepare SIGHUP handler for hot-reloading HTTP webhook config
    // Broadcast channel for clean shutdown of background tasks
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    #[cfg(unix)]
    {
        // Collect all channels that support secret updates
        let mut secret_updaters: Vec<Arc<dyn ChannelSecretUpdater>> = Vec::new();
        if let Some(ref state) = http_channel_state {
            secret_updaters.push(Arc::clone(state) as Arc<dyn ChannelSecretUpdater>);
        }

        let sighup_webhook_server = webhook_server.clone();
        let sighup_settings_store_clone = sighup_settings_store.clone();
        let sighup_secrets_store = components.secrets_store.clone();
        let sighup_owner_id = config.owner_id.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();

        tokio::spawn(async move {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sighup = match signal(SignalKind::hangup()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to register SIGHUP handler: {}", e);
                    return;
                }
            };

            loop {
                // Exit loop on shutdown signal or when SIGHUP is received
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        tracing::debug!("SIGHUP handler shutting down");
                        break;
                    }
                    _ = sighup.recv() => {
                        // Handle SIGHUP signal
                    }
                }
                tracing::info!("SIGHUP received — reloading HTTP webhook config");

                // Inject channel secrets from database into thread-safe overlay
                // (similar to inject_llm_keys_from_secrets for LLM providers)
                if let Some(ref secrets_store) = sighup_secrets_store {
                    // Inject HTTP webhook secret from encrypted store
                    if let Ok(webhook_secret) = secrets_store
                        .get_decrypted(&sighup_owner_id, "http_webhook_secret")
                        .await
                    {
                        // Thread-safe: Uses INJECTED_VARS mutex instead of unsafe std::env::set_var
                        // Config::from_env() will read from the overlay via optional_env()
                        ironclaw::config::inject_single_var(
                            "HTTP_WEBHOOK_SECRET",
                            webhook_secret.expose(),
                        );
                        tracing::debug!("Injected HTTP_WEBHOOK_SECRET from secrets store");
                    }
                }

                // Reload config (now with secrets injected into environment)
                let new_config = match &sighup_settings_store_clone {
                    Some(store) => {
                        ironclaw::config::Config::from_db(store.as_ref(), &sighup_owner_id).await
                    }
                    None => ironclaw::config::Config::from_env().await,
                };

                let new_config = match new_config {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("SIGHUP config reload failed: {}", e);
                        continue;
                    }
                };

                let new_http = match new_config.channels.http {
                    Some(c) => c,
                    None => {
                        tracing::warn!("SIGHUP: HTTP channel no longer configured, skipping");
                        continue;
                    }
                };

                // Compute new socket addr
                let new_addr: std::net::SocketAddr =
                    match format!("{}:{}", new_http.host, new_http.port).parse() {
                        Ok(a) => a,
                        Err(e) => {
                            tracing::error!("SIGHUP: invalid addr in config: {}", e);
                            continue;
                        }
                    };

                // Restart listener if addr changed.
                // Two-phase approach: bind outside the lock, then swap under lock.
                let mut restart_failed = false;
                if let Some(ref ws_arc) = sighup_webhook_server {
                    let (old_addr, router) = {
                        let ws = ws_arc.lock().await;
                        (ws.current_addr(), ws.merged_router_clone())
                    }; // Lock released here

                    if old_addr != new_addr {
                        tracing::info!(
                            "SIGHUP: HTTP addr {} -> {}, restarting listener",
                            old_addr,
                            new_addr
                        );

                        match router {
                            Some(app) => {
                                // Phase 1: Bind new listener WITHOUT holding the lock.
                                match tokio::net::TcpListener::bind(new_addr).await {
                                    Ok(listener) => {
                                        // Phase 2: Swap state under lock (no await inside).
                                        let (old_tx, old_handle) = {
                                            let mut ws = ws_arc.lock().await;
                                            ws.install_listener(new_addr, listener, app)
                                        }; // Lock released here

                                        // Phase 3: Shut down old listener outside the lock.
                                        if let Some(tx) = old_tx {
                                            let _ = tx.send(());
                                        }
                                        if let Some(handle) = old_handle {
                                            let _ = handle.await;
                                        }

                                        tracing::info!(
                                            "SIGHUP: webhook server restarted on {}",
                                            new_addr
                                        );
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "SIGHUP: failed to bind to {}: {}",
                                            new_addr,
                                            e
                                        );
                                        restart_failed = true;
                                    }
                                }
                            }
                            None => {
                                tracing::error!(
                                    "SIGHUP: cannot restart — server was never started"
                                );
                                restart_failed = true;
                            }
                        }
                    } else {
                        tracing::debug!("SIGHUP: addr unchanged ({})", old_addr);
                    }
                }

                // Update secrets in all configured channels (if restart succeeded or wasn't needed)
                if !restart_failed {
                    use secrecy::{ExposeSecret, SecretString};
                    let new_secret = new_http
                        .webhook_secret
                        .as_ref()
                        .map(|s| SecretString::from(s.expose_secret().to_string()));

                    // Update all channels that support secret swapping
                    for updater in &secret_updaters {
                        updater.update_secret(new_secret.clone()).await;
                    }
                }
            }
        });
    }

    // Notify user if sandbox is unavailable (Docker missing/not running)
    if let Some(warning) = docker_user_warning {
        let channels_ref = Arc::clone(&channels_for_warnings);
        tokio::spawn(async move {
            // Delay to let channels finish connecting before sending the warning.
            // 5s is generous but avoids the message being lost on slow startups.
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            tracing::debug!("Sending sandbox-unavailable warning to connected channels");
            let response = ironclaw::channels::OutgoingResponse {
                content: format!("Warning: {warning}"),
                thread_id: None,
                attachments: Vec::new(),
                metadata: serde_json::json!({
                    "source": "system",
                    "type": "warning",
                }),
            };
            let _ = channels_ref.broadcast_all("default", response).await;
        });
    }

    agent.run().await?;

    // ── Shutdown ────────────────────────────────────────────────────────

    // Checkpoint SQLite WAL so the .db file is self-contained before the
    // container stops and the orchestrator uploads it to S3 for persistence.
    if let Some(ref db) = shutdown_db {
        match db.checkpoint().await {
            Ok(()) => tracing::info!("SQLite WAL checkpoint completed"),
            Err(e) => tracing::warn!("SQLite WAL checkpoint failed: {}", e),
        }
    }

    // Signal background tasks (SIGHUP handler, etc.) to gracefully shut down
    let _ = shutdown_tx.send(());

    // Shut down all stdio MCP server child processes.
    components.mcp_process_manager.shutdown_all().await;

    // Flush LLM trace recording if enabled
    if let Some(ref recorder) = components.recording_handle
        && let Err(e) = recorder.flush().await
    {
        tracing::warn!("Failed to write LLM trace: {}", e);
    }

    if let Some(ref ws_arc) = webhook_server {
        let (shutdown_tx, handle) = {
            let mut ws = ws_arc.lock().await;
            ws.begin_shutdown()
        };
        if let Some(tx) = shutdown_tx {
            let _ = tx.send(());
        }
        if let Some(handle) = handle {
            let _ = handle.await;
        }
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
