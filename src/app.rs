//! Application builder for initializing core IronClaw components.
//!
//! Extracts the mechanical initialization phases from `main.rs` into a
//! reusable builder so that:
//!
//! - Tests can construct a full `AppComponents` without wiring channels
//! - Main stays focused on CLI dispatch and channel setup
//! - Each init phase is independently testable

use std::sync::Arc;

use crate::agent::SessionManager as AgentSessionManager;
use crate::channels::web::log_layer::LogBroadcaster;
use crate::config::Config;
use crate::context::ContextManager;
use crate::db::{Database, UserStore};
use crate::extensions::ExtensionManager;
use crate::hooks::HookRegistry;
use crate::llm::recording::HttpInterceptor;
use crate::llm::{LlmProvider, RecordingLlm, SessionManager};
use crate::secrets::SecretsStore;
use crate::tools::ToolRegistry;
use crate::tools::mcp::{McpProcessManager, McpSessionManager};
use crate::tools::wasm::SharedCredentialRegistry;
use crate::tools::wasm::WasmToolRuntime;
use crate::workspace::{EmbeddingCacheConfig, EmbeddingProvider, Workspace};
use ironclaw_safety::SafetyLayer;
use ironclaw_skills::SkillRegistry;
use ironclaw_skills::catalog::SkillCatalog;

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
    /// Workspace-backed `SettingsStore` adapter that dual-writes settings to
    /// both the legacy `settings` table and `.system/settings/**` workspace
    /// documents. Populated when both `db` and `workspace` are available.
    /// Consumers that only need a `SettingsStore` (permission tools, the
    /// SIGHUP reload handler) should prefer this over the raw `db` so that
    /// runtime settings writes flow through the workspace and pick up schema
    /// validation.
    pub settings_store: Option<Arc<dyn crate::db::SettingsStore + Send + Sync>>,
    /// Concrete cache handle for `flush()` / `invalidate_user()`.
    /// Same instance backing `settings_store` when a cache is active.
    pub settings_cache: Option<Arc<crate::db::cached_settings::CachedSettingsStore>>,
    pub extension_manager: Option<Arc<ExtensionManager>>,
    pub mcp_session_manager: Arc<McpSessionManager>,
    pub mcp_process_manager: Arc<McpProcessManager>,
    pub wasm_tool_runtime: Option<Arc<WasmToolRuntime>>,
    pub log_broadcaster: Arc<LogBroadcaster>,
    pub context_manager: Arc<ContextManager>,
    pub hooks: Arc<HookRegistry>,
    /// Shared thread/session manager used by the standard agent runtime.
    pub agent_session_manager: Arc<AgentSessionManager>,
    pub skill_registry: Option<Arc<std::sync::RwLock<SkillRegistry>>>,
    pub skill_catalog: Option<Arc<SkillCatalog>>,
    pub cost_guard: Arc<crate::agent::cost_guard::CostGuard>,
    pub recording_handle: Option<Arc<RecordingLlm>>,
    pub http_interceptor: Option<Arc<dyn HttpInterceptor>>,
    pub session: Arc<SessionManager>,
    pub catalog_entries: Vec<crate::extensions::RegistryEntry>,
    pub dev_loaded_tool_names: Vec<String>,
    pub builder: Option<Arc<dyn crate::tools::SoftwareBuilder>>,
    /// In-process write-through cache: `(channel, external_id)` → `Identity`.
    /// Populated by the pairing flow (Task 8). Pre-allocated here so all
    /// subsystems can hold an `Arc` to the same cache instance.
    pub ownership_cache: Arc<crate::ownership::OwnershipCache>,
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

    // Test overrides
    llm_override: Option<Arc<dyn LlmProvider>>,

    // Backend-specific handles needed by secrets store
    handles: Option<crate::db::DatabaseHandles>,
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
            llm_override: None,
            handles: None,
        }
    }

    /// Inject a pre-created database, skipping `init_database()`.
    ///
    /// **Warning:** this leaves `self.handles` as `None`, which means
    /// `init_secrets()` cannot construct a real `SecretsStore` (the store
    /// needs a backend-specific handle, not the generic `Arc<dyn Database>`).
    /// Tests that need credentials/OAuth/encrypted secrets must use
    /// [`AppBuilder::with_database_and_handles`] instead so the secrets
    /// path stays wired.
    pub fn with_database(&mut self, db: Arc<dyn Database>) {
        self.db = Some(db);
    }

    /// Inject a pre-created database **and** the matching backend-specific
    /// handles, skipping `init_database()`.
    ///
    /// Use this whenever the test will exercise code paths that touch
    /// `SecretsStore` (OAuth, encrypted credentials, secrets-backed WASM
    /// tools). For libSQL backends the handles are constructed via
    /// `LibSqlBackend::shared_db()`; for PostgreSQL via `PgBackend::pool()`.
    pub fn with_database_and_handles(
        &mut self,
        db: Arc<dyn Database>,
        handles: crate::db::DatabaseHandles,
    ) {
        self.db = Some(db);
        self.handles = Some(handles);
    }

    /// Inject a pre-created LLM provider, skipping `init_llm()`.
    pub fn with_llm(&mut self, llm: Arc<dyn LlmProvider>) {
        self.llm_override = Some(llm);
    }

    /// Phase 1: Initialize database backend.
    ///
    /// Creates the database connection, runs migrations, reloads config
    /// from DB, attaches DB to session manager, and cleans up stale jobs.
    pub async fn init_database(&mut self) -> Result<(), anyhow::Error> {
        if self.db.is_some() {
            tracing::debug!("Database already provided, skipping init_database()");
            return Ok(());
        }

        if self.flags.no_db {
            tracing::warn!("Running without database connection");
            return Ok(());
        }

        let (db, handles) = crate::db::connect_with_handles(&self.config.database)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        self.handles = Some(handles);

        // Post-init: ensure owner user row exists and rewrite 'default' user_id rows.
        bootstrap_ownership(db.as_ref(), &self.config)
            .await
            .map_err(|e| anyhow::anyhow!("bootstrap_ownership failed: {e}"))?;

        // Post-init: migrate disk config, reload config from DB, attach session, cleanup
        if let Err(e) =
            crate::bootstrap::migrate_disk_to_db(db.as_ref(), &self.config.owner_id).await
        {
            tracing::warn!("Disk-to-DB settings migration failed: {}", e);
        }

        let toml_path = self.toml_path.as_deref();
        // is_operator=true: owner_id is the operator/admin scope.
        match Config::from_db_with_toml(db.as_ref(), &self.config.owner_id, toml_path, true).await {
            Ok(db_config) => {
                self.config = db_config;
                tracing::debug!("Configuration reloaded from database");
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to reload config from DB, keeping env-based config: {}",
                    e
                );
            }
        }

        self.session
            .attach_store(db.clone(), &self.config.owner_id)
            .await;

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
                // No secrets DB available, but we can still load tokens from
                // OS credential stores (e.g., Anthropic OAuth via Claude Code's
                // macOS Keychain / Linux ~/.claude/.credentials.json).
                crate::config::inject_os_credentials();

                // Consume unused handles
                self.handles.take();

                // Re-resolve only the LLM config with OS credentials.
                let store: Option<&(dyn crate::db::SettingsStore + Sync)> =
                    self.db.as_ref().map(|db| db.as_ref() as _);
                let toml_path = self.toml_path.as_deref();
                let owner_id = self.config.owner_id.clone();
                if let Err(e) = self
                    .config
                    .re_resolve_llm(store, &owner_id, toml_path)
                    .await
                {
                    tracing::warn!(
                        "Failed to re-resolve LLM config after OS credential injection: {e}"
                    );
                }

                return Ok(());
            }
        };

        let crypto = match crate::secrets::SecretsCrypto::new(master_key.clone()) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::warn!("Failed to initialize secrets crypto: {}", e);
                self.handles.take();
                return Ok(());
            }
        };

        // Fallback covers the no-database path where `init_database` returned
        // early before populating `self.handles`.
        let empty_handles = crate::db::DatabaseHandles::default();
        let handles = self.handles.as_ref().unwrap_or(&empty_handles);
        let store = crate::secrets::create_secrets_store(crypto, handles);

        if let Some(ref secrets) = store {
            // Migrate any plaintext API keys from the settings table to the
            // encrypted secrets store. Idempotent — safe to run on every startup.
            if let Some(ref db) = self.db {
                crate::config::migrate_plaintext_llm_keys(
                    db.as_ref(),
                    secrets.as_ref(),
                    &self.config.owner_id,
                )
                .await;

                // Migrate NEAR AI session token from plaintext settings to
                // encrypted secrets. Idempotent — safe to run on every startup.
                migrate_session_credential(db.as_ref(), secrets.as_ref(), &self.config.owner_id)
                    .await;
            }

            // Inject LLM API keys from encrypted storage
            crate::config::inject_llm_keys_from_secrets(secrets.as_ref(), &self.config.owner_id)
                .await;

            // Re-resolve only the LLM config with newly available keys,
            // including keys hydrated from the secrets store.
            let settings_store: Option<&(dyn crate::db::SettingsStore + Sync)> =
                self.db.as_ref().map(|db| db.as_ref() as _);
            let toml_path = self.toml_path.as_deref();
            let owner_id = self.config.owner_id.clone();
            // is_operator=true: owner_id is the operator/admin scope.
            if let Err(e) = self
                .config
                .re_resolve_llm_with_secrets(
                    settings_store,
                    &owner_id,
                    toml_path,
                    Some(secrets.as_ref()),
                    true,
                )
                .await
            {
                tracing::warn!("Failed to re-resolve LLM config after secret injection: {e}");
            }

            // Wire the secrets store into the session manager so future
            // token saves go to encrypted storage.
            self.session.attach_secrets(Arc::clone(secrets)).await;
        }

        self.secrets_store = store;
        Ok(())
    }

    /// Phase 3: Initialize LLM provider chain.
    ///
    /// Delegates to `build_provider_chain` which applies all decorators
    /// (retry, smart routing, failover, circuit breaker, response cache).
    #[allow(clippy::type_complexity)]
    pub async fn init_llm(
        &self,
    ) -> Result<
        (
            Arc<dyn LlmProvider>,
            Option<Arc<dyn LlmProvider>>,
            Option<Arc<RecordingLlm>>,
        ),
        anyhow::Error,
    > {
        let (llm, cheap_llm, recording_handle) =
            crate::llm::build_provider_chain(&self.config.llm, self.session.clone()).await?;
        Ok((llm, cheap_llm, recording_handle))
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
            Option<Arc<dyn crate::tools::SoftwareBuilder>>,
            Arc<SharedCredentialRegistry>,
            Option<Arc<dyn HttpInterceptor>>,
        ),
        anyhow::Error,
    > {
        let safety = Arc::new(SafetyLayer::new(&self.config.safety));
        tracing::debug!("Safety layer initialized");

        // Initialize tool registry with credential injection support
        let credential_registry = Arc::new(SharedCredentialRegistry::new());
        let engine_version = if crate::bridge::is_engine_v2_enabled() {
            crate::tools::EngineVersion::V2
        } else {
            crate::tools::EngineVersion::V1
        };
        let mut registry = ToolRegistry::new().with_engine_version(engine_version);
        if let Some(ref db) = self.db {
            registry = registry.with_database(Arc::clone(db));
        }
        if let Some(ref ss) = self.secrets_store {
            registry = registry.with_credentials(Arc::clone(&credential_registry), Arc::clone(ss));
        }
        // Test-only HTTP host remapping. Gated to debug/test builds so a stray
        // `IRONCLAW_TEST_HTTP_REMAP` env var on a release deployment cannot
        // silently redirect outbound HTTP from production to a test endpoint.
        let http_interceptor = if cfg!(any(test, debug_assertions)) {
            crate::http_intercept::remap_from_env()
        } else {
            None
        };
        if let Some(ref interceptor) = http_interceptor {
            registry = registry.with_http_interceptor(Arc::clone(interceptor));
        }
        let tools = Arc::new(registry);
        tools.register_builtin_tools();
        tools.register_tool_info();
        tools.register_system_tools();

        if let Some(ref ss) = self.secrets_store {
            tools.register_secrets_tools(Arc::clone(ss));
        }

        // Create embeddings provider using the unified method
        let embeddings = self
            .config
            .embeddings
            .create_provider(
                &self.config.llm.nearai.base_url,
                self.session.clone(),
                self.config.llm.bedrock.as_ref(),
            )
            .await;

        // Register memory tools if database is available
        let workspace_user_id = self.config.owner_id.as_str();
        let workspace = if let Some(ref db) = self.db {
            let emb_cache_config = EmbeddingCacheConfig {
                max_entries: self.config.embeddings.cache_size,
            };
            let mut ws = Workspace::new_with_db(workspace_user_id, db.clone())
                .with_search_config(&self.config.search);

            if let Some(ref emb) = embeddings {
                ws = ws.with_embeddings_cached(emb.clone(), emb_cache_config.clone());
            }

            // Wire workspace-level settings (read scopes, memory layers)
            if !self.config.workspace.read_scopes.is_empty() {
                ws = ws.with_additional_read_scopes(self.config.workspace.read_scopes.clone());
                tracing::info!(
                    user_id = workspace_user_id,
                    read_scopes = ?ws.read_user_ids(),
                    "Workspace configured with multi-scope reads"
                );
            }
            ws = ws.with_memory_layers(self.config.workspace.memory_layers.clone());

            // Memory tools must resolve by `ctx.user_id`, not a fixed startup
            // workspace. Even outside authenticated multi-tenant mode, some
            // channels and test harnesses route non-owner users through
            // per-user tenant workspaces seeded on demand.
            let is_multi_tenant = db.has_any_users().await.unwrap_or(false);

            // In multi-tenant mode, enable admin system prompt on the owner
            // workspace so the dispatcher reads SYSTEM.md from __admin__ scope.
            //
            // NOTE: `is_multi_tenant` is evaluated once at startup. If the
            // server starts with no users (single-user mode) and users are
            // added later, the owner workspace frozen in `Arc` will NOT have
            // `admin_prompt_enabled`. A server restart is required after the
            // first user is created to activate admin prompts on the owner
            // workspace. Tenant workspaces created via `WorkspacePool` are
            // unaffected — they always call `.with_admin_prompt()`.
            if is_multi_tenant {
                ws = ws.with_admin_prompt();
            }

            let ws = Arc::new(ws);
            let pool = Arc::new(crate::channels::web::server::WorkspacePool::new(
                Arc::clone(db),
                embeddings.clone(),
                emb_cache_config,
                self.config.search.clone(),
                self.config.workspace.clone(),
            ));
            tools.register_memory_tools_with_resolver(pool);
            tracing::debug!(
                multi_tenant = is_multi_tenant,
                "Memory tools configured with per-user workspace resolver"
            );

            Some(ws)
        } else {
            None
        };

        // Register image/vision tools if we have a workspace and LLM API credentials
        if workspace.is_some() {
            let (api_base, api_key_opt) = if let Some(ref provider) = self.config.llm.provider {
                (
                    provider.base_url.clone(),
                    provider.api_key.as_ref().map(|s| {
                        use secrecy::ExposeSecret;
                        s.expose_secret().to_string()
                    }),
                )
            } else {
                (
                    self.config.llm.nearai.base_url.clone(),
                    self.config.llm.nearai.api_key.as_ref().map(|s| {
                        use secrecy::ExposeSecret;
                        s.expose_secret().to_string()
                    }),
                )
            };

            if let Some(api_key) = api_key_opt {
                // Check for image generation models
                let model_name = self
                    .config
                    .llm
                    .provider
                    .as_ref()
                    .map(|p| p.model.clone())
                    .unwrap_or_else(|| self.config.llm.nearai.model.clone());
                let models = vec![model_name.clone()];
                let gen_model = crate::llm::image_models::suggest_image_model(&models)
                    .unwrap_or("black-forest-labs/FLUX.2-klein-4B")
                    .to_string();
                tools.register_image_tools(api_base.clone(), api_key.clone(), gen_model, None);

                // Check for vision models
                let vision_model = crate::llm::vision_models::suggest_vision_model(&models)
                    .unwrap_or(&model_name)
                    .to_string();
                tools.register_vision_tools(api_base, api_key, vision_model, None);
            }
        }

        // Register builder tool if enabled
        let builder = if self.config.builder.enabled
            && (self.config.agent.allow_local_tools || !self.config.sandbox.enabled)
        {
            let b = tools
                .register_builder_tool(llm.clone(), Some(self.config.builder.to_builder_config()))
                .await;
            tracing::debug!("Builder mode enabled");
            Some(b)
        } else {
            None
        };

        Ok((
            safety,
            tools,
            embeddings,
            workspace,
            builder,
            credential_registry,
            http_interceptor,
        ))
    }

    /// Phase 5: Load WASM tools, MCP servers, and create extension manager.
    pub async fn init_extensions(
        &self,
        tools: &Arc<ToolRegistry>,
        hooks: &Arc<HookRegistry>,
        settings_store_override: Option<Arc<dyn crate::db::SettingsStore + Send + Sync>>,
    ) -> Result<
        (
            Arc<McpSessionManager>,
            Arc<McpProcessManager>,
            Option<Arc<WasmToolRuntime>>,
            Option<Arc<ExtensionManager>>,
            Vec<crate::extensions::RegistryEntry>,
            Vec<String>,
        ),
        anyhow::Error,
    > {
        use crate::tools::wasm::{WasmToolLoader, load_dev_tools};

        let mcp_session_manager = Arc::new(McpSessionManager::new());
        let mcp_process_manager = Arc::new(McpProcessManager::new());

        // Create WASM tool runtime eagerly so extensions installed after startup
        // (e.g. via the web UI) can still be activated. The tools directory is only
        // needed when loading modules, not for engine initialisation.
        let wasm_tool_runtime: Option<Arc<WasmToolRuntime>> = if self.config.wasm.enabled {
            WasmToolRuntime::new(self.config.wasm.to_runtime_config())
                .map(Arc::new)
                .map_err(|e| tracing::warn!("Failed to initialize WASM runtime: {}", e))
                .ok()
        } else {
            None
        };

        // Load WASM tools and MCP servers concurrently
        let wasm_tools_future = {
            let wasm_tool_runtime = wasm_tool_runtime.clone();
            let secrets_store = self.secrets_store.clone();
            let tools = Arc::clone(tools);
            let wasm_config = self.config.wasm.clone();
            let db = self.db.clone();
            async move {
                let mut dev_loaded_tool_names: Vec<String> = Vec::new();

                if let Some(ref runtime) = wasm_tool_runtime {
                    let mut loader = WasmToolLoader::new(Arc::clone(runtime), Arc::clone(&tools));
                    if let Some(ref secrets) = secrets_store {
                        loader = loader.with_secrets_store(Arc::clone(secrets));
                    }
                    if let Some(ref db) = db {
                        let role_lookup: Arc<dyn UserStore> = db.clone();
                        loader = loader.with_role_lookup(role_lookup);
                    }

                    match loader.load_from_dir(&wasm_config.tools_dir).await {
                        Ok(results) => {
                            if !results.loaded.is_empty() {
                                tracing::debug!(
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
                                tracing::debug!(
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
            let pm = Arc::clone(&mcp_process_manager);
            let owner_id = self.config.owner_id.clone();
            async move {
                let servers_result =
                    crate::tools::mcp::config::load_mcp_servers_ready(db.as_deref(), &owner_id)
                        .await;
                match servers_result {
                    Ok(servers) => {
                        let enabled: Vec<_> = servers.enabled_servers().cloned().collect();
                        if !enabled.is_empty() {
                            tracing::debug!(
                                "Loading {} configured MCP server(s)...",
                                enabled.len()
                            );
                        }

                        let mut join_set = tokio::task::JoinSet::new();
                        for server in enabled {
                            let mcp_sm = Arc::clone(&mcp_sm);
                            let secrets = secrets_store.clone();
                            let tools = Arc::clone(&tools);
                            let pm = Arc::clone(&pm);
                            let owner_id = owner_id.clone();

                            join_set.spawn(async move {
                                let server_name = server.name.clone();
                                let has_custom_auth_header = server.has_custom_auth_header();

                                let client = match crate::tools::mcp::create_client_from_config(
                                    server,
                                    &mcp_sm,
                                    &pm,
                                    secrets,
                                    &owner_id,
                                )
                                .await
                                {
                                    Ok(c) => c,
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to create MCP client for '{}': {}",
                                            server_name,
                                            e
                                        );
                                        return None;
                                    }
                                };

                                match client.list_tools().await {
                                    Ok(mcp_tools) => {
                                        let tool_count = mcp_tools.len();
                                        match client.create_tools().await {
                                            Ok(tool_impls) => {
                                                for tool in tool_impls {
                                                    tools.register(tool).await;
                                                }
                                                tracing::debug!(
                                                    "Loaded {} tools from MCP server '{}'",
                                                    tool_count,
                                                    server_name
                                                );
                                                return Some((
                                                    server_name,
                                                    Arc::new(client),
                                                ));
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
                                        if crate::tools::mcp::is_auth_error_message(&err_str)
                                        {
                                            if has_custom_auth_header {
                                                tracing::warn!(
                                                    "MCP server '{}' rejected its configured Authorization header. Update the configured credential and try again.",
                                                    server_name
                                                );
                                            } else {
                                                tracing::warn!(
                                                    "MCP server '{}' requires authentication. \
                                                     Run: ironclaw mcp auth {}",
                                                    server_name,
                                                    server_name
                                                );
                                            }
                                        } else {
                                            tracing::warn!(
                                                "Failed to connect to MCP server '{}': {}",
                                                server_name,
                                                e
                                            );
                                        }
                                    }
                                }
                                None
                            });
                        }

                        let mut startup_clients = Vec::new();
                        while let Some(result) = join_set.join_next().await {
                            match result {
                                Ok(Some(client_pair)) => {
                                    startup_clients.push(client_pair);
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    if e.is_panic() {
                                        tracing::error!("MCP server loading task panicked: {}", e);
                                    } else {
                                        tracing::warn!("MCP server loading task failed: {}", e);
                                    }
                                }
                            }
                        }
                        return startup_clients;
                    }
                    Err(e) => {
                        if matches!(
                            e,
                            crate::tools::mcp::config::ConfigError::InvalidConfig { .. }
                                | crate::tools::mcp::config::ConfigError::Json(_)
                        ) {
                            tracing::warn!(
                                "MCP server configuration is invalid: {}. \
                                 Fix or remove the corrupted config.",
                                e
                            );
                        } else {
                            tracing::debug!("No MCP servers configured ({})", e);
                        }
                    }
                }
                Vec::new()
            }
        };

        let (dev_loaded_tool_names, startup_mcp_clients) =
            tokio::join!(wasm_tools_future, mcp_servers_future);

        // Load registry catalog entries for extension discovery
        let mut catalog_entries = match crate::registry::RegistryCatalog::load_or_embedded() {
            Ok(catalog) => {
                let entries = catalog.discovery_entries();
                tracing::debug!(
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

        // Append builtin entries (e.g. channel-relay integrations) so they appear
        // in the web UI's available extensions list.
        let builtin = crate::extensions::registry::builtin_entries();
        for entry in builtin {
            if !catalog_entries.iter().any(|e| e.name == entry.name) {
                catalog_entries.push(entry);
            }
        }

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
            let mut em = ExtensionManager::new(
                Arc::clone(&mcp_session_manager),
                Arc::clone(&mcp_process_manager),
                ext_secrets,
                Arc::clone(tools),
                Some(Arc::clone(hooks)),
                wasm_tool_runtime.clone(),
                self.config.wasm.tools_dir.clone(),
                self.config.channels.wasm_channels_dir.clone(),
                self.config.tunnel.public_url.clone(),
                self.config.owner_id.clone(),
                self.db.clone(),
                catalog_entries.clone(),
            );
            if let Some(ref ss) = settings_store_override {
                em = em.with_settings_store(Arc::clone(ss));
            }
            let manager = Arc::new(em);
            tools.register_extension_tools(Arc::clone(&manager));

            // Register permission management tool and upgrade tool_list with
            // builtin registry support. Prefer the workspace-backed adapter
            // when the caller provides one (production wiring) so settings
            // writes flow through schema validation; fall back to the raw db
            // for test harnesses that don't have a workspace.
            let settings_store_for_perms: Option<Arc<dyn crate::db::SettingsStore + Send + Sync>> =
                settings_store_override.clone().or_else(|| {
                    self.db
                        .as_ref()
                        .map(|db| Arc::clone(db) as Arc<dyn crate::db::SettingsStore + Send + Sync>)
                });
            tools.register_permission_tools(settings_store_for_perms.clone());
            tools.upgrade_tool_list(Arc::clone(&manager), settings_store_for_perms);

            tracing::debug!("Extension manager initialized with in-chat discovery tools");

            if !startup_mcp_clients.is_empty() {
                tracing::info!(
                    count = startup_mcp_clients.len(),
                    "Injecting startup MCP clients into extension manager"
                );
                for (name, client) in startup_mcp_clients {
                    manager.inject_mcp_client(name, client).await;
                }
            }

            Some(manager)
        };

        // Validate ACP agent configs at startup (lightweight — no connections, just config check).
        {
            let acp_agents = if let Some(ref d) = self.db {
                crate::config::acp::load_acp_agents_from_db(d.as_ref(), &self.config.owner_id).await
            } else {
                crate::config::acp::load_acp_agents().await
            };
            match acp_agents {
                Ok(file) => {
                    let enabled: Vec<_> = file.enabled_agents().collect();
                    if !enabled.is_empty() {
                        let names: Vec<&str> = enabled.iter().map(|a| a.name.as_str()).collect();
                        tracing::info!(
                            "ACP agents configured: {} ({} enabled)",
                            names.join(", "),
                            enabled.len()
                        );
                    }
                }
                Err(e) => {
                    tracing::debug!("No ACP agents configured ({})", e);
                }
            }
        }

        // register_builder_tool() already calls register_dev_tools() internally,
        // so only register them here when the builder didn't already do it.
        let builder_registered_dev_tools = self.config.builder.enabled
            && (self.config.agent.allow_local_tools || !self.config.sandbox.enabled);
        if self.config.agent.allow_local_tools && !builder_registered_dev_tools {
            tools.register_dev_tools();
        }

        Ok((
            mcp_session_manager,
            mcp_process_manager,
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

        // Post-init validation: backends with dedicated config (nearai, gemini_oauth,
        // bedrock, openai_codex) handle their own credential resolution. For registry-based
        // backends, fail early if no provider config was resolved.
        if !matches!(
            self.config.llm.backend.as_str(),
            "nearai" | "gemini_oauth" | "bedrock" | "openai_codex"
        ) && self.config.llm.provider.is_none()
        {
            let backend = &self.config.llm.backend;
            anyhow::bail!(
                "LLM_BACKEND={backend} is configured but no credentials were found. \
                 Set the appropriate API key environment variable or run the setup wizard."
            );
        }

        let (llm, cheap_llm, recording_handle) = if let Some(llm) = self.llm_override.take() {
            (llm, None, None)
        } else {
            self.init_llm().await?
        };
        let (safety, tools, embeddings, workspace, builder, credential_registry, http_interceptor) =
            self.init_tools(&llm).await?;

        // Create hook registry early so runtime extension activation can register hooks.
        let hooks = Arc::new(HookRegistry::new());
        let agent_session_manager =
            Arc::new(AgentSessionManager::new().with_hooks(Arc::clone(&hooks)));

        // Build the workspace-backed `SettingsStore` BEFORE init_extensions so
        // tools registered there (`register_permission_tools`,
        // `upgrade_tool_list`) can be wired with the adapter from the start.
        // The same adapter instance is then exposed on `AppComponents.settings_store`
        // and reused by main.rs (e.g. for the SIGHUP reload handler).
        let (settings_store, settings_cache): (
            Option<Arc<dyn crate::db::SettingsStore + Send + Sync>>,
            Option<Arc<crate::db::cached_settings::CachedSettingsStore>>,
        ) = match (&workspace, &self.db) {
            (Some(ws), Some(db)) => {
                let adapter = Arc::new(crate::workspace::WorkspaceSettingsAdapter::new(
                    Arc::clone(ws),
                    Arc::clone(db),
                ));
                if let Err(e) = adapter.ensure_system_config().await {
                    tracing::debug!(
                        "WorkspaceSettingsAdapter eager seed failed (lazy seed will retry): {e}"
                    );
                }
                let cached = Arc::new(crate::db::cached_settings::CachedSettingsStore::new(
                    adapter as Arc<dyn crate::db::SettingsStore + Send + Sync>,
                ));
                (
                    Some(Arc::clone(&cached) as Arc<dyn crate::db::SettingsStore + Send + Sync>),
                    Some(cached),
                )
            }
            _ => (None, None),
        };

        let (
            mcp_session_manager,
            mcp_process_manager,
            wasm_tool_runtime,
            extension_manager,
            catalog_entries,
            dev_loaded_tool_names,
        ) = self
            .init_extensions(&tools, &hooks, settings_store.clone())
            .await?;

        // Load bootstrap-completed flag from settings so that existing users
        // who already completed onboarding don't re-get bootstrap injection.
        if let Some(ref ws) = workspace {
            let toml_path = crate::settings::Settings::default_toml_path();
            if let Ok(Some(settings)) = crate::settings::Settings::load_toml(&toml_path)
                && settings.profile_onboarding_completed
            {
                ws.mark_bootstrap_completed();
            }
        }

        // Seed workspace and backfill embeddings
        if let Some(ref ws) = workspace {
            // Import workspace files from disk FIRST if WORKSPACE_IMPORT_DIR is set.
            // This lets Docker images / deployment scripts ship customized
            // workspace templates (e.g., AGENTS.md, TOOLS.md) that override
            // the generic seeds. Only imports files that don't already exist
            // in the database — never overwrites user edits.
            //
            // Runs before seed_if_empty() so that custom templates take priority
            // over generic seeds. seed_if_empty() then fills any remaining gaps.
            if let Ok(import_dir) = std::env::var("WORKSPACE_IMPORT_DIR") {
                let import_path = std::path::Path::new(&import_dir);
                match ws.import_from_directory(import_path).await {
                    Ok(count) if count > 0 => {
                        tracing::debug!("Imported {} workspace file(s) from {}", count, import_dir);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(
                            "Failed to import workspace files from {}: {}",
                            import_dir,
                            e
                        );
                    }
                }
            }

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
                            tracing::debug!("Backfilled embeddings for {} chunks", count);
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
                .with_installed_dir(self.config.skills.installed_dir.clone())
                .with_bundled_content(crate::skills::bundled::load_bundled_skills())
                .with_max_scan_depth(self.config.skills.max_scan_depth);
            let loaded = registry.discover_all().await;
            if !loaded.is_empty() {
                tracing::debug!("Loaded {} skill(s): {}", loaded.len(), loaded.join(", "));
            }

            // Register credential mappings from skill frontmatter into the
            // shared registry so the HTTP tool can auto-inject credentials.
            crate::skills::register_skill_credentials(registry.skills(), &credential_registry);
            if let Some(db) = self.db.as_ref() {
                crate::skills::persist_skill_auth_descriptors(
                    registry.skills(),
                    Some(db.as_ref()),
                    &self.config.owner_id,
                )
                .await;
            }

            let registry = Arc::new(std::sync::RwLock::new(registry));
            let catalog = ironclaw_skills::catalog::shared_catalog();
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
                max_cost_per_user_per_day_cents: self.config.agent.max_cost_per_user_per_day_cents,
            },
        ));

        tracing::debug!(
            "Tool registry initialized with {} total tools",
            tools.count()
        );

        // Seed per-user tool permission defaults into the database.
        // This runs after all tools (built-in, WASM, MCP) are registered so
        // that every tool name is known.  Existing entries are never overwritten.
        seed_tool_permissions(&tools, self.db.as_ref(), &self.config.owner_id).await;

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
            settings_store,
            settings_cache,
            extension_manager,
            mcp_session_manager,
            mcp_process_manager,
            wasm_tool_runtime,
            log_broadcaster: self.log_broadcaster,
            context_manager,
            hooks,
            agent_session_manager,
            skill_registry,
            skill_catalog,
            cost_guard,
            recording_handle,
            http_interceptor,
            session: self.session,
            catalog_entries,
            dev_loaded_tool_names,
            builder,
            ownership_cache: Arc::new(crate::ownership::OwnershipCache::new()),
        })
    }
}

/// FK constraints applied after bootstrap_ownership rewrites 'default' rows.
/// NOT applied by the automatic refinery sweep — applied programmatically below.
///
/// PostgreSQL uses `ADD CONSTRAINT IF NOT EXISTS` to be idempotent.
/// libSQL (SQLite) does not support `ADD CONSTRAINT` at all — FK enforcement
/// there is handled by `PRAGMA foreign_keys = ON` in the schema declarations.
// TODO(ownership): Apply OWNERSHIP_FK_SQL on PostgreSQL after bootstrap completes.
// Requires detecting the database backend type from the Database trait object.
#[allow(dead_code)]
const OWNERSHIP_FK_SQL: &str = r#"
ALTER TABLE conversations    ADD CONSTRAINT IF NOT EXISTS fk_conversations_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE memory_documents ADD CONSTRAINT IF NOT EXISTS fk_memory_documents_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE heartbeat_state  ADD CONSTRAINT IF NOT EXISTS fk_heartbeat_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE secrets          ADD CONSTRAINT IF NOT EXISTS fk_secrets_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE wasm_tools       ADD CONSTRAINT IF NOT EXISTS fk_wasm_tools_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE routines         ADD CONSTRAINT IF NOT EXISTS fk_routines_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE settings         ADD CONSTRAINT IF NOT EXISTS fk_settings_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE agent_jobs       ADD CONSTRAINT IF NOT EXISTS fk_agent_jobs_user
    FOREIGN KEY (user_id) REFERENCES users(id);
"#;

/// Runs on every startup after migrations V1–V20.
/// Idempotent — safe to call multiple times.
///
/// 1. Ensures the owner user row exists in `users`.
/// 2. Rewrites all `user_id = 'default'` rows to the real owner_id.
pub async fn bootstrap_ownership(
    db: &dyn crate::db::Database,
    config: &crate::config::Config,
) -> Result<(), anyhow::Error> {
    let owner_id = &config.owner_id;

    // 1. Ensure owner user exists
    db.get_or_create_user(crate::db::UserRecord {
        id: owner_id.clone(),
        role: "admin".to_string(),
        display_name: "Owner".to_string(),
        status: "active".to_string(),
        email: None,
        last_login_at: None,
        created_by: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        metadata: serde_json::Value::Object(Default::default()),
    })
    .await?;

    // 2. Rewrite 'default' rows to the real owner_id
    db.migrate_default_owner(owner_id).await?;

    tracing::info!(
        owner_id = %owner_id,
        "bootstrap_ownership: owner user ensured, default rows migrated"
    );
    Ok(())
}

/// Migrate the NEAR AI session token from the plaintext settings table to the
/// encrypted secrets store.
///
/// The `nearai.session_token` settings key stores a JSON-serialized `SessionData`
/// object. This migration re-serializes it as a JSON string and stores it under
/// the `nearai_session_token` secret name.
///
/// Idempotent: if the secret already exists, the settings key is removed (cleanup).
/// If the settings key is absent, nothing happens.
async fn migrate_session_credential(
    db: &dyn crate::db::Database,
    secrets: &(dyn crate::secrets::SecretsStore + Send + Sync),
    user_id: &str,
) {
    // If already migrated and the secret decrypts to valid JSON, clean up the
    // plaintext copy and return. If the secret exists but is corrupt, fall
    // through to re-migrate from the plaintext settings value.
    match secrets.get_decrypted(user_id, "nearai_session_token").await {
        Ok(decrypted) => {
            if let Ok(secret_value) = serde_json::from_str::<serde_json::Value>(decrypted.expose())
            {
                // Verify the decrypted secret matches the plaintext setting (round-trip check).
                match db.get_setting(user_id, "nearai.session_token").await {
                    Ok(Some(settings_value)) if secret_value == settings_value => {
                        // Round-trip verified — safe to clean up plaintext copy.
                        let _ = db.delete_setting(user_id, "nearai.session_token").await;
                        return;
                    }
                    Ok(Some(_)) => {
                        // Secret doesn't match plaintext — fall through to re-migrate.
                        tracing::warn!(
                            "nearai_session_token secret doesn't match plaintext setting; re-migrating"
                        );
                    }
                    Ok(None) => {
                        // No plaintext left — treat as already migrated.
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to read nearai.session_token setting for round-trip check: {e}"
                        );
                        return;
                    }
                }
            } else {
                // Secret exists but failed JSON parsing — fall through to re-migrate.
                tracing::warn!(
                    "nearai_session_token secret exists but failed JSON validation; re-migrating"
                );
            }
        }
        Err(crate::secrets::SecretError::NotFound(_)) => {
            // Not yet migrated — continue.
        }
        Err(e) => {
            tracing::warn!("Failed to check secrets store for nearai_session_token: {e}");
            return;
        }
    }

    // Read the JSON value from settings.
    let value = match db.get_setting(user_id, "nearai.session_token").await {
        Ok(Some(v)) => v,
        Ok(None) => return, // Nothing to migrate.
        Err(e) => {
            tracing::warn!("Failed to read nearai.session_token from settings: {e}");
            return;
        }
    };

    // Re-serialize the JSON value to a string for secrets storage.
    let value_str = match &value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };

    let params = crate::secrets::CreateSecretParams::new("nearai_session_token", value_str)
        .with_provider("nearai");

    match secrets.create(user_id, params).await {
        Ok(_) => {
            tracing::info!("Migrated nearai.session_token from settings to encrypted secrets");
            let _ = db.delete_setting(user_id, "nearai.session_token").await;
        }
        Err(e) => {
            tracing::warn!("Failed to migrate nearai.session_token to secrets: {e}");
        }
    }
}

/// Seed tool permission defaults into the database for every registered tool
/// that has no explicit user override yet.
///
/// This is called once at startup after the full tool registry is built.
/// It is idempotent: existing entries in `tool_permissions.*` are never touched.
async fn seed_tool_permissions(
    tools: &crate::tools::ToolRegistry,
    db: Option<&Arc<dyn Database>>,
    owner_id: &str,
) {
    use crate::tools::permissions::{TOOL_RISK_DEFAULTS, effective_permission};

    let db = match db {
        Some(db) => db,
        None => {
            tracing::debug!("seed_tool_permissions: no database available, skipping");
            return;
        }
    };

    // Load existing tool permission overrides from the DB.
    let db_map = match db.get_all_settings(owner_id).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("seed_tool_permissions: failed to load settings: {}", e);
            return;
        }
    };
    let existing = crate::settings::Settings::from_db_map(&db_map).tool_permissions;

    let registered_names = tools.list().await;
    let mut seeded = 0u32;

    for name in &registered_names {
        if existing.contains_key(name.as_str()) {
            // User has an explicit override — do not touch it.
            continue;
        }

        // Only insert if the tool appears in the static defaults table.
        // Unknown/dynamic tools stay absent (they will fall back to AskEachTime
        // at runtime via effective_permission) to avoid polluting the DB.
        if TOOL_RISK_DEFAULTS.contains_key(name.as_str()) {
            let default_state = effective_permission(name, &existing);
            let json_value = match serde_json::to_value(default_state) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        "seed_tool_permissions: failed to serialize state for '{}': {}",
                        name,
                        e
                    );
                    continue;
                }
            };
            if let Err(e) = db
                .set_setting(owner_id, &format!("tool_permissions.{}", name), &json_value)
                .await
            {
                tracing::warn!("seed_tool_permissions: failed to set '{}': {}", name, e);
            } else {
                seeded += 1;
            }
        }
    }

    if seeded > 0 {
        tracing::debug!(
            count = seeded,
            "Seeded tool permission defaults into database"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::sync::mpsc;

    use crate::agent::SessionManager as AgentSessionManager;
    use crate::hooks::{
        Hook, HookContext, HookError, HookEvent, HookOutcome, HookPoint, HookRegistry,
    };

    struct SessionStartHook {
        tx: mpsc::UnboundedSender<(String, String)>,
    }

    #[async_trait]
    impl Hook for SessionStartHook {
        fn name(&self) -> &str {
            "session-start-test"
        }

        fn hook_points(&self) -> &[HookPoint] {
            &[HookPoint::OnSessionStart]
        }

        async fn execute(
            &self,
            event: &HookEvent,
            _ctx: &HookContext,
        ) -> Result<HookOutcome, HookError> {
            if let HookEvent::SessionStart {
                user_id,
                session_id,
            } = event
            {
                self.tx
                    .send((user_id.clone(), session_id.clone()))
                    .expect("test channel receiver should be alive");
            } else {
                panic!("SessionStartHook received an unexpected event: {event:?}");
            }
            Ok(HookOutcome::ok())
        }
    }

    #[tokio::test]
    async fn agent_session_manager_runs_session_start_hooks() {
        let hooks = Arc::new(HookRegistry::new());
        let (tx, mut rx) = mpsc::unbounded_channel();
        hooks.register(Arc::new(SessionStartHook { tx })).await;

        let manager = AgentSessionManager::new().with_hooks(Arc::clone(&hooks));
        manager.get_or_create_session("user-123").await;

        let (user_id, session_id) =
            tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                .await
                .expect("session start hook should fire")
                .expect("session start payload should be present");

        assert_eq!(user_id, "user-123");
        assert!(!session_id.is_empty());
    }

    /// Verify that `seed_tool_permissions` is idempotent: an existing user
    /// override must survive a re-seed.
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn seed_tool_permissions_preserves_user_overrides() {
        use crate::db::Database;
        use crate::db::libsql::LibSqlBackend;
        use crate::tools::ToolRegistry;
        use crate::tools::permissions::PermissionState;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_seed.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();
        let db: Arc<dyn Database> = Arc::new(backend);

        let registry = ToolRegistry::new();
        registry.register_builtin_tools();

        let owner = "test-user";

        // 1. Initial seed: creates defaults for all registered tools.
        super::seed_tool_permissions(&registry, Some(&db), owner).await;

        // Verify "echo" was seeded as AlwaysAllow.
        let map = db.get_all_settings(owner).await.unwrap();
        let settings = crate::settings::Settings::from_db_map(&map);
        assert_eq!(
            settings.tool_permissions.get("echo"),
            Some(&PermissionState::AlwaysAllow),
            "echo should be AlwaysAllow after initial seed"
        );

        // 2. User overrides echo → Disabled.
        let disabled_json = serde_json::to_value(PermissionState::Disabled).unwrap();
        db.set_setting(owner, "tool_permissions.echo", &disabled_json)
            .await
            .unwrap();

        // 3. Re-seed (e.g. after a restart).
        super::seed_tool_permissions(&registry, Some(&db), owner).await;

        // 4. Assert the override survived.
        let map = db.get_all_settings(owner).await.unwrap();
        let settings = crate::settings::Settings::from_db_map(&map);
        assert_eq!(
            settings.tool_permissions.get("echo"),
            Some(&PermissionState::Disabled),
            "user override to Disabled must survive re-seed"
        );
    }
}
