//! Axum HTTP server for the web gateway.
//!
//! Handles all API routes: chat, memory, jobs, health, and static file serving.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State, WebSocketUpgrade},
    http::{StatusCode, header},
    middleware,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post, put},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::StreamExt;
use tower_http::cors::{AllowHeaders, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use uuid::Uuid;

use axum::http::HeaderMap;

use crate::agent::SessionManager;
use crate::bootstrap::ironclaw_base_dir;
use crate::channels::IncomingMessage;
use crate::channels::relay::DEFAULT_RELAY_NAME;
use crate::channels::web::auth::{
    AdminUser, AuthenticatedUser, CombinedAuthState, UserIdentity, auth_middleware,
};
use crate::channels::web::handlers::chat::chat_events_handler;
use crate::channels::web::handlers::engine::{
    engine_mission_detail_handler, engine_mission_fire_handler, engine_mission_pause_handler,
    engine_mission_resume_handler, engine_missions_handler, engine_missions_summary_handler,
    engine_project_detail_handler, engine_projects_handler, engine_thread_detail_handler,
    engine_thread_events_handler, engine_thread_steps_handler, engine_threads_handler,
};
use crate::channels::web::handlers::jobs::{
    job_files_list_handler, job_files_read_handler, jobs_cancel_handler, jobs_detail_handler,
    jobs_events_handler, jobs_list_handler, jobs_prompt_handler, jobs_restart_handler,
    jobs_summary_handler,
};
use crate::channels::web::handlers::llm::{
    llm_list_models_handler, llm_providers_handler, llm_test_connection_handler,
};
use crate::channels::web::handlers::memory::{
    memory_list_handler, memory_read_handler, memory_search_handler, memory_tree_handler,
    memory_write_handler,
};
use crate::channels::web::handlers::routines::{
    routines_delete_handler, routines_detail_handler, routines_list_handler,
    routines_summary_handler, routines_toggle_handler, routines_trigger_handler,
};
use crate::channels::web::handlers::settings::{
    settings_delete_handler, settings_export_handler, settings_get_handler,
    settings_import_handler, settings_list_handler, settings_set_handler,
    settings_tools_list_handler, settings_tools_set_handler,
};
use crate::channels::web::handlers::skills::{
    skills_install_handler, skills_list_handler, skills_remove_handler, skills_search_handler,
};
use crate::channels::web::log_layer::LogBroadcaster;
use crate::channels::web::sse::SseManager;
use crate::channels::web::types::*;
use crate::channels::web::util::{build_turns_from_db_messages, truncate_preview};
use crate::db::Database;
use crate::extensions::ExtensionManager;
use crate::orchestrator::job_manager::ContainerJobManager;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;

/// Shared prompt queue: maps job IDs to pending follow-up prompts for Claude Code bridges.
pub type PromptQueue = Arc<
    tokio::sync::Mutex<
        std::collections::HashMap<
            uuid::Uuid,
            std::collections::VecDeque<crate::orchestrator::api::PendingPrompt>,
        >,
    >,
>;

/// Slot for the routine engine, filled at runtime after the agent starts.
pub type RoutineEngineSlot =
    Arc<tokio::sync::RwLock<Option<Arc<crate::agent::routine_engine::RoutineEngine>>>>;

fn redact_oauth_state_for_logs(state: &str) -> String {
    let digest = Sha256::digest(state.as_bytes());
    let mut short_hash = String::with_capacity(12);
    for byte in &digest[..6] {
        use std::fmt::Write as _;
        let _ = write!(&mut short_hash, "{byte:02x}");
    }
    format!("sha256:{short_hash}:len={}", state.len())
}

pub(crate) fn rate_limit_key_from_headers(headers: &HeaderMap) -> String {
    // Try X-Forwarded-For first (reverse proxy), then X-Real-IP.
    let xff = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .into_iter()
        .flat_map(|s| s.split(','))
        .map(str::trim)
        .find_map(|candidate| candidate.parse::<std::net::IpAddr>().ok())
        .map(|ip| ip.to_string());

    if let Some(ip) = xff {
        return ip;
    }

    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<std::net::IpAddr>().ok())
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Simple sliding-window rate limiter.
///
/// Tracks the number of requests in the current window. Resets when the window expires.
pub struct RateLimiter {
    /// Requests remaining in the current window.
    remaining: AtomicU64,
    /// Epoch second when the current window started.
    window_start: AtomicU64,
    /// Maximum requests per window.
    max_requests: u64,
    /// Window duration in seconds.
    window_secs: u64,
}

impl RateLimiter {
    pub fn new(max_requests: u64, window_secs: u64) -> Self {
        Self {
            remaining: AtomicU64::new(max_requests),
            window_start: AtomicU64::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            ),
            max_requests,
            window_secs,
        }
    }

    /// Try to consume one request. Returns `true` if allowed, `false` if rate limited.
    ///
    /// Note: There is a benign TOCTOU race between checking `window_start` and
    /// resetting it — two concurrent threads may both see an expired window
    /// and reset it, granting a few extra requests at the window boundary.
    /// This is acceptable for chat rate limiting where approximate enforcement
    /// is sufficient, and avoids the cost of a Mutex.
    pub fn check(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let window = self.window_start.load(Ordering::Relaxed);
        if now.saturating_sub(window) >= self.window_secs {
            // Window expired, reset
            self.window_start.store(now, Ordering::Relaxed);
            self.remaining
                .store(self.max_requests - 1, Ordering::Relaxed);
            return true;
        }

        // Try to decrement remaining
        loop {
            let current = self.remaining.load(Ordering::Relaxed);
            if current == 0 {
                return false;
            }
            if self
                .remaining
                .compare_exchange_weak(current, current - 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }
}

/// Snapshot of the active (resolved) configuration exposed to the frontend.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ActiveConfigSnapshot {
    pub llm_backend: String,
    pub llm_model: String,
    pub enabled_channels: Vec<String>,
}

/// Per-user rate limiter that maintains a separate sliding window per user_id.
///
/// Prevents one user from exhausting the rate limit for all users in multi-tenant mode.
pub struct PerUserRateLimiter {
    limiters: std::sync::Mutex<lru::LruCache<String, RateLimiter>>,
    max_requests: u64,
    window_secs: u64,
}

impl PerUserRateLimiter {
    // SAFETY: 2048 is non-zero, so the unwrap in `new()` is infallible.
    const MAX_KEYS: std::num::NonZeroUsize = match std::num::NonZeroUsize::new(2048) {
        Some(v) => v,
        None => unreachable!(),
    };

    pub fn new(max_requests: u64, window_secs: u64) -> Self {
        Self {
            limiters: std::sync::Mutex::new(lru::LruCache::new(Self::MAX_KEYS)),
            max_requests,
            window_secs,
        }
    }

    /// Try to consume one request for the given user. Returns `true` if allowed.
    pub fn check(&self, user_id: &str) -> bool {
        let mut map = match self.limiters.lock() {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("PerUserRateLimiter lock poisoned; recovering");
                e.into_inner()
            }
        };
        let limiter = map.get_or_insert_mut(user_id.to_string(), || {
            RateLimiter::new(self.max_requests, self.window_secs)
        });
        limiter.check()
    }
}

/// Per-user workspace pool: lazily creates and caches workspaces keyed by user_id.
///
/// In single-user mode, exactly one workspace is cached. In multi-user mode,
/// each authenticated user gets their own workspace with appropriate scopes,
/// search config, memory layers, and embedding cache settings.
///
/// Also implements [`WorkspaceResolver`] so it can be shared with memory tools,
/// avoiding a separate `PerUserWorkspaceResolver` with duplicated logic.
pub struct WorkspacePool {
    db: Arc<dyn Database>,
    embeddings: Option<Arc<dyn crate::workspace::EmbeddingProvider>>,
    embedding_cache_config: crate::workspace::EmbeddingCacheConfig,
    search_config: crate::config::WorkspaceSearchConfig,
    workspace_config: crate::config::WorkspaceConfig,
    cache: tokio::sync::RwLock<std::collections::HashMap<String, Arc<Workspace>>>,
}

impl WorkspacePool {
    pub fn new(
        db: Arc<dyn Database>,
        embeddings: Option<Arc<dyn crate::workspace::EmbeddingProvider>>,
        embedding_cache_config: crate::workspace::EmbeddingCacheConfig,
        search_config: crate::config::WorkspaceSearchConfig,
        workspace_config: crate::config::WorkspaceConfig,
    ) -> Self {
        Self {
            db,
            embeddings,
            embedding_cache_config,
            search_config,
            workspace_config,
            cache: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Build a workspace for a user, applying search config, embeddings,
    /// global read scopes, and memory layers.
    fn build_workspace(&self, user_id: &str) -> Workspace {
        let mut ws = Workspace::new_with_db(user_id, Arc::clone(&self.db))
            .with_search_config(&self.search_config);

        if let Some(ref emb) = self.embeddings {
            ws = ws.with_embeddings_cached(Arc::clone(emb), self.embedding_cache_config.clone());
        }

        if !self.workspace_config.read_scopes.is_empty() {
            ws = ws.with_additional_read_scopes(self.workspace_config.read_scopes.clone());
        }

        ws = ws.with_memory_layers(self.workspace_config.memory_layers.clone());
        ws
    }

    /// Get or create a workspace for the given user identity.
    ///
    /// Applies search config, memory layers, embedding cache, and read scopes
    /// (both from global config and from the token's `workspace_read_scopes`).
    pub async fn get_or_create(&self, identity: &UserIdentity) -> Arc<Workspace> {
        // Fast path: check read lock
        {
            let cache = self.cache.read().await;
            if let Some(ws) = cache.get(&identity.user_id) {
                return Arc::clone(ws);
            }
        }

        // Slow path: create workspace under write lock
        let mut cache = self.cache.write().await;
        // Double-check after acquiring write lock
        if let Some(ws) = cache.get(&identity.user_id) {
            return Arc::clone(ws);
        }

        let mut ws = self.build_workspace(&identity.user_id);

        // Apply per-token read scopes from identity.
        if !identity.workspace_read_scopes.is_empty() {
            ws = ws.with_additional_read_scopes(identity.workspace_read_scopes.clone());
        }

        let ws = Arc::new(ws);

        cache.insert(identity.user_id.clone(), Arc::clone(&ws));

        // Seed identity files after inserting into cache (so the lock can be
        // dropped) but before returning, so callers see a seeded workspace.
        // Drop the write lock explicitly before the async seed to avoid
        // blocking other workspace lookups.
        drop(cache);
        if let Err(e) = ws.seed_if_empty().await {
            tracing::warn!(
                user_id = identity.user_id,
                "Failed to seed workspace: {}",
                e
            );
        }

        ws
    }
}

#[async_trait::async_trait]
impl crate::tools::builtin::memory::WorkspaceResolver for WorkspacePool {
    async fn resolve(&self, user_id: &str) -> Arc<Workspace> {
        // Fast path: check read lock
        {
            let cache = self.cache.read().await;
            if let Some(ws) = cache.get(user_id) {
                return Arc::clone(ws);
            }
        }

        // Slow path: create workspace under write lock
        let mut cache = self.cache.write().await;
        if let Some(ws) = cache.get(user_id) {
            return Arc::clone(ws);
        }

        let ws = Arc::new(self.build_workspace(user_id));
        cache.insert(user_id.to_string(), Arc::clone(&ws));
        drop(cache);

        // Match the seeded workspace behavior used by the prompt-side lookup so
        // v1 memory tools and the v1 system prompt see the same per-user scope.
        if let Err(e) = ws.seed_if_empty().await {
            tracing::warn!(user_id = user_id, "Failed to seed workspace: {}", e);
        }

        tracing::debug!(user_id = user_id, "Created per-user workspace");
        ws
    }
}

/// Shared state for all gateway handlers.
pub struct GatewayState {
    /// Channel to send messages to the agent loop.
    pub msg_tx: tokio::sync::RwLock<Option<mpsc::Sender<IncomingMessage>>>,
    /// SSE broadcast manager (Arc-wrapped so extension manager can hold a reference).
    pub sse: Arc<SseManager>,
    /// Workspace for memory API (single-user fallback).
    pub workspace: Option<Arc<Workspace>>,
    /// Per-user workspace pool for multi-user mode.
    pub workspace_pool: Option<Arc<WorkspacePool>>,
    /// Session manager for thread info.
    pub session_manager: Option<Arc<SessionManager>>,
    /// Log broadcaster for the logs SSE endpoint.
    pub log_broadcaster: Option<Arc<LogBroadcaster>>,
    /// Handle for changing the tracing log level at runtime.
    pub log_level_handle: Option<Arc<crate::channels::web::log_layer::LogLevelHandle>>,
    /// Extension manager for extension management API.
    pub extension_manager: Option<Arc<ExtensionManager>>,
    /// Tool registry for listing registered tools.
    pub tool_registry: Option<Arc<ToolRegistry>>,
    /// Database store for sandbox job persistence.
    pub store: Option<Arc<dyn Database>>,
    /// Container job manager for sandbox operations.
    pub job_manager: Option<Arc<ContainerJobManager>>,
    /// Prompt queue for Claude Code follow-up prompts.
    pub prompt_queue: Option<PromptQueue>,
    /// Durable owner scope for persistence and unauthenticated callback flows.
    pub owner_id: String,
    /// Shutdown signal sender.
    pub shutdown_tx: tokio::sync::RwLock<Option<oneshot::Sender<()>>>,
    /// WebSocket connection tracker.
    pub ws_tracker: Option<Arc<crate::channels::web::ws::WsConnectionTracker>>,
    /// LLM provider for OpenAI-compatible API proxy.
    pub llm_provider: Option<Arc<dyn crate::llm::LlmProvider>>,
    /// Skill registry for skill management API.
    pub skill_registry: Option<Arc<std::sync::RwLock<ironclaw_skills::SkillRegistry>>>,
    /// Skill catalog for searching the ClawHub registry.
    pub skill_catalog: Option<Arc<ironclaw_skills::catalog::SkillCatalog>>,
    /// Scheduler for sending follow-up messages to running agent jobs.
    pub scheduler: Option<crate::tools::builtin::SchedulerSlot>,
    /// Per-user rate limiter for chat endpoints (30 messages per 60 seconds per user).
    pub chat_rate_limiter: PerUserRateLimiter,
    /// Per-IP rate limiter for OAuth/auth endpoints (20 requests per 60 seconds per IP).
    pub oauth_rate_limiter: PerUserRateLimiter,
    /// Rate limiter for webhook trigger endpoints (10 requests per 60 seconds).
    pub webhook_rate_limiter: RateLimiter,
    /// Registry catalog entries for the available extensions API.
    /// Populated at startup from `registry/` manifests, independent of extension manager.
    pub registry_entries: Vec<crate::extensions::RegistryEntry>,
    /// Cost guard for token/cost tracking.
    pub cost_guard: Option<Arc<crate::agent::cost_guard::CostGuard>>,
    /// Routine engine slot for manual routine triggering (filled at runtime).
    pub routine_engine: RoutineEngineSlot,
    /// Server startup time for uptime calculation.
    pub startup_time: std::time::Instant,
    /// Snapshot of active (resolved) configuration for the frontend.
    pub active_config: ActiveConfigSnapshot,
    /// Secrets store for admin secret provisioning.
    pub secrets_store: Option<Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
    /// DB auth cache for invalidation on security-critical actions.
    pub db_auth: Option<Arc<crate::channels::web::auth::DbAuthenticator>>,
    /// Shared pairing store (one instance per server, not per request).
    pub pairing_store: Option<Arc<crate::pairing::PairingStore>>,
    /// OAuth providers for social login (None when OAuth is disabled).
    pub oauth_providers: Option<
        Arc<
            std::collections::HashMap<
                String,
                Arc<dyn crate::channels::web::oauth::providers::OAuthProvider>,
            >,
        >,
    >,
    /// In-memory store for pending OAuth flows (CSRF + PKCE state).
    pub oauth_state_store: Option<Arc<crate::channels::web::oauth::state_store::OAuthStateStore>>,
    /// Base URL for constructing OAuth callback URLs.
    pub oauth_base_url: Option<String>,
    /// Email domains allowed for OAuth/OIDC login. Empty means allow all.
    pub oauth_allowed_domains: Vec<String>,
    /// NEAR wallet auth nonce store (None when NEAR auth is disabled).
    pub near_nonce_store: Option<Arc<crate::channels::web::oauth::near::NearNonceStore>>,
    /// NEAR RPC endpoint URL for access key verification.
    pub near_rpc_url: Option<String>,
    /// NEAR network name (mainnet/testnet) for the frontend wallet connector.
    pub near_network: Option<String>,
    /// Shutdown signal for OAuth/NEAR sweep background tasks.
    /// When this sender is dropped, the sweep loops exit gracefully.
    #[allow(dead_code)]
    pub oauth_sweep_shutdown: Option<tokio::sync::watch::Sender<()>>,
}

/// Start the gateway HTTP server.
///
/// Returns the actual bound `SocketAddr` (useful when binding to port 0).
pub async fn start_server(
    addr: SocketAddr,
    state: Arc<GatewayState>,
    auth: CombinedAuthState,
) -> Result<SocketAddr, crate::error::ChannelError> {
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        crate::error::ChannelError::StartupFailed {
            name: "gateway".to_string(),
            reason: format!("Failed to bind to {}: {}", addr, e),
        }
    })?;
    let bound_addr =
        listener
            .local_addr()
            .map_err(|e| crate::error::ChannelError::StartupFailed {
                name: "gateway".to_string(),
                reason: format!("Failed to get local addr: {}", e),
            })?;

