//! IronClaw - Main entry point.

use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use ironclaw::{
    agent::{Agent, AgentDeps, SessionManager},
    channels::{
        ChannelManager, GatewayChannel, HttpChannel, ReplChannel, WebhookServer,
        WebhookServerConfig,
        wasm::{
            RegisteredEndpoint, SharedWasmChannel, WasmChannelLoader, WasmChannelRouter,
            WasmChannelRuntime, WasmChannelRuntimeConfig, create_wasm_channel_router,
        },
        web::log_layer::{LogBroadcaster, WebLogLayer},
    },
    cli::{
        Cli, Command, run_mcp_command, run_pairing_command, run_service_command,
        run_status_command, run_tool_command,
    },
    config::Config,
    context::ContextManager,
    extensions::ExtensionManager,
    hooks::{HookRegistry, bootstrap_hooks},
    llm::{
        CachedProvider, CircuitBreakerConfig, CircuitBreakerProvider, CooldownConfig,
        FailoverProvider, LlmProvider, ResponseCacheConfig, RetryConfig, RetryProvider,
        SessionConfig, create_cheap_llm_provider, create_llm_provider,
        create_llm_provider_with_config, create_session_manager,
    },
    orchestrator::{
        ContainerJobConfig, ContainerJobManager, OrchestratorApi, TokenStore,
        api::OrchestratorState,
    },
    pairing::PairingStore,
    safety::SafetyLayer,
    secrets::SecretsStore,
    tools::{
        ToolRegistry,
        mcp::{McpClient, McpSessionManager, config::load_mcp_servers_from_db, is_authenticated},
        wasm::{WasmToolLoader, WasmToolRuntime, load_dev_tools},
    },
    workspace::{
        EmbeddingProvider, NearAiEmbeddings, OllamaEmbeddings, OpenAiEmbeddings, Workspace,
    },
};

