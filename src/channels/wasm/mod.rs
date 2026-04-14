//! WASM-extensible channel system.
//!
//! This module provides a runtime for executing WASM-based channels using a
//! Host-Managed Event Loop pattern. The host (Rust) manages infrastructure
//! (HTTP server, polling), while WASM modules define channel behavior through
//! callbacks.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────────┐
//! │                          Host-Managed Event Loop                                 │
//! │                                                                                  │
//! │   ┌─────────────┐     ┌──────────────┐     ┌──────────────┐                     │
//! │   │   HTTP      │     │   Polling    │     │   Timer      │                     │
//! │   │   Router    │     │   Scheduler  │     │   Scheduler  │                     │
//! │   └──────┬──────┘     └──────┬───────┘     └──────┬───────┘                     │
//! │          │                   │                    │                              │
//! │          └───────────────────┴────────────────────┘                              │
//! │                              │                                                   │
//! │                              ▼                                                   │
//! │                    ┌─────────────────┐                                           │
//! │                    │   Event Router  │                                           │
//! │                    └────────┬────────┘                                           │
//! │                             │                                                    │
//! │          ┌──────────────────┼──────────────────┐                                │
//! │          ▼                  ▼                  ▼                                 │
//! │   ┌─────────────┐   ┌─────────────┐   ┌─────────────┐                           │
//! │   │ on_http_req │   │  on_poll    │   │ on_respond  │  WASM Exports             │
//! │   └─────────────┘   └─────────────┘   └─────────────┘                           │
//! │          │                  │                  │                                 │
//! │          └──────────────────┴──────────────────┘                                │
//! │                             │                                                    │
//! │                             ▼                                                    │
//! │                    ┌─────────────────┐                                           │
//! │                    │  Host Imports   │                                           │
//! │                    │  emit_message   │──────────▶ MessageStream                 │
//! │                    │  http_request   │                                           │
//! │                    │  log, etc.      │                                           │
//! │                    └─────────────────┘                                           │
//! └─────────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Key Design Decisions
//!
//! 1. **Fresh Instance Per Callback** (NEAR Pattern) - Full isolation, no shared mutable state
//! 2. **Host Manages Infrastructure** - HTTP server, polling, timing in Rust
//! 3. **WASM Defines Behavior** - Callbacks for events, message parsing, response handling
//! 4. **Reuse Tool Runtime** - Share Wasmtime engine, extend capabilities
//!
//! # Security Model
//!
//! | Threat | Mitigation |
//! |--------|------------|
//! | Path hijacking | `allowed_paths` restricts registrable endpoints |
//! | Token exposure | Injected at host boundary, WASM never sees |
//! | State pollution | Fresh instance per callback |
//! | Workspace escape | Paths prefixed with `channels/<name>/` |
//! | Message spam | Rate limiting on `emit_message` |
//! | Resource exhaustion | Fuel metering, memory limits, callback timeout |
//! | Polling abuse | Minimum 30s interval enforced |
//!
//! # Example Usage
//!
//! ```ignore
//! use ironclaw::channels::wasm::{WasmChannelLoader, WasmChannelRuntime};
//!
//! // Create runtime (can share engine with tool runtime)
//! let runtime = WasmChannelRuntime::new(config)?;
//!
//! // Load channels from directory
//! let loader = WasmChannelLoader::new(runtime, pairing_store, settings_store, owner_scope_id);
//! let channels = loader.load_from_dir(Path::new("~/.ironclaw/channels/")).await?;
//!
//! // Add to channel manager
//! for channel in channels {
//!     manager.add(Box::new(channel));
//! }
//! ```

mod bundled;
mod capabilities;
mod error;
#[cfg(feature = "wasm-sandbox")]
mod host;
#[cfg(feature = "wasm-sandbox")]
mod loader;
#[cfg(feature = "wasm-sandbox")]
mod router;
#[cfg(feature = "wasm-sandbox")]
mod runtime;
mod schema;
#[cfg(feature = "wasm-sandbox")]
pub mod setup;
#[allow(dead_code)]
pub(crate) mod storage;
#[cfg(feature = "wasm-sandbox")]
mod telegram_host_config;
#[cfg(feature = "wasm-sandbox")]
mod wrapper;