    // Public routes (no auth)
    let public = Router::new()
        .route("/api/health", get(health_handler))
        .route("/oauth/callback", get(oauth_callback_handler))
        .route(
            "/oauth/slack/callback",
            get(slack_relay_oauth_callback_handler),
        )
        .route("/relay/events", post(relay_events_handler))
        .route(
            "/api/webhooks/{path}",
            post(crate::channels::web::handlers::webhooks::webhook_trigger_handler),
        )
        // User-scoped webhook endpoint for multi-tenant isolation
        .route(
            "/api/webhooks/u/{user_id}/{path}",
            post(crate::channels::web::handlers::webhooks::webhook_trigger_user_scoped_handler),
        )
        // OAuth social login routes (public, no auth required)
        .route(
            "/auth/providers",
            get(crate::channels::web::handlers::auth::providers_handler),
        )
        .route(
            "/auth/login/{provider}",
            get(crate::channels::web::handlers::auth::login_handler),
        )
        .route(
            "/auth/callback/{provider}",
            get(crate::channels::web::handlers::auth::callback_handler)
                .post(crate::channels::web::handlers::auth::callback_post_handler),
        )
        .route(
            "/auth/logout",
            post(crate::channels::web::handlers::auth::logout_handler),
        )
        // NEAR wallet auth (challenge-response, not OAuth redirect)
        .route(
            "/auth/near/challenge",
            get(crate::channels::web::handlers::auth::near_challenge_handler),
        )
        .route(
            "/auth/near/verify",
            post(crate::channels::web::handlers::auth::near_verify_handler),
        );

    // Protected routes (require auth)
    let auth_state = auth;
    let protected = Router::new()
        // Chat
        .route("/api/chat/send", post(chat_send_handler))
        .route("/api/chat/gate/resolve", post(chat_gate_resolve_handler))
        .route("/api/chat/approval", post(chat_approval_handler))
        .route("/api/chat/auth-token", post(chat_auth_token_handler))
        .route("/api/chat/auth-cancel", post(chat_auth_cancel_handler))
        .route("/api/chat/events", get(chat_events_handler))
        .route("/api/chat/ws", get(chat_ws_handler))
        .route("/api/chat/history", get(chat_history_handler))
        .route("/api/chat/threads", get(chat_threads_handler))
        .route("/api/chat/thread/new", post(chat_new_thread_handler))
        // Memory
        .route("/api/memory/tree", get(memory_tree_handler))
        .route("/api/memory/list", get(memory_list_handler))
        .route("/api/memory/read", get(memory_read_handler))
        .route("/api/memory/write", post(memory_write_handler))
        .route("/api/memory/search", post(memory_search_handler))
        // Jobs
        .route("/api/jobs", get(jobs_list_handler))
        .route("/api/jobs/summary", get(jobs_summary_handler))
        .route("/api/jobs/{id}", get(jobs_detail_handler))
        .route("/api/jobs/{id}/cancel", post(jobs_cancel_handler))
        .route("/api/jobs/{id}/restart", post(jobs_restart_handler))
        .route("/api/jobs/{id}/prompt", post(jobs_prompt_handler))
        .route("/api/jobs/{id}/events", get(jobs_events_handler))
        .route("/api/jobs/{id}/files/list", get(job_files_list_handler))
        .route("/api/jobs/{id}/files/read", get(job_files_read_handler))
        // Logs
        .route("/api/logs/events", get(logs_events_handler))
        .route("/api/logs/level", get(logs_level_get_handler))
        .route(
            "/api/logs/level",
            axum::routing::put(logs_level_set_handler),
        )
        // Extensions
        .route("/api/extensions", get(extensions_list_handler))
        .route("/api/extensions/tools", get(extensions_tools_handler))
        .route("/api/extensions/registry", get(extensions_registry_handler))
        .route("/api/extensions/install", post(extensions_install_handler))
        .route(
            "/api/extensions/{name}/activate",
            post(extensions_activate_handler),
        )
        .route(
            "/api/extensions/{name}/remove",
            post(extensions_remove_handler),
        )
        .route(
            "/api/extensions/{name}/setup",
            get(extensions_setup_handler).post(extensions_setup_submit_handler),
        )
        // Pairing
        .route("/api/pairing/{channel}", get(pairing_list_handler))
        .route(
            "/api/pairing/{channel}/approve",
            post(pairing_approve_handler),
        )
        // Routines
        .route("/api/routines", get(routines_list_handler))
        .route("/api/routines/summary", get(routines_summary_handler))
        .route("/api/routines/{id}", get(routines_detail_handler))
        .route("/api/routines/{id}/trigger", post(routines_trigger_handler))
        .route("/api/routines/{id}/toggle", post(routines_toggle_handler))
        .route(
            "/api/routines/{id}",
            axum::routing::delete(routines_delete_handler),
        )
        .route("/api/routines/{id}/runs", get(routines_runs_handler))
        // Engine v2
        .route("/api/engine/threads", get(engine_threads_handler))
        .route(
            "/api/engine/threads/{id}",
            get(engine_thread_detail_handler),
        )
        .route(
            "/api/engine/threads/{id}/steps",
            get(engine_thread_steps_handler),
        )
        .route(
            "/api/engine/threads/{id}/events",
            get(engine_thread_events_handler),
        )
        .route("/api/engine/projects", get(engine_projects_handler))
        .route(
            "/api/engine/projects/{id}",
            get(engine_project_detail_handler),
        )
        .route("/api/engine/missions", get(engine_missions_handler))
        .route(
            "/api/engine/missions/summary",
            get(engine_missions_summary_handler),
        )
        .route(
            "/api/engine/missions/{id}",
            get(engine_mission_detail_handler),
        )
        .route(
            "/api/engine/missions/{id}/fire",
            post(engine_mission_fire_handler),
        )
        .route(
            "/api/engine/missions/{id}/pause",
            post(engine_mission_pause_handler),
        )
        .route(
            "/api/engine/missions/{id}/resume",
            post(engine_mission_resume_handler),
        )
        // Skills
        .route("/api/skills", get(skills_list_handler))
        .route("/api/skills/search", post(skills_search_handler))
        .route("/api/skills/install", post(skills_install_handler))
        .route(
            "/api/skills/{name}",
            axum::routing::delete(skills_remove_handler),
        )
        // Settings
        .route("/api/settings", get(settings_list_handler))
        .route("/api/settings/export", get(settings_export_handler))
        .route("/api/settings/import", post(settings_import_handler))
        // NOTE: These static routes intentionally shadow `/api/settings/{key}` when
        // key="tools". Axum resolves static routes before parameterized ones, so this
        // works correctly. Avoid adding a setting named literally "tools".
        .route("/api/settings/tools", get(settings_tools_list_handler))
        .route(
            "/api/settings/tools/{name}",
            axum::routing::put(settings_tools_set_handler),
        )
        .route("/api/settings/{key}", get(settings_get_handler))
        .route(
            "/api/settings/{key}",
            axum::routing::put(settings_set_handler),
        )
        .route(
            "/api/settings/{key}",
            axum::routing::delete(settings_delete_handler),
        )
        // LLM utilities
        .route(
            "/api/llm/test_connection",
            post(llm_test_connection_handler),
        )
        .route("/api/llm/list_models", post(llm_list_models_handler))
        .route("/api/llm/providers", get(llm_providers_handler))
        // User management (admin)
        .route(
            "/api/admin/users",
            get(super::handlers::users::users_list_handler)
                .post(super::handlers::users::users_create_handler),
        )
        .route(
            "/api/admin/users/{id}",
            get(super::handlers::users::users_detail_handler)
                .patch(super::handlers::users::users_update_handler)
                .delete(super::handlers::users::users_delete_handler),
        )
        .route(
            "/api/admin/users/{id}/suspend",
            post(super::handlers::users::users_suspend_handler),
        )
        .route(
            "/api/admin/users/{id}/activate",
            post(super::handlers::users::users_activate_handler),
        )
        // Admin secrets provisioning (per-user)
        .route(
            "/api/admin/users/{user_id}/secrets",
            get(super::handlers::secrets::secrets_list_handler),
        )
        .route(
            "/api/admin/users/{user_id}/secrets/{name}",
            put(super::handlers::secrets::secrets_put_handler)
                .delete(super::handlers::secrets::secrets_delete_handler),
        )
        // Usage reporting (admin)
        .route(
            "/api/admin/usage",
            get(super::handlers::users::usage_stats_handler),
        )
        // User self-service profile
        .route(
            "/api/profile",
            get(super::handlers::users::profile_get_handler)
                .patch(super::handlers::users::profile_update_handler),
        )
        // Token management
        .route(
            "/api/tokens",
            get(super::handlers::tokens::tokens_list_handler)
                .post(super::handlers::tokens::tokens_create_handler),
        )
        .route(
            "/api/tokens/{id}",
            axum::routing::delete(super::handlers::tokens::tokens_revoke_handler),
        )
        // Gateway control plane
        .route("/api/gateway/status", get(gateway_status_handler))
        // OpenAI-compatible API
        .route(
            "/v1/chat/completions",
            post(super::openai_compat::chat_completions_handler),
        )
        .route("/v1/models", get(super::openai_compat::models_handler))
        // OpenAI Responses API (routes through the full agent loop)
        .route(
            "/v1/responses",
            post(super::responses_api::create_response_handler),
        )
        .route(
            "/v1/responses/{id}",
            get(super::responses_api::get_response_handler),
        )
        .route_layer(middleware::from_fn_with_state(
            auth_state.clone(),
            auth_middleware,
        ));

    // Static file routes (no auth, served from embedded strings)
    let statics = Router::new()
        .route("/", get(index_handler))
        .route("/style.css", get(css_handler))
        .route("/app.js", get(js_handler))
        .route("/theme-init.js", get(theme_init_handler))
        .route("/favicon.ico", get(favicon_handler))
        .route("/i18n/index.js", get(i18n_index_handler))
        .route("/i18n/en.js", get(i18n_en_handler))
        .route("/i18n/zh-CN.js", get(i18n_zh_handler))
        .route("/i18n/ko.js", get(i18n_ko_handler))
        .route("/i18n-app.js", get(i18n_app_handler));

    // Project file serving (behind auth to prevent unauthorized file access).
    let projects = Router::new()
        .route("/projects/{project_id}", get(project_redirect_handler))
        .route("/projects/{project_id}/", get(project_index_handler))
        .route("/projects/{project_id}/{*path}", get(project_file_handler))
        .route_layer(middleware::from_fn_with_state(
            auth_state.clone(),
            auth_middleware,
        ));

    // CORS: restrict to same-origin by default. Only localhost/127.0.0.1
    // origins are allowed, since the gateway is a local-first service.
    let cors = CorsLayer::new()
        .allow_origin([
            format!("http://{}:{}", addr.ip(), addr.port())
                .parse()
                .expect("valid origin"),
            format!("http://localhost:{}", addr.port())
                .parse()
                .expect("valid origin"),
        ])
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::PATCH,
            axum::http::Method::DELETE,
        ])
        .allow_headers(AllowHeaders::list([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
        ]))
        .allow_credentials(true);

    let app = Router::new()
        .merge(public)
        .merge(statics)
        .merge(projects)
        .merge(protected)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024)) // 10 MB max request body (image uploads)
        .layer(tower_http::catch_panic::CatchPanicLayer::custom(
            |panic_info: Box<dyn std::any::Any + Send + 'static>| {
                let detail = if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                    (*s).to_string()
                } else {
                    "unknown panic".to_string()
                };
                // Truncate panic payload to avoid leaking sensitive data into logs.
                // Use floor_char_boundary to avoid panicking on multi-byte UTF-8.
                let safe_detail = if detail.len() > 200 {
                    let end = detail.floor_char_boundary(200);
                    format!("{}…", &detail[..end])
                } else {
                    detail
                };
                tracing::error!("Handler panicked: {}", safe_detail);
                axum::http::Response::builder()
                    .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                    .header("content-type", "text/plain")
                    .body(axum::body::Body::from("Internal Server Error"))
                    .unwrap_or_else(|_| {
                        axum::http::Response::new(axum::body::Body::from("Internal Server Error"))
                    })
            },
        ))
        .layer(cors)
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            header::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            header::HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::HeaderName::from_static("content-security-policy"),
            header::HeaderValue::from_static(
                "default-src 'self'; \
                 script-src 'self' https://cdn.jsdelivr.net https://cdnjs.cloudflare.com https://esm.sh; \
                 style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
                 font-src https://fonts.gstatic.com data:; \
                 connect-src 'self' https://esm.sh https://rpc.mainnet.near.org https://rpc.testnet.near.org; \
                 img-src 'self' data: blob: https://*.googleusercontent.com https://avatars.githubusercontent.com; \
                 frame-src https://accounts.google.com https://appleid.apple.com; \
                 object-src 'none'; \
                 frame-ancestors 'none'; \
                 base-uri 'self'; \
                 form-action 'self' https://accounts.google.com https://github.com https://appleid.apple.com",
            ),
        ))
        .with_state(state.clone());

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    *state.shutdown_tx.write().await = Some(shutdown_tx);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
                tracing::debug!("Web gateway shutting down");
            })
            .await
        {
            tracing::error!("Web gateway server error: {}", e);
        }
    });

    Ok(bound_addr)
}

// --- Static file handlers ---

async fn index_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/index.html"),
    )
}

async fn css_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/style.css"),
    )
}

async fn js_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/app.js"),
    )
}

async fn theme_init_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/theme-init.js"),
    )
}

async fn favicon_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/x-icon"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        include_bytes!("static/favicon.ico").as_slice(),
    )
}

async fn i18n_index_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/i18n/index.js"),
    )
}

async fn i18n_en_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/i18n/en.js"),
    )
}

async fn i18n_zh_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/i18n/zh-CN.js"),
    )
}

async fn i18n_ko_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/i18n/ko.js"),
    )
}

async fn i18n_app_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/i18n-app.js"),
    )
}

// --- Health ---

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy",
        channel: "gateway",
    })
}

/// Return an OAuth error landing page response.
fn oauth_error_page(label: &str) -> axum::response::Response {
    let html = crate::cli::oauth_defaults::landing_html(label, false);
    axum::response::Html(html).into_response()
}

