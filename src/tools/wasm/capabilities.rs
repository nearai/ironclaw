//! Extended capabilities for WASM sandbox.
//!
//! Defines the capability system that controls what a WASM tool can do.
//! All capabilities are opt-in; tools have NO access by default.
//!
//! # Capability Types
//!
//! - **Workspace**: Read/write files from the agent's workspace
//! - **HTTP**: Make HTTP requests to allowlisted endpoints
//! - **ToolInvoke**: Call other tools via aliases
//! - **Secrets**: Check if secrets exist (never read values)
//! - **WebSocket**: Persistent WebSocket connections with connection pooling

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::secrets::CredentialMapping;

/// All capabilities that can be granted to a WASM tool.
///
/// By default, all capabilities are `None` (disabled).
/// Each must be explicitly granted.
#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    /// Read files from workspace.
    pub workspace_read: Option<WorkspaceCapability>,
    /// Write files to workspace (separate from read for least-privilege).
    pub workspace_write: Option<WorkspaceWriteCapability>,
    /// Make HTTP requests.
    pub http: Option<HttpCapability>,
    /// Invoke other tools.
    pub tool_invoke: Option<ToolInvokeCapability>,
    /// Check if secrets exist.
    pub secrets: Option<SecretsCapability>,
    /// WebSocket connections.
    pub websocket: Option<WebSocketCapability>,
}

impl Capabilities {
    /// Create capabilities with no permissions.
    pub fn none() -> Self {
        Self::default()
    }

    /// Enable workspace read with the given allowed prefixes.
    pub fn with_workspace_read(mut self, prefixes: Vec<String>) -> Self {
        self.workspace_read = Some(WorkspaceCapability {
            allowed_prefixes: prefixes,
            reader: None,
            writer: None,
        });
        self
    }

    /// Enable HTTP requests with the given configuration.
    pub fn with_http(mut self, http: HttpCapability) -> Self {
        self.http = Some(http);
        self
    }

    /// Enable tool invocation with the given aliases.
    pub fn with_tool_invoke(mut self, aliases: HashMap<String, String>) -> Self {
        self.tool_invoke = Some(ToolInvokeCapability {
            aliases,
            rate_limit: RateLimitConfig::default(),
        });
        self
    }

    /// Enable secret existence checks.
    pub fn with_secrets(mut self, allowed: Vec<String>) -> Self {
        self.secrets = Some(SecretsCapability {
            allowed_names: allowed,
        });
        self
    }

    /// Enable WebSocket connections.
    pub fn with_websocket(mut self, allowlist: Vec<WebSocketEndpoint>) -> Self {
        self.websocket = Some(WebSocketCapability::new(allowlist));
        self
    }
}

/// Workspace read capability configuration.
#[derive(Clone, Default)]
pub struct WorkspaceCapability {
    /// Allowed path prefixes (e.g., ["context/", "daily/"]).
    /// Empty means all paths allowed (within safety constraints).
    pub allowed_prefixes: Vec<String>,
    /// Function to actually read from workspace.
    /// This is injected by the runtime to avoid coupling to workspace impl.
    pub reader: Option<Arc<dyn WorkspaceReader>>,
    /// Function to write to workspace (optional).
    pub writer: Option<Arc<dyn WorkspaceWriter>>,
}

impl std::fmt::Debug for WorkspaceCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceCapability")
            .field("allowed_prefixes", &self.allowed_prefixes)
            .field("reader", &self.reader.is_some())
            .field("writer", &self.writer.is_some())
            .finish()
    }
}

/// Trait for reading from workspace (allows mocking in tests).
pub trait WorkspaceReader: Send + Sync {
    fn read(&self, path: &str) -> Option<String>;
}

/// Trait for writing to workspace (allows mocking in tests).
pub trait WorkspaceWriter: Send + Sync {
    fn write(&self, path: &str, content: &str) -> Result<(), String>;
}

/// Workspace write capability configuration (separate from read).
#[derive(Clone, Default)]
pub struct WorkspaceWriteCapability {
    /// Allowed path prefixes for writing.
    pub allowed_prefixes: Vec<String>,
    /// Function to actually write to workspace.
    pub writer: Option<Arc<dyn WorkspaceWriter>>,
}

