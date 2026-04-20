//! Gateway shared state and supporting primitives.
//!
//! This module owns the process-lifetime data structures the platform layer
//! and feature handlers borrow through [`GatewayState`]: the sliding-window
//! rate limiters, the per-user workspace pool, the frontend-HTML cache keys,
//! and the type aliases (`PromptQueue`, `RoutineEngineSlot`) that are too
//! noisy to spell inline.
//!
//! Handlers should depend on this module instead of the older
//! `crate::channels::web::server::*` path; the re-exports in `server.rs`
//! exist only for backward compatibility during the ironclaw#2599 migration
//! and will be removed once all call sites are updated.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::http::HeaderMap;
use tokio::sync::{mpsc, oneshot};

use crate::agent::SessionManager;
use crate::channels::IncomingMessage;
use crate::channels::web::auth::UserIdentity;
use crate::channels::web::log_layer::LogBroadcaster;
use crate::channels::web::sse::SseManager;
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

/// Extract a rate-limit key from request headers.
///
/// Prefers the first parseable `X-Forwarded-For` entry (reverse proxy
/// terminators prepend the client IP here), falling back to `X-Real-IP`,
/// and finally to the literal string `"unknown"` when neither header is
/// usable. The returned key is only used as a per-user rate-limit bucket
/// identifier; it is never logged or written to the database.
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
    /// Cached settings store. When present, settings reads/writes go through
    /// the cache layer for consistency with the agent loop. Concrete type so
    /// handlers can also call `invalidate_user()` / `flush()`.
    pub settings_cache: Option<Arc<crate::db::cached_settings::CachedSettingsStore>>,
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
    /// Hot-reload controller for the LLM provider chain. Populated at
    /// startup when the chain is built from config (not in test harnesses
    /// that inject a provider directly).
    pub llm_reload: Option<Arc<crate::llm::LlmReloadHandle>>,
    /// LLM session manager handed through to `LlmReloadHandle::reload` so
    /// the rebuilt chain keeps using the same (potentially authenticated)
    /// NEAR AI / OAuth session without forcing a re-login.
    pub llm_session_manager: Option<Arc<crate::llm::SessionManager>>,
    /// Optional TOML config path that produced the current `LlmConfig`.
    /// Needed so a hot-reload reads the same precedence layers
    /// (TOML → DB overlay) as startup.
    pub config_toml_path: Option<std::path::PathBuf>,
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
    pub active_config: Arc<tokio::sync::RwLock<ActiveConfigSnapshot>>,
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
