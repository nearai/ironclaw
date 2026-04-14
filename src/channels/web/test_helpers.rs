//! Shared test utilities for gateway integration tests.
//!
//! This module is always compiled (not `#[cfg(test)]`) because integration tests
//! in `tests/` import the crate as a regular dependency and `cfg(test)` is only
//! set when compiling *this* crate's unit tests.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::channels::IncomingMessage;
use crate::channels::web::auth::MultiAuthState;
use crate::channels::web::server::{GatewayState, PerUserRateLimiter, RateLimiter, start_server};
use crate::channels::web::sse::SseManager;
use crate::channels::web::ws::WsConnectionTracker;

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
            job_manager: None,
            prompt_queue: None,
            owner_id: self.user_id.clone(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
            llm_provider: self.llm_provider,
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
            active_config: crate::channels::web::server::ActiveConfigSnapshot::default(),
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