/// OAuth callback handler for the web gateway.
///
/// This is a PUBLIC route (no Bearer token required) because OAuth providers
/// redirect the user's browser here. The `state` query parameter correlates
/// the callback with a pending OAuth flow registered by `start_wasm_oauth()`.
///
/// Used on hosted instances where `IRONCLAW_OAUTH_CALLBACK_URL` points to
/// the gateway (e.g., `https://kind-deer.agent1.near.ai/oauth/callback`).
/// Local/desktop mode continues to use the TCP listener on port 9876.
async fn oauth_callback_handler(
    State(state): State<Arc<GatewayState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    use crate::cli::oauth_defaults;

    // Check for error from OAuth provider (e.g., user denied consent)
    if let Some(error) = params.get("error") {
        let description = params
            .get("error_description")
            .cloned()
            .unwrap_or_else(|| error.clone());
        return oauth_error_page(&description);
    }

    let state_param = match params.get("state") {
        Some(s) if !s.is_empty() => s.clone(),
        _ => {
            return oauth_error_page("IronClaw");
        }
    };

    let code = match params.get("code") {
        Some(c) if !c.is_empty() => c.clone(),
        _ => {
            return oauth_error_page("IronClaw");
        }
    };

    // Look up the pending flow by CSRF state (atomic remove prevents replay)
    let ext_mgr = match state.extension_manager.as_ref() {
        Some(mgr) => mgr,
        None => {
            return oauth_error_page("IronClaw");
        }
    };

    let decoded_state = match oauth_defaults::decode_hosted_oauth_state(&state_param) {
        Ok(decoded) => decoded,
        Err(error) => {
            let redacted_state = redact_oauth_state_for_logs(&state_param);
            tracing::warn!(
                state = %redacted_state,
                error = %error,
                "OAuth callback received with malformed state"
            );
            clear_auth_mode(&state, &state.owner_id).await;
            return oauth_error_page("IronClaw");
        }
    };
    let lookup_key = decoded_state.flow_id.clone();

    let flow = ext_mgr
        .pending_oauth_flows()
        .write()
        .await
        .remove(&lookup_key);

    let flow = match flow {
        Some(f) => f,
        None => {
            let redacted_state = redact_oauth_state_for_logs(&state_param);
            let redacted_lookup_key = redact_oauth_state_for_logs(&lookup_key);
            tracing::warn!(
                state = %redacted_state,
                lookup_key = %redacted_lookup_key,
                "OAuth callback received with unknown or expired state"
            );
            return oauth_error_page("IronClaw");
        }
    };

    // Check flow expiry (5 minutes, matching TCP listener timeout)
    if flow.created_at.elapsed() > oauth_defaults::OAUTH_FLOW_EXPIRY {
        tracing::warn!(
            extension = %flow.extension_name,
            "OAuth flow expired"
        );
        // Notify UI so auth card can show error instead of staying stuck
        if let Some(ref sse) = flow.sse_manager {
            sse.broadcast_for_user(
                &flow.user_id,
                AppEvent::AuthCompleted {
                    extension_name: flow.extension_name.clone(),
                    success: false,
                    message: "OAuth flow expired. Please try again.".to_string(),
                    thread_id: None,
                },
            );
        }
        clear_auth_mode(&state, &flow.user_id).await;
        return oauth_error_page(&flow.display_name);
    }

    // Exchange the authorization code for tokens.
    // Use the platform exchange proxy when configured, otherwise call the
    // provider's token URL directly.
    let exchange_proxy_url = oauth_defaults::exchange_proxy_url();

    let result: Result<(), String> = async {
        let token_response = if let Some(proxy_url) = &exchange_proxy_url {
            let oauth_proxy_auth_token = flow.oauth_proxy_auth_token().unwrap_or_default();
            oauth_defaults::exchange_via_proxy(oauth_defaults::ProxyTokenExchangeRequest {
                proxy_url,
                gateway_token: oauth_proxy_auth_token,
                token_url: &flow.token_url,
                client_id: &flow.client_id,
                client_secret: flow.client_secret.as_deref(),
                code: &code,
                redirect_uri: &flow.redirect_uri,
                code_verifier: flow.code_verifier.as_deref(),
                access_token_field: &flow.access_token_field,
                extra_token_params: &flow.token_exchange_extra_params,
            })
            .await
            .map_err(|e| e.to_string())?
        } else {
            oauth_defaults::exchange_oauth_code_with_params(
                &flow.token_url,
                &flow.client_id,
                flow.client_secret.as_deref(),
                &code,
                &flow.redirect_uri,
                flow.code_verifier.as_deref(),
                &flow.access_token_field,
                &flow.token_exchange_extra_params,
            )
            .await
            .map_err(|e| e.to_string())?
        };

        // Validate the token before storing (catches wrong account, etc.)
        if let Some(ref validation) = flow.validation_endpoint {
            oauth_defaults::validate_oauth_token(&token_response.access_token, validation)
                .await
                .map_err(|e| e.to_string())?;
        }

        // Store tokens encrypted in the secrets store
        oauth_defaults::store_oauth_tokens(
            flow.secrets.as_ref(),
            &flow.user_id,
            &flow.secret_name,
            flow.provider.as_deref(),
            &token_response.access_token,
            token_response.refresh_token.as_deref(),
            token_response.expires_in,
            &flow.scopes,
        )
        .await
        .map_err(|e| e.to_string())?;

        // Persist the client_id for flows that need it after the session ends
        // (for example DCR-based MCP refresh).
        if let Some(ref client_id_secret) = flow.client_id_secret_name {
            let params = crate::secrets::CreateSecretParams::new(client_id_secret, &flow.client_id)
                .with_provider(flow.provider.as_ref().cloned().unwrap_or_default());
            flow.secrets
                .create(&flow.user_id, params)
                .await
                .map_err(|e| {
                    tracing::warn!(
                        extension = %flow.extension_name,
                        secret_name = %client_id_secret,
                        error = %e,
                        "Failed to store OAuth client_id secret after callback"
                    );
                    "failed to store client credentials".to_string()
                })?;
        }

        if let (Some(client_secret_name), Some(client_secret)) = (
            flow.client_secret_secret_name.as_ref(),
            flow.client_secret.as_deref(),
        ) {
            let mut params =
                crate::secrets::CreateSecretParams::new(client_secret_name, client_secret)
                    .with_provider(flow.provider.as_ref().cloned().unwrap_or_default());
            if let Some(expires_at) = flow.client_secret_expires_at
                && let Some(dt) =
                    chrono::DateTime::<chrono::Utc>::from_timestamp(expires_at as i64, 0)
            {
                params = params.with_expiry(dt);
            }
            flow.secrets
                .create(&flow.user_id, params)
                .await
                .map_err(|e| {
                    tracing::warn!(
                        extension = %flow.extension_name,
                        secret_name = %client_secret_name,
                        error = %e,
                        "Failed to store OAuth client_secret secret after callback"
                    );
                    "failed to store client credentials".to_string()
                })?;
        }

        Ok(())
    }
    .await;

    let (success, message) = match &result {
        Ok(()) => (
            true,
            format!("{} authenticated successfully", flow.display_name),
        ),
        Err(e) => (
            false,
            format!("{} authentication failed: {}", flow.display_name, e),
        ),
    };

    match &result {
        Ok(()) => {
            tracing::info!(
                extension = %flow.extension_name,
                "OAuth completed successfully via gateway callback"
            );
        }
        Err(e) => {
            tracing::warn!(
                extension = %flow.extension_name,
                error = %e,
                "OAuth failed via gateway callback"
            );
        }
    }

    // Clear auth mode regardless of outcome so the next user message goes
    // through to the LLM instead of being intercepted as a token.
    clear_auth_mode(&state, &flow.user_id).await;

    // After successful OAuth, auto-activate the extension so it moves
    // from "Installed (Authenticate)" → "Active" without a second click.
    // OAuth success is independent of activation — tokens are already stored.
    // Report auth as successful and attempt activation as a bonus step.
    let final_message = if success && flow.auto_activate_extension {
        match ext_mgr.activate(&flow.extension_name, &flow.user_id).await {
            Ok(result) => result.message,
            Err(e) => {
                tracing::warn!(
                    extension = %flow.extension_name,
                    error = %e,
                    "Auto-activation after OAuth failed"
                );
                format!(
                    "{} authenticated successfully. Activation failed: {}. Try activating manually.",
                    flow.display_name, e
                )
            }
        }
    } else if success {
        format!("{} authenticated successfully", flow.display_name)
    } else {
        message
    };

    // Broadcast event to notify the web UI
    let extension_name = flow.extension_name.clone();
    if let Some(ref sse) = flow.sse_manager {
        sse.broadcast_for_user(
            &flow.user_id,
            AppEvent::AuthCompleted {
                extension_name: flow.extension_name,
                success,
                message: final_message.clone(),
                thread_id: None,
            },
        );
    }

    if success
        && let Err(e) =
            crate::bridge::resolve_engine_auth_callback(&flow.user_id, &extension_name).await
    {
        tracing::warn!(
            extension = %extension_name,
            user_id = %flow.user_id,
            error = %e,
            "Failed to resume pending engine auth gate after OAuth callback"
        );
    }

    let html = oauth_defaults::landing_html(&flow.display_name, success);
    axum::response::Html(html).into_response()
}

/// Webhook endpoint for receiving relay events from channel-relay.
///
/// PUBLIC route — authenticated via HMAC signature (X-Relay-Signature header).
async fn relay_events_handler(
    State(state): State<Arc<GatewayState>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let ext_mgr = match state.extension_manager.as_ref() {
        Some(mgr) => mgr,
        None => {
            return (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response();
        }
    };

    let signing_secret = match ext_mgr.relay_signing_secret() {
        Some(s) => s,
        None => {
            return (StatusCode::SERVICE_UNAVAILABLE, "relay not configured").into_response();
        }
    };

    // Verify signature
    let signature = match headers
        .get("x-relay-signature")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s.to_string(),
        None => {
            return (StatusCode::UNAUTHORIZED, "missing signature").into_response();
        }
    };

    let timestamp = match headers
        .get("x-relay-timestamp")
        .and_then(|v| v.to_str().ok())
    {
        Some(t) => t.to_string(),
        None => {
            return (StatusCode::UNAUTHORIZED, "missing timestamp").into_response();
        }
    };

    // Check timestamp freshness (5 min window)
    let ts: i64 = match timestamp.parse() {
        Ok(t) => t,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "malformed timestamp").into_response();
        }
    };
    let now = chrono::Utc::now().timestamp();
    if (now - ts).abs() > 300 {
        return (StatusCode::UNAUTHORIZED, "stale timestamp").into_response();
    }

    // Verify HMAC: sha256(secret, timestamp + "." + body)
    if !crate::channels::relay::webhook::verify_relay_signature(
        &signing_secret,
        &timestamp,
        &body,
        &signature,
    ) {
        return (StatusCode::UNAUTHORIZED, "invalid signature").into_response();
    }

    // Parse event
    let event: crate::channels::relay::client::ChannelEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "relay callback invalid JSON");
            return (StatusCode::BAD_REQUEST, "invalid JSON").into_response();
        }
    };

    // Push to relay channel
    let event_tx_guard = ext_mgr.relay_event_tx();
    let event_tx = event_tx_guard.lock().await;
    match event_tx.as_ref() {
        Some(tx) => {
            if let Err(e) = tx.try_send(event) {
                tracing::warn!(error = %e, "relay event channel full or closed");
                return (StatusCode::SERVICE_UNAVAILABLE, "event queue full").into_response();
            }
        }
        None => {
            return (StatusCode::SERVICE_UNAVAILABLE, "relay channel not active").into_response();
        }
    }

    Json(serde_json::json!({"ok": true})).into_response()
}

/// OAuth callback for Slack via channel-relay.
///
/// This is a PUBLIC route (no Bearer token required) because channel-relay
/// redirects the user's browser here after Slack OAuth completes.
/// Query params: `provider`, `team_id`.
async fn slack_relay_oauth_callback_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    // Rate limit
    let ip = rate_limit_key_from_headers(&headers);
    if !state.oauth_rate_limiter.check(&ip) {
        return axum::response::Html(
            "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
             <h2>Too Many Requests</h2>\
             <p>Please try again later.</p>\
             </body></html>"
                .to_string(),
        )
        .into_response();
    }

    // Validate team_id format: empty or T followed by alphanumeric (max 20 chars)
    let team_id = params.get("team_id").cloned().unwrap_or_default();
    if !team_id.is_empty() {
        let valid_team_id = team_id.len() <= 21
            && team_id.starts_with('T')
            && team_id[1..].chars().all(|c| c.is_ascii_alphanumeric());
        if !valid_team_id {
            return axum::response::Html(
                "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
                 <h2>Error</h2><p>Invalid callback parameters.</p></body></html>"
                    .to_string(),
            )
            .into_response();
        }
    }

    // Validate provider: must be "slack" (only supported provider)
    let provider = params
        .get("provider")
        .cloned()
        .unwrap_or_else(|| "slack".into());
    if provider != "slack" {
        return axum::response::Html(
            "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
             <h2>Error</h2><p>Invalid callback parameters.</p></body></html>"
                .to_string(),
        )
        .into_response();
    }

    let ext_mgr = match state.extension_manager.as_ref() {
        Some(mgr) => mgr,
        None => {
            return axum::response::Html(
                "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
                 <h2>Error</h2><p>Extension manager not available.</p></body></html>"
                    .to_string(),
            )
            .into_response();
        }
    };

    // Validate CSRF state parameter
    let state_param = match params.get("state") {
        Some(s) if !s.is_empty() && s.len() <= 128 => s.clone(),
        _ => {
            return axum::response::Html(
                "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
                 <h2>Error</h2><p>Invalid or expired authorization.</p></body></html>"
                    .to_string(),
            )
            .into_response();
        }
    };

    let state_key = format!("relay:{}:oauth_state", DEFAULT_RELAY_NAME);
    let stored_state = match ext_mgr
        .secrets()
        .get_decrypted(&state.owner_id, &state_key)
        .await
    {
        Ok(secret) => secret.expose().to_string(),
        Err(_) => {
            return axum::response::Html(
                "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
                 <h2>Error</h2><p>Invalid or expired authorization.</p></body></html>"
                    .to_string(),
            )
            .into_response();
        }
    };

    if state_param != stored_state {
        return axum::response::Html(
            "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
             <h2>Error</h2><p>Invalid or expired authorization.</p></body></html>"
                .to_string(),
        )
        .into_response();
    }

    // Delete the nonce (one-time use)
    let _ = ext_mgr.secrets().delete(&state.owner_id, &state_key).await;

    let result: Result<(), String> = async {
        let store = state.store.as_ref().ok_or_else(|| {
            "Relay activation requires persistent settings storage; no-db mode is unsupported."
                .to_string()
        })?;

        // Store team_id in settings
        let team_id_key = format!("relay:{}:team_id", DEFAULT_RELAY_NAME);
        tracing::info!(
            relay = DEFAULT_RELAY_NAME,
            owner_id = %state.owner_id,
            team_id_key = %team_id_key,
            "relay OAuth callback: storing team_id in settings"
        );
        store
            .set_setting(&state.owner_id, &team_id_key, &serde_json::json!(team_id))
            .await
            .map_err(|e| {
                tracing::error!(
                    relay = DEFAULT_RELAY_NAME,
                    owner_id = %state.owner_id,
                    error = %e,
                    "relay OAuth callback: failed to persist team_id to settings store"
                );
                format!("Failed to persist relay team_id: {e}")
            })?;

        // Activate the relay channel
        tracing::info!(
            relay = DEFAULT_RELAY_NAME,
            owner_id = %state.owner_id,
            "relay OAuth callback: activating relay channel"
        );
        ext_mgr
            .activate_stored_relay(DEFAULT_RELAY_NAME, &state.owner_id)
            .await
            .map_err(|e| format!("Failed to activate relay channel: {}", e))?;

        Ok(())
    }
    .await;

    let (success, message) = match &result {
        Ok(()) => (true, "Slack connected successfully!".to_string()),
        Err(e) => {
            tracing::error!(error = %e, "Slack relay OAuth callback failed");
            (
                false,
                "Connection failed. Check server logs for details.".to_string(),
            )
        }
    };

    // Broadcast event to notify the web UI
    state.sse.broadcast(AppEvent::AuthCompleted {
        extension_name: DEFAULT_RELAY_NAME.to_string(),
        success,
        message: message.clone(),
        thread_id: None,
    });

    if success {
        axum::response::Html(
            "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
             <h2>Slack Connected!</h2>\
             <p>You can close this tab and return to IronClaw.</p>\
             <script>window.close()</script>\
             </body></html>"
                .to_string(),
        )
        .into_response()
    } else {
        axum::response::Html(format!(
            "<html><body style='font-family: system-ui; text-align: center; padding: 60px;'>\
             <h2>Connection Failed</h2>\
             <p>{}</p>\
             </body></html>",
            message
        ))
        .into_response()
    }
}