impl std::fmt::Debug for WorkspaceWriteCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceWriteCapability")
            .field("allowed_prefixes", &self.allowed_prefixes)
            .field("writer", &self.writer.is_some())
            .finish()
    }
}

/// HTTP request capability configuration.
#[derive(Debug, Clone)]
pub struct HttpCapability {
    /// Allowed endpoint patterns.
    pub allowlist: Vec<EndpointPattern>,
    /// Credential mappings (secret name -> injection location).
    pub credentials: HashMap<String, CredentialMapping>,
    /// Rate limiting configuration.
    pub rate_limit: RateLimitConfig,
    /// Maximum request body size in bytes.
    pub max_request_bytes: usize,
    /// Maximum response body size in bytes.
    pub max_response_bytes: usize,
    /// Request timeout.
    pub timeout: Duration,
}

impl Default for HttpCapability {
    fn default() -> Self {
        Self {
            allowlist: Vec::new(),
            credentials: HashMap::new(),
            rate_limit: RateLimitConfig::default(),
            max_request_bytes: 1024 * 1024,       // 1 MB
            max_response_bytes: 10 * 1024 * 1024, // 10 MB
            timeout: Duration::from_secs(30),
        }
    }
}

impl HttpCapability {
    /// Create a new HTTP capability with an allowlist.
    pub fn new(allowlist: Vec<EndpointPattern>) -> Self {
        Self {
            allowlist,
            ..Default::default()
        }
    }

    /// Add a credential mapping.
    pub fn with_credential(mut self, name: impl Into<String>, mapping: CredentialMapping) -> Self {
        self.credentials.insert(name.into(), mapping);
        self
    }

    /// Set rate limiting.
    pub fn with_rate_limit(mut self, rate_limit: RateLimitConfig) -> Self {
        self.rate_limit = rate_limit;
        self
    }

    /// Set request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set max request body size.
    pub fn with_max_request_bytes(mut self, bytes: usize) -> Self {
        self.max_request_bytes = bytes;
        self
    }

    /// Set max response body size.
    pub fn with_max_response_bytes(mut self, bytes: usize) -> Self {
        self.max_response_bytes = bytes;
        self
    }
}

/// Pattern for matching allowed HTTP endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointPattern {
    /// Hostname pattern (e.g., "api.example.com", "*.example.com").
    pub host: String,
    /// Port constraint (optional, None = any port).
    #[serde(default)]
    pub port: Option<u16>,
    /// Path prefix (e.g., "/v1/", "/api/").
    pub path_prefix: Option<String>,
    /// Allowed HTTP methods (empty = all methods allowed).
    pub methods: Vec<String>,
}

impl EndpointPattern {
    /// Create a pattern for a specific host.
    pub fn host(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: None,
            path_prefix: None,
            methods: Vec::new(),
        }
    }

    /// Add a path prefix constraint.
    pub fn with_path_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.path_prefix = Some(prefix.into());
        self
    }

    /// Restrict to specific HTTP methods.
    pub fn with_methods(mut self, methods: Vec<String>) -> Self {
        self.methods = methods;
        self
    }

    /// Add a port constraint.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Check if this pattern matches a URL and method.
    pub fn matches(&self, url_host: &str, url_path: &str, method: &str) -> bool {
        // Check host
        if !self.host_matches(url_host) {
            return false;
        }

        // Check path prefix
        if let Some(ref prefix) = self.path_prefix
            && !url_path.starts_with(prefix)
        {
            return false;
        }

        // Check method
        if !self.methods.is_empty() {
            let method_upper = method.to_uppercase();
            if !self
                .methods
                .iter()
                .any(|m| m.to_uppercase() == method_upper)
            {
                return false;
            }
        }

        true
    }

    /// Check if this pattern matches a URL, port, and method.
    pub fn matches_with_port(
        &self,
        url_host: &str,
        url_port: Option<u16>,
        url_path: &str,
        method: &str,
    ) -> bool {
        if !self.host_matches(url_host) {
            return false;
        }

        // Enforce port constraint when specified
        if let Some(required_port) = self.port
            && url_port != Some(required_port)
        {
            return false;
        }

        if let Some(ref prefix) = self.path_prefix
            && !url_path.starts_with(prefix)
        {
            return false;
        }

        if !self.methods.is_empty() {
            let method_upper = method.to_uppercase();
            if !self
                .methods
                .iter()
                .any(|m| m.to_uppercase() == method_upper)
            {
                return false;
            }
        }

        true
    }

    /// Check if host pattern matches (public for allowlist validation).
    pub fn host_matches(&self, url_host: &str) -> bool {
        if self.host == url_host {
            return true;
        }

        // Support wildcard: *.example.com matches sub.example.com
        if let Some(suffix) = self.host.strip_prefix("*.")
            && url_host.ends_with(suffix)
            && url_host.len() > suffix.len()
        {
            // Ensure there's a dot before the suffix (or it's the whole thing)
            let prefix = &url_host[..url_host.len() - suffix.len()];
            if prefix.ends_with('.') || prefix.is_empty() {
                return true;
            }
        }

        false
    }
}

