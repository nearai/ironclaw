//! Application builder for initializing core IronClaw components.
//!
//! Extracts the mechanical initialization phases from `main.rs` into a
//! reusable builder so that:
//!
//! - Tests can construct a full `AppComponents` without wiring channels
//! - Main stays focused on CLI dispatch and channel setup
//! - Each init phase is independently testable

use std::sync::Arc;

use crate::channels::web::log_layer::LogBroadcaster;
use crate::config::Config;
use crate::context::ContextManager;
use crate::db::Database;
use crate::extensions::ExtensionManager;
use crate::hooks::HookRegistry;
use crate::llm::{LlmProvider, SessionManager};
use crate::safety::SafetyLayer;
use crate::secrets::SecretsStore;
use crate::skills::SkillRegistry;
use crate::skills::catalog::SkillCatalog;
use crate::tools::ToolRegistry;
use crate::tools::mcp::McpSessionManager;
use crate::tools::wasm::SharedCredentialRegistry;
use crate::tools::wasm::WasmToolRuntime;
use crate::workspace::{EmbeddingProvider, Workspace};

/// Fully initialized application components, ready for channel wiring
/// and agent construction.
pub struct AppComponents {
    /// The (potentially mutated) config after DB reload and secret injection.
    pub config: Config,
    pub db: Option<Arc<dyn Database>>,
    pub secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    pub llm: Arc<dyn LlmProvider>,
    pub cheap_llm: Option<Arc<dyn LlmProvider>>,
    pub safety: Arc<SafetyLayer>,
    pub tools: Arc<ToolRegistry>,
    pub embeddings: Option<Arc<dyn EmbeddingProvider>>,
    pub workspace: Option<Arc<Workspace>>,
    pub extension_manager: Option<Arc<ExtensionManager>>,
    pub mcp_session_manager: Arc<McpSessionManager>,
    pub wasm_tool_runtime: Option<Arc<WasmToolRuntime>>,
    pub log_broadcaster: Arc<LogBroadcaster>,
    pub context_manager: Arc<ContextManager>,
    pub hooks: Arc<HookRegistry>,
    pub skill_registry: Option<Arc<std::sync::RwLock<SkillRegistry>>>,
    pub skill_catalog: Option<Arc<SkillCatalog>>,
    pub cost_guard: Arc<crate::agent::cost_guard::CostGuard>,
    pub session: Arc<SessionManager>,
    pub catalog_entries: Vec<crate::extensions::RegistryEntry>,
    pub dev_loaded_tool_names: Vec<String>,
}

/// Options that control optional init phases.
#[derive(Default)]
pub struct AppBuilderFlags {
    pub no_db: bool,
}

/// Builder that orchestrates the 5 mechanical init phases.
pub struct AppBuilder {
    config: Config,
    flags: AppBuilderFlags,
    toml_path: Option<std::path::PathBuf>,
    session: Arc<SessionManager>,
    log_broadcaster: Arc<LogBroadcaster>,

    // Accumulated state
    db: Option<Arc<dyn Database>>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,

    // Backend-specific handles needed by secrets store
    #[cfg(feature = "postgres")]
    pg_pool: Option<deadpool_postgres::Pool>,
    #[cfg(feature = "libsql")]
    libsql_db: Option<Arc<libsql::Database>>,
}

impl AppBuilder {
    /// Create a new builder.
    ///
    /// The `session` and `log_broadcaster` are created before the builder
    /// because tracing must be initialized before any init phase runs,
    /// and the log broadcaster is part of the tracing layer.
    pub fn new(
        config: Config,
        flags: AppBuilderFlags,
        toml_path: Option<std::path::PathBuf>,
        session: Arc<SessionManager>,
        log_broadcaster: Arc<LogBroadcaster>,
    ) -> Self {
        Self {
            config,
            flags,
            toml_path,
            session,
            log_broadcaster,
            db: None,
            secrets_store: None,
            #[cfg(feature = "postgres")]
            pg_pool: None,
            #[cfg(feature = "libsql")]
            libsql_db: None,
        }
    }

