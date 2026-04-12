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
        IntoResponse, Response,
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
use crate::channels::web::handlers::frontend::{
    frontend_layout_handler, frontend_layout_update_handler, frontend_widget_file_handler,
    frontend_widgets_handler,
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
    /// Cached admin system prompt content. `None` = not yet loaded;
    /// `Some("")` = loaded but empty/not set.
    admin_prompt_cache: Arc<tokio::sync::RwLock<Option<String>>>,
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
            admin_prompt_cache: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Clear the admin prompt cache. Called after the PUT handler updates
    /// the prompt so all workspaces see the new content on the next turn.
    pub async fn invalidate_admin_prompt(&self) {
        let mut guard = self.admin_prompt_cache.write().await;
        *guard = None;
    }

    /// Build a workspace for a user, applying search config, embeddings,
    /// global read scopes, memory layers, and admin prompt.
    fn build_workspace(&self, user_id: &str) -> Workspace {
        let mut ws = Workspace::new_with_db(user_id, Arc::clone(&self.db))
            .with_search_config(&self.search_config)
            .with_admin_prompt()
            .with_admin_prompt_cache(Arc::clone(&self.admin_prompt_cache));

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
    /// Shared auth manager for gateway auth submission and readiness checks.
    pub auth_manager: Option<Arc<crate::bridge::auth_manager::AuthManager>>,
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
    /// Cache for the assembled frontend HTML served from `/`.
    ///
    /// The cache key is derived from the `updated_at` of
    /// `.system/gateway/layout.json` and the `.system/gateway/widgets/`
    /// directory — both returned by a single cheap `list(".system/gateway/")`
    /// call. A hit skips reading the layout, every widget manifest, every
    /// widget JS file, and every widget CSS file. A miss (or absent cache)
    /// falls through to the full `build_frontend_html()` path.
    pub frontend_html_cache: Arc<tokio::sync::RwLock<Option<FrontendHtmlCache>>>,
    /// Channel-agnostic tool dispatcher for routing handler operations through
    /// the tool pipeline with audit trail.
    pub tool_dispatcher: Option<Arc<crate::tools::dispatch::ToolDispatcher>>,
}

/// Cached result of `build_frontend_html()`, keyed by a cheap workspace
/// signature so the fast path only needs one `list()` call per request.
#[derive(Debug, Clone)]
pub struct FrontendHtmlCache {
    /// Signature the cache is valid for. The cache is bypassed when the
    /// current workspace signature differs from this one.
    pub key: FrontendCacheKey,
    /// The assembled HTML, or `None` if the layout had no customizations
    /// and the caller should serve the embedded default unchanged.
    pub html: Option<String>,
}

/// Cheap workspace fingerprint covering the inputs of `build_frontend_html`.
///
/// Uses the per-entry `updated_at` timestamps returned by `Workspace::list`
/// (the directory entry's `updated_at` is "latest among children", so widget
/// file edits bubble up automatically). Timestamps are stored as
/// `(seconds, nanoseconds)` pairs to avoid depending on `chrono` types here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendCacheKey {
    /// Signature for `.system/gateway/layout.json`, or `None` if absent.
    pub layout: Option<(i64, u32)>,
    /// Signature for `.system/gateway/widgets/` (max child mtime), or `None`
    /// if the directory is empty or absent.
    pub widgets: Option<(i64, u32)>,
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
        .route(
            "/api/extensions/readiness",
            get(extensions_readiness_handler),
        )
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
        // Admin tool policy
        .route(
            "/api/admin/tool-policy",
            get(super::handlers::tool_policy::tool_policy_get_handler)
                .put(super::handlers::tool_policy::tool_policy_put_handler),
        )
        // Admin system prompt — tighter body cap than the global 10 MB so an
        // oversized payload is rejected before being parsed into memory.
        .route(
            "/api/admin/system-prompt",
            get(super::handlers::system_prompt::get_handler)
                .put(super::handlers::system_prompt::put_handler)
                .layer(DefaultBodyLimit::max(128 * 1024)),
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
        // Frontend extension API
        .route(
            "/api/frontend/layout",
            get(frontend_layout_handler).put(frontend_layout_update_handler),
        )
        .route("/api/frontend/widgets", get(frontend_widgets_handler))
        .route(
            "/api/frontend/widget/{id}/{*file}",
            get(frontend_widget_file_handler),
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
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024)) // 10 MB max request body (inline attachments)
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
            BASE_CSP_HEADER.clone(),
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

// --- Content Security Policy ---
//
// A single source of truth for the gateway's CSP. The static value below is
// used by the global response-header layer for every endpoint. The
// ---- Content-Security-Policy construction ----------------------------
//
// The gateway serves two flavors of CSP on the same set of directives:
//
// * The static header applied by `SetResponseHeaderLayer` to *every*
//   response (see [`BASE_CSP_HEADER`]). No inline scripts are authorized.
// * A per-response variant produced by [`build_csp`] with a `'nonce-…'`
//   source added to `script-src`, used only by `index_handler` when it
//   serves customized HTML containing inline `<script>` blocks.
//
// Both variants MUST carry the same directive set except for `script-src`
// — if one grows a new `connect-src` origin, the other silently stays on
// the old policy, and customized pages end up under a stricter CSP than
// plain pages (or vice versa). Previous versions of this file duplicated
// the full directive string in two places, so adding a CDN to one was a
// latent regression waiting to happen. Keep every directive as a named
// constant and assemble both flavors via [`build_csp`] so there is a
// single source of truth.

/// `script-src` sources other than `'self'` and the per-response nonce.
const SCRIPT_SRC_EXTRAS: &str =
    "https://cdn.jsdelivr.net https://cdnjs.cloudflare.com https://esm.sh";
const STYLE_SRC: &str = "'self' 'unsafe-inline' https://fonts.googleapis.com";
const FONT_SRC: &str = "https://fonts.gstatic.com data:";
const CONNECT_SRC: &str =
    "'self' https://esm.sh https://rpc.mainnet.near.org https://rpc.testnet.near.org";
const IMG_SRC: &str =
    "'self' data: blob: https://*.googleusercontent.com https://avatars.githubusercontent.com";
const FRAME_SRC: &str = "https://accounts.google.com https://appleid.apple.com";
const FORM_ACTION: &str =
    "'self' https://accounts.google.com https://github.com https://appleid.apple.com";

/// Build a CSP string. When `nonce` is `Some`, the resulting policy adds
/// `'nonce-{nonce}'` to `script-src` so a single inline `<script
/// nonce="{nonce}">` block on the same response is authorized. When
/// `nonce` is `None`, the policy matches the static header emitted by
/// [`BASE_CSP_HEADER`]. This is the single source of truth for the
/// gateway CSP — edit per-directive constants above, not the format
/// string here.
fn build_csp(nonce: Option<&str>) -> String {
    let script_nonce = match nonce {
        Some(n) => format!(" 'nonce-{n}'"),
        None => String::new(),
    };
    format!(
        "default-src 'self'; \
         script-src 'self'{script_nonce} {SCRIPT_SRC_EXTRAS}; \
         style-src {STYLE_SRC}; \
         font-src {FONT_SRC}; \
         connect-src {CONNECT_SRC}; \
         img-src {IMG_SRC}; \
         frame-src {FRAME_SRC}; \
         object-src 'none'; \
         frame-ancestors 'none'; \
         base-uri 'self'; \
         form-action {FORM_ACTION}"
    )
}

/// Static CSP header applied to every gateway response by the
/// response-header layer. Assembled at first use via [`build_csp`] with no
/// nonce. Falls back to a minimally-permissive `default-src 'self'` if the
/// assembled value somehow fails to parse as a `HeaderValue` — in practice
/// the assembled string is pure ASCII and this branch is unreachable, but
/// production code in this repo doesn't use `.expect()` on request-path
/// values.
static BASE_CSP_HEADER: std::sync::LazyLock<header::HeaderValue> = std::sync::LazyLock::new(|| {
    header::HeaderValue::from_str(&build_csp(None))
        .unwrap_or_else(|_| header::HeaderValue::from_static("default-src 'self'"))
});

/// Build a CSP equivalent to the static header but with `'nonce-{nonce}'`
/// added to the `script-src` directive. Thin wrapper kept for call-site
/// readability (the name is the contract the nonce handler wants).
fn build_csp_with_nonce(nonce: &str) -> String {
    build_csp(Some(nonce))
}

/// Generate a fresh per-response CSP nonce. 16 random bytes hex-encoded
/// (32 chars) — well above the 128-bit minimum recommended for nonces and
/// matching the `OsRng + hex` pattern used elsewhere in this module
/// (see `tokens_create_handler`).
fn generate_csp_nonce() -> String {
    use rand::RngCore;
    use rand::rngs::OsRng;
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

// --- Frontend bundle assembly ---

use ironclaw_gateway::assets;
use ironclaw_gateway::{FrontendBundle, LayoutConfig, NONCE_PLACEHOLDER};

use crate::channels::web::handlers::frontend::{load_resolved_widgets, read_layout_config};

/// Compute a cheap cache key for `build_frontend_html` — one `list` call
/// against `.system/gateway/`. The directory entry for `widgets/` carries the
/// max `updated_at` of its children, so any widget file edit naturally bubbles
/// into the key without needing to read individual manifests.
async fn compute_frontend_cache_key(workspace: &crate::workspace::Workspace) -> FrontendCacheKey {
    let Ok(entries) = workspace.list(".system/gateway/").await else {
        return FrontendCacheKey {
            layout: None,
            widgets: None,
        };
    };
    let mut key = FrontendCacheKey {
        layout: None,
        widgets: None,
    };
    for entry in entries {
        let ts = entry
            .updated_at
            .map(|t| (t.timestamp(), t.timestamp_subsec_nanos()));
        match entry.name() {
            "layout.json" if !entry.is_directory => key.layout = ts,
            "widgets" if entry.is_directory => key.widgets = ts,
            _ => {}
        }
    }
    key
}

/// Build customized HTML from the workspace gateway config.
///
/// Returns `None` if the workspace is unavailable or the loaded layout has no
/// customizations and no widgets — in that case the caller serves the embedded
/// default HTML unchanged. Custom CSS is deliberately **not** included in the
/// returned bundle: `css_handler` appends `.system/gateway/custom.css` onto
/// `/style.css` so the stylesheet is the single source of truth for CSS
/// overrides.
///
/// The assembled HTML is cached in `GatewayState::frontend_html_cache` behind
/// a fingerprint of `.system/gateway/layout.json` and `.system/gateway/widgets/`
/// mtimes (computed with a single `list()` call). A cache hit skips reading
/// every widget manifest / JS / CSS file, which would otherwise fire on every
/// page load.
///
/// **Multi-tenant safety.** In multi-user mode (`workspace_pool` set) this
/// function ALWAYS returns `None`, regardless of whether `state.workspace` is
/// also populated. The customization assembly path is fundamentally
/// single-tenant: `index_handler` (`GET /`) is the unauthenticated bootstrap
/// route — no user identity is available at request time, so there is no way
/// to resolve the *correct* per-user workspace inside this function. Reading
/// `state.workspace` instead would expose one global workspace's
/// customizations to every user, and the process-wide
/// `frontend_html_cache` would pin the leak across requests. We refuse the
/// path entirely and serve the embedded default to all users; per-user
/// customization can ride a future JS-side fetch against
/// `/api/frontend/layout`, which is authenticated and routes through
/// `resolve_workspace(&state, &user)` so it returns the right workspace.
/// See `crates/ironclaw_gateway/static/app.js` — the layout-config IIFE
/// already reads `window.__IRONCLAW_LAYOUT__`, which a future change can
/// populate from a `fetch('/api/frontend/layout')` after auth.
///
/// **Cache key TOCTOU window (known and accepted).** The fast-path cache
/// key is computed by [`compute_frontend_cache_key`] in a single
/// `Workspace::list` call, but the slow-path data read
/// (`read_layout_config` + `load_resolved_widgets`) happens *after* that
/// key is observed, in separate workspace operations. A workspace write
/// landing between the two — operator edits `layout.json` while a
/// request is mid-rebuild — can therefore produce a cache entry whose
/// HTML was assembled from a layout *newer* than the key it's stored
/// under. The next request after the writes settle will recompute the
/// key, see a different fingerprint, and replace the cache entry, so
/// the staleness window is always self-correcting and bounded by one
/// rebuild round-trip.
///
/// This is intentional. Making the read+key+store sequence atomic would
/// require a workspace-level read lock that the rest of the gateway
/// doesn't take, and would punish the (much hotter) cache hit path with
/// extra coordination. The acceptability rests on three observations:
/// (a) the staleness window is bounded by a single `list()` call's
/// worth of wall time, (b) the cache is per-process so the staleness
/// can never outlive `Drop` of `GatewayState`, and (c) layout writes
/// are rare and operator-initiated — there is no realistic workload
/// that fires a write at the cadence required to keep the entry
/// permanently stale. If a future workload changes that calculus, the
/// right fix is a workspace version generation counter, not a lock
/// around this function.
async fn build_frontend_html(state: &GatewayState) -> Option<String> {
    if state.workspace_pool.is_some() {
        // Multi-tenant: refuse the assembly path entirely. See the function
        // doc comment above for the full rationale. The cache write below
        // is unreachable on this branch, so the cache stays empty and
        // cannot leak one user's customizations to another.
        return None;
    }

    let ws = state.workspace.as_ref()?;

    // Fast path — cache hit. One workspace `list()` call, no file reads.
    let cache_key = compute_frontend_cache_key(ws).await;
    {
        let cache = state.frontend_html_cache.read().await;
        if let Some(ref cached) = *cache
            && cached.key == cache_key
        {
            return cached.html.clone();
        }
    }

    // Slow path — rebuild.
    let layout = read_layout_config(ws).await;
    let widgets = load_resolved_widgets(ws, &layout).await;

    // Skip assembly when nothing is customized. `layout_has_customizations`
    // is the single source of truth so adding a new field to `LayoutConfig`
    // forces an update in one place instead of a big boolean expression here.
    let html = if widgets.is_empty() && !layout_has_customizations(&layout) {
        None
    } else {
        let bundle = FrontendBundle {
            layout,
            widgets,
            // Custom CSS is served via /style.css (css_handler) to avoid
            // double-application — see the doc comment on this function.
            custom_css: None,
        };
        Some(ironclaw_gateway::assemble_index(
            assets::INDEX_HTML,
            &bundle,
        ))
    };

    // Store in cache. If another request raced us here, either writer wins —
    // both produced the same HTML for the same key, so the cache ends up
    // consistent either way.
    *state.frontend_html_cache.write().await = Some(FrontendHtmlCache {
        key: cache_key,
        html: html.clone(),
    });

    html
}

/// Returns `true` if the layout config has any field that would affect the
/// rendered HTML. When this returns `false` and there are no widgets, the
/// gateway serves the embedded default unchanged.
fn layout_has_customizations(layout: &LayoutConfig) -> bool {
    let b = &layout.branding;
    let t = &layout.tabs;
    let c = &layout.chat;
    // `branding.colors` is opaque to this function — `BrandingColors` may
    // exist as `Some({})` (both fields `None`) or with values that the
    // `is_safe_css_color` validator strips at injection time. Treating
    // bare `colors.is_some()` as a customization forces the customized
    // HTML path (and the per-response nonce CSP that comes with it) for
    // layouts that produce zero effective branding output. Require at
    // least one trimmed-non-empty color field, mirroring what
    // `to_css_vars` actually emits.
    let has_branding_colors = b.colors.as_ref().is_some_and(|colors| {
        let nonempty = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.trim().is_empty());
        nonempty(&colors.primary) || nonempty(&colors.accent)
    });
    // Same precedent for URL fields: route through the `safe_logo_url`
    // / `safe_favicon_url` getters that apply `is_safe_url`. A
    // `layout.json` with `logo_url: "javascript:alert(1)"` would
    // otherwise force the customized HTML path even though the value
    // gets dropped at consumer time. Symmetric with how branding colors
    // are gated above.
    b.title.is_some()
        || b.subtitle.is_some()
        || b.safe_logo_url().is_some()
        || b.safe_favicon_url().is_some()
        || has_branding_colors
        || t.order.is_some()
        || t.hidden.is_some()
        || t.default_tab.is_some()
        || c.suggestions.is_some()
        || c.image_upload.is_some()
        || c.upgrade_inline_json.is_some()
        || !layout.widgets.is_empty()
}

// --- Static file handlers ---
//
// All frontend assets are embedded in the `ironclaw_gateway` crate.
// These handlers serve them with appropriate MIME types and cache headers.

/// Substitute [`NONCE_PLACEHOLDER`] sentinels in the assembled HTML with a
/// fresh per-response CSP nonce.
///
/// **Why an attribute-targeted replace, not a bare string replace.** The
/// assembled HTML embeds widget JavaScript inline (so a CSP-protected
/// `<script src>` doesn't need to authenticate against `/api/frontend/widget/...`).
/// A widget author has every right to write the literal string
/// `__IRONCLAW_CSP_NONCE__` inside their own source — in a comment, a log
/// line, a test fixture, or just as a constant they happen to define. A
/// naive `html.replace(NONCE_PLACEHOLDER, nonce)` would silently rewrite
/// every such occurrence into a per-request nonce, mutating widget code
/// in a way the author didn't ask for.
///
/// The substitution here targets the full attribute form
/// `nonce="__IRONCLAW_CSP_NONCE__"`, which is the exact shape
/// `assemble_index` emits when stamping nonces onto `<script>` tags. The
/// double-quoted sentinel is unambiguous in HTML context — it can never
/// accidentally match free text in a JS module body, a comment, or a
/// JSON payload. Inline `<style>` blocks deliberately get no nonce
/// (style-src allows `'unsafe-inline'`) so they're untouched either way.
fn stamp_nonce_into_html(html_with_placeholder: &str, nonce: &str) -> String {
    let placeholder_attr = format!("nonce=\"{NONCE_PLACEHOLDER}\"");
    let nonce_attr = format!("nonce=\"{nonce}\"");
    html_with_placeholder.replace(&placeholder_attr, &nonce_attr)
}

async fn index_handler(State(state): State<Arc<GatewayState>>) -> Response {
    // Try to assemble customized HTML from workspace frontend config.
    // Falls back to embedded HTML if workspace is unavailable or has no
    // customizations — in that case there are no inline scripts and the
    // global CSP layer applies unchanged.
    let assembled = build_frontend_html(&state).await;

    let Some(html_with_placeholder) = assembled else {
        return (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            assets::INDEX_HTML,
        )
            .into_response();
    };

    // Customized path: the assembled HTML contains inline `<script>` blocks
    // (layout config + widget modules) carrying [`NONCE_PLACEHOLDER`] in
    // their `nonce` attribute. Stamp a fresh per-response nonce in both
    // the HTML and the response's Content-Security-Policy header so the
    // browser actually executes the scripts.
    //
    // Setting `Content-Security-Policy` here suppresses the global
    // `SetResponseHeaderLayer::if_not_present` value for this response only.
    let nonce = generate_csp_nonce();
    let html = stamp_nonce_into_html(&html_with_placeholder, &nonce);
    let csp = build_csp_with_nonce(&nonce);

    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
            (header::CACHE_CONTROL, "no-cache".to_string()),
            (
                header::HeaderName::from_static("content-security-policy"),
                csp,
            ),
        ],
        html,
    )
        .into_response()
}