/// Tool invocation capability.
#[derive(Debug, Clone, Default)]
pub struct ToolInvokeCapability {
    /// Mapping from alias to real tool name.
    /// WASM calls tools by alias, never by real name.
    pub aliases: HashMap<String, String>,
    /// Rate limiting for tool calls.
    pub rate_limit: RateLimitConfig,
}

impl ToolInvokeCapability {
    /// Create with a set of aliases.
    pub fn new(aliases: HashMap<String, String>) -> Self {
        Self {
            aliases,
            rate_limit: RateLimitConfig::default(),
        }
    }

    /// Resolve an alias to a real tool name.
    pub fn resolve_alias(&self, alias: &str) -> Option<&str> {
        self.aliases.get(alias).map(|s| s.as_str())
    }
}

/// Secrets capability (existence check only).
#[derive(Debug, Clone, Default)]
pub struct SecretsCapability {
    /// Secret names this tool can check existence of.
    /// Supports glob: "openai_*" matches "openai_key", "openai_org".
    pub allowed_names: Vec<String>,
}

impl SecretsCapability {
    /// Check if a secret name is allowed.
    pub fn is_allowed(&self, name: &str) -> bool {
        for pattern in &self.allowed_names {
            if pattern == name {
                return true;
            }
            if let Some(prefix) = pattern.strip_suffix('*')
                && name.starts_with(prefix)
            {
                return true;
            }
        }
        false
    }
}

/// A pooled WebSocket connection entry.
///
/// Stores the raw split sink/stream alongside metadata for lifecycle management.
/// The pool key is the map key in `WsConnectionPool::connections`.
///
/// Includes a dedicated tokio runtime because WebSocket I/O resources are bound
/// to the reactor that created them. This runtime must outlive the connection.
pub struct PooledWsEntry {
    /// Sender half of the WebSocket connection.
    pub sink: Mutex<
        futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            tokio_tungstenite::tungstenite::Message,
        >,
    >,
    /// Receiver half of the WebSocket connection.
    pub stream: Mutex<
        futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
    >,
    /// Dedicated tokio runtime for driving this connection's I/O.
    /// WebSocket streams are bound to the reactor that created them,
    /// so we must keep the runtime alive alongside the connection.
    ///
    /// Wrapped in `Option` so we can `take()` it in `Drop` to call
    /// `shutdown_background()` (avoids panic when dropped from async context).
    pub runtime: Mutex<Option<tokio::runtime::Runtime>>,
    /// URL this connection is connected to.
    pub url: String,
    /// When the connection was last used (for TTL eviction).
    pub last_used: Mutex<Instant>,
}

impl PooledWsEntry {
    /// Borrow the runtime for blocking I/O operations.
    pub fn with_runtime<T>(
        &self,
        f: impl FnOnce(&tokio::runtime::Runtime) -> T,
    ) -> Result<T, String> {
        let guard = self
            .runtime
            .lock()
            .map_err(|_| "Failed to lock pooled runtime".to_string())?;
        let rt = guard
            .as_ref()
            .ok_or_else(|| "Pooled runtime already shut down".to_string())?;
        Ok(f(rt))
    }
}