// --- Chat handlers ---

/// Convert web gateway `ImageData` to `IncomingAttachment` objects.
pub(crate) fn images_to_attachments(
    images: &[ImageData],
) -> Vec<crate::channels::IncomingAttachment> {
    use base64::Engine;
    images
        .iter()
        .enumerate()
        .filter_map(|(i, img)| {
            if !img.media_type.starts_with("image/") {
                tracing::warn!(
                    "Skipping image {i}: invalid media type '{}' (must start with 'image/')",
                    img.media_type
                );
                return None;
            }
            let data = match base64::engine::general_purpose::STANDARD.decode(&img.data) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!("Skipping image {i}: invalid base64 data: {e}");
                    return None;
                }
            };
            Some(crate::channels::IncomingAttachment {
                id: format!("web-image-{i}"),
                kind: crate::channels::AttachmentKind::Image,
                mime_type: img.media_type.clone(),
                filename: Some(format!("image-{i}.{}", mime_to_ext(&img.media_type))),
                size_bytes: Some(data.len() as u64),
                source_url: None,
                storage_key: None,
                extracted_text: None,
                data,
                duration_secs: None,
            })
        })
        .collect()
}

/// Map MIME type to file extension.
fn mime_to_ext(mime: &str) -> &str {
    match mime {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        _ => "jpg",
    }
}

async fn chat_send_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    headers: axum::http::HeaderMap,
    Json(req): Json<SendMessageRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    tracing::trace!(
        "[chat_send_handler] Received message: content_len={}, thread_id={:?}",
        req.content.len(),
        req.thread_id
    );

    if !state.chat_rate_limiter.check(&user.user_id) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Try again shortly.".to_string(),
        ));
    }

    let mut msg = IncomingMessage::new("gateway", &user.user_id, &req.content);
    // Prefer timezone from JSON body, fall back to X-Timezone header
    let tz = req
        .timezone
        .as_deref()
        .or_else(|| headers.get("X-Timezone").and_then(|v| v.to_str().ok()));
    if let Some(tz) = tz {
        msg = msg.with_timezone(tz);
    }

    // Always include user_id in metadata so downstream SSE broadcasts can scope events.
    let mut meta = serde_json::json!({"user_id": &user.user_id});
    if let Some(ref thread_id) = req.thread_id {
        msg = msg.with_thread(thread_id);
        meta["thread_id"] = serde_json::json!(thread_id);
    }
    msg = msg.with_metadata(meta);

    // Convert uploaded images to IncomingAttachments
    if !req.images.is_empty() {
        let attachments = images_to_attachments(&req.images);
        msg = msg.with_attachments(attachments);
    }

    let msg_id = msg.id;
    tracing::trace!(
        "[chat_send_handler] Created message id={}, content_len={}, images={}",
        msg_id,
        req.content.len(),
        req.images.len()
    );

    // Clone sender to avoid holding RwLock read guard across send().await
    let tx = {
        let tx_guard = state.msg_tx.read().await;
        tx_guard
            .as_ref()
            .ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Channel not started".to_string(),
            ))?
            .clone()
    };

    tracing::debug!("[chat_send_handler] Sending message through channel");
    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })?;

    tracing::debug!("[chat_send_handler] Message sent successfully, returning 202 ACCEPTED");

    Ok((
        StatusCode::ACCEPTED,
        Json(SendMessageResponse {
            message_id: msg_id,
            status: "accepted",
        }),
    ))
}

async fn chat_approval_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<ApprovalRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    let (approved, always) = match req.action.as_str() {
        "approve" => (true, false),
        "always" => (true, true),
        "deny" => (false, false),
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Unknown action: {}", other),
            ));
        }
    };

    let request_id = Uuid::parse_str(&req.request_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid request_id (expected UUID)".to_string(),
        )
    })?;

    // Build a structured ExecApproval submission as JSON, sent through the
    // existing message pipeline so the agent loop picks it up.
    let approval = crate::agent::submission::Submission::ExecApproval {
        request_id,
        approved,
        always,
    };
    let content = serde_json::to_string(&approval).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize approval: {}", e),
        )
    })?;

    let mut msg = IncomingMessage::new("gateway", &user.user_id, content);

    if let Some(ref thread_id) = req.thread_id {
        msg = msg.with_thread(thread_id);
    }

    let msg_id = msg.id;

    // Clone sender to avoid holding RwLock read guard across send().await
    let tx = {
        let tx_guard = state.msg_tx.read().await;
        tx_guard
            .as_ref()
            .ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Channel not started".to_string(),
            ))?
            .clone()
    };

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })?;

    Ok((
        StatusCode::ACCEPTED,
        Json(SendMessageResponse {
            message_id: msg_id,
            status: "accepted",
        }),
    ))
}

async fn chat_gate_resolve_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<GateResolveRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    match req.resolution {
        GateResolutionPayload::Approved { always } => {
            let action = if always { "always" } else { "approve" }.to_string();
            let _ = chat_approval_handler(
                State(state),
                AuthenticatedUser(user),
                Json(ApprovalRequest {
                    request_id: req.request_id,
                    action,
                    thread_id: req.thread_id,
                }),
            )
            .await?;
            Ok(Json(ActionResponse::ok("Gate resolution accepted.")))
        }
        GateResolutionPayload::Denied => {
            let _ = chat_approval_handler(
                State(state),
                AuthenticatedUser(user),
                Json(ApprovalRequest {
                    request_id: req.request_id,
                    action: "deny".into(),
                    thread_id: req.thread_id,
                }),
            )
            .await?;
            Ok(Json(ActionResponse::ok("Gate resolution accepted.")))
        }
        GateResolutionPayload::CredentialProvided { token } => {
            let thread_id = req.thread_id.ok_or((
                StatusCode::BAD_REQUEST,
                "thread_id is required for credential resolution".to_string(),
            ))?;
            dispatch_engine_auth_resolution(&state, &user.user_id, &thread_id, token).await?;
            Ok(Json(ActionResponse::ok("Credential submitted.")))
        }
        GateResolutionPayload::Cancelled => {
            let thread_id = req.thread_id.ok_or((
                StatusCode::BAD_REQUEST,
                "thread_id is required for cancellation".to_string(),
            ))?;
            dispatch_engine_auth_resolution(&state, &user.user_id, &thread_id, "cancel".into())
                .await?;
            Ok(Json(ActionResponse::ok("Gate cancelled.")))
        }
    }
}

/// Submit an auth token directly to the extension manager, bypassing the message pipeline.
///
/// The token never touches the LLM, chat history, or SSE stream.
async fn chat_auth_token_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<AuthTokenRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    if let Some(ref thread_id) = req.thread_id
        && crate::bridge::get_engine_pending_auth(&user.user_id, Some(thread_id))
            .await
            .is_some()
    {
        dispatch_engine_auth_resolution(&state, &user.user_id, thread_id, req.token.clone())
            .await?;
        return Ok(Json(ActionResponse::ok("Credential submitted.")));
    }

    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Extension manager not available".to_string(),
    ))?;

    match ext_mgr
        .configure_token(&req.extension_name, &req.token, &user.user_id)
        .await
    {
        Ok(result) => {
            let mut resp = if result.verification.is_some() || result.activated {
                ActionResponse::ok(result.message.clone())
            } else {
                ActionResponse::fail(result.message.clone())
            };
            resp.activated = Some(result.activated);
            resp.auth_url = result.auth_url.clone();
            resp.verification = result.verification.clone();
            resp.instructions = result.verification.as_ref().map(|v| v.instructions.clone());
            resp.onboarding_state = result.onboarding_state;
            resp.onboarding = result.onboarding.clone();

            if result.verification.is_some() {
                state.sse.broadcast_for_user(
                    &user.user_id,
                    AppEvent::AuthRequired {
                        extension_name: req.extension_name.clone(),
                        instructions: Some(result.message),
                        auth_url: None,
                        setup_url: None,
                        thread_id: req.thread_id.clone(),
                    },
                );
            } else if result.activated {
                // Clear auth mode on the active thread
                clear_auth_mode(&state, &user.user_id).await;

                state.sse.broadcast_for_user(
                    &user.user_id,
                    AppEvent::AuthCompleted {
                        extension_name: req.extension_name.clone(),
                        success: true,
                        message: result.message,
                        thread_id: req.thread_id.clone(),
                    },
                );
                if result.pairing_required {
                    state.sse.broadcast_for_user(
                        &user.user_id,
                        AppEvent::PairingRequired {
                            channel: req.extension_name.clone(),
                            instructions: Some(pairing_instructions(&req.extension_name)),
                            onboarding: result
                                .onboarding
                                .clone()
                                .and_then(|onboarding| serde_json::to_value(onboarding).ok()),
                            thread_id: req.thread_id.clone(),
                        },
                    );
                }
            } else {
                state.sse.broadcast_for_user(
                    &user.user_id,
                    AppEvent::AuthCompleted {
                        extension_name: req.extension_name.clone(),
                        success: false,
                        message: result.message,
                        thread_id: req.thread_id.clone(),
                    },
                );
            }

            Ok(Json(resp))
        }
        Err(e) => {
            let msg = e.to_string();

            // Skill credential fallback: if the extension manager doesn't
            // recognize the name (skill credential like "github_token"),
            // store directly in the secrets store.
            if matches!(&e, crate::extensions::ExtensionError::NotInstalled(_))
                || msg.contains("not found")
            {
                let ss = state
                    .tool_registry
                    .as_ref()
                    .and_then(|tr| tr.secrets_store().cloned())
                    .or_else(|| {
                        state
                            .extension_manager
                            .as_ref()
                            .map(|em| std::sync::Arc::clone(em.secrets()))
                    });

                if let Some(ss) = ss {
                    let params =
                        crate::secrets::CreateSecretParams::new(&req.extension_name, &req.token);
                    match ss.create(&user.user_id, params).await {
                        Ok(_) => {
                            clear_auth_mode(&state, &user.user_id).await;
                            crate::bridge::clear_engine_pending_auth(
                                &user.user_id,
                                req.thread_id.as_deref(),
                            )
                            .await;
                            state.sse.broadcast_for_user(
                                &user.user_id,
                                AppEvent::AuthCompleted {
                                    extension_name: req.extension_name.clone(),
                                    success: true,
                                    message: format!(
                                        "Credential '{}' stored successfully.",
                                        req.extension_name
                                    ),
                                    thread_id: req.thread_id.clone(),
                                },
                            );
                            return Ok(Json(ActionResponse::ok(format!(
                                "Credential '{}' stored.",
                                req.extension_name
                            ))));
                        }
                        Err(se) => {
                            return Ok(Json(ActionResponse::fail(format!(
                                "Failed to store credential: {se}"
                            ))));
                        }
                    }
                } else {
                    return Ok(Json(ActionResponse::fail(format!(
                        "Cannot store credential '{}': no secrets store configured.",
                        req.extension_name
                    ))));
                }
            }

            // Re-emit auth_required for retry on validation errors
            if matches!(e, crate::extensions::ExtensionError::ValidationFailed(_)) {
                state.sse.broadcast_for_user(
                    &user.user_id,
                    AppEvent::AuthRequired {
                        extension_name: req.extension_name.clone(),
                        instructions: Some(msg.clone()),
                        auth_url: None,
                        setup_url: None,
                        thread_id: req.thread_id.clone(),
                    },
                );
            }
            Ok(Json(ActionResponse::fail(msg)))
        }
    }
}

/// Cancel an in-progress auth flow.
async fn chat_auth_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<AuthCancelRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    if let Some(ref thread_id) = req.thread_id
        && crate::bridge::get_engine_pending_auth(&user.user_id, Some(thread_id))
            .await
            .is_some()
    {
        dispatch_engine_auth_resolution(&state, &user.user_id, thread_id, "cancel".into()).await?;
        return Ok(Json(ActionResponse::ok("Auth cancelled")));
    }

    clear_auth_mode(&state, &user.user_id).await;
    // Also clear engine v2 pending auth so the next message isn't consumed as a token.
    crate::bridge::clear_engine_pending_auth(&user.user_id, req.thread_id.as_deref()).await;
    Ok(Json(ActionResponse::ok("Auth cancelled")))
}

/// Clear pending auth mode on the active thread.
pub async fn clear_auth_mode(state: &GatewayState, user_id: &str) {
    if let Some(ref sm) = state.session_manager {
        let session = sm.get_or_create_session(user_id).await;
        let mut sess = session.lock().await;
        if let Some(thread_id) = sess.active_thread
            && let Some(thread) = sess.threads.get_mut(&thread_id)
        {
            thread.pending_auth = None;
        }
    }
}

/// Check whether an Origin header value points to a local address.
///
/// Extracts the host from the origin (handling both IPv4/hostname and IPv6
/// literal formats) and compares it against known local addresses. Used to
/// prevent cross-site WebSocket hijacking while allowing localhost access.
fn is_local_origin(origin: &str) -> bool {
    let host = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .and_then(|rest| {
            if rest.starts_with('[') {
                // IPv6 literal: extract "[::1]" up to and including ']'
                rest.find(']').map(|i| &rest[..=i])
            } else {
                // IPv4 or hostname: take up to the first ':' (port) or '/' (path)
                rest.split(':').next()?.split('/').next()
            }
        })
        .unwrap_or("");

    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

async fn chat_ws_handler(
    AuthenticatedUser(user): AuthenticatedUser,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<GatewayState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Validate Origin header to prevent cross-site WebSocket hijacking.
    // Require the header outright; browsers always send it for WS upgrades,
    // so a missing Origin means a non-browser client trying to bypass the check.
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                "WebSocket Origin header required".to_string(),
            )
        })?;

    let is_local = is_local_origin(origin);
    if !is_local {
        return Err((
            StatusCode::FORBIDDEN,
            "WebSocket origin not allowed".to_string(),
        ));
    }
    Ok(ws.on_upgrade(move |socket| {
        crate::channels::web::ws::handle_ws_connection(socket, state, user)
    }))
}

#[derive(Deserialize)]
struct HistoryQuery {
    thread_id: Option<String>,
    limit: Option<usize>,
    before: Option<String>,
}

async fn engine_pending_gate_info(
    user_id: &str,
    thread_id: Option<&str>,
) -> Option<PendingGateInfo> {
    let pending = crate::bridge::get_engine_pending_gate(user_id, thread_id)
        .await
        .ok()??;
    Some(PendingGateInfo {
        request_id: pending.request_id,
        thread_id: pending.thread_id.to_string(),
        gate_name: pending.gate_name,
        tool_name: pending.tool_name,
        description: pending.description,
        parameters: pending.parameters,
        resume_kind: serde_json::to_value(pending.resume_kind).unwrap_or_default(),
    })
}

async fn history_pending_gate_info(
    user_id: &str,
    thread_id: Option<&str>,
) -> Option<PendingGateInfo> {
    let scoped = engine_pending_gate_info(user_id, thread_id).await;
    if scoped.is_some() || thread_id.is_none() {
        return scoped;
    }
    engine_pending_gate_info(user_id, None).await
}

async fn dispatch_engine_auth_resolution(
    state: &GatewayState,
    user_id: &str,
    thread_id: &str,
    content: String,
) -> Result<(), (StatusCode, String)> {
    let tx = {
        let tx_guard = state.msg_tx.read().await;
        tx_guard
            .as_ref()
            .ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Channel not started".to_string(),
            ))?
            .clone()
    };

    let msg = IncomingMessage::new("gateway", user_id, content)
        .with_thread(thread_id.to_string())
        .into_internal();

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })
}