// Core types
pub use bundled::{available_channel_names, bundled_channel_names, install_bundled_channel};
pub use capabilities::{ChannelCapabilities, EmitRateLimitConfig, HttpEndpointConfig, PollConfig};
pub use error::WasmChannelError;
#[cfg(feature = "wasm-sandbox")]
pub use host::{ChannelEmitRateLimiter, ChannelHostState, EmittedMessage};
#[cfg(feature = "wasm-sandbox")]
pub use loader::{
    DiscoveredChannel, LoadResults, LoadedChannel, WasmChannelLoader, default_channels_dir,
    discover_channels,
};
#[cfg(feature = "wasm-sandbox")]
pub use router::{RegisteredEndpoint, WasmChannelRouter, create_wasm_channel_router};
#[cfg(feature = "wasm-sandbox")]
pub use runtime::{PreparedChannelModule, WasmChannelRuntime, WasmChannelRuntimeConfig};
pub use schema::{
    ChannelCapabilitiesFile, ChannelConfig, SecretSetupSchema, SetupSchema, WebhookSchema,
};
#[cfg(feature = "wasm-sandbox")]
pub(crate) use setup::is_reserved_wasm_channel_name;
#[cfg(feature = "wasm-sandbox")]
pub use setup::{WasmChannelSetup, inject_channel_credentials, setup_wasm_channels};
#[cfg(feature = "wasm-sandbox")]
pub(crate) use telegram_host_config::{TELEGRAM_CHANNEL_NAME, bot_username_setting_key};
#[cfg(feature = "wasm-sandbox")]
pub use wrapper::{HttpResponse, SharedWasmChannel, WasmChannel};

// ---------------------------------------------------------------------------
// Stub types when `wasm-sandbox` feature is disabled (e.g. armv7 builds).
// ---------------------------------------------------------------------------

#[cfg(not(feature = "wasm-sandbox"))]
mod stubs {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::capabilities::ChannelCapabilities;
    use super::error::WasmChannelError;
    use super::schema::ChannelConfig;
    use crate::tools::wasm::ResourceLimits;

    pub const TELEGRAM_CHANNEL_NAME: &str = "telegram";

    pub fn bot_username_setting_key(_: &str) -> String {
        String::new()
    }

    pub(crate) fn is_reserved_wasm_channel_name(_name: &str) -> bool {
        false
    }

    /// Stub runtime config.
    #[derive(Debug, Clone, Default)]
    pub struct WasmChannelRuntimeConfig {
        pub default_limits: ResourceLimits,
    }

    impl WasmChannelRuntimeConfig {
        pub fn for_testing() -> Self {
            Self::default()
        }
    }

    /// Stub channel runtime.
    #[derive(Debug)]
    pub struct WasmChannelRuntime;

    impl WasmChannelRuntime {
        pub fn new(_config: WasmChannelRuntimeConfig) -> Result<Self, WasmChannelError> {
            Err(WasmChannelError::Compilation(
                "WASM support is not compiled (wasm-sandbox feature disabled)".to_string(),
            ))
        }
    }

    /// Stub prepared channel module.
    #[derive(Debug)]
    pub struct PreparedChannelModule;

    /// Stub channel loader.
    pub struct WasmChannelLoader;

    impl WasmChannelLoader {
        pub fn new(
            _runtime: Arc<WasmChannelRuntime>,
            _pairing_store: Arc<crate::pairing::PairingStore>,
            _settings_store: Option<Arc<dyn crate::db::SettingsStore>>,
            _owner_scope_id: String,
        ) -> Self {
            Self
        }

        pub fn with_secrets_store(
            self,
            _store: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
        ) -> Self {
            self
        }

        pub async fn load_from_dir(
            &self,
            _dir: &Path,
        ) -> Result<Vec<LoadedChannel>, WasmChannelError> {
            Ok(Vec::new())
        }

        pub async fn load_from_files(
            &self,
            _name: &str,
            _wasm_path: &Path,
            _cap_path: Option<&Path>,
        ) -> Result<LoadedChannel, WasmChannelError> {
            Err(WasmChannelError::Compilation(
                "WASM support not compiled".into(),
            ))
        }
    }

    /// Stub discovered channel.
    #[derive(Debug)]
    pub struct DiscoveredChannel {
        pub name: String,
        pub wasm_path: PathBuf,
        pub capabilities_path: Option<PathBuf>,
    }

    /// Stub load results.
    #[derive(Debug)]
    pub struct LoadResults {
        pub loaded: Vec<String>,
        pub errors: Vec<(String, WasmChannelError)>,
    }

    /// Stub loaded channel.
    pub struct LoadedChannel {
        pub name: String,
        pub channel: WasmChannel,
    }

    impl LoadedChannel {
        pub fn name(&self) -> &str {
            &self.name
        }
        pub fn webhook_secret_header(&self) -> Option<&str> {
            None
        }
        pub fn webhook_secret_name(&self) -> String {
            format!("{}_webhook_secret", self.name)
        }
        pub fn signature_key_secret_name(&self) -> Option<String> {
            None
        }
        pub fn hmac_secret_name(&self) -> Option<String> {
            None
        }
    }

    /// Stub WASM channel (not constructible without the feature).
    pub struct WasmChannel;