impl Drop for PooledWsEntry {
    fn drop(&mut self) {
        // Take the runtime out and shut it down in the background to avoid
        // panic when dropped from an async context.
        if let Ok(mut guard) = self.runtime.lock()
            && let Some(rt) = guard.take()
        {
            rt.shutdown_background();
        }
    }
}

impl std::fmt::Debug for PooledWsEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledWsEntry")
            .field("url", &self.url)
            .finish()
    }
}

/// Shared WebSocket connection pool that persists across WASM invocations.
///
/// Connections are keyed by a caller-provided pool key (e.g., session ID)
/// and survive beyond individual tool executions. This enables stateful
/// protocols where server-side state is tied to connection lifetime.
///
/// Thread-safe: the outer `Mutex` protects the map for insert/remove;
/// individual entry sink/stream have their own `Mutex` for concurrent
/// send/recv (though in practice WASM calls are sequential).
#[derive(Debug)]
pub struct WsConnectionPool {
    connections: Mutex<HashMap<String, Arc<PooledWsEntry>>>,
    /// Time-to-live for idle connections (default: 5 minutes).
    pub idle_ttl: Duration,
    /// Maximum number of pooled connections (default: 16).
    max_size: usize,
}

/// Default maximum number of connections in the pool.
const DEFAULT_POOL_MAX_SIZE: usize = 16;

impl Default for WsConnectionPool {
    fn default() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            idle_ttl: Duration::from_secs(300),
            max_size: DEFAULT_POOL_MAX_SIZE,
        }
    }
}

impl WsConnectionPool {
    /// Create a new pool with the given idle TTL.
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            idle_ttl: ttl,
            max_size: DEFAULT_POOL_MAX_SIZE,
        }
    }

    /// Get an existing connection by pool key, if it exists and is not expired.
    ///
    /// Expired entries are removed lazily on access.
    pub fn get(&self, key: &str) -> Option<Arc<PooledWsEntry>> {
        let mut conns = self.connections.lock().ok()?;

        if let Some(entry) = conns.get(key) {
            let last = entry.last_used.lock().ok().map(|t| *t);
            if let Some(last_used) = last.filter(|t| t.elapsed() > self.idle_ttl) {
                let _ = last_used; // used for the filter above
                tracing::debug!(pool_key = %key, "Evicting expired pooled WebSocket connection");
                conns.remove(key);
                return None;
            }
            // Touch the last-used timestamp
            if let Ok(mut lu) = entry.last_used.lock() {
                *lu = Instant::now();
            }
            Some(Arc::clone(entry))
        } else {
            None
        }
    }

    /// Insert a connection into the pool.
    ///
    /// If a connection already exists for this key, the old one is replaced
    /// (and dropped, closing the underlying WebSocket). If the pool is at
    /// capacity, expired entries are evicted first; if still full, the
    /// least-recently-used entry is evicted.
    pub fn insert(&self, key: String, entry: Arc<PooledWsEntry>) {
        if let Ok(mut conns) = self.connections.lock() {
            // If this key already exists, it will be replaced (no size change)
            if !conns.contains_key(&key) && conns.len() >= self.max_size {
                // Evict expired entries first
                let ttl = self.idle_ttl;
                conns.retain(|_, e| {
                    e.last_used
                        .lock()
                        .ok()
                        .map(|t| t.elapsed() <= ttl)
                        .unwrap_or(false)
                });

                // If still at capacity, evict the least-recently-used entry
                if conns.len() >= self.max_size {
                    let lru_key = conns
                        .iter()
                        .filter_map(|(k, e)| e.last_used.lock().ok().map(|t| (k.clone(), *t)))
                        .min_by_key(|(_, t)| *t)
                        .map(|(k, _)| k);
                    if let Some(k) = lru_key {
                        tracing::debug!(pool_key = %k, "Evicting LRU connection to stay within pool max_size");
                        conns.remove(&k);
                    }
                }
            }
            conns.insert(key, entry);
        }
    }

    /// Remove a connection from the pool by key.
    pub fn remove(&self, key: &str) -> Option<Arc<PooledWsEntry>> {
        self.connections.lock().ok()?.remove(key)
    }

    /// Remove a connection from the pool by pointer equality.
    ///
    /// Returns the removed pool key when an entry was found.
    pub fn remove_entry(&self, target: &Arc<PooledWsEntry>) -> Option<String> {
        let mut conns = self.connections.lock().ok()?;
        let key = conns
            .iter()
            .find_map(|(key, entry)| Arc::ptr_eq(entry, target).then_some(key.clone()))?;
        conns.remove(&key);
        Some(key)
    }

    /// Evict all connections that have exceeded the idle TTL.
    pub fn evict_expired(&self) {
        if let Ok(mut conns) = self.connections.lock() {
            let ttl = self.idle_ttl;
            conns.retain(|key, entry| {
                let keep = entry
                    .last_used
                    .lock()
                    .ok()
                    .map(|t| t.elapsed() <= ttl)
                    .unwrap_or(false);
                if !keep {
                    tracing::debug!(pool_key = %key, "Evicting expired pooled WebSocket connection");
                }
                keep
            });
        }
    }

    /// Number of connections currently in the pool.
    pub fn len(&self) -> usize {
        self.connections.lock().ok().map(|c| c.len()).unwrap_or(0)
    }

    /// Whether the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// WebSocket capability for persistent connections.