async fn chat_history_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager.get_or_create_session(&user.user_id).await;
    let sess = session.lock().await;

    let limit = query.limit.unwrap_or(50);
    let before_cursor = query
        .before
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        "Invalid 'before' timestamp".to_string(),
                    )
                })
        })
        .transpose()?;

    // Find the thread
    let thread_id = if let Some(ref tid) = query.thread_id {
        Uuid::parse_str(tid)
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid thread_id".to_string()))?
    } else {
        sess.active_thread
            .ok_or((StatusCode::NOT_FOUND, "No active thread".to_string()))?
    };
    let thread_id_str = thread_id.to_string();
    let thread_scope = Some(thread_id_str.as_str());

    // Verify the thread belongs to the authenticated user before returning any data.
    // In-memory threads are already scoped by user via session_manager, but DB
    // lookups could expose another user's conversation if the UUID is guessed.
    if query.thread_id.is_some()
        && let Some(ref store) = state.store
    {
        let owned = store
            .conversation_belongs_to_user(thread_id, &user.user_id)
            .await
            .map_err(|e| {
                tracing::error!(thread_id = %thread_id, error = %e, "DB error during thread ownership check");
                (StatusCode::INTERNAL_SERVER_ERROR, "Database error".to_string())
            })?;
        if !owned && !sess.threads.contains_key(&thread_id) {
            return Err((StatusCode::NOT_FOUND, "Thread not found".to_string()));
        }
    }

    // For paginated requests (before cursor set), always go to DB
    if before_cursor.is_some()
        && let Some(ref store) = state.store
    {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, before_cursor, limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
        let turns = build_turns_from_db_messages(&messages);
        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more,
            oldest_timestamp,
            pending_gate: history_pending_gate_info(&user.user_id, thread_scope).await,
        }));
    }

    // Try in-memory first (freshest data for active threads)
    if let Some(thread) = sess.threads.get(&thread_id)
        && (!thread.turns.is_empty() || thread.pending_approval.is_some())
    {
        let turns: Vec<TurnInfo> = thread
            .turns
            .iter()
            .map(|t| TurnInfo {
                turn_number: t.turn_number,
                user_input: t.user_input.clone(),
                response: t.response.clone(),
                state: format!("{:?}", t.state),
                started_at: t.started_at.to_rfc3339(),
                completed_at: t.completed_at.map(|dt| dt.to_rfc3339()),
                tool_calls: t
                    .tool_calls
                    .iter()
                    .map(|tc| ToolCallInfo {
                        name: tc.name.clone(),
                        has_result: tc.result.is_some(),
                        has_error: tc.error.is_some(),
                        result_preview: tc.result.as_ref().map(|r| {
                            let s = match r {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            truncate_preview(&s, 500)
                        }),
                        error: tc.error.clone(),
                        rationale: tc.rationale.clone(),
                    })
                    .collect(),
                narrative: t.narrative.clone(),
            })
            .collect();

        let pending_gate = history_pending_gate_info(&user.user_id, thread_scope)
            .await
            .or_else(|| {
                thread.pending_approval.as_ref().map(|pa| PendingGateInfo {
                    request_id: pa.request_id.to_string(),
                    thread_id: thread_id.to_string(),
                    gate_name: "approval".into(),
                    tool_name: pa.tool_name.clone(),
                    description: pa.description.clone(),
                    parameters: serde_json::to_string_pretty(&pa.parameters).unwrap_or_default(),
                    resume_kind: serde_json::json!({"Approval":{"allow_always":true}}),
                })
            });

        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more: false,
            oldest_timestamp: None,
            pending_gate,
        }));
    }

    // Fall back to DB for historical threads not in memory (paginated)
    if let Some(ref store) = state.store {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, None, limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if !messages.is_empty() {
            let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
            let turns = build_turns_from_db_messages(&messages);
            return Ok(Json(HistoryResponse {
                thread_id,
                turns,
                has_more,
                oldest_timestamp,
                pending_gate: history_pending_gate_info(&user.user_id, thread_scope).await,
            }));
        }
    }

    // Empty thread (just created, no messages yet)
    Ok(Json(HistoryResponse {
        thread_id,
        turns: Vec::new(),
        has_more: false,
        oldest_timestamp: None,
        pending_gate: history_pending_gate_info(&user.user_id, thread_scope).await,
    }))
}

async fn chat_threads_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<ThreadListResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager.get_or_create_session(&user.user_id).await;
    let sess = session.lock().await;

    // Try DB first for persistent thread list
    if let Some(ref store) = state.store {
        // Auto-create assistant thread if it doesn't exist
        let assistant_id = store
            .get_or_create_assistant_conversation(&user.user_id, "gateway")
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        // Seed the bootstrap greeting if this is a brand-new assistant thread.
        // Use add_conversation_message_if_empty to avoid duplicates on concurrent requests.
        static GREETING: &str = include_str!("../../workspace/seeds/GREETING.md");
        if let Err(e) = store
            .add_conversation_message_if_empty(assistant_id, "assistant", GREETING)
            .await
        {
            tracing::warn!(
                user_id = %user.user_id,
                error = %e,
                "Failed to seed assistant greeting"
            );
        }

        match store
            .list_conversations_all_channels(&user.user_id, 50)
            .await
        {
            Ok(summaries) => {
                let mut assistant_thread = None;
                let mut threads = Vec::new();

                for s in &summaries {
                    let info = ThreadInfo {
                        id: s.id,
                        state: "Idle".to_string(),
                        turn_count: s.message_count.max(0) as usize,
                        created_at: s.started_at.to_rfc3339(),
                        updated_at: s.last_activity.to_rfc3339(),
                        title: s.title.clone(),
                        thread_type: s.thread_type.clone(),
                        channel: Some(s.channel.clone()),
                    };

                    if s.id == assistant_id {
                        assistant_thread = Some(info);
                    } else {
                        threads.push(info);
                    }
                }

                // If assistant wasn't in the list (0 messages), synthesize it
                if assistant_thread.is_none() {
                    assistant_thread = Some(ThreadInfo {
                        id: assistant_id,
                        state: "Idle".to_string(),
                        turn_count: 0,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        updated_at: chrono::Utc::now().to_rfc3339(),
                        title: None,
                        thread_type: Some("assistant".to_string()),
                        channel: Some("gateway".to_string()),
                    });
                }

                return Ok(Json(ThreadListResponse {
                    assistant_thread,
                    threads,
                    active_thread: sess.active_thread,
                }));
            }
            Err(e) => {
                tracing::error!(user_id = %user.user_id, error = %e, "DB error listing threads; falling back to in-memory");
            }
        }
    }

    // Fallback: in-memory only (no assistant thread without DB)
    let mut sorted_threads: Vec<_> = sess.threads.values().collect();
    sorted_threads.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
    let threads: Vec<ThreadInfo> = sorted_threads
        .into_iter()
        .map(|t| ThreadInfo {
            id: t.id,
            state: format!("{:?}", t.state),
            turn_count: t.turns.len(),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
            title: None,
            thread_type: None,
            channel: Some("gateway".to_string()),
        })
        .collect();

    Ok(Json(ThreadListResponse {
        assistant_thread: None,
        threads,
        active_thread: sess.active_thread,
    }))
}

async fn chat_new_thread_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<ThreadInfo>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager.get_or_create_session(&user.user_id).await;
    let (thread_id, info) = {
        let mut sess = session.lock().await;
        let thread = sess.create_thread(Some("gateway"));
        let id = thread.id;
        let info = ThreadInfo {
            id: thread.id,
            state: format!("{:?}", thread.state),
            turn_count: thread.turns.len(),
            created_at: thread.created_at.to_rfc3339(),
            updated_at: thread.updated_at.to_rfc3339(),
            title: None,
            thread_type: Some("thread".to_string()),
            channel: Some("gateway".to_string()),
        };
        (id, info)
    };

    // Persist the empty conversation row with thread_type metadata synchronously
    // so that the subsequent loadThreads() call from the frontend sees it.
    if let Some(ref store) = state.store {
        match store
            .ensure_conversation(thread_id, "gateway", &user.user_id, None, Some("gateway"))
            .await
        {
            Ok(true) => {}
            Ok(false) => tracing::warn!(
                user = %user.user_id,
                thread_id = %thread_id,
                "Skipped persisting new thread due to ownership/channel conflict"
            ),
            Err(e) => tracing::warn!("Failed to persist new thread: {}", e),
        }
        let metadata_val = serde_json::json!("thread");
        if let Err(e) = store
            .update_conversation_metadata_field(thread_id, "thread_type", &metadata_val)
            .await
        {
            tracing::warn!("Failed to set thread_type metadata: {}", e);
        }
    }

    Ok(Json(info))
}

// Job handlers moved to handlers/jobs.rs
// --- Logs handlers ---

async fn logs_events_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let broadcaster = state.log_broadcaster.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Log broadcaster not available".to_string(),
    ))?;

    // Replay recent history so late-joining browsers see startup logs.
    // Subscribe BEFORE snapshotting to avoid a gap between history and live.
    let rx = broadcaster.subscribe();
    let history = broadcaster.recent_entries();

    let history_stream = futures::stream::iter(history).map(|entry| {
        let data = serde_json::to_string(&entry).unwrap_or_default();
        Ok::<_, Infallible>(Event::default().event("log").data(data))
    });

    let live_stream = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|result| result.ok())
        .map(|entry| {
            let data = serde_json::to_string(&entry).unwrap_or_default();
            Ok::<_, Infallible>(Event::default().event("log").data(data))
        });

    let stream = history_stream.chain(live_stream);

    Ok((
        [("X-Accel-Buffering", "no"), ("Cache-Control", "no-cache")],
        Sse::new(stream).keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(30))
                .text(""),
        ),
    ))
}

async fn logs_level_get_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let handle = state.log_level_handle.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Log level control not available".to_string(),
    ))?;
    Ok(Json(serde_json::json!({ "level": handle.current_level() })))
}

async fn logs_level_set_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let handle = state.log_level_handle.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Log level control not available".to_string(),
    ))?;

    let level = body
        .get("level")
        .and_then(|v| v.as_str())
        .ok_or((StatusCode::BAD_REQUEST, "missing 'level' field".to_string()))?;

    handle
        .set_level(level)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    tracing::info!(user_id = %user.user_id, "Log level changed to '{}'", handle.current_level());
    Ok(Json(serde_json::json!({ "level": handle.current_level() })))
}

// --- Extension handlers ---

async fn extensions_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<ExtensionListResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let installed = ext_mgr
        .list(None, false, &user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut extensions = Vec::with_capacity(installed.len());
    for ext in installed {
        let ready_for_active = if ext.kind == crate::extensions::ExtensionKind::WasmChannel {
            !ext_mgr.channel_requires_pairing(&ext.name).await
        } else {
            false
        };
        let activation_status =
            crate::channels::web::handlers::extensions::derive_activation_status(
                &ext,
                ready_for_active,
            );
        let onboarding_state = if ext.kind == crate::extensions::ExtensionKind::WasmChannel {
            Some(if ext.activation_error.is_some() {
                ChannelOnboardingState::Failed
            } else if !ext.authenticated {
                ChannelOnboardingState::SetupRequired
            } else if ext.active {
                if ready_for_active {
                    ChannelOnboardingState::Ready
                } else {
                    ChannelOnboardingState::PairingRequired
                }
            } else {
                ChannelOnboardingState::ActivationInProgress
            })
        } else {
            None
        };
        let onboarding = if let Some(state) = onboarding_state {
            ext_mgr.channel_onboarding_for_state(&ext.name, state).await
        } else {
            None
        };
        extensions.push(ExtensionInfo {
            name: ext.name,
            display_name: ext.display_name,
            kind: ext.kind.to_string(),
            description: ext.description,
            url: ext.url,
            authenticated: ext.authenticated,
            active: ext.active,
            tools: ext.tools,
            needs_setup: ext.needs_setup,
            has_auth: ext.has_auth,
            activation_status,
            activation_error: ext.activation_error,
            version: ext.version,
            onboarding_state,
            onboarding,
        });
    }

    Ok(Json(ExtensionListResponse { extensions }))
}

async fn extensions_tools_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Result<Json<ToolListResponse>, (StatusCode, String)> {
    let registry = state.tool_registry.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Tool registry not available".to_string(),
    ))?;

    let definitions = registry.tool_definitions().await;
    let tools = definitions
        .into_iter()
        .map(|td| ToolInfo {
            name: td.name,
            description: td.description,
        })
        .collect();

    Ok(Json(ToolListResponse { tools }))
}

async fn extensions_install_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<InstallExtensionRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // When extension manager isn't available, check registry entries for a helpful message
    let Some(ext_mgr) = state.extension_manager.as_ref() else {
        // Look up the entry in the catalog to give a specific error
        if let Some(entry) = state.registry_entries.iter().find(|e| e.name == req.name) {
            let msg = match &entry.source {
                crate::extensions::ExtensionSource::WasmBuildable { .. } => {
                    format!(
                        "'{}' requires building from source. \
                         Run `ironclaw registry install {}` from the CLI.",
                        req.name, req.name
                    )
                }
                _ => format!(
                    "Extension manager not available (secrets store required). \
                     Configure DATABASE_URL or a secrets backend to enable installation of '{}'.",
                    req.name
                ),
            };
            return Ok(Json(ActionResponse::fail(msg)));
        }
        return Ok(Json(ActionResponse::fail(
            "Extension manager not available (secrets store required)".to_string(),
        )));
    };

    let kind_hint = req.kind.as_deref().and_then(|k| match k {
        "mcp_server" => Some(crate::extensions::ExtensionKind::McpServer),
        "wasm_tool" => Some(crate::extensions::ExtensionKind::WasmTool),
        "wasm_channel" => Some(crate::extensions::ExtensionKind::WasmChannel),
        "acp_agent" => Some(crate::extensions::ExtensionKind::AcpAgent),
        _ => None,
    });

    match ext_mgr
        .install(&req.name, req.url.as_deref(), kind_hint, &user.user_id)
        .await
    {
        Ok(result) => {
            let mut resp = ActionResponse::ok(result.message);

            // Auto-activate WASM tools after install (install = active).
            if result.kind == crate::extensions::ExtensionKind::WasmTool {
                if let Err(e) = ext_mgr.activate(&req.name, &user.user_id).await {
                    tracing::debug!(
                        extension = %req.name,
                        error = %e,
                        "Auto-activation after install failed"
                    );
                }

                // Check auth after activation. This may initiate OAuth both for scope
                // expansion and for first-time auth when credentials are already
                // configured (e.g., built-in providers). We only surface an auth_url
                // when the extension reports it is awaiting authorization.
                match ext_mgr.auth(&req.name, &user.user_id).await {
                    Ok(auth_result) if auth_result.auth_url().is_some() => {
                        // Scope expansion or initial OAuth: user needs to authorize
                        resp.auth_url = auth_result.auth_url().map(String::from);
                    }
                    _ => {}
                }
            }

            Ok(Json(resp))
        }
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

async fn extensions_activate_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    tracing::trace!(
        extension = %name,
        user_id = %user.user_id,
        "extensions_activate_handler: received activate request"
    );
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.activate(&name, &user.user_id).await {
        Ok(result) => {
            tracing::info!(
                extension = %name,
                "extensions_activate_handler: activation succeeded"
            );
            // Activation loaded the WASM module. Check if the tool needs
            // OAuth scope expansion (e.g., adding google-docs when gmail
            // already has a token but missing the documents scope).
            // Initial OAuth setup is triggered via configure.
            let mut resp = ActionResponse::ok(result.message);
            if let Ok(auth_result) = ext_mgr.auth(&name, &user.user_id).await
                && auth_result.auth_url().is_some()
            {
                resp.auth_url = auth_result.auth_url().map(String::from);
            }
            Ok(Json(resp))
        }
        Err(activate_err) => {
            let needs_auth = matches!(
                &activate_err,
                crate::extensions::ExtensionError::AuthRequired
            );

            tracing::trace!(
                extension = %name,
                error = %activate_err,
                needs_auth = needs_auth,
                "extensions_activate_handler: activation failed, attempting auth fallback"
            );

            if !needs_auth {
                return Ok(Json(ActionResponse::fail(activate_err.to_string())));
            }

            // Activation failed due to auth; try authenticating first.
            match ext_mgr.auth(&name, &user.user_id).await {
                Ok(auth_result) if auth_result.is_authenticated() => {
                    tracing::trace!(
                        extension = %name,
                        "extensions_activate_handler: auth reports authenticated, retrying activate"
                    );
                    // Auth succeeded, retry activation.
                    match ext_mgr.activate(&name, &user.user_id).await {
                        Ok(result) => Ok(Json(ActionResponse::ok(result.message))),
                        Err(e) => {
                            tracing::warn!(
                                extension = %name,
                                error = %e,
                                "extensions_activate_handler: retry after auth still failed"
                            );
                            Ok(Json(ActionResponse::fail(e.to_string())))
                        }
                    }
                }
                Ok(auth_result) => {
                    // Auth in progress (OAuth URL or awaiting manual token).
                    let mut resp = ActionResponse::fail(
                        auth_result
                            .instructions()
                            .map(String::from)
                            .unwrap_or_else(|| format!("'{}' requires authentication.", name)),
                    );
                    resp.auth_url = auth_result.auth_url().map(String::from);
                    resp.awaiting_token = Some(auth_result.is_awaiting_token());
                    resp.instructions = auth_result.instructions().map(String::from);
                    Ok(Json(resp))
                }
                Err(auth_err) => Ok(Json(ActionResponse::fail(format!(
                    "Authentication failed: {}",
                    auth_err
                )))),
            }
        }
    }
}

