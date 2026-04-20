//! Web gateway channel for browser-based access to IronClaw.
//!
//! Provides a single-page web UI with:
//! - Chat with the agent (via REST + SSE)
//! - Workspace/memory browsing
//! - Job management
//!
//! ```text
//! Browser ─── POST /api/chat/send ──► Agent Loop
//!         ◄── GET  /api/chat/events ── SSE stream
//!         ─── GET  /api/chat/ws ─────► WebSocket (bidirectional)
//!         ─── GET  /api/memory/* ────► Workspace
//!         ─── GET  /api/jobs/* ──────► Database
//!         ◄── GET  / ───────────────── Static HTML/CSS/JS
//! ```

pub(crate) mod features;
pub(crate) mod handlers;
pub mod log_layer;
pub mod oauth;
pub(crate) mod onboarding;
pub mod openai_compat;
pub mod platform;
pub mod responses_api;
pub mod server;
pub mod types;
pub(crate) mod util;

// Backward-compat re-exports for the ironclaw#2599 migration. The auth,
// SSE, and WebSocket modules moved to `platform::*` in stage 3; every
// existing `crate::channels::web::{auth,sse,ws}::...` call site
// continues to resolve via these re-exports until a follow-up PR
// updates them directly.
pub use platform::auth;
pub use platform::sse;
pub use platform::ws;

/// Test helpers for gateway tests.
///
/// Always compiled (not behind `#[cfg(test)]`) so that integration tests in
/// `tests/` -- which import this crate as a regular dependency -- can use
/// [`TestGatewayBuilder`](test_helpers::TestGatewayBuilder). The
/// cross-slice `pub(crate)` builders inside the module
/// (`test_gateway_state`, `test_gateway_state_with_dependencies`,
/// `test_gateway_state_with_store_and_session_manager`) are individually
/// `#[cfg(test)]`-gated since they only have in-crate unit-test callers.
pub mod test_helpers;

#[cfg(test)]
mod tests;

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::agent::SessionManager;
use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::config::GatewayConfig;
use crate::db::Database;
use crate::error::ChannelError;
use crate::extensions::ExtensionManager;
use crate::orchestrator::job_manager::ContainerJobManager;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;
use ironclaw_skills::catalog::SkillCatalog;
use ironclaw_skills::registry::SkillRegistry;

use self::log_layer::{LogBroadcaster, LogLevelHandle};

use self::auth::{CombinedAuthState, DbAuthenticator, MultiAuthState};
use self::server::GatewayState;
use self::sse::SseManager;
use self::types::AppEvent;

fn build_gateway_auth_manager(
    state: &GatewayState,
) -> Option<Arc<crate::bridge::auth_manager::AuthManager>> {
    state
        .tool_registry
        .as_ref()
        .and_then(|tr| tr.secrets_store().cloned())
        .or_else(|| state.secrets_store.clone())
        .or_else(|| {
            state
                .extension_manager
                .as_ref()
                .map(|em| Arc::clone(em.secrets()))
        })
        .map(|secrets| {
            Arc::new(crate::bridge::auth_manager::AuthManager::new(
                secrets,
                state.skill_registry.clone(),
                state.extension_manager.clone(),
                state.tool_registry.clone(),
            ))
        })
}

/// Web gateway channel implementing the Channel trait.
pub struct GatewayChannel {
    config: GatewayConfig,
    state: Arc<GatewayState>,
    /// Combined auth state: env-var tokens + optional DB-backed tokens.
    auth: CombinedAuthState,
}