#[cfg(feature = "libsql")]
use ironclaw::secrets::LibSqlSecretsStore;
#[cfg(feature = "postgres")]
use ironclaw::secrets::PostgresSecretsStore;
use ironclaw::secrets::SecretsCrypto;
#[cfg(any(feature = "postgres", feature = "libsql"))]
use ironclaw::setup::{SetupConfig, SetupWizard};
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Handle non-agent commands first (they don't need full setup)
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
        Some(Command::Config(config_cmd)) => {
            // Config commands need DB access for settings
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
                )
                .init();

            return ironclaw::cli::run_config_command(config_cmd.clone()).await;
        }
        Some(Command::Mcp(mcp_cmd)) => {
            // Simple logging for MCP commands
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
                )
                .init();

            return run_mcp_command(mcp_cmd.clone()).await;
        }
        Some(Command::Memory(mem_cmd)) => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
                )
                .init();

            // Memory commands need database (and optionally embeddings)
            let config = Config::from_env()
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            // Set up embeddings if available
            let session = ironclaw::llm::create_session_manager(ironclaw::llm::SessionConfig {
                auth_base_url: config.llm.nearai.auth_base_url.clone(),
                session_path: config.llm.nearai.session_path.clone(),
            })
            .await;

            let embeddings: Option<Arc<dyn ironclaw::workspace::EmbeddingProvider>> =
                if config.embeddings.enabled {
                    match config.embeddings.provider.as_str() {
                        "nearai" => Some(Arc::new(
                            ironclaw::workspace::NearAiEmbeddings::new(
                                &config.llm.nearai.base_url,
                                session,
                            )
                            .with_model(&config.embeddings.model, config.embeddings.dimension),
                        )),
                        "ollama" => Some(Arc::new(
                            ironclaw::workspace::OllamaEmbeddings::new(
                                &config.embeddings.ollama_base_url,
                            )
                            .with_model(&config.embeddings.model, config.embeddings.dimension),
                        )),
                        _ => {
                            if let Some(api_key) = config.embeddings.openai_api_key() {
                                Some(Arc::new(ironclaw::workspace::OpenAiEmbeddings::with_model(
                                    api_key,
                                    &config.embeddings.model,
                                    config.embeddings.dimension,
                                )))
                            } else {
                                None
                            }
                        }
                    }
                } else {
                    None
                };

            // Warn if libSQL backend is used with non-1536 embedding dimension.
            // libSQL schema uses F32_BLOB(1536) which cannot be altered without a
            // table rebuild, so non-1536 embeddings will cause storage failures.
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

            // Create a Database-trait-backed workspace for the memory command
            let db: Arc<dyn ironclaw::db::Database> =
                ironclaw::db::connect_from_config(&config.database)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;

            return ironclaw::cli::run_memory_command_with_db(mem_cmd.clone(), db, embeddings)
                .await;
        }
        Some(Command::Pairing(pairing_cmd)) => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
                )
                .init();

            return run_pairing_command(pairing_cmd.clone()).map_err(|e| anyhow::anyhow!("{}", e));
        }
        Some(Command::Service(service_cmd)) => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
                )
                .init();

            return run_service_command(service_cmd);
        }
        Some(Command::Doctor) => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
                )
                .init();

            return ironclaw::cli::run_doctor_command().await;
        }
        Some(Command::Status) => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
                )
                .init();

            return run_status_command().await;
        }
        Some(Command::Worker {
            job_id,
            orchestrator_url,
            max_iterations,
        }) => {
            // Worker mode: runs inside a Docker container.
            // Simple logging (no TUI, no DB, no channels).
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| EnvFilter::new("ironclaw=info")),
                )
                .init();

            tracing::info!(
                "Starting worker for job {} (orchestrator: {})",
                job_id,
                orchestrator_url
            );

            let config = ironclaw::worker::runtime::WorkerConfig {
                job_id: *job_id,
                orchestrator_url: orchestrator_url.clone(),
                max_iterations: *max_iterations,
                timeout: std::time::Duration::from_secs(600),
            };

            let runtime = ironclaw::worker::WorkerRuntime::new(config)
                .map_err(|e| anyhow::anyhow!("Worker init failed: {}", e))?;

            runtime
                .run()
                .await
                .map_err(|e| anyhow::anyhow!("Worker failed: {}", e))?;

            return Ok(());
        }
        Some(Command::ClaudeBridge {
            job_id,
            orchestrator_url,
            max_turns,
            model,
        }) => {
            // Claude Code bridge mode: runs inside a Docker container.
            // Spawns the `claude` CLI and streams output to the orchestrator.
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| EnvFilter::new("ironclaw=info")),
                )
                .init();

            tracing::info!(
                "Starting Claude Code bridge for job {} (orchestrator: {}, model: {})",
                job_id,
                orchestrator_url,
                model
            );

            let config = ironclaw::worker::claude_bridge::ClaudeBridgeConfig {
                job_id: *job_id,
                orchestrator_url: orchestrator_url.clone(),
                max_turns: *max_turns,
                model: model.clone(),
                timeout: std::time::Duration::from_secs(1800),
                allowed_tools: ironclaw::config::ClaudeCodeConfig::from_env().allowed_tools,
            };

            let runtime = ironclaw::worker::ClaudeBridgeRuntime::new(config)
                .map_err(|e| anyhow::anyhow!("Claude bridge init failed: {}", e))?;

            runtime
                .run()
                .await
                .map_err(|e| anyhow::anyhow!("Claude bridge failed: {}", e))?;

            return Ok(());
        }
        Some(Command::Onboard {
            skip_auth,
            channels_only,
        }) => {
            // Load .env files before running onboarding wizard.
            // Standard ./.env first (higher priority), then ~/.ironclaw/.env.
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

    // Load .env files early so DATABASE_URL (and any other vars) are
    // available to all subsequent env-based config resolution.
    // Standard ./.env first (higher priority), then ~/.ironclaw/.env.
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
    let mut config = match Config::from_env_with_toml(toml_path).await {
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

    // Initialize tracing
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("ironclaw=info,tower_http=warn"));

    // Create log broadcaster before tracing init so the WebLogLayer can capture all events.
    // This gets wired to the gateway's /api/logs/events SSE endpoint later.
    let log_broadcaster = Arc::new(LogBroadcaster::new());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_writer(ironclaw::tracing_fmt::TruncatingStderr::default()),
        )
        .with(WebLogLayer::new(Arc::clone(&log_broadcaster)))
        .init();

    // Create CLI channel
    let repl_channel = if let Some(ref msg) = cli.message {
        Some(ReplChannel::with_message(msg.clone()))
    } else if config.channels.cli.enabled {
        let repl = ReplChannel::new();
        // Suppress the one-liner banner; boot screen will be shown instead.
        repl.suppress_banner();
        Some(repl)
    } else {
        None
    };

    tracing::info!("Starting IronClaw...");
    tracing::info!("Loaded configuration for agent: {}", config.agent.name);
    tracing::info!("LLM backend: {}", config.llm.backend);

    // Initialize database backend.
    //
    // Creates an `Arc<dyn Database>` that all consumers share.
    // Backend is selected by the `DATABASE_BACKEND` env var / config.
    //
    // NOTE: For simpler call sites (CLI commands, Memory handler) use the shared
    // helper `ironclaw::db::connect_from_config()`. This block is kept inline
    // because it also captures backend-specific handles (`pg_pool`, `libsql_db`)
    // needed by the secrets store.
    #[cfg(feature = "postgres")]
    let mut pg_pool: Option<deadpool_postgres::Pool> = None;
    #[cfg(feature = "libsql")]
    let mut libsql_db: Option<std::sync::Arc<libsql::Database>> = None;

    let db: Option<Arc<dyn ironclaw::db::Database>> = if cli.no_db {
        tracing::warn!("Running without database connection");
        None
    } else {
        match config.database.backend {
            #[cfg(feature = "libsql")]
            ironclaw::config::DatabaseBackend::LibSql => {
                use ironclaw::db::Database as _;
                use ironclaw::db::libsql::LibSqlBackend;
                use secrecy::ExposeSecret as _;

                let default_path = ironclaw::config::default_libsql_path();
                let db_path = config
                    .database
                    .libsql_path
                    .as_deref()
                    .unwrap_or(&default_path);

                let backend = if let Some(ref url) = config.database.libsql_url {
                    let token = config.database.libsql_auth_token.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("LIBSQL_AUTH_TOKEN is required when LIBSQL_URL is set")
                    })?;
                    LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret()).await?
                } else {
                    LibSqlBackend::new_local(db_path).await?
                };
                backend.run_migrations().await?;
                tracing::info!("libSQL database connected and migrations applied");

                // Capture the Database handle for SecretsStore (connection-per-op)
                libsql_db = Some(backend.shared_db());

                Some(Arc::new(backend) as Arc<dyn ironclaw::db::Database>)
            }
            #[cfg(feature = "postgres")]
            _ => {
                use ironclaw::db::Database as _;
                let pg = ironclaw::db::postgres::PgBackend::new(&config.database)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                pg.run_migrations()
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                tracing::info!("PostgreSQL database connected and migrations applied");

                pg_pool = Some(pg.pool());
                Some(Arc::new(pg) as Arc<dyn ironclaw::db::Database>)
            }
            #[cfg(not(feature = "postgres"))]
            _ => {
                anyhow::bail!(
                    "No database backend available. Enable 'postgres' or 'libsql' feature."
                );
            }
        }
    };

    // Post-init operations using the database
    if let Some(ref db) = db {
        // One-time migration: move disk config files into the DB settings table.
        if let Err(e) = ironclaw::bootstrap::migrate_disk_to_db(db.as_ref(), "default").await {
            tracing::warn!("Disk-to-DB settings migration failed: {}", e);
        }

        // Reload config from DB now that we have a connection.
        match Config::from_db_with_toml(db.as_ref(), "default", toml_path).await {
            Ok(db_config) => {
                config = db_config;
                tracing::info!("Configuration reloaded from database");
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to reload config from DB, keeping env-based config: {}",
                    e
                );
            }
        }

        // Attach DB to session manager so tokens save to DB too
        session.attach_store(Arc::clone(db), "default").await;

        // Mark any jobs left in "running" or "creating" state as "interrupted".
        if let Err(e) = db.cleanup_stale_sandbox_jobs().await {
            tracing::warn!("Failed to cleanup stale sandbox jobs: {}", e);
        }
    }

    // Create secrets store early: needed for injecting LLM API keys from encrypted
    // storage before creating the LLM provider, and later for MCP auth + WASM channels.
    //
    // When both `postgres` and `libsql` features are compiled, the runtime-selected
    // backend determines which store is created: whichever DB init branch ran will
    // have set its handle (pg_pool or libsql_db), and the or_else chain picks it up.
    let secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>> =
        if let Some(master_key) = config.secrets.master_key() {
            match SecretsCrypto::new(master_key.clone()) {
                Ok(crypto) => {
                    let crypto = Arc::new(crypto);
                    let store: Option<Arc<dyn SecretsStore + Send + Sync>> = None;

                    #[cfg(feature = "libsql")]
                    let store = store.or_else(|| {
                        libsql_db.take().map(|db| {
                            Arc::new(LibSqlSecretsStore::new(db, Arc::clone(&crypto)))
                                as Arc<dyn SecretsStore + Send + Sync>
                        })
                    });

                    #[cfg(feature = "postgres")]
                    let store = store.or_else(|| {
                        pg_pool.as_ref().map(|pool| {
                            Arc::new(PostgresSecretsStore::new(pool.clone(), Arc::clone(&crypto)))
                                as Arc<dyn SecretsStore + Send + Sync>
                        })
                    });

                    store
                }
                Err(e) => {
                    tracing::warn!("Failed to initialize secrets crypto: {}", e);
                    #[cfg(feature = "libsql")]
                    let _ = libsql_db.take();
                    None
                }
            }
        } else {
            #[cfg(feature = "libsql")]
            let _ = libsql_db.take();
            None
        };

    // Inject LLM API keys from the encrypted secrets store into a thread-safe
    // overlay so that optional_env() (used by LlmConfig::resolve()) picks them
    // up. Then re-resolve LlmConfig with the newly available keys (backend may
    // have been set during onboarding but the API key is in the secrets store).
    if let Some(ref secrets) = secrets_store {
        ironclaw::config::inject_llm_keys_from_secrets(secrets.as_ref(), "default").await;

        // Re-resolve LlmConfig now that secrets overlay has been populated
        if let Some(ref db_ref) = db {
            match Config::from_db_with_toml(db_ref.as_ref(), "default", toml_path).await {
                Ok(refreshed) => {
                    config = refreshed;
                    tracing::debug!("LlmConfig re-resolved after secret injection");
                }
                Err(e) => {
                    tracing::warn!("Failed to re-resolve config after secret injection: {}", e);
                }
            }
        }
    }

    // Session-based auth is only needed for NEAR AI backend without an API key.
    // Do this after DB-backed config reload so provider selection from onboarding
    // is respected (e.g. OpenAI/OpenAI-compatible should not trigger NEAR auth).
    if config.llm.backend == ironclaw::config::LlmBackend::NearAi
        && config.llm.nearai.api_key.is_none()
    {
        session.ensure_authenticated().await?;
    }

    // Start managed tunnel if configured and no static URL is already set.
    //
    // The tunnel process runs in the background, exposing the local gateway
    // port to the internet. The resulting public URL is injected into
    // config.tunnel.public_url so channels and extensions pick it up.
    let active_tunnel: Option<Box<dyn ironclaw::tunnel::Tunnel>> =
        if config.tunnel.public_url.is_some() {
            tracing::info!(
                "Static tunnel URL in use: {}",
                config.tunnel.public_url.as_deref().unwrap_or("?")
            );
            None
        } else if let Some(ref provider_config) = config.tunnel.provider {
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
                            Some(tunnel)
                        }
                        Err(e) => {
                            tracing::error!("Failed to start tunnel: {}", e);
                            None
                        }
                    }
                }
                Ok(None) => None,
                Err(e) => {
                    tracing::error!("Failed to create tunnel: {}", e);
                    None
                }
            }
        } else {
            None
        };

    // Initialize LLM provider (clone session so we can reuse it for embeddings)
    let llm = create_llm_provider(&config.llm, session.clone())?;
    tracing::info!("LLM provider initialized: {}", llm.model_name());

    // Wrap each provider with RetryProvider for automatic retries on transient errors.
    // RetryProvider sits inside FailoverProvider so each provider in the failover chain
    // gets its own retry attempts before the failover moves to the next provider.
    let retry_config = RetryConfig {
        max_retries: config.llm.nearai.max_retries,
    };
    let llm: Arc<dyn LlmProvider> = if retry_config.max_retries > 0 {
        tracing::info!(
            max_retries = retry_config.max_retries,
            "LLM retry wrapper enabled"
        );
        Arc::new(RetryProvider::new(llm, retry_config.clone()))
    } else {
        llm
    };

    // Wrap in failover if a fallback model is configured
    let llm: Arc<dyn LlmProvider> =
        if let Some(fallback_model) = config.llm.nearai.fallback_model.as_ref() {
            if fallback_model == &config.llm.nearai.model {
                tracing::warn!(
                    "fallback_model is the same as primary model, failover may not be effective"
                );
            }
            let mut fallback_config = config.llm.nearai.clone();
            fallback_config.model = fallback_model.clone();
            let fallback = create_llm_provider_with_config(&fallback_config, session.clone())?;
            tracing::info!(
                primary = %llm.model_name(),
                fallback = %fallback.model_name(),
                "LLM failover enabled"
            );
            // Wrap fallback with retry too
            let fallback: Arc<dyn LlmProvider> = if retry_config.max_retries > 0 {
                Arc::new(RetryProvider::new(fallback, retry_config.clone()))
            } else {
                fallback
            };
            let cooldown_config = CooldownConfig {
                cooldown_duration: std::time::Duration::from_secs(
                    config.llm.nearai.failover_cooldown_secs,
                ),
                failure_threshold: config.llm.nearai.failover_cooldown_threshold,
            };
            Arc::new(FailoverProvider::with_cooldown(
                vec![llm, fallback],
                cooldown_config,
            )?)
        } else {
            llm
        };

    // Wrap in circuit breaker if configured
    let llm: Arc<dyn LlmProvider> =
        if let Some(threshold) = config.llm.nearai.circuit_breaker_threshold {
            let cb_config = CircuitBreakerConfig {
                failure_threshold: threshold,
                recovery_timeout: std::time::Duration::from_secs(
                    config.llm.nearai.circuit_breaker_recovery_secs,
                ),
                ..CircuitBreakerConfig::default()
            };
            tracing::info!(
                threshold,
                recovery_secs = config.llm.nearai.circuit_breaker_recovery_secs,
                "LLM circuit breaker enabled"
            );
            Arc::new(CircuitBreakerProvider::new(llm, cb_config))
        } else {
            llm
        };

    // Wrap in response cache if configured
    let llm: Arc<dyn LlmProvider> = if config.llm.nearai.response_cache_enabled {
        let rc_config = ResponseCacheConfig {
            ttl: std::time::Duration::from_secs(config.llm.nearai.response_cache_ttl_secs),
            max_entries: config.llm.nearai.response_cache_max_entries,
        };
        tracing::info!(
            ttl_secs = config.llm.nearai.response_cache_ttl_secs,
            max_entries = config.llm.nearai.response_cache_max_entries,
            "LLM response cache enabled"
        );
        Arc::new(CachedProvider::new(llm, rc_config))
    } else {
        llm
    };

    // Initialize cheap LLM provider for lightweight tasks (heartbeat, evaluation)
    let cheap_llm = create_cheap_llm_provider(&config.llm, session.clone())?;
    if let Some(ref cheap) = cheap_llm {
        tracing::info!("Cheap LLM provider initialized: {}", cheap.model_name());
    }

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
                    "Embeddings enabled via NEAR AI (model: {}, dim: {})",
                    config.embeddings.model,
                    config.embeddings.dimension,
                );
                Some(Arc::new(
                    NearAiEmbeddings::new(&config.llm.nearai.base_url, session.clone())
                        .with_model(&config.embeddings.model, config.embeddings.dimension),
                ))
            }
            "ollama" => {
                tracing::info!(
                    "Embeddings enabled via Ollama (model: {}, url: {}, dim: {})",
                    config.embeddings.model,
                    config.embeddings.ollama_base_url,
                    config.embeddings.dimension,
                );
                Some(Arc::new(
                    OllamaEmbeddings::new(&config.embeddings.ollama_base_url)
                        .with_model(&config.embeddings.model, config.embeddings.dimension),
                ))
            }
            _ => {
                // Default to OpenAI for unknown providers
                if let Some(api_key) = config.embeddings.openai_api_key() {
                    tracing::info!(
                        "Embeddings enabled via OpenAI (model: {}, dim: {})",
                        config.embeddings.model,
                        config.embeddings.dimension,
                    );
                    Some(Arc::new(OpenAiEmbeddings::with_model(
                        api_key,
                        &config.embeddings.model,
                        config.embeddings.dimension,
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

    // Register memory tools if database is available
    if let Some(ref db) = db {
        let mut workspace = Workspace::new_with_db("default", Arc::clone(db));
        if let Some(ref emb) = embeddings {
            workspace = workspace.with_embeddings(emb.clone());
        }
        let workspace = Arc::new(workspace);
        tools.register_memory_tools(workspace);
    }

    // Register builder tool if enabled.
    // When sandbox is enabled and allow_local_tools is false, skip builder registration
    // because register_builder_tool also registers dev tools (shell, file ops) that would
    // bypass the sandbox. The builder runs inside containers instead.
    if config.builder.enabled && (config.agent.allow_local_tools || !config.sandbox.enabled) {
        tools
            .register_builder_tool(
                llm.clone(),
                safety.clone(),
                Some(config.builder.to_builder_config()),
            )
            .await;
        tracing::info!("Builder mode enabled");
    }

    let mcp_session_manager = Arc::new(McpSessionManager::new());

    // Create hook registry early so runtime extension activation can register hooks.
    let hooks = Arc::new(HookRegistry::new());

    // Create WASM tool runtime (sync, just builds the wasmtime engine)
    let wasm_tool_runtime: Option<Arc<WasmToolRuntime>> =
        if config.wasm.enabled && config.wasm.tools_dir.exists() {
            match WasmToolRuntime::new(config.wasm.to_runtime_config()) {
                Ok(runtime) => Some(Arc::new(runtime)),
                Err(e) => {
                    tracing::warn!("Failed to initialize WASM runtime: {}", e);
                    None
                }
            }
        } else {
            None
        };

    // Load WASM tools and MCP servers concurrently.
    // Both register into the shared ToolRegistry (RwLock-based) so concurrent writes are safe.
    let wasm_tools_future = async {
        let mut dev_loaded_tool_names: Vec<String> = Vec::new();

        if let Some(ref runtime) = wasm_tool_runtime {
            let mut loader = WasmToolLoader::new(Arc::clone(runtime), Arc::clone(&tools));
            if let Some(ref secrets) = secrets_store {
                loader = loader.with_secrets_store(Arc::clone(secrets));
            }

            // Load installed tools from ~/.ironclaw/tools/
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

            // Load dev tools from build artifacts (overrides installed if newer)
            match load_dev_tools(&loader, &config.wasm.tools_dir).await {
                Ok(results) => {
                    dev_loaded_tool_names.extend(results.loaded.iter().cloned());
                    if !results.loaded.is_empty() {
                        tracing::info!(
                            "Loaded {} dev WASM tools from build artifacts",
                            results.loaded.len()
                        );
                    }
                }
                Err(e) => {
                    tracing::debug!("No dev WASM tools found: {}", e);
                }
            }
        }

        dev_loaded_tool_names
    };

    let mcp_servers_future = async {
        if let Some(ref secrets) = secrets_store {
            let servers_result = if let Some(ref d) = db {
                load_mcp_servers_from_db(d.as_ref(), "default").await
            } else {
                ironclaw::tools::mcp::config::load_mcp_servers().await
            };
            match servers_result {
                Ok(servers) => {
                    let enabled: Vec<_> = servers.enabled_servers().cloned().collect();
                    if !enabled.is_empty() {
                        tracing::info!("Loading {} configured MCP server(s)...", enabled.len());
                    }

                    let mut join_set = tokio::task::JoinSet::new();
                    for server in enabled {
                        let mcp_sm = Arc::clone(&mcp_session_manager);
                        let secrets = Arc::clone(secrets);
                        let tools = Arc::clone(&tools);

                        join_set.spawn(async move {
                            let server_name = server.name.clone();
                            tracing::debug!(
                                "Checking authentication for MCP server '{}'...",
                                server_name
                            );
                            let has_tokens = is_authenticated(&server, &secrets, "default").await;
                            tracing::debug!(
                                "MCP server '{}' has_tokens={}",
                                server_name,
                                has_tokens
                            );

                            let client = if has_tokens || server.requires_auth() {
                                McpClient::new_authenticated(server, mcp_sm, secrets, "default")
                            } else {
                                McpClient::new_with_name(&server_name, &server.url)
                            };

                            tracing::debug!("Fetching tools from MCP server '{}'...", server_name);
                            match client.list_tools().await {
                                Ok(mcp_tools) => {
                                    let tool_count = mcp_tools.len();
                                    tracing::debug!(
                                        "Got {} tools from MCP server '{}'",
                                        tool_count,
                                        server_name
                                    );
                                    match client.create_tools().await {
                                        Ok(tool_impls) => {
                                            for tool in tool_impls {
                                                tools.register(tool).await;
                                            }
                                            tracing::info!(
                                                "Loaded {} tools from MCP server '{}'",
                                                tool_count,
                                                server_name
                                            );
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "Failed to create tools from MCP server '{}': {}",
                                                server_name,
                                                e
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    let err_str = e.to_string();
                                    if err_str.contains("401") || err_str.contains("authentication")
                                    {
                                        tracing::warn!(
                                            "MCP server '{}' requires authentication. \
                                             Run: ironclaw mcp auth {}",
                                            server_name,
                                            server_name
                                        );
                                    } else {
                                        tracing::warn!(
                                            "Failed to connect to MCP server '{}': {}",
                                            server_name,
                                            e
                                        );
                                    }
                                }
                            }
                        });
                    }

                    while let Some(result) = join_set.join_next().await {
                        if let Err(e) = result {
                            tracing::warn!("MCP server loading task panicked: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("No MCP servers configured ({})", e);
                }
            }
        }
    };

    let (dev_loaded_tool_names, _) = tokio::join!(wasm_tools_future, mcp_servers_future);

    // Create extension manager for in-chat discovery/install/auth/activate
    let extension_manager = if let Some(ref secrets) = secrets_store {
        let manager = Arc::new(ExtensionManager::new(
            Arc::clone(&mcp_session_manager),
            Arc::clone(secrets),
            Arc::clone(&tools),
            Some(Arc::clone(&hooks)),
            wasm_tool_runtime.clone(),
            config.wasm.tools_dir.clone(),
            config.channels.wasm_channels_dir.clone(),
            config.tunnel.public_url.clone(),
            "default".to_string(),
            db.clone(),
        ));
        tools.register_extension_tools(Arc::clone(&manager));
        tracing::info!("Extension manager initialized with in-chat discovery tools");
        Some(manager)
    } else {
        tracing::debug!(
            "Extension manager not available (no secrets store). \
             Extension tools won't be registered."
        );
        None
    };

    // Set up orchestrator for sandboxed job execution
    // When allow_local_tools is false (default), the LLM uses create_job for FS/shell work.
    // When allow_local_tools is true, dev tools are also registered directly (current behavior).
    if config.agent.allow_local_tools {
        tools.register_dev_tools();
        tracing::info!(
            "Local tools enabled (allow_local_tools=true), dev tools registered directly"
        );
    }

    // Shared state for job events (used by both orchestrator and web gateway)
    let job_event_tx: Option<
        tokio::sync::broadcast::Sender<(uuid::Uuid, ironclaw::channels::web::types::SseEvent)>,
    > = if config.sandbox.enabled {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        Some(tx)
    } else {
        None
    };
    let prompt_queue = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::<
        uuid::Uuid,
        std::collections::VecDeque<ironclaw::orchestrator::api::PendingPrompt>,
    >::new()));

    let container_job_manager: Option<Arc<ContainerJobManager>> = if config.sandbox.enabled {
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
            llm: llm.clone(),
            job_manager: Arc::clone(&jm),
            token_store,
            job_event_tx: job_event_tx.clone(),
            prompt_queue: Arc::clone(&prompt_queue),
            store: db.clone(),
            secrets_store: secrets_store.clone(),
            user_id: "default".to_string(),
        };

        tokio::spawn(async move {
            if let Err(e) = OrchestratorApi::start(orchestrator_state, 50051).await {
                tracing::error!("Orchestrator API failed: {}", e);
            }
        });

        tracing::info!("Orchestrator API started on :50051, sandbox delegation enabled");
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

    tracing::info!(
        "Tool registry initialized with {} total tools",
        tools.count()
    );

    // Initialize channel manager
    let mut channels = ChannelManager::new();
    let mut channel_names: Vec<String> = Vec::new();
    let mut loaded_wasm_channel_names: Vec<String> = Vec::new();

    if let Some(repl) = repl_channel {
        channels.add(Box::new(repl));
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
        match WasmChannelRuntime::new(WasmChannelRuntimeConfig::default()) {
            Ok(runtime) => {
                let runtime = Arc::new(runtime);
                let pairing_store = Arc::new(PairingStore::new());
                let loader = WasmChannelLoader::new(Arc::clone(&runtime), pairing_store);

                match loader
                    .load_from_dir(&config.channels.wasm_channels_dir)
                    .await
                {
                    Ok(results) => {
                        let wasm_router = Arc::new(WasmChannelRouter::new());
                        let mut has_webhook_channels = false;

                        for loaded in results.loaded {
                            let channel_name = loaded.name().to_string();
                            loaded_wasm_channel_names.push(channel_name.clone());
                            tracing::info!("Loaded WASM channel: {}", channel_name);

                            let secret_name = loaded.webhook_secret_name();

                            let webhook_secret = if let Some(ref secrets) = secrets_store {
                                secrets
                                    .get_decrypted("default", &secret_name)
                                    .await
                                    .ok()
                                    .map(|s| s.expose().to_string())
                            } else {
                                None
                            };

                            let secret_header =
                                loaded.webhook_secret_header().map(|s| s.to_string());

                            let webhook_path = format!("/webhook/{}", channel_name);
                            let endpoints = vec![RegisteredEndpoint {
                                channel_name: channel_name.clone(),
                                path: webhook_path.clone(),
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

                                // Inject owner_id for Telegram so the bot only responds
                                // to the bound user account.
                                if channel_name == "telegram"
                                    && let Some(owner_id) = config.channels.telegram_owner_id
                                {
                                    config_updates.insert(
                                        "owner_id".to_string(),
                                        serde_json::json!(owner_id),
                                    );
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
                            has_webhook_channels = true;

                            if let Some(ref secrets) = secrets_store {
                                match inject_channel_credentials(
                                    &channel_arc,
                                    secrets.as_ref(),
                                    &channel_name,
                                )
                                .await
                                {
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

                            channel_names.push(channel_name.clone());
                            channels.add(Box::new(SharedWasmChannel::new(channel_arc)));
                        }

                        if has_webhook_channels {
                            webhook_routes.push(create_wasm_channel_router(
                                wasm_router,
                                extension_manager.as_ref().map(Arc::clone),
                            ));
                        }

                        for (path, err) in &results.errors {
                            tracing::warn!(
                                "Failed to load WASM channel {}: {}",
                                path.display(),
                                err
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to scan WASM channels directory: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to initialize WASM channel runtime: {}", e);
            }
        }
    }

    // Add HTTP channel if configured and not CLI-only mode.
    // Extract its routes for the unified server; the channel itself just
    // provides the mpsc stream.
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
        channels.add(Box::new(http_channel));
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

    // Create workspace for agent (shared with memory tools)
    let workspace = if let Some(ref db_ref) = db {
        let mut ws = Workspace::new_with_db("default", Arc::clone(db_ref));
        if let Some(ref emb) = embeddings {
            ws = ws.with_embeddings(emb.clone());
        }
        Some(Arc::new(ws))
    } else {
        None
    };

    // Seed workspace with core identity files on first boot
    if let Some(ref ws) = workspace {
        match ws.seed_if_empty().await {
            Ok(count) if count > 0 => {
                tracing::info!("Workspace seeded with {} core files", count);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("Failed to seed workspace: {}", e);
            }
        }
    }

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

    // Create context manager (shared between job tools and agent)
    let context_manager = Arc::new(ContextManager::new(config.agent.max_parallel_jobs));

    // Register bundled/plugin/workspace hooks.
    let active_tool_names = tools.list().await;

    let hook_bootstrap = bootstrap_hooks(
        &hooks,
        workspace.as_ref(),
        &config.wasm.tools_dir,
        &config.channels.wasm_channels_dir,
        &active_tool_names,
        &loaded_wasm_channel_names,
        &dev_loaded_tool_names,
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
    let session_manager = Arc::new(SessionManager::new().with_hooks(hooks.clone()));

    // Register job tools (sandbox deps auto-injected when container_job_manager is available)
    tools.register_job_tools(
        Arc::clone(&context_manager),
        container_job_manager.clone(),
        db.clone(),
        job_event_tx.clone(),
        Some(channels.inject_sender()),
        if config.sandbox.enabled {
            Some(Arc::clone(&prompt_queue))
        } else {
            None
        },
        secrets_store.clone(),
    );

    // Initialize skills system (before gateway so we can wire into GatewayState)
    let (skill_registry, skill_catalog) = if config.skills.enabled {
        let mut registry = ironclaw::skills::SkillRegistry::new(config.skills.local_dir.clone());
        let loaded = registry.discover_all().await;
        if !loaded.is_empty() {
            tracing::info!("Loaded {} skill(s): {}", loaded.len(), loaded.join(", "));
        }
        let registry = Arc::new(std::sync::RwLock::new(registry));

        // Register skill management tools
        let catalog = ironclaw::skills::catalog::shared_catalog();
        tools.register_skill_tools(Arc::clone(&registry), Arc::clone(&catalog));

        (Some(registry), Some(catalog))
    } else {
        (None, None)
    };

    // Add web gateway channel if configured
    let mut gateway_url: Option<String> = None;
    if let Some(ref gw_config) = config.channels.gateway {
        let mut gw = GatewayChannel::new(gw_config.clone()).with_llm_provider(Arc::clone(&llm));
        if let Some(ref ws) = workspace {
            gw = gw.with_workspace(Arc::clone(ws));
        }
        gw = gw.with_session_manager(Arc::clone(&session_manager));
        gw = gw.with_log_broadcaster(Arc::clone(&log_broadcaster));
        gw = gw.with_tool_registry(Arc::clone(&tools));
        if let Some(ref ext_mgr) = extension_manager {
            gw = gw.with_extension_manager(Arc::clone(ext_mgr));
        }
        if let Some(ref d) = db {
            gw = gw.with_store(Arc::clone(d));
        }
        if let Some(ref jm) = container_job_manager {
            gw = gw.with_job_manager(Arc::clone(jm));
        }
        if let Some(ref sr) = skill_registry {
            gw = gw.with_skill_registry(Arc::clone(sr));
        }
        if let Some(ref sc) = skill_catalog {
            gw = gw.with_skill_catalog(Arc::clone(sc));
        }
        if config.sandbox.enabled {
            gw = gw.with_prompt_queue(Arc::clone(&prompt_queue));

            // Spawn a task to forward job events from the broadcast channel to SSE
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

        tracing::info!(
            "Web gateway enabled on {}:{}",
            gw_config.host,
            gw_config.port
        );
        tracing::info!("Web UI: http://{}:{}/", gw_config.host, gw_config.port);

        channel_names.push("gateway".to_string());
        channels.add(Box::new(gw));
    }

    // Capture boot screen info before moving Arcs into AgentDeps.
    let boot_tool_count = tools.count();
    let boot_llm_model = llm.model_name().to_string();
    let boot_cheap_model = cheap_llm.as_ref().map(|c| c.model_name().to_string());

    // Create and run the agent
    let cost_guard = Arc::new(ironclaw::agent::cost_guard::CostGuard::new(
        ironclaw::agent::cost_guard::CostGuardConfig {
            max_cost_per_day_cents: config.agent.max_cost_per_day_cents,
            max_actions_per_hour: config.agent.max_actions_per_hour,
        },
    ));
    let deps = AgentDeps {
        store: db,
        llm,
        cheap_llm,
        safety,
        tools,
        workspace,
        extension_manager,
        skill_registry,
        skills_config: config.skills.clone(),
        hooks,
        cost_guard,
    };
    let agent = Agent::new(
        config.agent.clone(),
        deps,
        channels,
        Some(config.heartbeat.clone()),
        Some(config.routines.clone()),
        Some(context_manager),
        Some(session_manager),
    );

    tracing::info!("Agent initialized, starting main loop...");

    // Print boot screen for interactive CLI mode (not single-message mode).
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
            claude_code_enabled: config.claude_code.enabled,
            routines_enabled: config.routines.enabled,
            channels: channel_names,
            tunnel_url: active_tunnel
                .as_ref()
                .and_then(|t| t.public_url())
                .or_else(|| config.tunnel.public_url.clone()),
            tunnel_provider: active_tunnel.as_ref().map(|t| t.name().to_string()),
        };
        ironclaw::boot_screen::print_boot_screen(&boot_info);
    }

    // Run the agent (blocks until shutdown)
    agent.run().await?;

    // Shut down the webhook server if one was started
    if let Some(ref mut server) = webhook_server {
        server.shutdown().await;
    }

    // Stop managed tunnel if one was started
    if let Some(tunnel) = active_tunnel {
        tracing::info!("Stopping {} tunnel...", tunnel.name());
        if let Err(e) = tunnel.stop().await {
            tracing::warn!("Failed to stop tunnel cleanly: {}", e);
        }
    }

    tracing::info!("Agent shutdown complete");
    Ok(())
}

/// Check if onboarding is needed and return the reason.
///
/// Returns `Some(reason)` if onboarding should be triggered, `None` otherwise.
/// Called after `load_ironclaw_env()`, so DATABASE_URL from `~/.ironclaw/.env`
/// is already in the environment.
#[cfg(any(feature = "postgres", feature = "libsql"))]
fn check_onboard_needed() -> Option<&'static str> {
    let has_db = std::env::var("DATABASE_URL").is_ok()
        || std::env::var("LIBSQL_PATH").is_ok()
        || ironclaw::config::default_libsql_path().exists();

    if !has_db {
        return Some("Database not configured");
    }

    // First run (onboarding never completed and no session).
    // Reads NEARAI_API_KEY env var directly because this function runs
    // before Config is loaded -- Config::from_env() may fail without a
    // database URL, which is what triggers onboarding in the first place.
    if std::env::var("NEARAI_API_KEY").is_err() {
        let settings = ironclaw::settings::Settings::load();
        let session_path = ironclaw::llm::session::default_session_path();
        if !settings.onboard_completed && !session_path.exists() {
            return Some("First run");
        }
    }

    None
}

/// Inject credentials for a channel based on naming convention.
///
/// Looks for secrets matching the pattern `{channel_name}_*` and injects them
/// as credential placeholders (e.g., `telegram_bot_token` -> `{TELEGRAM_BOT_TOKEN}`).
///
/// Returns the number of credentials injected.
async fn inject_channel_credentials(
    channel: &Arc<ironclaw::channels::wasm::WasmChannel>,
    secrets: &dyn SecretsStore,
    channel_name: &str,
) -> anyhow::Result<usize> {
    // List all secrets for this user and filter by channel prefix
    let all_secrets = secrets
        .list("default")
        .await
        .map_err(|e| anyhow::anyhow!("Failed to list secrets: {}", e))?;

    let prefix = format!("{}_", channel_name);
    let mut count = 0;

    for secret_meta in all_secrets {
        // Only process secrets matching the channel prefix
        if !secret_meta.name.starts_with(&prefix) {
            continue;
        }

        // Get the decrypted value
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

        // Convert secret name to placeholder format (SCREAMING_SNAKE_CASE)
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
