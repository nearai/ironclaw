//! Shared test utilities for gateway tests.
//!
//! This module is **always compiled** (not `#[cfg(test)]`) because integration
//! tests in `tests/` import the crate as a regular dependency and `cfg(test)`
//! is only set when compiling *this* crate's unit tests. The publicly exposed
//! [`TestGatewayBuilder`] is therefore unconditionally visible.
//!
//! The three cross-slice `pub(crate)` functions below — `test_gateway_state`,
//! `test_gateway_state_with_dependencies`,
//! `test_gateway_state_with_store_and_session_manager` — are scoped to unit
//! tests and are individually gated with `#[cfg(test)]`. They previously
//! lived inside `server.rs::tests` where they were unreachable from other
//! slice test modules; promoting them here (ironclaw#2599 stage-6
//! prerequisite) lets caller-level tests migrate out of `server.rs::tests`
//! and into the feature slice they actually exercise (chat, oauth, pairing,
//! extensions).

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::channels::IncomingMessage;
use crate::channels::web::auth::MultiAuthState;
use crate::channels::web::server::{GatewayState, PerUserRateLimiter, RateLimiter, start_server};
use crate::channels::web::sse::SseManager;
use crate::channels::web::ws::WsConnectionTracker;

#[cfg(test)]
use crate::channels::web::auth::DbAuthenticator;
#[cfg(test)]
use crate::channels::web::server::ActiveConfigSnapshot;

/// Builder for constructing a [`GatewayState`] with sensible test defaults.
///
/// Every optional field defaults to `None` and can be overridden via builder
/// methods.  Call [`build`](Self::build) to get the `Arc<GatewayState>`, or
/// [`start`](Self::start) to also bind an Axum server on a random port.
pub struct TestGatewayBuilder {
    msg_tx: Option<mpsc::Sender<IncomingMessage>>,
    llm_provider: Option<Arc<dyn crate::llm::LlmProvider>>,
    user_id: String,
}

impl Default for TestGatewayBuilder {
    fn default() -> Self {
        Self {
            msg_tx: None,
            llm_provider: None,
            user_id: "test-user".to_string(),
        }
    }
}

impl TestGatewayBuilder {
    /// Create a new builder with all defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the agent message sender (the channel the gateway forwards
    /// incoming chat messages to).
    pub fn msg_tx(mut self, tx: mpsc::Sender<IncomingMessage>) -> Self {
        self.msg_tx = Some(tx);
        self
    }

    /// Set the LLM provider (needed for OpenAI-compatible API tests).
    pub fn llm_provider(mut self, provider: Arc<dyn crate::llm::LlmProvider>) -> Self {
        self.llm_provider = Some(provider);
        self
    }

    /// Override the user ID (default: `"test-user"`).
    pub fn user_id(mut self, id: impl Into<String>) -> Self {
        self.user_id = id.into();
        self
    }

    /// Build the `Arc<GatewayState>` without starting a server.
    pub fn build(self) -> Arc<GatewayState> {
        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(self.msg_tx),
            sse: Arc::new(SseManager::new()),
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
            owner_id: self.user_id.clone(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
            llm_provider: self.llm_provider,
            llm_reload: None,
            llm_session_manager: None,
            config_toml_path: None,
            skill_registry: None,
            skill_catalog: None,
            auth_manager: None,
            scheduler: None,
            chat_rate_limiter: PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
            webhook_rate_limiter: RateLimiter::new(10, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: Arc::new(tokio::sync::RwLock::new(
                crate::channels::web::server::ActiveConfigSnapshot::default(),
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
        })
    }

    /// Build the state and start a gateway server on `127.0.0.1:0` (random
    /// port).  Returns the bound address and the shared state.
    pub async fn start(
        self,
        auth_token: &str,
    ) -> Result<(SocketAddr, Arc<GatewayState>), crate::error::ChannelError> {
        let auth = MultiAuthState::single(auth_token.to_string(), "test-user".to_string());
        let state = self.build();
        let addr: SocketAddr = "127.0.0.1:0"
            .parse()
            .expect("hard-coded address must parse"); // safety: constant literal
        let bound = start_server(addr, state.clone(), auth.into()).await?;
        Ok((bound, state))
    }

    /// Build the state and start a gateway server with multi-user auth.
    /// Returns the bound address and the shared state.
    pub async fn start_multi(
        self,
        auth: MultiAuthState,
    ) -> Result<(SocketAddr, Arc<GatewayState>), crate::error::ChannelError> {
        let state = self.build();
        let addr: SocketAddr = "127.0.0.1:0"
            .parse()
            .expect("hard-coded address must parse"); // safety: constant literal
        let bound = start_server(addr, state.clone(), auth.into()).await?;
        Ok((bound, state))
    }
}

// ---------------------------------------------------------------------------
// Cross-slice positional builders used by unit tests in `server.rs::tests`
// and (per the ironclaw#2599 stage-6 plan) the chat / extensions / oauth /
// pairing slice test modules. Kept as `pub(crate)` free functions with
// the same positional signatures they had when they lived in
// `server.rs::tests`, so call sites migrate in later PRs without touching
// argument lists. Gated to `cfg(test)` because the surrounding module is
// always-compiled (so integration tests in `tests/` can reach
// `TestGatewayBuilder`), but these three functions only have in-crate
// unit-test callers.
// ---------------------------------------------------------------------------

/// Build a minimal `GatewayState` with every optional subsystem `None`
/// except `extension_manager`.
///
/// Equivalent to calling
/// [`test_gateway_state_with_dependencies(ext_mgr, None, None, None)`].
#[cfg(test)]
pub(crate) fn test_gateway_state(
    ext_mgr: Option<Arc<crate::extensions::ExtensionManager>>,
) -> Arc<GatewayState> {
    test_gateway_state_with_dependencies(ext_mgr, None, None, None)
}

/// Build a `GatewayState` with the four subsystems most commonly exercised
/// by cross-slice handler tests (extensions, store, db-auth, pairing).
/// Every field not reachable from these four dependencies stays `None`.
#[cfg(test)]
pub(crate) fn test_gateway_state_with_dependencies(
    ext_mgr: Option<Arc<crate::extensions::ExtensionManager>>,
    store: Option<Arc<dyn crate::db::Database>>,
    db_auth: Option<Arc<DbAuthenticator>>,
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
        settings_cache: None,
        job_manager: None,
        prompt_queue: None,
        owner_id: "test".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: None,
        llm_provider: None,
        llm_reload: None,
        llm_session_manager: None,
        config_toml_path: None,
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
        active_config: Arc::new(tokio::sync::RwLock::new(ActiveConfigSnapshot::default())),
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

/// Build a `GatewayState` wired to a real store + `SessionManager` for
/// chat-slice caller-level tests (history / approval / auth-token / gate).
#[cfg(test)]
pub(crate) fn test_gateway_state_with_store_and_session_manager(
    store: Arc<dyn crate::db::Database>,
    session_manager: Arc<crate::agent::SessionManager>,
) -> Arc<GatewayState> {
    Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: Arc::new(SseManager::new()),
        workspace: None,
        workspace_pool: None,
        session_manager: Some(session_manager),
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: Some(store),
        settings_cache: None,
        job_manager: None,
        prompt_queue: None,
        owner_id: "test".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: None,
        llm_provider: None,
        llm_reload: None,
        llm_session_manager: None,
        config_toml_path: None,
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
        active_config: Arc::new(tokio::sync::RwLock::new(ActiveConfigSnapshot::default())),
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
    })
}