impl GatewayChannel {
    /// Create a new gateway channel.
    ///
    /// If no auth token is configured, generates a random one and prints it.
    /// Builds a single-user `MultiAuthState` from the config.
    pub fn new(config: GatewayConfig, owner_id: String) -> Self {
        let auth_token = config.auth_token.clone().unwrap_or_else(|| {
            use rand::RngCore;
            use rand::rngs::OsRng;
            let mut bytes = [0u8; 32];
            OsRng.fill_bytes(&mut bytes);
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        });

        let oidc_state = config.oidc.as_ref().and_then(|oidc_config| {
            match auth::OidcState::from_config(oidc_config) {
                Ok(state) => {
                    tracing::info!(
                        header = %oidc_config.header,
                        jwks_url = %oidc_config.jwks_url,
                        "OIDC JWT authentication enabled"
                    );
                    Some(state)
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to initialize OIDC auth — falling back to token-only auth");
                    None
                }
            }
        });

        let auth = CombinedAuthState {
            env_auth: MultiAuthState::single(auth_token, owner_id.clone()),
            db_auth: None,
            oidc: oidc_state,
            oidc_allowed_domains: Vec::new(),
        };

        let state = Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: Arc::new(SseManager::with_max_connections_and_buffer(
                config.max_connections,
                config.broadcast_buffer,
            )),
            workspace: None,
            workspace_pool: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: None,
            settings_cache: None,
            job_manager: None,
            prompt_queue: None,
            scheduler: None,
            owner_id,
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: Some(Arc::new(ws::WsConnectionTracker::new())),
            llm_provider: None,
            llm_reload: None,
            llm_session_manager: None,
            config_toml_path: None,
            skill_registry: None,
            skill_catalog: None,
            auth_manager: None,
            chat_rate_limiter: server::PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: server::PerUserRateLimiter::new(20, 60),
            webhook_rate_limiter: server::RateLimiter::new(10, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: Arc::new(tokio::sync::RwLock::new(
                server::ActiveConfigSnapshot::default(),
            )),
            secrets_store: None,
            db_auth: None,
            pairing_store: None,
            oauth_providers: None,
            oauth_state_store: None,
            oauth_base_url: None,
            oauth_allowed_domains: Vec::new(),
            near_nonce_store: None,
            near_rpc_url: None,
            near_network: None,
            oauth_sweep_shutdown: None,
            frontend_html_cache: Arc::new(tokio::sync::RwLock::new(None)),
            tool_dispatcher: None,
        });