/// Compute the strong ETag value for a CSS body.
///
/// Strong validators are quoted, sha-prefixed, and truncated to 16 hex chars
/// (64 bits) — collisions are statistically irrelevant for cache validation
/// and the short form keeps headers compact. The same scheme is used for
/// both the embedded base stylesheet and the workspace-customized variant
/// so a flip between the two flavors naturally invalidates the client's
/// cached copy.
fn css_etag(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    let hex = hex::encode(digest);
    // 16 hex chars = 64 bits, plenty for content addressing.
    format!("\"sha256-{}\"", &hex[..16])
}

async fn css_handler(State(state): State<Arc<GatewayState>>, headers: HeaderMap) -> Response {
    // Append custom CSS from `.system/gateway/custom.css` if it exists.
    //
    // The hot path (no workspace overlay) borrows `assets::STYLE_CSS` directly
    // via `Cow::Borrowed` so we don't allocate / copy the entire embedded
    // stylesheet on every request. We only fall through to an owned
    // `format!` when there's actually content to append.
    //
    // **Multi-tenant safety.** This must mirror the same guard
    // `build_frontend_html` already enforces (see its doc comment): in
    // multi-user mode (`workspace_pool.is_some()`) we cannot resolve a
    // per-user workspace because `/style.css` is the unauthenticated
    // bootstrap stylesheet — there is no user identity at request time.
    // Reading from `state.workspace` here would expose one global
    // workspace's `custom.css` to every user, defeating the
    // `index_handler` guard at the sibling endpoint. Refuse the overlay
    // path entirely in multi-tenant mode and serve the embedded base
    // stylesheet to all users; per-user CSS overrides can ride a future
    // authenticated `/api/frontend/custom-css` endpoint.
    let css: std::borrow::Cow<'static, str> = if state.workspace_pool.is_some() {
        std::borrow::Cow::Borrowed(assets::STYLE_CSS)
    } else {
        match &state.workspace {
            Some(ws) => match ws.read(".system/gateway/custom.css").await {
                Ok(doc) if !doc.content.trim().is_empty() => std::borrow::Cow::Owned(format!(
                    "{}\n/* --- custom overrides --- */\n{}",
                    assets::STYLE_CSS,
                    doc.content
                )),
                _ => std::borrow::Cow::Borrowed(assets::STYLE_CSS),
            },
            None => std::borrow::Cow::Borrowed(assets::STYLE_CSS),
        }
    };

    // Strong validator over the assembled body. The cache key naturally
    // tracks both base stylesheet edits (compile-time) and `custom.css`
    // edits (workspace mutation) — operators no longer need to ask users
    // to hard-refresh after tweaking branding.
    let etag = css_etag(&css);

    // Conditional GET: if the client already holds this exact body, send a
    // 304 with no body and let the browser reuse its cached copy. RFC 9110
    // §13.1.2 — `If-None-Match` is a list of validators; we accept either
    // an exact match or the literal `*`. Anything else falls through to a
    // full 200 response.
    if let Some(value) = headers.get(header::IF_NONE_MATCH)
        && let Ok(s) = value.to_str()
        && s.split(',').any(|v| {
            let v = v.trim();
            v == "*" || v == etag
        })
    {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag.as_str()),
                (header::CACHE_CONTROL, "no-cache"),
            ],
        )
            .into_response();
    }

    (
        [
            (header::CONTENT_TYPE, "text/css".to_string()),
            // Keep `no-cache` so the browser always revalidates — combined
            // with the ETag this gives us "fast 304" semantics rather than
            // a stale `max-age` window where operator edits don't show up.
            (header::CACHE_CONTROL, "no-cache".to_string()),
            (header::ETAG, etag),
        ],
        css,
    )
        .into_response()
}