#[derive(Debug, Clone)]
pub struct WebSocketCapability {
    /// Allowed WebSocket endpoint patterns.
    pub allowlist: Vec<WebSocketEndpoint>,
    /// Rate limiting configuration.
    pub rate_limit: RateLimitConfig,
    /// Maximum message size in bytes.
    pub max_message_bytes: usize,
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Read timeout.
    pub read_timeout: Duration,
    /// Shared connection pool for `ws-connect-pooled`.
    /// `None` means pooling is disabled (backwards-compatible default).
    pub connection_pool: Option<Arc<WsConnectionPool>>,
}

impl Default for WebSocketCapability {
    fn default() -> Self {
        Self {
            allowlist: Vec::new(),
            rate_limit: RateLimitConfig::default(),
            max_message_bytes: 1024 * 1024, // 1 MB
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
            connection_pool: None,
        }
    }
}

impl WebSocketCapability {
    /// Create a new WebSocket capability with an allowlist.
    pub fn new(allowlist: Vec<WebSocketEndpoint>) -> Self {
        Self {
            allowlist,
            ..Default::default()
        }
    }

    /// Enable connection pooling with default TTL (5 minutes).
    pub fn with_pool(mut self) -> Self {
        self.connection_pool = Some(Arc::new(WsConnectionPool::default()));
        self
    }

    /// Enable connection pooling with a custom TTL.
    pub fn with_pool_ttl(mut self, ttl: Duration) -> Self {
        self.connection_pool = Some(Arc::new(WsConnectionPool::with_ttl(ttl)));
        self
    }

    /// Check if a WebSocket URL is allowed.
    pub fn is_allowed(&self, url: &str) -> bool {
        let parsed = match url::Url::parse(url) {
            Ok(u) => u,
            Err(_) => return false,
        };

        let host = parsed.host_str().unwrap_or("");
        let scheme = parsed.scheme();
        let port = parsed.port();

        for endpoint in &self.allowlist {
            if endpoint.matches(host, scheme, port) {
                return true;
            }
        }

        false
    }
}

/// Pattern for matching allowed WebSocket endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketEndpoint {
    /// Hostname pattern (e.g., "localhost", "127.0.0.1", "*.example.com").
    pub host: String,
    /// Port (optional, None = any port).
    pub port: Option<u16>,
}