    impl WasmChannel {
        pub async fn set_credential(&self, _name: &str, _value: String) {}
        pub fn capabilities(&self) -> ChannelCapabilities {
            ChannelCapabilities::default()
        }
        pub async fn update_config(&self, _updates: HashMap<String, serde_json::Value>) {}
        pub async fn call_on_start(&self) -> Result<ChannelConfig, WasmChannelError> {
            Err(WasmChannelError::Compilation(
                "wasm-sandbox feature not enabled".to_string(),
            ))
        }
        pub fn with_owner_actor_id(self, _owner_actor_id: Option<String>) -> Self {
            self
        }
        pub fn channel_name(&self) -> &str {
            ""
        }
    }

    /// Stub shared WASM channel.
    pub struct SharedWasmChannel;

    impl SharedWasmChannel {
        pub fn new(_channel: Arc<WasmChannel>) -> Self {
            Self
        }
    }

    #[async_trait::async_trait]
    impl crate::channels::Channel for SharedWasmChannel {
        fn name(&self) -> &str {
            "wasm-stub"
        }

        async fn start(
            &self,
        ) -> Result<crate::channels::MessageStream, crate::error::ChannelError> {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
        }

        async fn respond(
            &self,
            _msg: &crate::channels::IncomingMessage,
            _response: crate::channels::OutgoingResponse,
        ) -> Result<(), crate::error::ChannelError> {
            Ok(())
        }

        async fn health_check(&self) -> Result<(), crate::error::ChannelError> {
            Ok(())
        }
    }

    /// Stub registered endpoint.
    #[derive(Debug, Clone)]
    pub struct RegisteredEndpoint {
        pub path: String,
        pub channel_name: String,
        pub methods: Vec<String>,
        pub require_secret: bool,
    }

    /// Stub channel router.
    #[derive(Default)]
    pub struct WasmChannelRouter;

    impl WasmChannelRouter {
        pub fn new() -> Self {
            Self
        }

        pub async fn get_channel_for_path(&self, _path: &str) -> Option<Arc<WasmChannel>> {
            None
        }

        pub async fn register(
            &self,
            _channel: Arc<WasmChannel>,
            _endpoints: Vec<RegisteredEndpoint>,
            _secret: Option<String>,
            _secret_header: Option<String>,
        ) {
        }

        pub async fn register_signature_key(
            &self,
            _channel_name: &str,
            _public_key_hex: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        pub async fn register_hmac_secret(&self, _channel_name: &str, _secret: &str) {}
        pub async fn update_secret(&self, _channel_name: &str, _secret: String) {}
    }

    /// Stub HTTP response.
    #[derive(Debug)]
    pub struct HttpResponse {
        pub status: u16,
        pub body: String,
    }

    /// Stub channel host state.
    #[derive(Debug)]
    pub struct ChannelHostState;

    /// Stub emitted message.
    #[derive(Debug)]
    pub struct EmittedMessage;

    /// Stub emit rate limiter.
    pub struct ChannelEmitRateLimiter;

    pub fn default_channels_dir() -> PathBuf {
        crate::bootstrap::ironclaw_base_dir().join("channels")
    }

    pub async fn discover_channels(
        _dir: &Path,
    ) -> Result<HashMap<String, DiscoveredChannel>, std::io::Error> {
        Ok(HashMap::new())
    }

    pub fn create_wasm_channel_router() -> WasmChannelRouter {
        WasmChannelRouter::new()
    }

    /// Stub channel setup result.
    pub struct WasmChannelSetup {
        pub channels: Vec<(String, Box<dyn crate::channels::Channel>)>,
        pub channel_names: Vec<String>,
        pub webhook_routes: Option<axum::Router>,
        pub wasm_channel_runtime: Arc<WasmChannelRuntime>,
        pub pairing_store: Arc<crate::pairing::PairingStore>,
        pub wasm_channel_router: Arc<WasmChannelRouter>,
    }

    pub async fn setup_wasm_channels(
        _config: &crate::config::Config,
        _secrets_store: &Option<Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
        _extension_manager: Option<&Arc<crate::extensions::ExtensionManager>>,
        _database: Option<&Arc<dyn crate::db::Database>>,
        _registered_channel_names: &[String],
        _ownership_cache: Arc<crate::ownership::OwnershipCache>,
    ) -> Option<WasmChannelSetup> {
        tracing::warn!(
            "WASM channels are disabled (wasm-sandbox feature not enabled); skipping setup"
        );
        None
    }

    pub async fn inject_channel_credentials(
        _secrets: &dyn crate::secrets::SecretsStore,
        _channels: &[Arc<WasmChannel>],
        _user_id: &str,
    ) {
    }
}

#[cfg(not(feature = "wasm-sandbox"))]
pub(crate) use stubs::is_reserved_wasm_channel_name;
#[cfg(not(feature = "wasm-sandbox"))]
pub use stubs::*;