async fn js_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::APP_JS,
    )
}

async fn theme_init_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::THEME_INIT_JS,
    )
}

async fn favicon_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/x-icon"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        assets::FAVICON_ICO,
    )
}

async fn i18n_index_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::I18N_INDEX_JS,
    )
}

async fn i18n_en_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::I18N_EN_JS,
    )
}

async fn i18n_zh_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::I18N_ZH_CN_JS,
    )
}

async fn i18n_ko_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::I18N_KO_JS,
    )
}

async fn i18n_app_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        assets::I18N_APP_JS,
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
    let html = crate::auth::oauth::landing_html(label, false);
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
    use crate::auth::oauth;

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

    let decoded_state = match oauth::decode_hosted_oauth_state(&state_param) {
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
    if flow.created_at.elapsed() > oauth::OAUTH_FLOW_EXPIRY {
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
    let exchange_proxy_url = oauth::exchange_proxy_url();

    let result: Result<(), String> = async {
        let token_response = if let Some(proxy_url) = &exchange_proxy_url {
            let oauth_proxy_auth_token = flow.oauth_proxy_auth_token().unwrap_or_default();
            oauth::exchange_via_proxy(oauth::ProxyTokenExchangeRequest {
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
            oauth::exchange_oauth_code_with_params(
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
            oauth::validate_oauth_token(&token_response.access_token, validation)
                .await
                .map_err(|e| e.to_string())?;
        }

        // Store tokens encrypted in the secrets store
        oauth::store_oauth_tokens(
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
        match ext_mgr
            .ensure_extension_ready(
                &flow.extension_name,
                &flow.user_id,
                crate::extensions::EnsureReadyIntent::ExplicitActivate,
            )
            .await
        {
            Ok(crate::extensions::EnsureReadyOutcome::Ready { activation, .. }) => activation
                .map(|result| result.message)
                .unwrap_or_else(|| format!("{} authenticated successfully", flow.display_name)),
            Ok(crate::extensions::EnsureReadyOutcome::NeedsAuth { auth, .. }) => auth
                .instructions()
                .map(String::from)
                .unwrap_or_else(|| format!("{} authenticated successfully", flow.display_name)),
            Ok(crate::extensions::EnsureReadyOutcome::NeedsSetup { instructions, .. }) => {
                instructions
            }
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

    if success {
        match crate::bridge::resolve_engine_auth_callback(&flow.user_id, &flow.secret_name).await {
            Ok(crate::bridge::AuthCallbackContinuation::ResolveGateExternal {
                channel,
                thread_scope,
                request_id,
            }) => {
                if let Some(tx) = state.msg_tx.read().await.as_ref().cloned() {
                    let callback =
                        crate::agent::submission::Submission::ExternalCallback { request_id };
                    match serde_json::to_string(&callback) {
                        Ok(content) => {
                            let mut msg = IncomingMessage::new(&channel, &flow.user_id, content);
                            if let Some(thread_id) = thread_scope {
                                msg = msg.with_thread(thread_id);
                            }
                            if let Err(e) = tx.send(msg).await {
                                tracing::warn!(
                                    extension = %extension_name,
                                    user_id = %flow.user_id,
                                    error = %e,
                                    "Failed to resolve pending engine auth gate after OAuth callback"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                extension = %extension_name,
                                user_id = %flow.user_id,
                                error = %e,
                                "Failed to serialize external callback submission"
                            );
                        }
                    }
                }
            }
            Ok(crate::bridge::AuthCallbackContinuation::ReplayMessage {
                channel,
                thread_scope,
                content,
            }) => {
                if let Some(tx) = state.msg_tx.read().await.as_ref().cloned() {
                    let mut msg = IncomingMessage::new(&channel, &flow.user_id, content);
                    if let Some(thread_id) = thread_scope {
                        msg = msg.with_thread(thread_id);
                    }
                    if let Err(e) = tx.send(msg).await {
                        tracing::warn!(
                            extension = %extension_name,
                            user_id = %flow.user_id,
                            error = %e,
                            "Failed to replay pending engine auth request after OAuth callback"
                        );
                    }
                }
            }
            Ok(crate::bridge::AuthCallbackContinuation::None) => {}
            Err(e) => {
                tracing::warn!(
                    extension = %extension_name,
                    user_id = %flow.user_id,
                    error = %e,
                    "Failed to resume pending engine auth gate after OAuth callback"
                );
            }
        }
    }

    let html = oauth::landing_html(&flow.display_name, success);
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

/// Convert legacy web gateway `ImageData` payloads to `IncomingAttachment` objects.
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
                local_path: None,
                extracted_text: None,
                data,
                duration_secs: None,
            })
        })
        .collect()
}

/// Convert web gateway `AttachmentData` payloads to `IncomingAttachment` objects.
pub(crate) fn web_attachments_to_incoming(
    attachments: &[crate::channels::web::types::AttachmentData],
) -> Vec<crate::channels::IncomingAttachment> {
    use base64::Engine;

    attachments
        .iter()
        .enumerate()
        .filter_map(|(i, att)| {
            let data = match base64::engine::general_purpose::STANDARD.decode(&att.data_base64) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!("Skipping attachment {i}: invalid base64 data: {e}");
                    return None;
                }
            };

            let filename = att
                .filename
                .clone()
                .or_else(|| Some(format!("attachment-{i}.{}", mime_to_ext(&att.mime_type))));

            Some(crate::channels::IncomingAttachment {
                id: format!("web-attachment-{i}"),
                kind: crate::channels::AttachmentKind::from_mime_type(&att.mime_type),
                mime_type: att.mime_type.clone(),
                filename,
                size_bytes: Some(data.len() as u64),
                source_url: None,
                storage_key: None,
                local_path: None,
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
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/markdown" => "md",
        "text/csv" => "csv",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/vnd.ms-powerpoint" => "ppt",
        _ if mime.starts_with("image/") => "jpg",
        _ => "bin",
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

    // Convert uploaded files to IncomingAttachments.
    let mut attachments = web_attachments_to_incoming(&req.attachments);
    if !req.images.is_empty() {
        attachments.extend(images_to_attachments(&req.images));
    }
    if !attachments.is_empty() {
        msg = msg.with_attachments(attachments);
    }

    let msg_id = msg.id;
    tracing::trace!(
        "[chat_send_handler] Created message id={}, content_len={}, images={}, attachments={}",
        msg_id,
        req.content.len(),
        req.images.len(),
        req.attachments.len()
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

/// Submit an auth token directly to the shared auth manager, bypassing the message pipeline.
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

    let auth_manager = state
        .auth_manager
        .clone()
        .or_else(|| {
            state
                .tool_registry
                .as_ref()
                .and_then(|tr| tr.secrets_store().cloned())
                .or_else(|| state.secrets_store.clone())
                .or_else(|| {
                    state
                        .extension_manager
                        .as_ref()
                        .map(|em| std::sync::Arc::clone(em.secrets()))
                })
                .map(|secrets| {
                    Arc::new(crate::bridge::auth_manager::AuthManager::new(
                        secrets,
                        state.skill_registry.clone(),
                        state.extension_manager.clone(),
                        state.tool_registry.clone(),
                    ))
                })
        })
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Auth manager not available".to_string(),
        ))?;

    match auth_manager
        .submit_auth_token(&req.extension_name, &req.token, &user.user_id)
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
    if thread_id.is_some() {
        // Thread-scoped pending gates are authoritative once the client sends a
        // thread_id. The unscoped fallback only exists for legacy callers that
        // do not know which thread owns the gate yet.
        return engine_pending_gate_info(user_id, thread_id).await;
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

    let msg = IncomingMessage::new("gateway", user_id, content).with_thread(thread_id.to_string());

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

    let mut owner_bound_channels = std::collections::HashSet::new();
    let mut paired_channels = std::collections::HashSet::new();
    for ext in &installed {
        if ext.kind == crate::extensions::ExtensionKind::WasmChannel {
            if ext_mgr.has_wasm_channel_owner_binding(&ext.name).await {
                owner_bound_channels.insert(ext.name.clone());
            }
            if ext_mgr.has_wasm_channel_pairing(&ext.name).await {
                paired_channels.insert(ext.name.clone());
            }
        }
    }
    let extensions = installed
        .into_iter()
        .map(|ext| {
            let activation_status =
                crate::channels::web::handlers::extensions::derive_activation_status(
                    &ext,
                    paired_channels.contains(&ext.name),
                    owner_bound_channels.contains(&ext.name),
                );
            ExtensionInfo {
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
                onboarding_state: None,
                onboarding: None,
            }
        })
        .collect();

    Ok(Json(ExtensionListResponse { extensions }))
}

fn extension_phase_for_web(
    ext: &crate::extensions::InstalledExtension,
) -> crate::extensions::ExtensionPhase {
    if ext.activation_error.is_some() {
        crate::extensions::ExtensionPhase::Error
    } else if ext.needs_setup {
        crate::extensions::ExtensionPhase::NeedsSetup
    } else if ext.has_auth && !ext.authenticated {
        crate::extensions::ExtensionPhase::NeedsAuth
    } else if ext.active
        || matches!(
            ext.kind,
            crate::extensions::ExtensionKind::WasmChannel
                | crate::extensions::ExtensionKind::ChannelRelay
        )
    {
        crate::extensions::ExtensionPhase::Ready
    } else {
        crate::extensions::ExtensionPhase::NeedsActivation
    }
}

async fn extensions_readiness_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<ExtensionReadinessResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let installed = ext_mgr
        .list(None, false, &user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let extensions = installed
        .into_iter()
        .map(|ext| {
            let phase = match extension_phase_for_web(&ext) {
                crate::extensions::ExtensionPhase::Installed => "installed",
                crate::extensions::ExtensionPhase::NeedsSetup => "needs_setup",
                crate::extensions::ExtensionPhase::NeedsAuth => "needs_auth",
                crate::extensions::ExtensionPhase::NeedsActivation => "needs_activation",
                crate::extensions::ExtensionPhase::Activating => "activating",
                crate::extensions::ExtensionPhase::Ready => "ready",
                crate::extensions::ExtensionPhase::Error => "error",
            }
            .to_string();
            ExtensionReadinessInfo {
                name: ext.name,
                kind: ext.kind.to_string(),
                phase,
                authenticated: ext.authenticated,
                active: ext.active,
                activation_error: ext.activation_error,
            }
        })
        .collect();

    Ok(Json(ExtensionReadinessResponse { extensions }))
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
            match ext_mgr
                .ensure_extension_ready(
                    &req.name,
                    &user.user_id,
                    crate::extensions::EnsureReadyIntent::PostInstall,
                )
                .await
            {
                Ok(readiness) => apply_extension_readiness_to_response(&mut resp, readiness, true),
                Err(e) => {
                    tracing::debug!(
                        extension = %req.name,
                        error = %e,
                        "Post-install readiness follow-through failed"
                    );
                }
            }

            Ok(Json(resp))
        }
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

fn apply_extension_readiness_to_response(
    resp: &mut ActionResponse,
    readiness: crate::extensions::EnsureReadyOutcome,
    preserve_success: bool,
) {
    match readiness {
        crate::extensions::EnsureReadyOutcome::Ready { activation, .. } => {
            if let Some(activation) = activation {
                resp.message = activation.message;
                resp.activated = Some(true);
            }
        }
        crate::extensions::EnsureReadyOutcome::NeedsAuth { auth, .. } => {
            let fallback = format!("'{}' requires authentication.", auth.name);
            if !preserve_success {
                resp.success = false;
                resp.message = auth
                    .instructions()
                    .map(String::from)
                    .unwrap_or_else(|| fallback.clone());
            } else if let Some(instructions) = auth.instructions() {
                resp.message = format!("{}. {}", resp.message, instructions);
            }
            resp.auth_url = auth.auth_url().map(String::from);
            resp.awaiting_token = Some(auth.is_awaiting_token());
            resp.instructions = auth.instructions().map(String::from);
        }
        crate::extensions::EnsureReadyOutcome::NeedsSetup {
            instructions,
            setup_url,
            ..
        } => {
            if !preserve_success {
                resp.success = false;
                resp.message = instructions.clone();
            } else {
                resp.message = format!("{}. {}", resp.message, instructions);
            }
            resp.instructions = Some(instructions);
            resp.auth_url = setup_url;
        }
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

    match ext_mgr
        .ensure_extension_ready(
            &name,
            &user.user_id,
            crate::extensions::EnsureReadyIntent::ExplicitActivate,
        )
        .await
    {
        Ok(readiness) => {
            let mut resp = ActionResponse::ok(format!("Extension '{}' is ready.", name));
            apply_extension_readiness_to_response(&mut resp, readiness, false);
            Ok(Json(resp))
        }
        Err(err) => Ok(Json(ActionResponse::fail(err.to_string()))),
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
        onboarding_state: None,
        onboarding: None,
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
            }
            Ok(Json(resp))
        }
        Err(e) => {
            // Preserve the `activated` field on the failure path so clients
            // (and regression tests) see an explicit `false` rather than
            // `null`. `ActionResponse::fail` leaves `activated` as `None`,
            // which serializes to `null` and makes "did activation fail?"
            // ambiguous from the wire.
            let mut resp = ActionResponse::fail(e.to_string());
            resp.activated = Some(false);
            Ok(Json(resp))
        }
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
        Ok(()) => Ok(Json(ActionResponse::ok("Pairing approved.".to_string()))),
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
    use crate::auth::oauth;
    use crate::channels::web::types::{
        ExtensionActivationStatus, classify_wasm_channel_activation,
    };
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
            auth_manager: None,
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
            frontend_html_cache: Arc::new(tokio::sync::RwLock::new(None)),
            tool_dispatcher: None,
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
    ) -> crate::auth::oauth::PendingOAuthFlow {
        crate::auth::oauth::PendingOAuthFlow {
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

        // Use underscore-only name: `canonicalize_extension_name` rewrites
        // hyphens to underscores, but `configure`'s capabilities-file lookup
        // does not fall back to the legacy hyphen form, so a hyphenated test
        // channel name causes `Capabilities file not found` and the handler
        // takes the `Err` branch (no `activated` field) instead of the
        // intended "saved but activation failed" branch.
        let channel_name = "test_failing_channel";
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

    #[test]
    fn test_extension_phase_for_web_prefers_error_then_readiness() {
        let mut ext = crate::extensions::InstalledExtension {
            name: "notion".to_string(),
            kind: crate::extensions::ExtensionKind::McpServer,
            display_name: None,
            description: None,
            url: None,
            authenticated: false,
            active: false,
            tools: Vec::new(),
            needs_setup: false,
            has_auth: true,
            installed: true,
            activation_error: Some("boom".to_string()),
            version: None,
        };
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::Error
        );

        ext.activation_error = None;
        ext.needs_setup = true;
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::NeedsSetup
        );

        ext.needs_setup = false;
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::NeedsAuth
        );

        ext.authenticated = true;
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::NeedsActivation
        );

        ext.active = true;
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::Ready
        );
    }

    #[tokio::test]
    async fn test_extensions_readiness_handler_reports_phase_summary() {
        use axum::body::Body;
        use tower::ServiceExt;

        // DB-backed manager so the install path does not fall back to the
        // developer's real `~/.ironclaw/mcp-servers.json` (which would
        // panic with `AlreadyInstalled("notion")` on dev machines that
        // already have a notion entry configured).
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir, _db_dir) = test_ext_mgr_with_db().await;
        let mut server =
            crate::tools::mcp::McpServerConfig::new("notion", "https://mcp.notion.com/mcp");
        server.description = Some("Notion".to_string());
        ext_mgr
            .install(
                "notion",
                Some(&server.url),
                Some(crate::extensions::ExtensionKind::McpServer),
                "test",
            )
            .await
            .expect("install notion mcp");

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route(
                "/api/extensions/readiness",
                get(extensions_readiness_handler),
            )
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/extensions/readiness")
            .body(Body::empty())
            .expect("request");
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
        let notion = parsed["extensions"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["name"] == "notion"))
            .expect("notion readiness entry");
        assert_eq!(notion["kind"], "mcp_server");
        assert_eq!(notion["phase"], "needs_auth");
        assert_eq!(notion["authenticated"], false);
        assert_eq!(notion["active"], false);
    }

    #[tokio::test]
    async fn test_extensions_setup_submit_telegram_verification_does_not_broadcast_auth_required() {
        use axum::body::Body;
        use tokio::time::{Duration, timeout};
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, wasm_channels_dir) = test_ext_mgr(secrets);

        std::fs::write(
            wasm_channels_dir.path().join("telegram.wasm"),
            b"\0asm fake",
        )
        .expect("write fake telegram wasm");
        let caps = serde_json::json!({
            "type": "channel",
            "name": "telegram",
            "setup": {
                "required_secrets": [
                    {
                        "name": "telegram_bot_token",
                        "prompt": "Enter your Telegram Bot API token (from @BotFather)"
                    }
                ]
            }
        });
        std::fs::write(
            wasm_channels_dir.path().join("telegram.capabilities.json"),
            serde_json::to_string(&caps).expect("serialize telegram caps"),
        )
        .expect("write telegram caps");

        ext_mgr
            .set_test_telegram_pending_verification("iclaw-7qk2m9", Some("test_hot_bot"))
            .await;

        let state = test_gateway_state(Some(ext_mgr));
        let mut receiver = state.sse.sender().subscribe();
        let app = Router::new()
            .route(
                "/api/extensions/{name}/setup",
                post(extensions_setup_submit_handler),
            )
            .with_state(state);

        let req_body = serde_json::json!({
            "secrets": {
                "telegram_bot_token": "123456789:ABCdefGhI"
            }
        });
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/extensions/telegram/setup")
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
        assert_eq!(parsed["success"], serde_json::Value::Bool(true));
        assert_eq!(parsed["activated"], serde_json::Value::Bool(false));
        assert_eq!(parsed["verification"]["code"], "iclaw-7qk2m9");

        let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match timeout(remaining, receiver.recv()).await {
                Ok(Ok(scoped))
                    if matches!(
                        scoped.event,
                        crate::channels::web::types::AppEvent::AuthRequired { .. }
                    ) =>
                {
                    panic!("verification responses should not emit auth_required SSE events")
                }
                Ok(Ok(_)) => continue,
                Ok(Err(_)) | Err(_) => break,
            }
        }
    }

    #[tokio::test]
    async fn test_llm_test_connection_allows_admin_private_base_url() {
        use axum::body::Body;
        use tower::ServiceExt;

        let state = test_gateway_state(None);
        let app = Router::new()
            .route(
                "/api/llm/test_connection",
                post(llm_test_connection_handler),
            )
            .with_state(state);

        let req_body = serde_json::json!({
            "adapter": "openai",
            "base_url": "http://127.0.0.1:9/v1",
            "model": "test-model"
        });
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/llm/test_connection")
            .header("content-type", "application/json")
            .body(Body::from(req_body.to_string()))
            .expect("request");
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
        assert_eq!(parsed["ok"], serde_json::Value::Bool(false));
        let message = parsed["message"].as_str().unwrap_or_default();
        assert!(
            !message.contains("Invalid base URL"),
            "private localhost endpoint should pass validation: {message}"
        );
    }

    #[tokio::test]
    async fn test_llm_test_connection_requires_admin_role() {
        use axum::body::Body;
        use tower::ServiceExt;

        let state = test_gateway_state(None);
        let app = Router::new()
            .route(
                "/api/llm/test_connection",
                post(llm_test_connection_handler),
            )
            .with_state(state);

        let req_body = serde_json::json!({
            "adapter": "openai",
            "base_url": "http://127.0.0.1:9/v1",
            "model": "test-model"
        });
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/llm/test_connection")
            .header("content-type", "application/json")
            .body(Body::from(req_body.to_string()))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "member".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_llm_list_models_requires_admin_role() {
        use axum::body::Body;
        use tower::ServiceExt;

        let state = test_gateway_state(None);
        let app = Router::new()
            .route("/api/llm/list_models", post(llm_list_models_handler))
            .with_state(state);

        let req_body = serde_json::json!({
            "adapter": "openai",
            "base_url": "http://127.0.0.1:9/v1"
        });
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/llm/list_models")
            .header("content-type", "application/json")
            .body(Body::from(req_body.to_string()))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "member".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    fn expired_flow_created_at() -> Option<std::time::Instant> {
        std::time::Instant::now()
            .checked_sub(oauth::OAUTH_FLOW_EXPIRY + std::time::Duration::from_secs(1))
    }

    #[test]
    fn apply_extension_readiness_preserves_install_success_for_auth_followup() {
        let mut resp = ActionResponse::ok("Installed notion");
        apply_extension_readiness_to_response(
            &mut resp,
            crate::extensions::EnsureReadyOutcome::NeedsAuth {
                name: "notion".to_string(),
                kind: crate::extensions::ExtensionKind::McpServer,
                phase: crate::extensions::ExtensionPhase::NeedsAuth,
                credential_name: Some("notion_api_token".to_string()),
                auth: crate::extensions::AuthResult::awaiting_authorization(
                    "notion",
                    crate::extensions::ExtensionKind::McpServer,
                    "https://example.com/oauth".to_string(),
                    "gateway".to_string(),
                ),
            },
            true,
        );

        assert!(resp.success);
        assert_eq!(resp.auth_url.as_deref(), Some("https://example.com/oauth"));
        assert_eq!(resp.awaiting_token, Some(false));
    }

    #[test]
    fn apply_extension_readiness_fails_activate_when_auth_is_required() {
        let mut resp = ActionResponse::ok("placeholder");
        apply_extension_readiness_to_response(
            &mut resp,
            crate::extensions::EnsureReadyOutcome::NeedsAuth {
                name: "notion".to_string(),
                kind: crate::extensions::ExtensionKind::McpServer,
                phase: crate::extensions::ExtensionPhase::NeedsAuth,
                credential_name: Some("notion_api_token".to_string()),
                auth: crate::extensions::AuthResult::awaiting_token(
                    "notion",
                    crate::extensions::ExtensionKind::McpServer,
                    "Paste your Notion token".to_string(),
                    None,
                ),
            },
            false,
        );

        assert!(!resp.success);
        assert_eq!(resp.awaiting_token, Some(true));
        assert_eq!(
            resp.instructions.as_deref(),
            Some("Paste your Notion token")
        );
        assert_eq!(resp.message, "Paste your Notion token");
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

    #[test]
    fn test_base_and_nonce_csp_agree_outside_script_src() {
        // Regression for the drift risk flagged in PR #1725 review: the
        // static header and the per-response nonce header must share every
        // directive except `script-src`. Build both, strip `script-src …;`
        // from each, and assert the remaining policy is byte-identical.
        let base = build_csp(None);
        let nonce = build_csp(Some("feedc0de"));

        fn strip_script_src(csp: &str) -> String {
            // Directives are separated by `; `. Drop the one that starts
            // with `script-src` and rejoin the rest.
            csp.split("; ")
                .filter(|d| !d.trim_start().starts_with("script-src"))
                .collect::<Vec<_>>()
                .join("; ")
        }

        assert_eq!(
            strip_script_src(&base),
            strip_script_src(&nonce),
            "base CSP and nonce CSP must agree on every directive except script-src\n\
             base:  {base}\n\
             nonce: {nonce}"
        );
    }

    #[test]
    fn test_base_csp_header_matches_build_csp_none() {
        // The lazy static header used by the response-header layer must be
        // byte-identical to `build_csp(None)`. If the fallback branch of
        // the LazyLock ever fires, the header would regress to
        // `default-src 'self'` and this test would catch it.
        let lazy = BASE_CSP_HEADER.to_str().expect("static CSP is ASCII");
        assert_eq!(lazy, build_csp(None));
    }

    #[test]
    fn test_build_csp_with_nonce_includes_nonce_source() {
        // Per-response CSP must add `'nonce-…'` to script-src so a single
        // inline `<script nonce="…">` block is authorized for that response.
        let csp = build_csp_with_nonce("deadbeefcafebabe");
        assert!(
            csp.contains("script-src 'self' 'nonce-deadbeefcafebabe' https://cdn.jsdelivr.net"),
            "nonce source must appear immediately after 'self' in script-src; got: {csp}"
        );
        // The other directives must match the static BASE_CSP so the
        // per-response value never accidentally relaxes anything else.
        for needle in [
            "default-src 'self'",
            "style-src 'self' 'unsafe-inline'",
            "object-src 'none'",
            "frame-ancestors 'none'",
            "base-uri 'self'",
        ] {
            assert!(csp.contains(needle), "missing directive: {needle}");
        }
        // And it must NOT contain `'unsafe-inline'` for scripts.
        assert!(
            !csp.contains("script-src 'self' 'unsafe-inline'"),
            "script-src must not allow 'unsafe-inline'"
        );
    }

    #[test]
    fn test_generate_csp_nonce_is_unique_and_hex() {
        let a = generate_csp_nonce();
        let b = generate_csp_nonce();
        assert_eq!(a.len(), 32, "16 bytes hex-encoded should be 32 chars");
        assert_ne!(a, b, "nonces must be unique per call");
        assert!(
            a.chars().all(|c| c.is_ascii_hexdigit()),
            "nonce must be lowercase hex"
        );
    }

    #[test]
    fn test_css_etag_is_strong_validator_format() {
        // Strong validators are double-quoted (no `W/` prefix). The
        // sha-prefix lets future readers identify the digest function at a
        // glance, and 16 hex chars (64 bits) is plenty for content-address
        // collision avoidance on a single-tenant CSS payload.
        let etag = css_etag("body { color: red; }");
        assert!(etag.starts_with("\"sha256-"));
        assert!(etag.ends_with('"'));
        assert!(!etag.starts_with("W/"));
        // Header value must be ASCII so it can land in a `HeaderValue`.
        assert!(etag.is_ascii());
    }

    #[test]
    fn test_css_etag_changes_when_body_changes() {
        // The whole point of the ETag: editing `custom.css` must produce
        // a new validator so the browser fetches the updated body.
        let base = css_etag("body { color: red; }");
        let edited = css_etag("body { color: blue; }");
        assert_ne!(base, edited);
        // Adding even a single byte must invalidate.
        let appended = css_etag("body { color: red; } ");
        assert_ne!(base, appended);
    }

    #[test]
    fn test_css_etag_stable_for_identical_body() {
        // Two requests against the same assembled body must produce the
        // same validator — otherwise every request misses the cache.
        let body = "body { color: red; }";
        assert_eq!(css_etag(body), css_etag(body));
    }

    #[tokio::test]
    async fn test_css_handler_returns_etag_and_serves_304_on_match() {
        use axum::body::Body;
        use tower::ServiceExt;

        // Pure-static path: no workspace overlay, so the body is exactly
        // the embedded `STYLE_CSS`. Cheap and deterministic.
        let state = test_gateway_state(None);
        let app = Router::new()
            .route("/style.css", get(css_handler))
            .with_state(state);

        // First request: 200 with ETag header.
        let req = axum::http::Request::builder()
            .uri("/style.css")
            .body(Body::empty())
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let etag = resp
            .headers()
            .get(header::ETAG)
            .expect("ETag header must be present on 200")
            .to_str()
            .expect("ETag is ASCII")
            .to_string();
        assert!(etag.starts_with("\"sha256-"));

        // Second request with `If-None-Match` matching the validator: 304
        // and an empty body. The browser keeps its cached copy.
        let req = axum::http::Request::builder()
            .uri("/style.css")
            .header(header::IF_NONE_MATCH, &etag)
            .body(Body::empty())
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
        let body = axum::body::to_bytes(resp.into_body(), 1024)
            .await
            .expect("body");
        assert!(body.is_empty(), "304 must have an empty body");

        // Third request with a stale validator: 200 again. Operators
        // expect this when `custom.css` changes underneath them — the
        // browser revalidates, sees the body shifted, and fetches anew.
        let req = axum::http::Request::builder()
            .uri("/style.css")
            .header(header::IF_NONE_MATCH, "\"sha256-0000000000000000\"")
            .body(Body::empty())
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Multi-tenant safety symmetry: in multi-user mode the CSS handler
    /// must mirror `build_frontend_html` and refuse to layer
    /// `.system/gateway/custom.css` from `state.workspace`. The
    /// `/style.css` route is unauthenticated bootstrap, so there is no
    /// user identity at request time — reading the global workspace
    /// would leak one operator's `custom.css` to every other tenant.
    ///
    /// The bait here is a global workspace seeded with hostile-looking
    /// custom CSS. If `css_handler` ever stops short-circuiting on
    /// `workspace_pool.is_some()`, the bait would land in the response
    /// body and this test would fail loudly with the leaked content.
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_css_handler_returns_base_in_multi_tenant_mode() {
        use axum::body::Body;
        use tower::ServiceExt;

        use crate::config::{WorkspaceConfig, WorkspaceSearchConfig};
        use crate::db::Database as _;
        use crate::db::libsql::LibSqlBackend;
        use crate::workspace::EmbeddingCacheConfig;

        let dir = tempfile::tempdir().expect("tempdir");
        let backend = LibSqlBackend::new_local(&dir.path().join("multi_tenant_css.db"))
            .await
            .expect("backend");
        backend.run_migrations().await.expect("migrations");
        let db: Arc<dyn Database> = Arc::new(backend);

        // Bait: a global workspace with a hostile-looking custom.css.
        // If css_handler ever reads state.workspace in multi-tenant
        // mode, the marker would leak into the response body and this
        // test would fail with an actionable diagnostic.
        let global_ws = Arc::new(Workspace::new_with_db("tenant-leak-bait", Arc::clone(&db)));
        global_ws
            .write(
                ".system/gateway/custom.css",
                "body { background: #ff0000; } /* TENANT-LEAK-BAIT */",
            )
            .await
            .expect("seed bait custom.css");

        let pool = Arc::new(WorkspacePool::new(
            Arc::clone(&db),
            None,
            EmbeddingCacheConfig::default(),
            WorkspaceSearchConfig::default(),
            WorkspaceConfig::default(),
        ));

        let mut state = test_gateway_state(None);
        let state_mut = Arc::get_mut(&mut state).expect("test state must be uniquely owned");
        state_mut.workspace = Some(global_ws);
        state_mut.workspace_pool = Some(pool);

        let app = Router::new()
            .route("/style.css", get(css_handler))
            .with_state(state);

        let req = axum::http::Request::builder()
            .uri("/style.css")
            .body(Body::empty())
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .expect("body");
        let body_str = String::from_utf8_lossy(&body);

        // Contract 1: the bait marker is absent. If a future regression
        // re-reads state.workspace in multi-tenant mode, the marker
        // would land here and this assertion fails with the leaked
        // content visible in the diagnostic.
        assert!(
            !body_str.contains("TENANT-LEAK-BAIT"),
            "custom.css from global workspace leaked into multi-tenant /style.css \
             response — css_handler is missing its workspace_pool guard"
        );

        // Contract 2: the response is exactly the embedded base
        // stylesheet, byte-for-byte. This catches a subtler regression
        // where the leak content is dropped but the multi-tenant path
        // still does the owned `format!` (turning what should be a
        // borrowed hot-path response into an allocation).
        assert_eq!(
            body_str.as_ref(),
            assets::STYLE_CSS,
            "multi-tenant /style.css must serve the embedded base stylesheet \
             unchanged — no overlay, no allocation"
        );
    }

    #[test]
    fn test_stamp_nonce_into_html_replaces_attribute() {
        // Vanilla case: a placeholder inside a `nonce="…"` attribute on
        // a script tag must be substituted with the real nonce. Both
        // the layout-config script and any widget script tags emitted
        // by `assemble_index` carry the same attribute shape, so a
        // single test covers every emission point.
        let html = format!("<script nonce=\"{NONCE_PLACEHOLDER}\">window.X = 1;</script>");
        let stamped = stamp_nonce_into_html(&html, "deadbeef");
        assert!(
            stamped.contains("nonce=\"deadbeef\""),
            "real nonce attribute must be present after substitution: {stamped}"
        );
        assert!(
            !stamped.contains(NONCE_PLACEHOLDER),
            "placeholder must be gone after substitution: {stamped}"
        );
    }

    #[test]
    fn test_stamp_nonce_into_html_does_not_mutate_widget_body() {
        // Regression for the PR #1725 Copilot finding: a bare-string
        // replace would also rewrite any *body content* that happens to
        // contain the literal sentinel — e.g. a widget JS module that
        // mentions `__IRONCLAW_CSP_NONCE__` in a comment, log line, or
        // string constant. The attribute-targeted replace must leave
        // those untouched.
        //
        // Build a fragment with TWO sentinels: one inside the
        // legitimate `nonce="…"` attribute (must be replaced) and one
        // inside the script body as a string constant (must NOT be
        // replaced).
        let html = format!(
            "<script type=\"module\" nonce=\"{NONCE_PLACEHOLDER}\">\n\
             // hostile widget body — author writes the sentinel as a constant\n\
             const SENTINEL = \"{NONCE_PLACEHOLDER}\";\n\
             console.log(SENTINEL);\n\
             </script>"
        );
        let stamped = stamp_nonce_into_html(&html, "cafebabe");

        // Contract 1: the attribute was rewritten.
        assert!(
            stamped.contains("nonce=\"cafebabe\""),
            "attribute must carry the per-response nonce: {stamped}"
        );

        // Contract 2: the body sentinel survived intact. The widget
        // author's source must round-trip byte-for-byte.
        assert!(
            stamped.contains(&format!("const SENTINEL = \"{NONCE_PLACEHOLDER}\"")),
            "widget body sentinel must NOT be rewritten: {stamped}"
        );

        // Contract 3: exactly one occurrence of the placeholder remains
        // (the one in the body). If a future regression switches to a
        // bare-string replace, this count would drop to 0 and the test
        // would fail loudly with the diff.
        assert_eq!(
            stamped.matches(NONCE_PLACEHOLDER).count(),
            1,
            "exactly one placeholder occurrence (in widget body) must \
             survive; the attribute one must be replaced. Got: {stamped}"
        );
    }

    /// Multi-tenant cache safety: when `workspace_pool` is set,
    /// `build_frontend_html` must refuse the assembly path entirely and
    /// return `None` regardless of what `state.workspace` contains.
    ///
    /// Background: `index_handler` (`GET /`) is the unauthenticated
    /// bootstrap route, so it has no user identity at request time.
    /// Reading `state.workspace` in multi-tenant mode would expose one
    /// global workspace's customizations to every user, and the
    /// process-wide `frontend_html_cache` would pin the leak across
    /// requests. The bait here is a global workspace seeded with a
    /// hostile-looking layout — if the function ever stops short-
    /// circuiting on `workspace_pool.is_some()`, that layout would land
    /// in the assembled HTML and this test would fail loudly.
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_build_frontend_html_returns_none_in_multi_tenant_mode() {
        use crate::config::{WorkspaceConfig, WorkspaceSearchConfig};
        use crate::db::Database as _;
        use crate::db::libsql::LibSqlBackend;
        use crate::workspace::EmbeddingCacheConfig;

        let dir = tempfile::tempdir().expect("tempdir");
        let backend = LibSqlBackend::new_local(&dir.path().join("multi_tenant_index.db"))
            .await
            .expect("backend");
        backend.run_migrations().await.expect("migrations");
        let db: Arc<dyn Database> = Arc::new(backend);

        // Bait: a *global* workspace with customizations. If
        // build_frontend_html ever read state.workspace in multi-tenant
        // mode, the title "TENANT-LEAK-BAIT" would appear in the
        // assembled HTML for every user. The assertions below pin the
        // refusal contract — both the return value AND the cache slot.
        let global_ws = Arc::new(Workspace::new_with_db("tenant-leak-bait", Arc::clone(&db)));
        global_ws
            .write(
                ".system/gateway/layout.json",
                r#"{"branding":{"title":"TENANT-LEAK-BAIT"}}"#,
            )
            .await
            .expect("seed bait layout");

        let pool = Arc::new(WorkspacePool::new(
            Arc::clone(&db),
            None,
            EmbeddingCacheConfig::default(),
            WorkspaceSearchConfig::default(),
            WorkspaceConfig::default(),
        ));

        // Build state via the standard test helper, then mutate the
        // workspace + workspace_pool fields. `Arc::get_mut` succeeds here
        // because no other strong reference exists yet — the helper just
        // returned the freshly-constructed Arc.
        let mut state = test_gateway_state(None);
        let state_mut = Arc::get_mut(&mut state).expect("test state must be uniquely owned");
        state_mut.workspace = Some(global_ws);
        state_mut.workspace_pool = Some(pool);

        // Contract 1: build_frontend_html refuses to assemble.
        let html = build_frontend_html(&state).await;
        assert!(
            html.is_none(),
            "build_frontend_html must return None in multi-tenant mode \
             (got Some HTML — bait layout may have leaked across tenants)"
        );

        // Contract 2: the cache slot is still empty. The early return
        // above MUST short-circuit before the cache write at the bottom
        // of the function — otherwise a poisoned cache entry would serve
        // the leaked HTML to subsequent requests even after the bug is
        // fixed.
        let cache = state.frontend_html_cache.read().await;
        assert!(
            cache.is_none(),
            "frontend_html_cache must remain empty in multi-tenant mode"
        );
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
        let flow = crate::auth::oauth::PendingOAuthFlow {
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
        let flow = crate::auth::oauth::PendingOAuthFlow {
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
        let flow = crate::auth::oauth::PendingOAuthFlow {
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
        let flow = crate::auth::oauth::PendingOAuthFlow {
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
            crate::auth::oauth::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

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
        let flow = crate::auth::oauth::PendingOAuthFlow {
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
        let versioned_state = crate::auth::oauth::encode_hosted_oauth_state("test_nonce", None);

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
            crate::auth::oauth::oauth_proxy_auth_token(),
        );

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::auth::oauth::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

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
            crate::auth::oauth::oauth_proxy_auth_token(),
        );

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state = crate::auth::oauth::encode_hosted_oauth_state("test_nonce", None);

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
            crate::auth::oauth::oauth_proxy_auth_token(),
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
            crate::auth::oauth::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

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
            crate::auth::oauth::oauth_proxy_auth_token(),
        );

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::auth::oauth::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

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

    /// DB-backed `ExtensionManager` for tests that exercise MCP install/list
    /// paths.
    ///
    /// `test_ext_mgr` builds the manager with `store: None`, which makes
    /// `load_mcp_servers` fall back to the file-based path
    /// `~/.ironclaw/mcp-servers.json`. Any test that calls `install` for an
    /// MCP server with `store: None` will read the developer's real config
    /// and may panic with `AlreadyInstalled("notion")` (or similar) on
    /// machines that have configured MCP servers locally.
    ///
    /// This sibling builds an isolated in-memory libsql DB AND pre-seeds
    /// an empty `mcp_servers` setting for the test user so that
    /// `load_mcp_servers_from_db` does not silently fall back to disk
    /// (it falls back when the DB has no entry, see `mcp/config.rs:625`).
    async fn test_ext_mgr_with_db() -> (
        Arc<ExtensionManager>,
        tempfile::TempDir,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let secrets = test_secrets_store();
        let tool_registry = Arc::new(ToolRegistry::new());
        let mcp_sm = Arc::new(crate::tools::mcp::session::McpSessionManager::new());
        let mcp_pm = Arc::new(crate::tools::mcp::process::McpProcessManager::new());
        let wasm_tools_dir = tempfile::tempdir().expect("temp wasm tools dir");
        let wasm_channels_dir = tempfile::tempdir().expect("temp wasm channels dir");
        let (db, db_dir) = crate::testing::test_db().await;

        // Pre-seed an empty servers list so the DB-backed loader does not
        // fall back to `~/.ironclaw/mcp-servers.json` on dev machines.
        let empty_servers = crate::tools::mcp::config::McpServersFile::default();
        crate::tools::mcp::config::save_mcp_servers_to_db(db.as_ref(), "test", &empty_servers)
            .await
            .expect("seed empty mcp_servers setting");

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
            Some(db),
            vec![],
        ));
        (ext_mgr, wasm_tools_dir, wasm_channels_dir, db_dir)
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