impl WebSocketEndpoint {
    /// Create a pattern for a specific host.
    pub fn host(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: None,
        }
    }

    /// Add a port constraint.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Check if this endpoint matches a host, scheme, and port.
    pub fn matches(&self, url_host: &str, _scheme: &str, url_port: Option<u16>) -> bool {
        if !self.host_matches(url_host) {
            return false;
        }

        // Enforce port constraint when specified in the endpoint pattern.
        // `None` on the endpoint means "any port is allowed".
        if let Some(required_port) = self.port
            && url_port != Some(required_port)
        {
            return false;
        }

        true
    }

    /// Check if host pattern matches.
    pub fn host_matches(&self, url_host: &str) -> bool {
        if self.host == url_host {
            return true;
        }

        // Support wildcard: *.example.com matches sub.example.com
        if let Some(suffix) = self.host.strip_prefix("*.")
            && url_host.ends_with(suffix)
            && url_host.len() > suffix.len()
        {
            let prefix = &url_host[..url_host.len() - suffix.len()];
            if prefix.ends_with('.') || prefix.is_empty() {
                return true;
            }
        }

        false
    }
}

/// Rate limiting configuration.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum requests per minute.
    pub requests_per_minute: u32,
    /// Maximum requests per hour.
    pub requests_per_hour: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: 60,
            requests_per_hour: 1000,
        }
    }
}

impl RateLimitConfig {
    /// Create a restrictive rate limit.
    pub fn restrictive() -> Self {
        Self {
            requests_per_minute: 10,
            requests_per_hour: 100,
        }
    }