// --- Project file serving handlers ---

/// Redirect `/projects/{id}` to `/projects/{id}/` so relative paths in
/// the served HTML resolve within the project namespace.
async fn project_redirect_handler(
    State(state): State<Arc<GatewayState>>,
    super::auth::AuthenticatedUser(user): super::auth::AuthenticatedUser,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    if !verify_project_ownership(&state, &project_id, &user.user_id).await {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    axum::response::Redirect::permanent(&format!("/projects/{project_id}/")).into_response()
}

/// Serve `index.html` when hitting `/projects/{project_id}/`.
async fn project_index_handler(
    State(state): State<Arc<GatewayState>>,
    super::auth::AuthenticatedUser(user): super::auth::AuthenticatedUser,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    if !verify_project_ownership(&state, &project_id, &user.user_id).await {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    serve_project_file(&project_id, "index.html").await
}

/// Serve any file under `/projects/{project_id}/{path}`.
async fn project_file_handler(
    State(state): State<Arc<GatewayState>>,
    super::auth::AuthenticatedUser(user): super::auth::AuthenticatedUser,
    Path((project_id, path)): Path<(String, String)>,
) -> impl IntoResponse {
    if !verify_project_ownership(&state, &project_id, &user.user_id).await {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    serve_project_file(&project_id, &path).await
}

/// Check that a project directory belongs to a job owned by the given user.
/// Returns false if the store is unavailable or the project is not found.
async fn verify_project_ownership(state: &GatewayState, project_id: &str, user_id: &str) -> bool {
    let Some(ref store) = state.store else {
        return false;
    };
    // The project_id is a sandbox job UUID used as the directory name.
    let Ok(job_id) = project_id.parse::<uuid::Uuid>() else {
        return false;
    };
    match store.get_sandbox_job(job_id).await {
        Ok(Some(job)) => job.user_id == user_id,
        _ => false,
    }
}

/// Shared logic: resolve the file inside `~/.ironclaw/projects/{project_id}/`,
/// guard against path traversal, and stream the content with the right MIME type.
async fn serve_project_file(project_id: &str, path: &str) -> axum::response::Response {
    // Reject project_id values that could escape the projects directory.
    if project_id.contains('/')
        || project_id.contains('\\')
        || project_id.contains("..")
        || project_id.is_empty()
    {
        return (StatusCode::BAD_REQUEST, "Invalid project ID").into_response();
    }

    let base = ironclaw_base_dir().join("projects").join(project_id);

    let file_path = base.join(path);

    // Path traversal guard
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let base_canonical = match base.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    if !canonical.starts_with(&base_canonical) {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    }

    match tokio::fs::read(&canonical).await {
        Ok(contents) => {
            let mime = mime_guess::from_path(&canonical)
                .first_or_octet_stream()
                .to_string();
            ([(header::CONTENT_TYPE, mime)], contents).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

async fn extensions_remove_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.remove(&name, &user.user_id).await {
        Ok(message) => Ok(Json(ActionResponse::ok(message))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

async fn extensions_registry_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(params): Query<RegistrySearchQuery>,
) -> Json<RegistrySearchResponse> {
    let query = params.query.unwrap_or_default();
    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();

    // Filter registry entries by query (or return all if empty)
    let matching: Vec<&crate::extensions::RegistryEntry> = if tokens.is_empty() {
        state.registry_entries.iter().collect()
    } else {
        state
            .registry_entries
            .iter()
            .filter(|e| {
                let name = e.name.to_lowercase();
                let display = e.display_name.to_lowercase();
                let desc = e.description.to_lowercase();
                tokens.iter().any(|t| {
                    name.contains(t)
                        || display.contains(t)
                        || desc.contains(t)
                        || e.keywords.iter().any(|k| k.to_lowercase().contains(t))
                })
            })
            .collect()
    };

    // Cross-reference with installed extensions by (name, kind) to avoid
    // false positives when the same name exists as different kinds.
    let installed: std::collections::HashSet<(String, String)> =
        if let Some(ext_mgr) = state.extension_manager.as_ref() {
            ext_mgr
                .list(None, false, &user.user_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|ext| (ext.name, ext.kind.to_string()))
                .collect()
        } else {
            std::collections::HashSet::new()
        };

    let entries = matching
        .into_iter()
        .map(|e| {
            let kind_str = e.kind.to_string();
            RegistryEntryInfo {
                name: e.name.clone(),
                display_name: e.display_name.clone(),
                installed: installed.contains(&(e.name.clone(), kind_str.clone())),
                kind: kind_str,
                description: e.description.clone(),
                keywords: e.keywords.clone(),
                version: e.version.clone(),
            }
        })
        .collect();

    Json(RegistrySearchResponse { entries })
}

async fn extensions_setup_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
) -> Result<Json<ExtensionSetupResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let setup = ext_mgr
        .get_setup_schema(&name, &user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let kind = ext_mgr
        .list(None, false, &user.user_id)
        .await
        .ok()
        .and_then(|list| list.into_iter().find(|e| e.name == name))
        .map(|e| e.kind.to_string())
        .unwrap_or_default();

    Ok(Json(ExtensionSetupResponse {
        name,
        kind,
        secrets: setup.secrets,
        fields: setup.fields,
        onboarding_state: setup.onboarding_state,
        onboarding: setup.onboarding,
    }))
}

async fn extensions_setup_submit_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
    Json(req): Json<ExtensionSetupRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    // Clear auth mode regardless of outcome so the next user message goes
    // through to the LLM instead of being intercepted as a token.
    clear_auth_mode(&state, &user.user_id).await;

    match ext_mgr
        .configure(&name, &req.secrets, &req.fields, &user.user_id)
        .await
    {
        Ok(result) => {
            let mut resp = if result.verification.is_some() || result.activated {
                ActionResponse::ok(result.message)
            } else {
                ActionResponse::fail(result.message)
            };
            resp.activated = Some(result.activated);
            if result.restart_required || !result.activated {
                resp.needs_restart = Some(true);
            }
            resp.auth_url = result.auth_url.clone();
            resp.verification = result.verification.clone();
            resp.instructions = result.verification.as_ref().map(|v| v.instructions.clone());
            resp.onboarding_state = result.onboarding_state;
            resp.onboarding = result.onboarding.clone();
            if result.verification.is_none() {
                // Broadcast auth_completed so the chat UI can dismiss any in-progress
                // auth card or setup modal that was triggered by tool_auth/tool_activate.
                state.sse.broadcast_for_user(
                    &user.user_id,
                    AppEvent::AuthCompleted {
                        extension_name: name.clone(),
                        success: result.activated,
                        message: resp.message.clone(),
                        thread_id: None,
                    },
                );
                if result.pairing_required {
                    state.sse.broadcast_for_user(
                        &user.user_id,
                        AppEvent::PairingRequired {
                            channel: name.clone(),
                            instructions: Some(pairing_instructions(&name)),
                            onboarding: result
                                .onboarding
                                .clone()
                                .and_then(|onboarding| serde_json::to_value(onboarding).ok()),
                            thread_id: None,
                        },
                    );
                }
            }
            Ok(Json(resp))
        }
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

fn pairing_instructions(channel: &str) -> String {
    if channel == "telegram" {
        "Open your Telegram bot, send it any message such as hi or /start, wait for the pairing code reply, then paste that code here. Telegram bots cannot message you first.".to_string()
    } else {
        format!(
            "Open {channel}, send it any message to receive a pairing code, then paste that code here, or run `ironclaw pairing approve {channel} <CODE>`."
        )
    }
}

// --- Pairing handlers ---

async fn pairing_list_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_user): AdminUser,
    Path(channel): Path<String>,
) -> Result<Json<PairingListResponse>, (StatusCode, String)> {
    let store = state.pairing_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Pairing store not available".to_string(),
    ))?;
    let requests: Vec<crate::db::PairingRequestRecord> =
        store.list_pending(&channel).await.map_err(|e| {
            tracing::warn!(error = %e, "pairing list failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error listing pairing requests".to_string(),
            )
        })?;

    let infos = requests
        .into_iter()
        .map(|r| PairingRequestInfo {
            code: r.code,
            sender_id: r.external_id,
            meta: None,
            created_at: r.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(PairingListResponse {
        channel,
        requests: infos,
    }))
}

async fn pairing_approve_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(channel): Path<String>,
    Json(req): Json<PairingApproveRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let flow = crate::pairing::PairingCodeChallenge::new(&channel);
    let Some(code) =
        crate::code_challenge::CodeChallengeFlow::normalize_submission(&flow, &req.code)
    else {
        return Ok(Json(ActionResponse::fail(
            "Pairing code is required.".to_string(),
        )));
    };

    let store = state.pairing_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Pairing store not available".to_string(),
    ))?;
    let owner_id = crate::ownership::OwnerId::from(user.user_id.clone());
    match store.approve(&channel, &code, &owner_id).await {
        Ok(()) => {
            let onboarding = if let Some(ext_mgr) = state.extension_manager.as_ref() {
                ext_mgr
                    .channel_onboarding_for_state(&channel, ChannelOnboardingState::Ready)
                    .await
            } else {
                None
            };
            state.sse.broadcast_for_user(
                &user.user_id,
                AppEvent::PairingCompleted {
                    channel: channel.clone(),
                    success: true,
                    message: format!("Ownership claimed for '{channel}'. The channel is ready."),
                    thread_id: None,
                },
            );
            state.sse.broadcast_for_user(
                &user.user_id,
                AppEvent::ExtensionStatus {
                    extension_name: channel.clone(),
                    status: "active".to_string(),
                    message: None,
                },
            );
            let mut resp = ActionResponse::ok(format!(
                "Ownership claimed for '{channel}'. The channel is ready."
            ));
            resp.onboarding_state = Some(ChannelOnboardingState::Ready);
            resp.onboarding = onboarding;
            Ok(Json(resp))
        }
        Err(crate::error::DatabaseError::NotFound { .. }) => Ok(Json(ActionResponse::fail(
            "Invalid or expired pairing code.".to_string(),
        ))),
        Err(e) => {
            tracing::warn!(error = %e, "pairing approval failed");
            Ok(Json(ActionResponse::fail(
                "Internal error processing approval.".to_string(),
            )))
        }
    }
}

async fn routines_runs_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    // Verify ownership before listing runs.
    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    if routine.user_id != user.user_id {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

    let runs = store
        .list_routine_runs(routine_id, 50)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let run_infos: Vec<RoutineRunInfo> = runs
        .iter()
        .map(|run| RoutineRunInfo {
            id: run.id,
            trigger_type: run.trigger_type.clone(),
            started_at: run.started_at.to_rfc3339(),
            completed_at: run.completed_at.map(|dt| dt.to_rfc3339()),
            status: run.status.to_string(),
            result_summary: run.result_summary.clone(),
            tokens_used: run.tokens_used,
            job_id: run.job_id,
        })
        .collect();

    Ok(Json(serde_json::json!({
        "routine_id": routine_id,
        "runs": run_infos,
    })))
}

// --- Gateway control plane handlers ---

async fn gateway_status_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Json<GatewayStatusResponse> {
    let sse_connections = state.sse.connection_count();
    let ws_connections = state
        .ws_tracker
        .as_ref()
        .map(|t| t.connection_count())
        .unwrap_or(0);

    let uptime_secs = state.startup_time.elapsed().as_secs();

    let (daily_cost, actions_this_hour, model_usage) = if let Some(ref cg) = state.cost_guard {
        let cost = cg.daily_spend().await;
        let actions = cg.actions_this_hour().await;
        let usage = cg.model_usage().await;
        let models: Vec<ModelUsageEntry> = usage
            .into_iter()
            .map(|(model, tokens)| ModelUsageEntry {
                model,
                input_tokens: tokens.input_tokens,
                output_tokens: tokens.output_tokens,
                cost: format!("{:.6}", tokens.cost),
            })
            .collect();
        (Some(format!("{:.4}", cost)), Some(actions), Some(models))
    } else {
        (None, None, None)
    };

    let restart_enabled = std::env::var("IRONCLAW_IN_DOCKER")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);

    Json(GatewayStatusResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        sse_connections,
        ws_connections,
        total_connections: sse_connections + ws_connections,
        uptime_secs,
        restart_enabled,
        daily_cost,
        actions_this_hour,
        model_usage,
        llm_backend: state.active_config.llm_backend.clone(),
        llm_model: state.active_config.llm_model.clone(),
        enabled_channels: state.active_config.enabled_channels.clone(),
    })
}

#[derive(serde::Serialize)]
struct ModelUsageEntry {
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cost: String,
}

#[derive(serde::Serialize)]
struct GatewayStatusResponse {
    version: String,
    sse_connections: u64,
    ws_connections: u64,
    total_connections: u64,
    uptime_secs: u64,
    restart_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    daily_cost: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actions_this_hour: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_usage: Option<Vec<ModelUsageEntry>>,
    llm_backend: String,
    llm_model: String,
    enabled_channels: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::web::types::{
        ExtensionActivationStatus, classify_wasm_channel_activation,
    };
    use crate::cli::oauth_defaults;
    use crate::extensions::{ExtensionKind, InstalledExtension};
    use crate::testing::credentials::TEST_GATEWAY_CRYPTO_KEY;