    /// Phase 1: Initialize database backend.
    ///
    /// Creates the database connection, runs migrations, reloads config
    /// from DB, attaches DB to session manager, and cleans up stale jobs.
    pub async fn init_database(&mut self) -> Result<(), anyhow::Error> {
        if self.flags.no_db {
            tracing::warn!("Running without database connection");
            return Ok(());
        }

        let db: Arc<dyn Database> = match self.config.database.backend {
            #[cfg(feature = "libsql")]
            crate::config::DatabaseBackend::LibSql => {
                use crate::db::Database as _;
                use crate::db::libsql::LibSqlBackend;
                use secrecy::ExposeSecret as _;

                let default_path = crate::config::default_libsql_path();
                let db_path = self
                    .config
                    .database
                    .libsql_path
                    .as_deref()
                    .unwrap_or(&default_path);

                let backend = if let Some(ref url) = self.config.database.libsql_url {
                    let token =
                        self.config
                            .database
                            .libsql_auth_token
                            .as_ref()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "LIBSQL_AUTH_TOKEN is required when LIBSQL_URL is set"
                                )
                            })?;
                    LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret()).await?
                } else {
                    LibSqlBackend::new_local(db_path).await?
                };
                backend.run_migrations().await?;
                tracing::info!("libSQL database connected and migrations applied");

                #[cfg(feature = "libsql")]
                {
                    self.libsql_db = Some(backend.shared_db());
                }

                Arc::new(backend) as Arc<dyn Database>
            }
            #[cfg(feature = "postgres")]
            _ => {
                use crate::db::Database as _;
                let pg = crate::db::postgres::PgBackend::new(&self.config.database)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                pg.run_migrations()
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                tracing::info!("PostgreSQL database connected and migrations applied");

                #[cfg(feature = "postgres")]
                {
                    self.pg_pool = Some(pg.pool());
                }

                Arc::new(pg) as Arc<dyn Database>
            }
            #[cfg(not(feature = "postgres"))]
            _ => {
                anyhow::bail!(
                    "No database backend available. Enable 'postgres' or 'libsql' feature."
                );
            }
        };

        // Post-init: migrate disk config, reload config from DB, attach session, cleanup
        if let Err(e) = crate::bootstrap::migrate_disk_to_db(db.as_ref(), "default").await {
            tracing::warn!("Disk-to-DB settings migration failed: {}", e);
        }

        let toml_path = self.toml_path.as_deref();
        match Config::from_db_with_toml(db.as_ref(), "default", toml_path).await {
            Ok(db_config) => {
                self.config = db_config;
                tracing::info!("Configuration reloaded from database");
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to reload config from DB, keeping env-based config: {}",
                    e
                );
            }
        }

        self.session.attach_store(db.clone(), "default").await;

        // Fire-and-forget housekeeping — no need to block startup.
        let db_cleanup = db.clone();
        tokio::spawn(async move {
            if let Err(e) = db_cleanup.cleanup_stale_sandbox_jobs().await {
                tracing::warn!("Failed to cleanup stale sandbox jobs: {}", e);
            }
        });

        self.db = Some(db);
        Ok(())
    }

    /// Phase 2: Create secrets store.
    ///
    /// Requires a master key and a backend-specific DB handle. After creating
    /// the store, injects any encrypted LLM API keys into the config overlay
    /// and re-resolves config.
    pub async fn init_secrets(&mut self) -> Result<(), anyhow::Error> {
        let master_key = match self.config.secrets.master_key() {
            Some(k) => k,
            None => {
                // Consume unused handles
                #[cfg(feature = "libsql")]
                {
                    self.libsql_db.take();
                }
                return Ok(());
            }
        };

        let crypto = match crate::secrets::SecretsCrypto::new(master_key.clone()) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::warn!("Failed to initialize secrets crypto: {}", e);
                #[cfg(feature = "libsql")]
                {
                    self.libsql_db.take();
                }
                return Ok(());
            }
        };

        let store: Option<Arc<dyn SecretsStore + Send + Sync>> = None;

        #[cfg(feature = "libsql")]
        let store = store.or_else(|| {
            self.libsql_db.take().map(|db| {
                Arc::new(crate::secrets::LibSqlSecretsStore::new(
                    db,
                    Arc::clone(&crypto),
                )) as Arc<dyn SecretsStore + Send + Sync>
            })
        });

        #[cfg(feature = "postgres")]
        let store = store.or_else(|| {
            self.pg_pool.as_ref().map(|pool| {
                Arc::new(crate::secrets::PostgresSecretsStore::new(
                    pool.clone(),
                    Arc::clone(&crypto),
                )) as Arc<dyn SecretsStore + Send + Sync>
            })
        });

        if let Some(ref secrets) = store {
            // Inject LLM API keys from encrypted storage
            crate::config::inject_llm_keys_from_secrets(secrets.as_ref(), "default").await;

            // Re-resolve config with newly available keys
            if let Some(ref db) = self.db {
                let toml_path = self.toml_path.as_deref();
                match Config::from_db_with_toml(db.as_ref(), "default", toml_path).await {
                    Ok(refreshed) => {
                        self.config = refreshed;
                        tracing::debug!("LlmConfig re-resolved after secret injection");
                    }
                    Err(e) => {
                        tracing::warn!("Failed to re-resolve config after secret injection: {}", e);
                    }
                }
            }
        }

        self.secrets_store = store;
        Ok(())
    }

    /// Phase 3: Initialize LLM provider chain.
    ///
    /// Delegates to `build_provider_chain` which applies all decorators
    /// (retry, smart routing, failover, circuit breaker, response cache).
    #[allow(clippy::type_complexity)]
    pub fn init_llm(
        &self,
    ) -> Result<(Arc<dyn LlmProvider>, Option<Arc<dyn LlmProvider>>), anyhow::Error> {
        let (llm, cheap_llm) =
            crate::llm::build_provider_chain(&self.config.llm, self.session.clone())?;
        Ok((llm, cheap_llm))
    }

    /// Phase 4: Initialize safety, tools, embeddings, and workspace.
    pub async fn init_tools(
        &self,
        llm: &Arc<dyn LlmProvider>,
    ) -> Result<
        (
            Arc<SafetyLayer>,
            Arc<ToolRegistry>,
            Option<Arc<dyn EmbeddingProvider>>,
            Option<Arc<Workspace>>,
        ),
        anyhow::Error,
    > {
        let safety = Arc::new(SafetyLayer::new(&self.config.safety));
        tracing::info!("Safety layer initialized");

        // Initialize tool registry with credential injection support
        let credential_registry = Arc::new(SharedCredentialRegistry::new());
        let tools = if let Some(ref ss) = self.secrets_store {
            Arc::new(
                ToolRegistry::new()
                    .with_credentials(Arc::clone(&credential_registry), Arc::clone(ss)),
            )
        } else {
            Arc::new(ToolRegistry::new())
        };
        tools.register_builtin_tools();

        // Create embeddings provider using the unified method
        let embeddings = self
            .config
            .embeddings
            .create_provider(&self.config.llm.nearai.base_url, self.session.clone());

        // Warn if libSQL backend is used with non-1536 embedding dimension.
        if self.config.database.backend == crate::config::DatabaseBackend::LibSql
            && self.config.embeddings.enabled
            && self.config.embeddings.dimension != 1536
        {
            tracing::warn!(
                configured_dimension = self.config.embeddings.dimension,
                "Embedding dimension {} is not 1536. The libSQL schema uses \
                 F32_BLOB(1536) which requires exactly 1536 dimensions. \
                 Embedding storage will fail. Use PostgreSQL or set \
                 EMBEDDING_DIMENSION=1536.",
                self.config.embeddings.dimension
            );
        }

        // Register memory tools if database is available
        let workspace = if let Some(ref db) = self.db {
            let mut ws = Workspace::new_with_db("default", db.clone());
            if let Some(ref emb) = embeddings {
                ws = ws.with_embeddings(emb.clone());
            }
            let ws = Arc::new(ws);
            tools.register_memory_tools(Arc::clone(&ws));
            Some(ws)
        } else {
            None
        };

        // Register builder tool if enabled
        if self.config.builder.enabled
            && (self.config.agent.allow_local_tools || !self.config.sandbox.enabled)
        {
            tools
                .register_builder_tool(
                    llm.clone(),
                    safety.clone(),
                    Some(self.config.builder.to_builder_config()),
                )
                .await;
            tracing::info!("Builder mode enabled");
        }

        Ok((safety, tools, embeddings, workspace))
    }

    /// Phase 5: Load WASM tools, MCP servers, and create extension manager.
    pub async fn init_extensions(
        &self,
        tools: &Arc<ToolRegistry>,
        hooks: &Arc<HookRegistry>,
    ) -> Result<
        (
            Arc<McpSessionManager>,
            Option<Arc<WasmToolRuntime>>,
            Option<Arc<ExtensionManager>>,
            Vec<crate::extensions::RegistryEntry>,
            Vec<String>,
        ),
        anyhow::Error,
    > {
        use crate::tools::mcp::{McpClient, config::load_mcp_servers_from_db, is_authenticated};
        use crate::tools::wasm::{WasmToolLoader, load_dev_tools};

        let mcp_session_manager = Arc::new(McpSessionManager::new());

        // Create WASM tool runtime eagerly so extensions installed after startup
        // (e.g. via the web UI) can still be activated. The tools directory is only
        // needed when loading modules, not for engine initialisation.
        let wasm_tool_runtime: Option<Arc<WasmToolRuntime>> = if self.config.wasm.enabled {
            match WasmToolRuntime::new(self.config.wasm.to_runtime_config()) {
                Ok(runtime) => Some(Arc::new(runtime)),
                Err(e) => {
                    tracing::warn!("Failed to initialize WASM runtime: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Load WASM tools and MCP servers concurrently
        let wasm_tools_future = {
            let wasm_tool_runtime = wasm_tool_runtime.clone();
            let secrets_store = self.secrets_store.clone();
            let tools = Arc::clone(tools);
            let wasm_config = self.config.wasm.clone();
            async move {
                let mut dev_loaded_tool_names: Vec<String> = Vec::new();

                if let Some(ref runtime) = wasm_tool_runtime {
                    let mut loader = WasmToolLoader::new(Arc::clone(runtime), Arc::clone(&tools));
                    if let Some(ref secrets) = secrets_store {
                        loader = loader.with_secrets_store(Arc::clone(secrets));
                    }

                    match loader.load_from_dir(&wasm_config.tools_dir).await {
                        Ok(results) => {
                            if !results.loaded.is_empty() {
                                tracing::info!(
                                    "Loaded {} WASM tools from {}",
                                    results.loaded.len(),
                                    wasm_config.tools_dir.display()
                                );
                            }
                            for (path, err) in &results.errors {
                                tracing::warn!(
                                    "Failed to load WASM tool {}: {}",
                                    path.display(),
                                    err
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to scan WASM tools directory: {}", e);
                        }
                    }

                    match load_dev_tools(&loader, &wasm_config.tools_dir).await {
                        Ok(results) => {
                            dev_loaded_tool_names.extend(results.loaded.iter().cloned());
                            if !dev_loaded_tool_names.is_empty() {
                                tracing::info!(
                                    "Loaded {} dev WASM tools from build artifacts",
                                    dev_loaded_tool_names.len()
                                );
                            }
                        }
                        Err(e) => {
                            tracing::debug!("No dev WASM tools found: {}", e);
                        }
                    }
                }

                dev_loaded_tool_names
            }
        };

        let mcp_servers_future = {
            let secrets_store = self.secrets_store.clone();
            let db = self.db.clone();
            let tools = Arc::clone(tools);
            let mcp_sm = Arc::clone(&mcp_session_manager);
            async move {
                if let Some(ref secrets) = secrets_store {
                    let servers_result = if let Some(ref d) = db {
                        load_mcp_servers_from_db(d.as_ref(), "default").await
                    } else {
                        crate::tools::mcp::config::load_mcp_servers().await
                    };
                    match servers_result {
                        Ok(servers) => {
                            let enabled: Vec<_> = servers.enabled_servers().cloned().collect();
                            if !enabled.is_empty() {
                                tracing::info!(
                                    "Loading {} configured MCP server(s)...",
                                    enabled.len()
                                );
                            }

                            let mut join_set = tokio::task::JoinSet::new();
                            for server in enabled {
                                let mcp_sm = Arc::clone(&mcp_sm);
                                let secrets = Arc::clone(secrets);
                                let tools = Arc::clone(&tools);

                                join_set.spawn(async move {
                                    let server_name = server.name.clone();
                                    let has_tokens =
                                        is_authenticated(&server, &secrets, "default").await;

                                    let client = if has_tokens || server.requires_auth() {
                                        McpClient::new_authenticated(
                                            server, mcp_sm, secrets, "default",
                                        )
                                    } else {
                                        McpClient::new_with_name(&server_name, &server.url)
                                    };

                                    match client.list_tools().await {
                                        Ok(mcp_tools) => {
                                            let tool_count = mcp_tools.len();
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
                                            if err_str.contains("401")
                                                || err_str.contains("authentication")
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
            }
        };

        let (dev_loaded_tool_names, _) = tokio::join!(wasm_tools_future, mcp_servers_future);

        // Load registry catalog entries for extension discovery
        let catalog_entries = match crate::registry::RegistryCatalog::load_or_embedded() {
            Ok(catalog) => {
                let entries: Vec<_> = catalog
                    .all()
                    .iter()
                    .map(|m| m.to_registry_entry())
                    .collect();
                tracing::info!(
                    count = entries.len(),
                    "Loaded registry catalog entries for extension discovery"
                );
                entries
            }
            Err(e) => {
                tracing::warn!("Failed to load registry catalog: {}", e);
                Vec::new()
            }
        };

        // Create extension manager. Use ephemeral in-memory secrets if no
        // persistent store is configured (listing/install/activate still work).
        let ext_secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> = if let Some(ref s) =
            self.secrets_store
        {
            Arc::clone(s)
        } else {
            use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
            let ephemeral_key =
                secrecy::SecretString::from(crate::secrets::keychain::generate_master_key_hex());
            let crypto = Arc::new(SecretsCrypto::new(ephemeral_key).expect("ephemeral crypto"));
            tracing::debug!("Using ephemeral in-memory secrets store for extension manager");
            Arc::new(InMemorySecretsStore::new(crypto))
        };
        let extension_manager = {
            let manager = Arc::new(ExtensionManager::new(
                Arc::clone(&mcp_session_manager),
                ext_secrets,
                Arc::clone(tools),
                Some(Arc::clone(hooks)),
                wasm_tool_runtime.clone(),
                self.config.wasm.tools_dir.clone(),
                self.config.channels.wasm_channels_dir.clone(),
                self.config.tunnel.public_url.clone(),
                "default".to_string(),
                self.db.clone(),
                catalog_entries.clone(),
            ));
            tools.register_extension_tools(Arc::clone(&manager));
            tracing::info!("Extension manager initialized with in-chat discovery tools");
            Some(manager)
        };

        // register_builder_tool() already calls register_dev_tools() internally,
        // so only register them here when the builder didn't already do it.
        let builder_registered_dev_tools = self.config.builder.enabled
            && (self.config.agent.allow_local_tools || !self.config.sandbox.enabled);
        if self.config.agent.allow_local_tools && !builder_registered_dev_tools {
            tools.register_dev_tools();
        }

        Ok((
            mcp_session_manager,
            wasm_tool_runtime,
            extension_manager,
            catalog_entries,
            dev_loaded_tool_names,
        ))
    }

    /// Run all init phases in order and return the assembled components.
    pub async fn build_all(mut self) -> Result<AppComponents, anyhow::Error> {
        self.init_database().await?;
        self.init_secrets().await?;

        let (llm, cheap_llm) = self.init_llm()?;
        let (safety, tools, embeddings, workspace) = self.init_tools(&llm).await?;

        // Create hook registry early so runtime extension activation can register hooks.
        let hooks = Arc::new(HookRegistry::new());

        let (
            mcp_session_manager,
            wasm_tool_runtime,
            extension_manager,
            catalog_entries,
            dev_loaded_tool_names,
        ) = self.init_extensions(&tools, &hooks).await?;

        // Seed workspace and backfill embeddings
        if let Some(ref ws) = workspace {
            match ws.seed_if_empty().await {
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Failed to seed workspace: {}", e);
                }
            }

            if embeddings.is_some() {
                let ws_bg = Arc::clone(ws);
                tokio::spawn(async move {
                    match ws_bg.backfill_embeddings().await {
                        Ok(count) if count > 0 => {
                            tracing::info!("Backfilled embeddings for {} chunks", count);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!("Failed to backfill embeddings: {}", e);
                        }
                    }
                });
            }
        }

        // Skills system
        let (skill_registry, skill_catalog) = if self.config.skills.enabled {
            let mut registry = SkillRegistry::new(self.config.skills.local_dir.clone())
                .with_installed_dir(self.config.skills.installed_dir.clone());
            let loaded = registry.discover_all().await;
            if !loaded.is_empty() {
                tracing::info!("Loaded {} skill(s): {}", loaded.len(), loaded.join(", "));
            }
            let registry = Arc::new(std::sync::RwLock::new(registry));
            let catalog = crate::skills::catalog::shared_catalog();
            tools.register_skill_tools(Arc::clone(&registry), Arc::clone(&catalog));
            (Some(registry), Some(catalog))
        } else {
            (None, None)
        };

        let context_manager = Arc::new(ContextManager::new(self.config.agent.max_parallel_jobs));
        let cost_guard = Arc::new(crate::agent::cost_guard::CostGuard::new(
            crate::agent::cost_guard::CostGuardConfig {
                max_cost_per_day_cents: self.config.agent.max_cost_per_day_cents,
                max_actions_per_hour: self.config.agent.max_actions_per_hour,
            },
        ));

        tracing::info!(
            "Tool registry initialized with {} total tools",
            tools.count()
        );

        Ok(AppComponents {
            config: self.config,
            db: self.db,
            secrets_store: self.secrets_store,
            llm,
            cheap_llm,
            safety,
            tools,
            embeddings,
            workspace,
            extension_manager,
            mcp_session_manager,
            wasm_tool_runtime,
            log_broadcaster: self.log_broadcaster,
            context_manager,
            hooks,
            skill_registry,
            skill_catalog,
            cost_guard,
            session: self.session,
            catalog_entries,
            dev_loaded_tool_names,
        })
    }
}