        Self {
            config,
            state,
            auth,
        }
    }

    /// Helper to rebuild state, copying existing fields and applying a mutation.
    fn rebuild_state(&mut self, mutate: impl FnOnce(&mut GatewayState)) {
        let mut new_state = GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            // Preserve the existing broadcast channel so sender handles remain valid.
            // The broadcast channel capacity is already baked into `tx` at
            // creation time; `from_sender` cannot resize it.
            sse: Arc::new(SseManager::from_sender(
                self.state.sse.sender(),
                self.state.sse.max_connections(),
            )),
            workspace: self.state.workspace.clone(),
            workspace_pool: self.state.workspace_pool.clone(),
            session_manager: self.state.session_manager.clone(),
            log_broadcaster: self.state.log_broadcaster.clone(),
            log_level_handle: self.state.log_level_handle.clone(),
            extension_manager: self.state.extension_manager.clone(),
            tool_registry: self.state.tool_registry.clone(),
            store: self.state.store.clone(),
            settings_cache: self.state.settings_cache.clone(),
            job_manager: self.state.job_manager.clone(),
            prompt_queue: self.state.prompt_queue.clone(),
            scheduler: self.state.scheduler.clone(),
            owner_id: self.state.owner_id.clone(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: self.state.ws_tracker.clone(),
            llm_provider: self.state.llm_provider.clone(),
            llm_reload: self.state.llm_reload.clone(),
            llm_session_manager: self.state.llm_session_manager.clone(),
            config_toml_path: self.state.config_toml_path.clone(),
            skill_registry: self.state.skill_registry.clone(),
            skill_catalog: self.state.skill_catalog.clone(),
            auth_manager: self.state.auth_manager.clone(),
            chat_rate_limiter: server::PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: server::PerUserRateLimiter::new(20, 60),
            webhook_rate_limiter: server::RateLimiter::new(10, 60),
            registry_entries: self.state.registry_entries.clone(),
            cost_guard: self.state.cost_guard.clone(),
            routine_engine: Arc::clone(&self.state.routine_engine),
            startup_time: self.state.startup_time,
            active_config: Arc::clone(&self.state.active_config),
            secrets_store: self.state.secrets_store.clone(),
            db_auth: self.state.db_auth.clone(),
            pairing_store: self.state.pairing_store.clone(),
            oauth_providers: self.state.oauth_providers.clone(),
            oauth_state_store: self.state.oauth_state_store.clone(),
            oauth_base_url: self.state.oauth_base_url.clone(),
            oauth_allowed_domains: self.state.oauth_allowed_domains.clone(),
            near_nonce_store: self.state.near_nonce_store.clone(),
            near_rpc_url: self.state.near_rpc_url.clone(),
            near_network: self.state.near_network.clone(),
            oauth_sweep_shutdown: None, // sweep tasks are managed by with_oauth
            // Preserve the existing cache — workspace state hasn't changed
            // just because a `with_*` builder added a new subsystem.
            frontend_html_cache: Arc::clone(&self.state.frontend_html_cache),
            tool_dispatcher: self.state.tool_dispatcher.clone(),
        };
        mutate(&mut new_state);
        new_state.auth_manager = build_gateway_auth_manager(&new_state);
        self.state = Arc::new(new_state);
    }

    /// Inject the workspace reference for the memory API.
    pub fn with_workspace(mut self, workspace: Arc<Workspace>) -> Self {
        self.rebuild_state(|s| s.workspace = Some(workspace));
        self
    }

    /// Inject the session manager for thread/session info.
    pub fn with_session_manager(mut self, sm: Arc<SessionManager>) -> Self {
        self.rebuild_state(|s| s.session_manager = Some(sm));
        self
    }

    /// Inject the log broadcaster for the logs SSE endpoint.
    pub fn with_log_broadcaster(mut self, lb: Arc<LogBroadcaster>) -> Self {
        self.rebuild_state(|s| s.log_broadcaster = Some(lb));
        self
    }

    /// Inject the log level handle for runtime log level control.
    pub fn with_log_level_handle(mut self, h: Arc<LogLevelHandle>) -> Self {
        self.rebuild_state(|s| s.log_level_handle = Some(h));
        self
    }

    /// Inject the extension manager for the extensions API.
    pub fn with_extension_manager(mut self, em: Arc<ExtensionManager>) -> Self {
        self.rebuild_state(|s| s.extension_manager = Some(em));
        self
    }

    /// Inject the tool registry for the extensions API.
    pub fn with_tool_registry(mut self, tr: Arc<ToolRegistry>) -> Self {
        self.rebuild_state(|s| s.tool_registry = Some(tr));
        self
    }

    /// Inject the database store for sandbox job persistence.
    pub fn with_store(mut self, store: Arc<dyn Database>) -> Self {
        self.rebuild_state(|s| s.store = Some(store));
        self
    }

    pub fn with_settings_cache(
        mut self,
        cache: Arc<crate::db::cached_settings::CachedSettingsStore>,
    ) -> Self {
        self.rebuild_state(|s| s.settings_cache = Some(cache));
        self
    }

    /// Inject the channel-agnostic tool dispatcher for routing handler
    /// operations through the tool pipeline with audit trail.
    pub fn with_tool_dispatcher(
        mut self,
        dispatcher: Arc<crate::tools::dispatch::ToolDispatcher>,
    ) -> Self {
        self.rebuild_state(|s| s.tool_dispatcher = Some(dispatcher));
        self
    }

    /// Enable DB-backed token authentication alongside env-var tokens.
    pub fn with_db_auth(mut self, store: Arc<dyn Database>) -> Self {
        let authenticator = DbAuthenticator::new(store);
        // Share the same DbAuthenticator (and its cache) between the auth
        // middleware and GatewayState so handlers can invalidate the cache
        // on security-critical actions (suspend, role change, token revoke).
        self.rebuild_state(|s| s.db_auth = Some(Arc::new(authenticator.clone())));
        self.auth.db_auth = Some(authenticator);
        self
    }

    /// Inject the container job manager for sandbox operations.
    pub fn with_job_manager(mut self, jm: Arc<ContainerJobManager>) -> Self {
        self.rebuild_state(|s| s.job_manager = Some(jm));
        self
    }

    /// Inject the prompt queue for Claude Code follow-up prompts.
    pub fn with_prompt_queue(
        mut self,
        pq: Arc<
            tokio::sync::Mutex<
                std::collections::HashMap<
                    uuid::Uuid,
                    std::collections::VecDeque<crate::orchestrator::api::PendingPrompt>,
                >,
            >,
        >,
    ) -> Self {
        self.rebuild_state(|s| s.prompt_queue = Some(pq));
        self
    }

    /// Inject the scheduler for sending follow-up messages to agent jobs.
    pub fn with_scheduler(mut self, slot: crate::tools::builtin::SchedulerSlot) -> Self {
        self.rebuild_state(|s| s.scheduler = Some(slot));
        self
    }

    /// Inject the skill registry for skill management API.
    pub fn with_skill_registry(mut self, sr: Arc<std::sync::RwLock<SkillRegistry>>) -> Self {
        self.rebuild_state(|s| s.skill_registry = Some(sr));
        self
    }

    /// Inject the skill catalog for skill search API.
    pub fn with_skill_catalog(mut self, sc: Arc<SkillCatalog>) -> Self {
        self.rebuild_state(|s| s.skill_catalog = Some(sc));
        self
    }

    /// Inject the LLM provider for OpenAI-compatible API proxy.
    pub fn with_llm_provider(mut self, llm: Arc<dyn crate::llm::LlmProvider>) -> Self {
        self.rebuild_state(|s| s.llm_provider = Some(llm));
        self
    }

    /// Inject the LLM hot-reload controller for the settings handlers.
    pub fn with_llm_reload(mut self, reload: Arc<crate::llm::LlmReloadHandle>) -> Self {
        self.rebuild_state(|s| s.llm_reload = Some(reload));
        self
    }

    /// Inject the LLM session manager so a hot-reload can rebuild the
    /// provider chain without dropping the current auth session.
    pub fn with_llm_session_manager(mut self, sm: Arc<crate::llm::SessionManager>) -> Self {
        self.rebuild_state(|s| s.llm_session_manager = Some(sm));
        self
    }

    /// Inject the TOML config path so `Config::from_db_with_toml` can be
    /// replayed identically during a hot-reload.
    pub fn with_config_toml_path(mut self, path: std::path::PathBuf) -> Self {
        self.rebuild_state(|s| s.config_toml_path = Some(path));
        self
    }

    /// Inject registry catalog entries for the available extensions API.
    pub fn with_registry_entries(mut self, entries: Vec<crate::extensions::RegistryEntry>) -> Self {
        self.rebuild_state(|s| s.registry_entries = entries);
        self
    }

    /// Inject the cost guard for token/cost tracking in the status popover.
    pub fn with_cost_guard(mut self, cg: Arc<crate::agent::cost_guard::CostGuard>) -> Self {
        self.rebuild_state(|s| s.cost_guard = Some(cg));
        self
    }

    /// Inject a shared routine engine slot used by other HTTP ingress paths.
    pub fn with_routine_engine_slot(mut self, slot: server::RoutineEngineSlot) -> Self {
        self.rebuild_state(|s| s.routine_engine = slot);
        self
    }

    /// Inject the active (resolved) configuration snapshot for the status endpoint.
    pub fn with_active_config(mut self, config: server::ActiveConfigSnapshot) -> Self {
        self.rebuild_state(|s| {
            s.active_config = Arc::new(tokio::sync::RwLock::new(config));
        });
        self
    }

    /// Inject the secrets store for admin secret provisioning.
    pub fn with_secrets_store(
        mut self,
        store: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
    ) -> Self {
        self.rebuild_state(|s| s.secrets_store = Some(store));
        self
    }

    /// Enable OAuth social login with the given configuration.
    ///
    /// Creates provider instances for each configured provider, initializes
    /// the in-memory state store, and resolves the callback base URL.
    pub fn with_oauth(mut self, config: crate::config::OAuthConfig, gateway_port: u16) -> Self {
        if !config.enabled {
            return self;
        }

        use crate::channels::web::oauth::providers::{
            AppleProvider, GitHubProvider, GoogleProvider, OAuthProvider,
        };
        use crate::channels::web::oauth::state_store::OAuthStateStore;
        use std::collections::HashMap;

        let mut providers: HashMap<String, Arc<dyn OAuthProvider>> = HashMap::new();

        if let Some(ref google) = config.google {
            providers.insert(
                "google".to_string(),
                Arc::new(GoogleProvider::new(
                    google.client_id.clone(),
                    google.client_secret.clone(),
                    google.allowed_hd.clone(),
                )),
            );
        }

        if let Some(ref github) = config.github {
            providers.insert(
                "github".to_string(),
                Arc::new(GitHubProvider::new(
                    github.client_id.clone(),
                    github.client_secret.clone(),
                )),
            );
        }

        if let Some(ref apple) = config.apple {
            providers.insert(
                "apple".to_string(),
                Arc::new(AppleProvider::new(
                    apple.client_id.clone(),
                    apple.team_id.clone(),
                    apple.key_id.clone(),
                    apple.private_key_pem.clone(),
                )),
            );
        }

        // Apply domain restrictions to OIDC regardless of whether OAuth providers
        // are configured — OIDC runs via reverse-proxy header, not our providers.
        let allowed_domains = config.allowed_domains;
        if !allowed_domains.is_empty() {
            self.auth.oidc_allowed_domains = allowed_domains.clone();
        }

        // Shutdown signal for background sweep tasks. When the sender is dropped
        // (e.g., gateway rebuild or process shutdown), the sweep loops exit.
        let (shutdown_tx, _) = tokio::sync::watch::channel(());

        // Set up NEAR wallet auth if configured (independent of OAuth providers).
        let near_nonce_store = config.near.as_ref().map(|_| {
            let store = Arc::new(crate::channels::web::oauth::near::NearNonceStore::new());
            let sweep = Arc::clone(&store);
            let mut shutdown_rx = shutdown_tx.subscribe();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    tokio::select! {
                        _ = interval.tick() => sweep.sweep_expired().await,
                        _ = shutdown_rx.changed() => break,
                    }
                }
            });
            store
        });
        let near_rpc_url = config.near.as_ref().map(|n| n.rpc_url.clone());
        let near_network = config.near.as_ref().map(|n| n.network.clone());

        let has_near = near_nonce_store.is_some();

        if providers.is_empty() && !has_near {
            // No OAuth providers and no NEAR — still apply domain restrictions
            // to OIDC if configured.
            self.rebuild_state(|s| {
                s.oauth_allowed_domains = allowed_domains;
            });
            if !self.auth.oidc_allowed_domains.is_empty() {
                return self;
            }
            tracing::warn!("OAuth enabled but no providers configured");
            return self;
        }

        let base_url = config
            .base_url
            .unwrap_or_else(|| format!("http://localhost:{gateway_port}"));

        let provider_names: Vec<&str> = providers.keys().map(|s| s.as_str()).collect();
        tracing::info!(?provider_names, "OAuth social login enabled");

        let providers = Arc::new(providers);
        let state_store = Arc::new(OAuthStateStore::new());

        // Spawn a background task to sweep expired OAuth states.
        let sweep_store = Arc::clone(&state_store);
        let mut shutdown_rx2 = shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = interval.tick() => sweep_store.sweep_expired().await,
                    _ = shutdown_rx2.changed() => break,
                }
            }
        });

        self.rebuild_state(|s| {
            s.oauth_providers = Some(providers);
            s.oauth_state_store = Some(state_store);
            s.oauth_base_url = Some(base_url);
            s.oauth_allowed_domains = allowed_domains;
            s.near_nonce_store = near_nonce_store;
            s.near_rpc_url = near_rpc_url;
            s.near_network = near_network;
            s.oauth_sweep_shutdown = Some(shutdown_tx);
        });
        self
    }

    /// Inject the per-user workspace pool for multi-user mode.
    pub fn with_workspace_pool(mut self, pool: Arc<server::WorkspacePool>) -> Self {
        self.rebuild_state(|s| s.workspace_pool = Some(pool));
        self
    }

    /// Inject the shared pairing store for the pairing API endpoints.
    pub fn with_pairing_store(mut self, store: Arc<crate::pairing::PairingStore>) -> Self {
        self.rebuild_state(|s| s.pairing_store = Some(store));
        self
    }

    /// Get the first auth token (for printing to console on startup).
    pub fn auth_token(&self) -> &str {
        self.auth.env_auth.first_token().unwrap_or("")
    }

    /// Get a reference to the shared gateway state (for the agent to push SSE events).
    pub fn state(&self) -> &Arc<GatewayState> {
        &self.state
    }
}