    /// Create a permissive rate limit.
    pub fn permissive() -> Self {
        Self {
            requests_per_minute: 120,
            requests_per_hour: 5000,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::wasm::capabilities::{
        Capabilities, EndpointPattern, SecretsCapability, WebSocketCapability, WebSocketEndpoint,
        WsConnectionPool,
    };
    use std::time::Duration;

    #[test]
    fn test_capabilities_default_is_none() {
        let caps = Capabilities::default();
        assert!(caps.workspace_read.is_none());
        assert!(caps.http.is_none());
        assert!(caps.tool_invoke.is_none());
        assert!(caps.secrets.is_none());
    }

    #[test]
    fn test_endpoint_pattern_exact_host() {
        let pattern = EndpointPattern::host("api.example.com");

        assert!(pattern.matches("api.example.com", "/", "GET"));
        assert!(!pattern.matches("other.example.com", "/", "GET"));
    }

    #[test]
    fn test_endpoint_pattern_wildcard_host() {
        let pattern = EndpointPattern::host("*.example.com");

        assert!(pattern.matches("api.example.com", "/", "GET"));
        assert!(pattern.matches("sub.api.example.com", "/", "GET"));
        assert!(!pattern.matches("example.com", "/", "GET"));
        assert!(!pattern.matches("notexample.com", "/", "GET"));
    }

    #[test]
    fn test_endpoint_pattern_path_prefix() {
        let pattern = EndpointPattern::host("api.example.com").with_path_prefix("/v1/");

        assert!(pattern.matches("api.example.com", "/v1/users", "GET"));
        assert!(pattern.matches("api.example.com", "/v1/", "GET"));
        assert!(!pattern.matches("api.example.com", "/v2/users", "GET"));
        assert!(!pattern.matches("api.example.com", "/", "GET"));
    }

    #[test]
    fn test_endpoint_pattern_methods() {
        let pattern = EndpointPattern::host("api.example.com")
            .with_methods(vec!["GET".to_string(), "POST".to_string()]);

        assert!(pattern.matches("api.example.com", "/", "GET"));
        assert!(pattern.matches("api.example.com", "/", "get")); // case insensitive
        assert!(pattern.matches("api.example.com", "/", "POST"));
        assert!(!pattern.matches("api.example.com", "/", "DELETE"));
    }

    #[test]
    fn test_secrets_capability_exact_match() {
        let cap = SecretsCapability {
            allowed_names: vec!["openai_key".to_string()],
        };

        assert!(cap.is_allowed("openai_key"));
        assert!(!cap.is_allowed("anthropic_key"));
    }

    #[test]
    fn test_secrets_capability_glob() {
        let cap = SecretsCapability {
            allowed_names: vec!["openai_*".to_string()],
        };

        assert!(cap.is_allowed("openai_key"));
        assert!(cap.is_allowed("openai_org"));
        assert!(!cap.is_allowed("anthropic_key"));
    }

    #[test]
    fn test_capabilities_builder() {
        let caps = Capabilities::none()
            .with_workspace_read(vec!["context/".to_string()])
            .with_secrets(vec!["test_*".to_string()]);

        assert!(caps.workspace_read.is_some());
        assert!(caps.secrets.is_some());
        assert!(caps.http.is_none());
    }

    // --- WebSocketEndpoint tests ---

    #[test]
    fn test_ws_endpoint_exact_host_any_port() {
        let ep = WebSocketEndpoint::host("localhost");
        assert!(ep.matches("localhost", "ws", Some(9222)));
        assert!(ep.matches("localhost", "ws", Some(8080)));
        assert!(ep.matches("localhost", "ws", None));
        assert!(!ep.matches("other.host", "ws", Some(9222)));
    }

    #[test]
    fn test_ws_endpoint_port_enforcement() {
        let ep = WebSocketEndpoint::host("localhost").with_port(9222);
        assert!(ep.matches("localhost", "ws", Some(9222)));
        assert!(!ep.matches("localhost", "ws", Some(8080)));
        // URL without explicit port → does not match a required port
        assert!(!ep.matches("localhost", "ws", None));
    }

    #[test]
    fn test_ws_endpoint_wildcard_host() {
        let ep = WebSocketEndpoint::host("*.example.com").with_port(443);
        assert!(ep.matches("api.example.com", "wss", Some(443)));
        assert!(!ep.matches("api.example.com", "wss", Some(8080)));
        assert!(!ep.matches("example.com", "wss", Some(443)));
    }

    // --- WebSocketCapability.is_allowed() tests ---

    #[test]
    fn test_ws_capability_allowed_with_port() {
        let cap =
            WebSocketCapability::new(vec![WebSocketEndpoint::host("localhost").with_port(9222)]);
        assert!(cap.is_allowed("ws://localhost:9222/devtools"));
        assert!(!cap.is_allowed("ws://localhost:8080/devtools"));
        assert!(!cap.is_allowed("ws://evil.com:9222/devtools"));
    }

    #[test]
    fn test_ws_capability_allowed_any_port() {
        let cap = WebSocketCapability::new(vec![WebSocketEndpoint::host("localhost")]);
        assert!(cap.is_allowed("ws://localhost:9222/devtools"));
        assert!(cap.is_allowed("ws://localhost:1234/other"));
    }

    #[test]
    fn test_ws_capability_denied_empty_allowlist() {
        let cap = WebSocketCapability::new(vec![]);
        assert!(!cap.is_allowed("ws://localhost:9222/devtools"));
    }

    #[test]
    fn test_ws_capability_invalid_url() {
        let cap = WebSocketCapability::new(vec![WebSocketEndpoint::host("localhost")]);
        assert!(!cap.is_allowed("not a url"));
        assert!(!cap.is_allowed(""));
    }

    // --- WsConnectionPool tests ---

    #[test]
    fn test_pool_insert_and_get() {
        let pool = WsConnectionPool::default();
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);

        // We can't create real WebSocket connections in a unit test, but we can
        // test the pool mechanics with a dummy entry.
        // Since PooledWsEntry requires real WS types, test via insert/get/remove
        // semantics by verifying the pool tracks keys correctly.

        // Pool starts empty, get returns None
        assert!(pool.get("session-1").is_none());
    }

    #[test]
    fn test_pool_remove() {
        let pool = WsConnectionPool::default();
        // Remove from empty pool returns None
        assert!(pool.remove("nonexistent").is_none());
    }

    #[test]
    fn test_pool_ttl_eviction() {
        // Create pool with 0-second TTL (everything expires immediately)
        let pool = WsConnectionPool::with_ttl(Duration::from_secs(0));
        assert!(pool.is_empty());

        // Even after evict_expired on empty pool, no panic
        pool.evict_expired();
        assert!(pool.is_empty());
    }

    #[test]
    fn test_pool_len_and_empty() {
        let pool = WsConnectionPool::default();
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn test_pool_with_custom_ttl() {
        let pool = WsConnectionPool::with_ttl(Duration::from_secs(60));
        assert_eq!(pool.idle_ttl, Duration::from_secs(60));
        assert!(pool.is_empty());
    }
}