    #[test]
    fn test_build_turns_from_db_messages_complete() {
        let now = chrono::Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Hello".to_string(),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Hi there!".to_string(),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "How are you?".to_string(),
                created_at: now + chrono::TimeDelta::seconds(2),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Doing well!".to_string(),
                created_at: now + chrono::TimeDelta::seconds(3),
            },
        ];

        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user_input, "Hello");
        assert_eq!(turns[0].response.as_deref(), Some("Hi there!"));
        assert_eq!(turns[0].state, "Completed");
        assert_eq!(turns[1].user_input, "How are you?");
        assert_eq!(turns[1].response.as_deref(), Some("Doing well!"));
    }

    #[test]
    fn test_build_turns_from_db_messages_incomplete_last() {
        let now = chrono::Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Hello".to_string(),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Hi!".to_string(),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Lost message".to_string(),
                created_at: now + chrono::TimeDelta::seconds(2),
            },
        ];

        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].user_input, "Lost message");
        assert!(turns[1].response.is_none());
        assert_eq!(turns[1].state, "Failed");
    }

    #[test]
    fn test_build_turns_from_db_messages_empty() {
        let turns = build_turns_from_db_messages(&[]);
        assert!(turns.is_empty());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn workspace_pool_resolve_seeds_new_user_workspace() {
        let (db, _dir) = crate::testing::test_db().await;
        let pool = WorkspacePool::new(
            db,
            None,
            crate::workspace::EmbeddingCacheConfig::default(),
            crate::config::WorkspaceSearchConfig::default(),
            crate::config::WorkspaceConfig::default(),
        );

        let ws = crate::tools::builtin::memory::WorkspaceResolver::resolve(&pool, "alice").await;

        let readme = ws.read(crate::workspace::paths::README).await.unwrap();
        let identity = ws.read(crate::workspace::paths::IDENTITY).await.unwrap();

        assert!(!readme.content.trim().is_empty());
        assert!(!identity.content.trim().is_empty());
    }

    #[test]
    fn test_wasm_channel_activation_status_owner_bound_counts_as_active() -> Result<(), String> {
        let ext = InstalledExtension {
            name: "telegram".to_string(),
            kind: ExtensionKind::WasmChannel,
            display_name: Some("Telegram".to_string()),
            description: None,
            url: None,
            authenticated: true,
            active: true,
            tools: Vec::new(),
            needs_setup: true,
            has_auth: false,
            installed: true,
            activation_error: None,
            version: None,
        };

        let owner_bound = classify_wasm_channel_activation(&ext, false, true);
        if owner_bound != Some(ExtensionActivationStatus::Active) {
            return Err(format!(
                "owner-bound channel should be active, got {:?}",
                owner_bound
            ));
        }

        let unbound = classify_wasm_channel_activation(&ext, false, false);
        if unbound != Some(ExtensionActivationStatus::Pairing) {
            return Err(format!(
                "unbound channel should be pairing, got {:?}",
                unbound
            ));
        }

        Ok(())
    }

    #[test]
    fn test_channel_relay_activation_status_is_preserved() -> Result<(), String> {
        let relay = InstalledExtension {
            name: "signal".to_string(),
            kind: ExtensionKind::ChannelRelay,
            display_name: Some("Signal".to_string()),
            description: None,
            url: None,
            authenticated: true,
            active: false,
            tools: Vec::new(),
            needs_setup: true,
            has_auth: false,
            installed: true,
            activation_error: None,
            version: None,
        };

        let status = if relay.kind == crate::extensions::ExtensionKind::WasmChannel {
            classify_wasm_channel_activation(&relay, false, false)
        } else if relay.kind == crate::extensions::ExtensionKind::ChannelRelay {
            Some(if relay.active {
                ExtensionActivationStatus::Active
            } else if relay.authenticated {
                ExtensionActivationStatus::Configured
            } else {
                ExtensionActivationStatus::Installed
            })
        } else {
            None
        };

        if status != Some(ExtensionActivationStatus::Configured) {
            return Err(format!(
                "channel relay should retain configured status, got {:?}",
                status
            ));
        }

        Ok(())
    }

    // --- OAuth callback handler tests ---

    /// Build a minimal `GatewayState` for handler tests.
    fn test_gateway_state_with_dependencies(
        ext_mgr: Option<Arc<ExtensionManager>>,
        store: Option<Arc<dyn Database>>,
        db_auth: Option<Arc<crate::channels::web::auth::DbAuthenticator>>,
        pairing_store: Option<Arc<crate::pairing::PairingStore>>,
    ) -> Arc<GatewayState> {
        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: Arc::new(SseManager::new()),
            workspace: None,
            workspace_pool: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: ext_mgr,
            tool_registry: None,
            store,
            job_manager: None,
            prompt_queue: None,
            owner_id: "test".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            skill_registry: None,
            skill_catalog: None,
            scheduler: None,
            chat_rate_limiter: PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
            webhook_rate_limiter: RateLimiter::new(10, 60),
            registry_entries: vec![],
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: ActiveConfigSnapshot::default(),
            secrets_store: None,
            db_auth,
            pairing_store,
            oauth_providers: None,
            oauth_state_store: None,
            oauth_base_url: None,
            oauth_allowed_domains: Vec::new(),
            near_nonce_store: None,
            near_rpc_url: None,
            near_network: None,
            oauth_sweep_shutdown: None,
        })
    }

    fn test_gateway_state(ext_mgr: Option<Arc<ExtensionManager>>) -> Arc<GatewayState> {
        test_gateway_state_with_dependencies(ext_mgr, None, None, None)
    }

    /// Build a test router with just the OAuth callback route.
    fn test_oauth_router(state: Arc<GatewayState>) -> Router {
        Router::new()
            .route("/oauth/callback", get(oauth_callback_handler))
            .with_state(state)
    }

    #[cfg(feature = "libsql")]
    async fn insert_test_user(db: &Arc<dyn Database>, id: &str, role: &str) {
        db.get_or_create_user(crate::db::UserRecord {
            id: id.to_string(),
            role: role.to_string(),
            display_name: id.to_string(),
            status: "active".to_string(),
            email: None,
            last_login_at: None,
            created_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("create test user");
    }

    #[cfg(feature = "libsql")]
    async fn make_pairing_test_state() -> (
        Arc<GatewayState>,
        Arc<dyn Database>,
        Arc<crate::pairing::PairingStore>,
        tempfile::TempDir,
    ) {
        let (db, tmp) = crate::testing::test_db().await;
        insert_test_user(&db, "admin-1", "admin").await;
        insert_test_user(&db, "member-1", "member").await;
        let pairing_store = Arc::new(crate::pairing::PairingStore::new(
            Arc::clone(&db),
            Arc::new(crate::ownership::OwnershipCache::new()),
        ));
        let state = test_gateway_state_with_dependencies(
            None,
            Some(Arc::clone(&db)),
            None,
            Some(Arc::clone(&pairing_store)),
        );
        (state, db, pairing_store, tmp)
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_pairing_list_requires_admin_role() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (state, _db, pairing_store, _tmp) = make_pairing_test_state().await;
        pairing_store
            .upsert_request("telegram", "tg-user-1", None)
            .await
            .expect("create pairing request");

        let app = Router::new()
            .route("/api/pairing/{channel}", get(pairing_list_handler))
            .with_state(state);

        let mut member_req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/pairing/telegram")
            .body(Body::empty())
            .expect("member request");
        member_req.extensions_mut().insert(UserIdentity {
            user_id: "member-1".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let member_resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), member_req)
            .await
            .expect("member response");
        assert_eq!(member_resp.status(), StatusCode::FORBIDDEN);

        let mut admin_req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/pairing/telegram")
            .body(Body::empty())
            .expect("admin request");
        admin_req.extensions_mut().insert(UserIdentity {
            user_id: "admin-1".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let admin_resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, admin_req)
            .await
            .expect("admin response");
        assert_eq!(admin_resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(admin_resp.into_body(), 1024 * 64)
            .await
            .expect("admin body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("pairing list json");
        assert_eq!(
            parsed["channel"],
            serde_json::Value::String("telegram".to_string())
        );
        assert_eq!(parsed["requests"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            parsed["requests"][0]["sender_id"],
            serde_json::Value::String("tg-user-1".to_string())
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_pairing_approve_claims_code_for_authenticated_user() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (state, _db, pairing_store, _tmp) = make_pairing_test_state().await;
        let request = pairing_store
            .upsert_request("telegram", "tg-user-claim", None)
            .await
            .expect("create pairing request");

        let app = Router::new()
            .route(
                "/api/pairing/{channel}/approve",
                post(pairing_approve_handler),
            )
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/pairing/telegram/approve")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "code": request.code.to_ascii_lowercase() }).to_string(),
            ))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "member-1".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(parsed["success"], serde_json::Value::Bool(true));

        let identity = pairing_store
            .resolve_identity("telegram", "tg-user-claim")
            .await
            .expect("resolve identity")
            .expect("claimed identity");
        assert_eq!(identity.owner_id.as_str(), "member-1");
        assert!(
            pairing_store
                .list_pending("telegram")
                .await
                .expect("pending list")
                .is_empty()
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_pairing_approve_rejects_blank_code() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (state, _db, _pairing_store, _tmp) = make_pairing_test_state().await;
        let app = Router::new()
            .route(
                "/api/pairing/{channel}/approve",
                post(pairing_approve_handler),
            )
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/pairing/telegram/approve")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({ "code": "   " }).to_string()))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "member-1".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(parsed["success"], serde_json::Value::Bool(false));
        assert_eq!(
            parsed["message"],
            serde_json::Value::String("Pairing code is required.".to_string())
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_delete_user_evicts_auth_and_pairing_caches() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (db, _tmp) = crate::testing::test_db().await;
        insert_test_user(&db, "admin-1", "admin").await;
        insert_test_user(&db, "member-1", "member").await;

        let token = "member-token-123";
        let hash = crate::channels::web::auth::hash_token(token);
        db.create_api_token("member-1", "test-token", &hash, &token[..8], None) // safety: test-only, ASCII literal
            .await
            .expect("create api token");

        let db_auth = Arc::new(crate::channels::web::auth::DbAuthenticator::new(
            Arc::clone(&db),
        ));
        let pairing_store = Arc::new(crate::pairing::PairingStore::new(
            Arc::clone(&db),
            Arc::new(crate::ownership::OwnershipCache::new()),
        ));

        let auth_identity = db_auth
            .authenticate(token)
            .await
            .expect("db auth lookup")
            .expect("db auth identity");
        assert_eq!(auth_identity.user_id, "member-1");

        let request = pairing_store
            .upsert_request("telegram", "tg-delete-1", None)
            .await
            .expect("create pairing request");
        pairing_store
            .approve(
                "telegram",
                &request.code,
                &crate::ownership::OwnerId::from("member-1"),
            )
            .await
            .expect("approve pairing");
        assert!(
            pairing_store
                .resolve_identity("telegram", "tg-delete-1")
                .await
                .expect("prime pairing cache")
                .is_some()
        );

        let state = test_gateway_state_with_dependencies(
            None,
            Some(Arc::clone(&db)),
            Some(Arc::clone(&db_auth)),
            Some(Arc::clone(&pairing_store)),
        );
        let app = Router::new()
            .route(
                "/api/admin/users/{id}",
                axum::routing::delete(crate::channels::web::handlers::users::users_delete_handler),
            )
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("DELETE")
            .uri("/api/admin/users/member-1")
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "admin-1".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        assert!(
            db_auth
                .authenticate(token)
                .await
                .expect("post-delete auth lookup")
                .is_none()
        );
        assert!(
            pairing_store
                .resolve_identity("telegram", "tg-delete-1")
                .await
                .expect("post-delete pairing lookup")
                .is_none()
        );
    }

    #[derive(Clone, Debug)]
    struct RecordedOauthProxyRequest {
        authorization: Option<String>,
        form: std::collections::HashMap<String, String>,
    }

    #[derive(Clone)]
    struct MockOauthProxyState {
        requests: Arc<tokio::sync::Mutex<Vec<RecordedOauthProxyRequest>>>,
    }

    struct MockOauthProxyServer {
        addr: std::net::SocketAddr,
        requests: Arc<tokio::sync::Mutex<Vec<RecordedOauthProxyRequest>>>,
        shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
        server_task: Option<tokio::task::JoinHandle<()>>,
    }

    impl MockOauthProxyServer {
        async fn start() -> Self {
            async fn exchange_handler(
                State(state): State<MockOauthProxyState>,
                headers: axum::http::HeaderMap,
                axum::Form(form): axum::Form<std::collections::HashMap<String, String>>,
            ) -> Json<serde_json::Value> {
                state.requests.lock().await.push(RecordedOauthProxyRequest {
                    authorization: headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string),
                    form,
                });
                Json(serde_json::json!({
                    "access_token": "proxy-access-token",
                    "refresh_token": "proxy-refresh-token",
                    "expires_in": 7200
                }))
            }

            let requests = Arc::new(tokio::sync::Mutex::new(Vec::new()));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock oauth proxy");
            let addr = listener.local_addr().expect("mock oauth proxy addr");
            let app = Router::new()
                .route("/oauth/exchange", post(exchange_handler))
                .with_state(MockOauthProxyState {
                    requests: Arc::clone(&requests),
                });
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
            let server_task = tokio::spawn(async move {
                let _ = axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await;
            });

            Self {
                addr,
                requests,
                shutdown_tx: Some(shutdown_tx),
                server_task: Some(server_task),
            }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }

        async fn requests(&self) -> Vec<RecordedOauthProxyRequest> {
            self.requests.lock().await.clone()
        }

        async fn shutdown(mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(task) = self.server_task.take() {
                let _ = task.await;
            }
        }
    }

    impl Drop for MockOauthProxyServer {
        fn drop(&mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(task) = self.server_task.take() {
                task.abort();
            }
        }
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: Tests use lock_env() to serialize environment access.
            unsafe {
                if let Some(ref value) = self.original {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
        let original = std::env::var(key).ok();
        // SAFETY: Tests use lock_env() to serialize environment access.
        unsafe {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
        EnvVarGuard { key, original }
    }

    fn fresh_pending_oauth_flow(
        secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
        sse_manager: Option<Arc<SseManager>>,
        oauth_proxy_auth_token: Option<String>,
    ) -> crate::cli::oauth_defaults::PendingOAuthFlow {
        crate::cli::oauth_defaults::PendingOAuthFlow {
            extension_name: "test_tool".to_string(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: Some("test-code-verifier".to_string()),
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: Some("google".to_string()),
            validation_endpoint: None,
            scopes: vec!["email".to_string()],
            user_id: "test".to_string(),
            secrets,
            sse_manager,
            gateway_token: oauth_proxy_auth_token,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at: std::time::Instant::now(),
            auto_activate_extension: true,
        }
    }

    #[tokio::test]
    async fn test_extensions_setup_submit_returns_failure_when_not_activated() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, wasm_channels_dir) = test_ext_mgr(secrets);

        let channel_name = "test-failing-channel";
        std::fs::write(
            wasm_channels_dir
                .path()
                .join(format!("{channel_name}.wasm")),
            b"\0asm fake",
        )
        .expect("write fake wasm");
        let caps = serde_json::json!({
            "type": "channel",
            "name": channel_name,
            "setup": {
                "required_secrets": [
                    {"name": "BOT_TOKEN", "prompt": "Enter bot token"}
                ]
            }
        });
        std::fs::write(
            wasm_channels_dir
                .path()
                .join(format!("{channel_name}.capabilities.json")),
            serde_json::to_string(&caps).expect("serialize caps"),
        )
        .expect("write capabilities");

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route(
                "/api/extensions/{name}/setup",
                post(extensions_setup_submit_handler),
            )
            .with_state(state);

        let req_body = serde_json::json!({
            "secrets": {
                "BOT_TOKEN": "dummy-token"
            }
        });
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri(format!("/api/extensions/{channel_name}/setup"))
            .header("content-type", "application/json")
            .body(Body::from(req_body.to_string()))
            .expect("request");
        // Inject AuthenticatedUser so the handler's extractor succeeds
        // without needing the full auth middleware layer.
        req.extensions_mut().insert(UserIdentity {
            user_id: "test".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json response");
        assert_eq!(parsed["success"], serde_json::Value::Bool(false));
        assert_eq!(parsed["activated"], serde_json::Value::Bool(false));
        assert!(
            parsed["message"]
                .as_str()
                .unwrap_or_default()
                .contains("Activation failed"),
            "expected activation failure in message: {:?}",
            parsed
        );
    }

    fn expired_flow_created_at() -> Option<std::time::Instant> {
        std::time::Instant::now()
            .checked_sub(oauth_defaults::OAUTH_FLOW_EXPIRY + std::time::Duration::from_secs(1))
    }

    #[tokio::test]
    async fn test_csp_header_present_on_responses() {
        use std::net::SocketAddr;

        let state = test_gateway_state(None);

        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let auth = CombinedAuthState::from(crate::channels::web::auth::MultiAuthState::single(
            "test-token".to_string(),
            "test".to_string(),
        ));
        let bound = start_server(addr, state.clone(), auth)
            .await
            .expect("server should start");

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("http://{}/api/health", bound))
            .send()
            .await
            .expect("health request should succeed");

        assert_eq!(resp.status(), 200);

        let csp = resp
            .headers()
            .get("content-security-policy")
            .expect("CSP header must be present");

        let csp_str = csp.to_str().expect("CSP header should be valid UTF-8");
        assert!(
            csp_str.contains("default-src 'self'"),
            "CSP must contain default-src"
        );
        assert!(
            csp_str.contains(
                "script-src 'self' https://cdn.jsdelivr.net https://cdnjs.cloudflare.com https://esm.sh"
            ),
            "CSP must allow the explicit script CDNs without unsafe-inline"
        );
        assert!(
            csp_str.contains("object-src 'none'"),
            "CSP must contain object-src 'none'"
        );
        assert!(
            csp_str.contains("frame-ancestors 'none'"),
            "CSP must contain frame-ancestors 'none'"
        );

        if let Some(tx) = state.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }
    }

    #[tokio::test]
    async fn test_oauth_callback_missing_params() {
        use axum::body::Body;
        use tower::ServiceExt;

        let state = test_gateway_state(None);
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_error_from_provider() {
        use axum::body::Body;
        use tower::ServiceExt;

        let state = test_gateway_state(None);
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?error=access_denied&error_description=access_denied")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_unknown_state() {
        use axum::body::Body;
        use tower::ServiceExt;

        // Build an ExtensionManager so the handler can look up flows
        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets);

        let state = test_gateway_state(Some(ext_mgr));
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=test_code&state=unknown_state_value")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_expired_flow() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());
        let Some(created_at) = expired_flow_created_at() else {
            eprintln!("Skipping expired OAuth flow test: monotonic uptime below expiry window");
            return;
        };

        // Insert an expired flow.
        let flow = crate::cli::oauth_defaults::PendingOAuthFlow {
            extension_name: "test_tool".to_string(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: None,
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("expired_state".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr));
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=test_code&state=expired_state")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        // Expired flow → error landing page
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_expired_flow_broadcasts_auth_completed_failure() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let Some(created_at) = expired_flow_created_at() else {
            eprintln!("Skipping expired OAuth flow SSE test: monotonic uptime below expiry window");
            return;
        };
        let flow = crate::cli::oauth_defaults::PendingOAuthFlow {
            extension_name: "test_tool".to_string(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: Some(sse_mgr),
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("expired_state".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr));
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=test_code&state=expired_state")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        match receiver.recv().await.expect("auth_completed event").event {
            crate::channels::web::types::AppEvent::AuthCompleted {
                extension_name,
                success,
                message,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert!(!success, "expired OAuth flow should broadcast failure");
                assert_eq!(message, "OAuth flow expired. Please try again.");
            }
            event => panic!("expected AuthCompleted event, got {event:?}"),
        }
    }

    #[tokio::test]
    async fn test_oauth_callback_no_extension_manager() {
        use axum::body::Body;
        use tower::ServiceExt;

        // No extension manager set → graceful error
        let state = test_gateway_state(None);
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=test_code&state=some_state")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_strips_instance_prefix() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        // Insert a flow keyed by raw nonce "test_nonce" (without instance prefix).
        // Use an expired flow so the handler exits before attempting a real HTTP
        // token exchange — we only need to verify that the instance prefix was
        // stripped and the flow was found by the raw nonce.
        let Some(created_at) = expired_flow_created_at() else {
            eprintln!("Skipping OAuth state-prefix test: monotonic uptime below expiry window");
            return;
        };
        let flow = crate::cli::oauth_defaults::PendingOAuthFlow {
            extension_name: "test_tool".to_string(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: None,
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            // Expired — handler will reject after lookup (no network I/O)
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);

        // Send callback with instance prefix: "myinstance:test_nonce"
        // The handler should strip "myinstance:" and find the flow keyed by "test_nonce"
        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=fake_code&state=myinstance:test_nonce")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);

        // The flow was found (stripped prefix matched) but is expired, so the
        // handler returns an error landing page. The flow being consumed from
        // the registry (checked below) proves the prefix was stripped correctly.
        assert!(
            html.contains("Authorization Failed"),
            "Expected error page, html was: {}",
            &html[..html.len().min(500)]
        );

        // Verify the flow was consumed (removed from registry)
        assert!(
            ext_mgr
                .pending_oauth_flows()
                .read()
                .await
                .get("test_nonce")
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_oauth_callback_accepts_versioned_hosted_state() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        let Some(created_at) = expired_flow_created_at() else {
            eprintln!("Skipping versioned OAuth state test: monotonic uptime below expiry window");
            return;
        };
        let flow = crate::cli::oauth_defaults::PendingOAuthFlow {
            extension_name: "test_tool".to_string(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: None,
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::cli::oauth_defaults::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
        assert!(
            ext_mgr
                .pending_oauth_flows()
                .read()
                .await
                .get("test_nonce")
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_oauth_callback_accepts_versioned_hosted_state_without_instance_name() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        let Some(created_at) = expired_flow_created_at() else {
            eprintln!(
                "Skipping versioned OAuth state without instance test: monotonic uptime below expiry window"
            );
            return;
        };
        let flow = crate::cli::oauth_defaults::PendingOAuthFlow {
            extension_name: "test_tool".to_string(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: None,
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::cli::oauth_defaults::encode_hosted_oauth_state("test_nonce", None);

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
        assert!(
            ext_mgr
                .pending_oauth_flows()
                .read()
                .await
                .get("test_nonce")
                .is_none()
        );
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_oauth_callback_happy_path_with_gateway_token_fallback() {
        use axum::body::Body;
        use tower::ServiceExt;

        let proxy = MockOauthProxyServer::start().await;
        // Keep the process-wide env locked for the full callback so the handler
        // sees a stable proxy URL/token configuration throughout the test.
        let _env_guard = crate::config::helpers::lock_env();
        let _exchange_url_guard =
            set_env_var("IRONCLAW_OAUTH_EXCHANGE_URL", Some(&proxy.base_url()));
        let _proxy_auth_guard = set_env_var("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN", None);
        let _gateway_token_guard = set_env_var("GATEWAY_AUTH_TOKEN", Some("gateway-test-token"));

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(Arc::clone(&secrets));
        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let flow = fresh_pending_oauth_flow(
            Arc::clone(&secrets),
            Some(Arc::clone(&sse_mgr)),
            crate::cli::oauth_defaults::oauth_proxy_auth_token(),
        );

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::cli::oauth_defaults::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Test Tool Connected"));

        let requests = proxy.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer gateway-test-token")
        );
        assert_eq!(
            requests[0].form.get("code").map(String::as_str),
            Some("fake_code")
        );
        assert_eq!(
            requests[0].form.get("code_verifier").map(String::as_str),
            Some("test-code-verifier")
        );

        let access_token = secrets
            .get_decrypted("test", "test_token")
            .await
            .expect("access token stored");
        assert_eq!(access_token.expose(), "proxy-access-token");

        let refresh_token = secrets
            .get_decrypted("test", "test_token_refresh_token")
            .await
            .expect("refresh token stored");
        assert_eq!(refresh_token.expose(), "proxy-refresh-token");

        match receiver.recv().await.expect("auth_completed event").event {
            crate::channels::web::types::AppEvent::AuthCompleted {
                extension_name,
                success,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert!(success, "OAuth callback should broadcast success");
            }
            event => panic!("expected AuthCompleted event, got {event:?}"),
        }

        proxy.shutdown().await;
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_oauth_callback_happy_path_with_dedicated_proxy_auth_token() {
        use axum::body::Body;
        use tower::ServiceExt;

        let proxy = MockOauthProxyServer::start().await;
        // Keep the process-wide env locked for the full callback so the handler
        // sees a stable proxy URL/token configuration throughout the test.
        let _env_guard = crate::config::helpers::lock_env();
        let _exchange_url_guard =
            set_env_var("IRONCLAW_OAUTH_EXCHANGE_URL", Some(&proxy.base_url()));
        let _proxy_auth_guard = set_env_var(
            "IRONCLAW_OAUTH_PROXY_AUTH_TOKEN",
            Some("shared-oauth-proxy-secret"),
        );
        let _gateway_token_guard = set_env_var("GATEWAY_AUTH_TOKEN", None);

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(Arc::clone(&secrets));
        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let flow = fresh_pending_oauth_flow(
            Arc::clone(&secrets),
            Some(Arc::clone(&sse_mgr)),
            crate::cli::oauth_defaults::oauth_proxy_auth_token(),
        );

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::cli::oauth_defaults::encode_hosted_oauth_state("test_nonce", None);

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Test Tool Connected"));

        let requests = proxy.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer shared-oauth-proxy-secret")
        );
        assert_eq!(
            requests[0].form.get("code").map(String::as_str),
            Some("fake_code")
        );
        assert_eq!(
            requests[0].form.get("code_verifier").map(String::as_str),
            Some("test-code-verifier")
        );

        let access_token = secrets
            .get_decrypted("test", "test_token")
            .await
            .expect("access token stored");
        assert_eq!(access_token.expose(), "proxy-access-token");

        let refresh_token = secrets
            .get_decrypted("test", "test_token_refresh_token")
            .await
            .expect("refresh token stored");
        assert_eq!(refresh_token.expose(), "proxy-refresh-token");

        match receiver.recv().await.expect("auth_completed event").event {
            crate::channels::web::types::AppEvent::AuthCompleted {
                extension_name,
                success,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert!(success, "OAuth callback should broadcast success");
            }
            event => panic!("expected AuthCompleted event, got {event:?}"),
        }

        proxy.shutdown().await;
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_oauth_callback_happy_path_without_auto_activation() {
        use axum::body::Body;
        use tower::ServiceExt;

        let proxy = MockOauthProxyServer::start().await;
        let _env_guard = crate::config::helpers::lock_env();
        let _exchange_url_guard =
            set_env_var("IRONCLAW_OAUTH_EXCHANGE_URL", Some(&proxy.base_url()));
        let _proxy_auth_guard = set_env_var("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN", None);
        let _gateway_token_guard = set_env_var("GATEWAY_AUTH_TOKEN", Some("gateway-test-token"));

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(Arc::clone(&secrets));
        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let mut flow = fresh_pending_oauth_flow(
            Arc::clone(&secrets),
            Some(Arc::clone(&sse_mgr)),
            crate::cli::oauth_defaults::oauth_proxy_auth_token(),
        );
        flow.auto_activate_extension = false;

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::cli::oauth_defaults::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        match receiver.recv().await.expect("auth_completed event").event {
            crate::channels::web::types::AppEvent::AuthCompleted {
                extension_name,
                success,
                message,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert!(success, "OAuth callback should broadcast success");
                assert_eq!(message, "Test Tool authenticated successfully");
            }
            event => panic!("expected AuthCompleted event, got {event:?}"),
        }

        proxy.shutdown().await;
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_oauth_callback_exchange_failure_broadcasts_auth_completed_failure() {
        use axum::body::Body;
        use tower::ServiceExt;

        let _env_guard = crate::config::helpers::lock_env();
        let _exchange_url_guard =
            set_env_var("IRONCLAW_OAUTH_EXCHANGE_URL", Some("http://127.0.0.1:1"));
        let _proxy_auth_guard = set_env_var("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN", None);
        let _gateway_token_guard = set_env_var("GATEWAY_AUTH_TOKEN", Some("gateway-test-token"));

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(Arc::clone(&secrets));
        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let flow = fresh_pending_oauth_flow(
            Arc::clone(&secrets),
            Some(Arc::clone(&sse_mgr)),
            crate::cli::oauth_defaults::oauth_proxy_auth_token(),
        );

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::cli::oauth_defaults::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        match receiver.recv().await.expect("auth_completed event").event {
            crate::channels::web::types::AppEvent::AuthCompleted {
                extension_name,
                success,
                message,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert!(!success, "OAuth callback failure should broadcast failure");
                assert!(message.contains("authentication failed"));
            }
            event => panic!("expected AuthCompleted event, got {event:?}"),
        }
    }

    // --- Slack relay OAuth CSRF tests ---

    fn test_relay_oauth_router(state: Arc<GatewayState>) -> Router {
        Router::new()
            .route(
                "/oauth/slack/callback",
                get(slack_relay_oauth_callback_handler),
            )
            .with_state(state)
    }

    fn test_secrets_store() -> Arc<dyn crate::secrets::SecretsStore + Send + Sync> {
        Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
            crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                "test-key-at-least-32-chars-long!!".to_string(),
            ))
            .expect("crypto"),
        )))
    }

    fn test_ext_mgr(
        secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
    ) -> (Arc<ExtensionManager>, tempfile::TempDir, tempfile::TempDir) {
        let tool_registry = Arc::new(ToolRegistry::new());
        let mcp_sm = Arc::new(crate::tools::mcp::session::McpSessionManager::new());
        let mcp_pm = Arc::new(crate::tools::mcp::process::McpProcessManager::new());
        let wasm_tools_dir = tempfile::tempdir().expect("temp wasm tools dir");
        let wasm_channels_dir = tempfile::tempdir().expect("temp wasm channels dir");
        let ext_mgr = Arc::new(ExtensionManager::new(
            mcp_sm,
            mcp_pm,
            secrets,
            tool_registry,
            None,
            None,
            wasm_tools_dir.path().to_path_buf(),
            wasm_channels_dir.path().to_path_buf(),
            None,
            "test".to_string(),
            None,
            vec![],
        ));
        (ext_mgr, wasm_tools_dir, wasm_channels_dir)
    }

    #[tokio::test]
    async fn test_relay_oauth_callback_missing_state_param() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets);
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        // Callback without state param should be rejected
        let req = axum::http::Request::builder()
            .uri("/oauth/slack/callback?team_id=T123&provider=slack")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("Invalid or expired authorization"),
            "Expected CSRF error, got: {}",
            &html[..html.len().min(300)]
        );
    }

    #[tokio::test]
    async fn test_relay_oauth_callback_wrong_state_param() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();

        // Store a valid nonce
        secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams::new(
                    format!("relay:{}:oauth_state", DEFAULT_RELAY_NAME),
                    "correct-nonce-value",
                ),
            )
            .await
            .expect("store nonce");

        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets);
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        // Callback with wrong state param
        let req = axum::http::Request::builder()
            .uri("/oauth/slack/callback?team_id=T123&provider=slack&state=wrong-nonce")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("Invalid or expired authorization"),
            "Expected CSRF error for wrong nonce, got: {}",
            &html[..html.len().min(300)]
        );
    }

    #[tokio::test]
    async fn test_relay_oauth_callback_correct_state_proceeds() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let nonce = "valid-test-nonce-12345";

        // Store the correct nonce
        secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams::new(
                    format!("relay:{}:oauth_state", DEFAULT_RELAY_NAME),
                    nonce,
                ),
            )
            .await
            .expect("store nonce");

        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        // Callback with correct state param — will pass CSRF check
        // but may fail downstream (no real relay service) — that's OK,
        // we just verify it doesn't return a CSRF error.
        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/slack/callback?team_id=T123&provider=slack&state={}",
                nonce
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        // Should NOT contain the CSRF error message
        assert!(
            !html.contains("Invalid or expired authorization"),
            "Should have passed CSRF check, got: {}",
            &html[..html.len().min(300)]
        );

        // Verify the nonce was consumed (deleted)
        let state_key = format!("relay:{}:oauth_state", DEFAULT_RELAY_NAME);
        let exists = secrets.exists("test", &state_key).await.unwrap_or(true);
        assert!(!exists, "CSRF nonce should be deleted after use");
    }

    #[test]
    fn test_is_local_origin_localhost() {
        assert!(is_local_origin("http://localhost:3001"));
        assert!(is_local_origin("http://localhost"));
        assert!(is_local_origin("https://localhost:3001"));
    }

    #[test]
    fn test_is_local_origin_ipv4() {
        assert!(is_local_origin("http://127.0.0.1:3001"));
        assert!(is_local_origin("http://127.0.0.1"));
    }

    #[test]
    fn test_is_local_origin_ipv6() {
        assert!(is_local_origin("http://[::1]:3001"));
        assert!(is_local_origin("http://[::1]"));
    }

    #[test]
    fn test_is_local_origin_rejects_remote() {
        assert!(!is_local_origin("http://evil.com"));
        assert!(!is_local_origin("http://localhost.evil.com"));
        assert!(!is_local_origin("http://192.168.1.1:3001"));
    }

    #[test]
    fn test_is_local_origin_rejects_garbage() {
        assert!(!is_local_origin("not-a-url"));
        assert!(!is_local_origin(""));
    }
}