#[async_trait]
impl Channel for GatewayChannel {
    fn name(&self) -> &str {
        "gateway"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(256);
        *self.state.msg_tx.write().await = Some(tx);

        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port)
            .parse()
            .map_err(|e| ChannelError::StartupFailed {
                name: "gateway".to_string(),
                reason: format!(
                    "Invalid address '{}:{}': {}",
                    self.config.host, self.config.port, e
                ),
            })?;

        server::start_server(addr, self.state.clone(), self.auth.clone()).await?;

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let thread_id = match &msg.thread_id {
            Some(tid) => tid.clone(),
            None => {
                return Err(ChannelError::MissingRoutingTarget {
                    name: "gateway".to_string(),
                    reason: "respond() requires a thread_id on the incoming message".to_string(),
                });
            }
        };

        self.state.sse.broadcast_for_user(
            &msg.user_id,
            AppEvent::Response {
                content: response.content,
                thread_id,
            },
        );

        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let thread_id = metadata
            .get("thread_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let event = match status {
            StatusUpdate::Thinking(msg) => AppEvent::Thinking {
                message: msg,
                thread_id: thread_id.clone(),
            },
            StatusUpdate::ToolStarted {
                name,
                detail,
                call_id,
            } => AppEvent::ToolStarted {
                name,
                detail,
                call_id,
                thread_id: thread_id.clone(),
            },
            StatusUpdate::ToolCompleted {
                name,
                success,
                error,
                parameters,
                call_id,
                duration_ms,
            } => AppEvent::ToolCompleted {
                name,
                success,
                error,
                parameters,
                call_id,
                duration_ms,
                thread_id: thread_id.clone(),
            },
            StatusUpdate::ToolResult {
                name,
                preview,
                call_id,
            } => AppEvent::ToolResult {
                name,
                preview,
                call_id,
                thread_id: thread_id.clone(),
            },
            StatusUpdate::StreamChunk(content) => AppEvent::StreamChunk {
                content,
                thread_id: thread_id.clone(),
            },
            StatusUpdate::Status(msg) => AppEvent::Status {
                message: msg,
                thread_id: thread_id.clone(),
            },
            StatusUpdate::JobStarted {
                job_id,
                title,
                browse_url,
            } => AppEvent::JobStarted {
                job_id,
                title,
                browse_url,
            },
            StatusUpdate::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters,
                allow_always,
            } => AppEvent::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters: serde_json::to_string_pretty(&parameters)
                    .unwrap_or_else(|_| parameters.to_string()),
                thread_id,
                allow_always,
            },
            StatusUpdate::AuthRequired {
                extension_name,
                instructions,
                auth_url,
                setup_url,
                request_id,
            } => AppEvent::OnboardingState {
                extension_name,
                state: ironclaw_common::OnboardingStateDto::AuthRequired,
                request_id,
                message: None,
                instructions,
                auth_url,
                setup_url,
                onboarding: None,
                thread_id: thread_id.clone(),
            },
            StatusUpdate::AuthCompleted {
                extension_name,
                success,
                message,
            } => AppEvent::OnboardingState {
                extension_name,
                state: if success {
                    ironclaw_common::OnboardingStateDto::Ready
                } else {
                    ironclaw_common::OnboardingStateDto::Failed
                },
                request_id: None,
                message: Some(message),
                instructions: None,
                auth_url: None,
                setup_url: None,
                onboarding: None,
                thread_id: thread_id.clone(),
            },
            StatusUpdate::ImageGenerated {
                event_id,
                data_url,
                path,
            } => AppEvent::ImageGenerated {
                event_id,
                data_url,
                path,
                thread_id: thread_id.clone(),
            },
            StatusUpdate::Suggestions { suggestions } => AppEvent::Suggestions {
                suggestions,
                thread_id: thread_id.clone(),
            },
            StatusUpdate::ReasoningUpdate {
                narrative,
                decisions,
            } => AppEvent::ReasoningUpdate {
                narrative,
                decisions: decisions
                    .into_iter()
                    .map(|d| crate::channels::web::types::ToolDecisionDto {
                        tool_name: d.tool_name,
                        rationale: d.rationale,
                    })
                    .collect(),
                thread_id,
            },
            StatusUpdate::TurnCost {
                input_tokens,
                output_tokens,
                cost_usd,
            } => AppEvent::TurnCost {
                input_tokens,
                output_tokens,
                cost_usd,
                thread_id,
            },
            StatusUpdate::JobStatus { job_id, status } => AppEvent::JobStatus {
                job_id,
                message: status,
            },
            StatusUpdate::JobResult { job_id, status } => AppEvent::JobResult {
                job_id,
                status,
                session_id: None,
                fallback_deliverable: None,
            },
            StatusUpdate::SkillActivated {
                skill_names,
                feedback,
            } => AppEvent::SkillActivated {
                skill_names,
                thread_id,
                feedback,
            },
            StatusUpdate::RoutineUpdate { .. }
            | StatusUpdate::ContextPressure { .. }
            | StatusUpdate::SandboxStatus { .. }
            | StatusUpdate::SecretsStatus { .. }
            | StatusUpdate::CostGuard { .. }
            | StatusUpdate::ThreadList { .. }
            | StatusUpdate::EngineThreadList { .. }
            | StatusUpdate::ConversationHistory { .. } => {
                return Ok(());
            }
        };

        // Scope events to the user when user_id is available in metadata.
        // When user_id is missing (heartbeat, routines), events go to all
        // subscribers. In multi-tenant mode this leaks status across users.
        if let Some(uid) = metadata.get("user_id").and_then(|v| v.as_str()) {
            self.state.sse.broadcast_for_user(uid, event);
        } else {
            tracing::debug!("Status event missing user_id in metadata; broadcasting globally");
            self.state.sse.broadcast(event);
        }
        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let thread_id = match response.thread_id {
            Some(tid) => tid,
            None => {
                // Proactive broadcasts (mission notifications, self-repair,
                // extension activation) don't always have a thread context.
                // Route to the user's assistant conversation so the message
                // appears in a known location instead of being rejected.
                match self.state.store.as_ref() {
                    Some(store) => store
                        .get_or_create_assistant_conversation(user_id, "gateway")
                        .await
                        .map(|id| id.to_string())
                        .map_err(|e| ChannelError::SendFailed {
                            name: "gateway".to_string(),
                            reason: format!(
                                "broadcast() has no thread_id and assistant thread lookup failed: {e}"
                            ),
                        })?,
                    None => {
                        return Err(ChannelError::MissingRoutingTarget {
                            name: "gateway".to_string(),
                            reason: "broadcast() has no thread_id and no DB to resolve assistant thread".to_string(),
                        });
                    }
                }
            }
        };
        self.state.sse.broadcast_for_user(
            user_id,
            AppEvent::Response {
                content: response.content,
                thread_id,
            },
        );
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        if self.state.msg_tx.read().await.is_some() {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: "gateway".to_string(),
            })
        }
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        if let Some(tx) = self.state.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }
        *self.state.msg_tx.write().await = None;
        Ok(())
    }
}
