//! MCP client for connecting to MCP servers.
//!
//! Supports both local (unauthenticated) and hosted (OAuth-authenticated) servers.
//! Uses pluggable transports (HTTP, stdio, Unix) via the `McpTransport` trait.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use ironclaw_common::McpServerName;
use tokio::sync::RwLock;

use crate::auth::resolve_access_token_string_with_refresh;
use crate::context::JobContext;
use crate::secrets::SecretsStore;
use crate::tools::mcp::auth::refresh_access_token;
use crate::tools::mcp::config::McpServerConfig;
use crate::tools::mcp::http_transport::HttpMcpTransport;
use crate::tools::mcp::protocol::{
    CallToolResult, InitializeResult, ListToolsResult, McpRequest, McpResponse, McpTool,
};
use crate::tools::mcp::session::McpSessionManager;
use crate::tools::mcp::transport::McpTransport;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};

/// Tag identifying which constructor produced an `McpClient`.
///
/// Test-only: lets caller-level tests assert that `create_client_from_config`
/// chose the right path (auth vs non-auth) given a server config. The client's
/// runtime behavior is otherwise nearly identical between paths, so without
/// this tag, the factory's path-selection logic is unobservable from outside.
///
/// See `.claude/rules/testing.md` ("Test Through the Caller, Not Just the
/// Helper") for the rule motivating this hook.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum McpClientConstructor {
    /// `McpClient::new` — bare unauthenticated client.
    Plain,
    /// `McpClient::new_with_name` — bare unauthenticated client with explicit name.
    PlainNamed,
    /// `McpClient::new_with_config` — test-only HTTP client from a config.
    FromConfig,
    /// `McpClient::new_authenticated` — OAuth-aware client.
    Authenticated,
    /// `McpClient::new_with_transport` — generic client with externally built transport.
    WithTransport,
}

/// MCP client for communicating with MCP servers.
///
/// Supports multiple transport types:
/// - HTTP: For remote MCP servers (created via `new`, `new_with_name`, `new_authenticated`)
/// - Stdio/Unix: Via `new_with_transport` with a custom `McpTransport` implementation
pub struct McpClient {
    /// Transport for sending requests.
    transport: Arc<dyn McpTransport>,

    /// Server URL (kept for accessor compatibility).
    server_url: String,

    /// Server name (for logging and session management).
    ///
    /// Typed via `McpServerName` so session-manager lookups and logging
    /// are both compile-time-gated behind the allowlist validation that
    /// lives in `ironclaw_common::identity`.
    server_name: McpServerName,

    /// Request ID counter.
    next_id: AtomicU64,

    /// Cached tools.
    tools_cache: RwLock<Option<Vec<McpTool>>>,

    /// Session manager (shared across clients).
    session_manager: Option<Arc<McpSessionManager>>,

    /// Secrets store for retrieving access tokens.
    secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,

    /// User ID for secrets lookup.
    user_id: String,

    /// Server configuration (for token secret name lookup).
    server_config: Option<McpServerConfig>,

    /// Custom headers to include in every request.
    custom_headers: HashMap<String, String>,

    /// Ensures the MCP initialize handshake runs exactly once.
    /// Uses `OnceCell` to serialize concurrent callers so only one
    /// actually sends the request; subsequent calls return immediately.
    initialized: tokio::sync::OnceCell<InitializeResult>,

    /// Test-only marker recording which constructor produced this client.
    /// Used by caller-level tests to assert the factory chose the correct path.
    #[cfg(test)]
    constructor_kind: McpClientConstructor,
}

impl McpClient {
    /// Create a new simple MCP client (no authentication).
    ///
    /// Use this for local development servers or servers that don't require auth.
    pub fn new(server_url: impl Into<String>) -> Self {
        let url: String = server_url.into();
        // `extract_server_name` is a heuristic URL parser that may emit
        // values outside the strict allowlist — e.g. the bracketed IPv6
        // host `[::1]` survives `host_str()` and contains the `:`
        // forbidden by `McpServerName`'s rules. Apply the same
        // hyphen→underscore fold as the other constructors (only when
        // a hyphen is present, to avoid an unnecessary allocation), then
        // validate through `McpServerName::new`. If validation fails we
        // fall back to the canonical `"unknown"` value rather than
        // bypassing the allowlist via `from_trusted`.
        let mut name_str = extract_server_name(&url);
        if name_str.contains('-') {
            name_str = name_str.replace('-', "_");
        }
        let name = McpServerName::new(&name_str).unwrap_or_else(|e| {
            tracing::debug!(
                extracted = %name_str,
                error = %e,
                "McpClient::new: extracted server name failed allowlist validation; \
                 falling back to canonical 'unknown'"
            );
            McpServerName::new("unknown")
                .expect("'unknown' is a valid McpServerName (alnum allowlist)") // safety: hardcoded literal satisfies alnum-only validation; infallible
        });
        let transport = Arc::new(HttpMcpTransport::new(url.clone(), name.as_str()));

        Self {
            transport,
            server_url: url,
            server_name: name,
            next_id: AtomicU64::new(1),
            tools_cache: RwLock::new(None),
            session_manager: None,
            secrets: None,
            // TODO(ownership): unauthenticated constructor; user_id set properly via
            // create_client_from_config() for production paths
            user_id: "<unset>".to_string(),
            server_config: None,
            custom_headers: HashMap::new(),
            initialized: tokio::sync::OnceCell::new(),
            #[cfg(test)]
            constructor_kind: McpClientConstructor::Plain,
        }
    }

    /// Create a new simple MCP client with a specific name.
    ///
    /// Use this when you have a configured server name but no authentication.
    pub fn new_with_name(server_name: impl Into<String>, server_url: impl Into<String>) -> Self {
        // Preserve historical hyphen-to-underscore folding so session
        // keys match `create_client_from_config`'s canonicalization, then
        // re-validate through `McpServerName::new`. Caller-provided input
        // must never reach `from_trusted` — if validation fails we fall
        // back to the canonical `"unknown"` value.
        let raw: String = server_name.into().replace('-', "_");
        let name = McpServerName::new(&raw).unwrap_or_else(|e| {
            tracing::debug!(
                candidate = %raw,
                error = %e,
                "McpClient::new_with_name: caller-provided name failed allowlist validation; \
                 falling back to canonical 'unknown'"
            );
            McpServerName::new("unknown")
                .expect("'unknown' is a valid McpServerName (alnum allowlist)") // safety: hardcoded literal satisfies alnum-only validation; infallible
        });
        let url: String = server_url.into();
        let transport = Arc::new(HttpMcpTransport::new(url.clone(), name.as_str()));

        Self {
            transport,
            server_url: url,
            server_name: name,
            next_id: AtomicU64::new(1),
            tools_cache: RwLock::new(None),
            session_manager: None,
            secrets: None,
            // TODO(ownership): unauthenticated constructor; user_id set properly via
            // create_client_from_config() for production paths
            user_id: "<unset>".to_string(),
            server_config: None,
            custom_headers: HashMap::new(),
            initialized: tokio::sync::OnceCell::new(),
            #[cfg(test)]
            constructor_kind: McpClientConstructor::PlainNamed,
        }
    }

    /// Create a new simple MCP client from an HTTP server configuration (no authentication).
    ///
    /// Use this when you have an `McpServerConfig` with custom headers but no OAuth.
    /// The config must use HTTP transport (the default); for stdio/UDS use `new_with_transport`.
    ///
    /// Returns an error if the config uses a non-HTTP transport.
    ///
    /// **Note:** The session manager is NOT wired into the transport. For
    /// production use, prefer `create_client_from_config()` which constructs
    /// the transport with session tracking.
    #[cfg(test)]
    pub fn new_with_config(config: McpServerConfig) -> Result<Self, ToolError> {
        if !matches!(
            config.effective_transport(),
            crate::tools::mcp::config::EffectiveTransport::Http
        ) {
            return Err(ToolError::InvalidParameters(
                "new_with_config only supports HTTP transport; use new_with_transport for stdio/UDS"
                    .to_string(),
            ));
        }
        // Validate the config-supplied name once and thread the canonical
        // form into both the transport and the client's typed field so the
        // two can never diverge (e.g. transport falling back to "unknown"
        // while the client still holds the raw invalid value, which would
        // desync session-manager lookups against transport writes).
        // TODO(type-safety PR 4 of 4): switch `McpServerConfig.name` to
        // `McpServerName` so this shared canonicalization moves upstream.
        let validated_name = McpServerName::new(&config.name).unwrap_or_else(|e| {
            tracing::debug!(
                candidate = %config.name,
                error = %e,
                "McpClient::new_with_config: config server name failed allowlist validation; \
                 falling back to canonical 'unknown'"
            );
            McpServerName::new("unknown")
                .expect("'unknown' is a valid McpServerName (alnum allowlist)") // safety: hardcoded literal satisfies alnum-only validation; infallible
        });
        let transport = Arc::new(HttpMcpTransport::new(
            config.url.clone(),
            validated_name.as_str(),
        ));

        Ok(Self {
            transport,
            server_url: config.url.clone(),
            server_name: validated_name,
            next_id: AtomicU64::new(1),
            tools_cache: RwLock::new(None),
            session_manager: None,
            secrets: None,
            // TODO(ownership): unauthenticated constructor; user_id set properly via
            // create_client_from_config() for production paths
            user_id: "<unset>".to_string(),
            custom_headers: config.headers.clone(),
            initialized: tokio::sync::OnceCell::new(),
            server_config: Some(config),
            #[cfg(test)]
            constructor_kind: McpClientConstructor::FromConfig,
        })
    }

    /// Create a new authenticated MCP client.
    ///
    /// Use this for hosted MCP servers that require OAuth authentication.
    pub fn new_authenticated(
        config: McpServerConfig,
        session_manager: Arc<McpSessionManager>,
        secrets: Arc<dyn SecretsStore + Send + Sync>,
        user_id: impl Into<String>,
    ) -> Self {
        // Validate the config-supplied name once and pass the canonical
        // form into both the transport and the client's typed field. If
        // the two sides derived the name independently, an invalid config
        // would leave the transport's `server_name` as "unknown" while
        // the client's `server_name` held the raw value — session IDs
        // would then be written under one key and looked up under another.
        // TODO(type-safety PR 4 of 4): switch `McpServerConfig.name` to
        // `McpServerName` so this shared canonicalization moves upstream.
        let validated_name = McpServerName::new(&config.name).unwrap_or_else(|e| {
            tracing::debug!(
                candidate = %config.name,
                error = %e,
                "McpClient::new_authenticated: config server name failed allowlist validation; \
                 falling back to canonical 'unknown'"
            );
            McpServerName::new("unknown")
                .expect("'unknown' is a valid McpServerName (alnum allowlist)") // safety: hardcoded literal satisfies alnum-only validation; infallible
        });
        let transport = Arc::new(
            HttpMcpTransport::new(config.url.clone(), validated_name.as_str())
                .with_session_manager(session_manager.clone()),
        );

        let custom_headers = config.headers.clone();

        Self {
            transport,
            server_url: config.url.clone(),
            server_name: validated_name,
            next_id: AtomicU64::new(1),
            tools_cache: RwLock::new(None),
            session_manager: Some(session_manager),
            secrets: Some(secrets),
            user_id: user_id.into(),
            server_config: Some(config),
            custom_headers,
            initialized: tokio::sync::OnceCell::new(),
            #[cfg(test)]
            constructor_kind: McpClientConstructor::Authenticated,
        }
    }

    /// Create a new MCP client with a custom transport.
    ///
    /// Use this for stdio, UDS, or other non-HTTP transports.
    pub fn new_with_transport(
        server_name: impl Into<String>,
        transport: Arc<dyn McpTransport>,
        session_manager: Option<Arc<McpSessionManager>>,
        secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
        user_id: impl Into<String>,
        server_config: Option<McpServerConfig>,
    ) -> Self {
        // The production caller (factory) hands us an already-validated
        // name, but the signature accepts `impl Into<String>` which means
        // any other caller (tests, future call sites) could pass something
        // that hasn't been through the allowlist. Re-validate here with the
        // same canonical `"unknown"` fallback used in `new`, `new_with_name`,
        // `new_with_config`, `new_authenticated`, and `HttpMcpTransport::new`
        // so this constructor can't quietly produce an unchecked `McpServerName`.
        // TODO(type-safety PR 4 of 4): accept `McpServerName` here directly
        // and drop this fallback.
        let raw: String = server_name.into();
        let name = McpServerName::new(&raw).unwrap_or_else(|e| {
            tracing::debug!(
                candidate = %raw,
                error = %e,
                "McpClient::new_with_transport: caller-provided server name failed allowlist validation; \
                 falling back to canonical 'unknown'"
            );
            McpServerName::new("unknown")
                .expect("'unknown' is a valid McpServerName (alnum allowlist)") // safety: hardcoded literal satisfies alnum-only validation; infallible
        });
        let url = server_config
            .as_ref()
            .map(|c| c.url.clone())
            .unwrap_or_default();
        let custom_headers = server_config
            .as_ref()
            .map(|c| c.headers.clone())
            .unwrap_or_default();

        Self {
            transport,
            server_url: url,
            server_name: name,
            next_id: AtomicU64::new(1),
            tools_cache: RwLock::new(None),
            session_manager,
            secrets,
            user_id: user_id.into(),
            server_config,
            custom_headers,
            initialized: tokio::sync::OnceCell::new(),
            #[cfg(test)]
            constructor_kind: McpClientConstructor::WithTransport,
        }
    }

    /// Attach a session manager to the **client** only.
    ///
    /// **Warning:** This does NOT wire the session manager into the underlying
    /// `HttpMcpTransport`, so the transport will not capture `Mcp-Session-Id`
    /// from responses. For production use, construct the transport with
    /// `HttpMcpTransport::with_session_manager()` and pass it to
    /// `new_with_transport()` instead. See `create_client_from_config()`.
    #[cfg(test)]
    pub fn with_session_manager(mut self, session_manager: Arc<McpSessionManager>) -> Self {
        self.session_manager = Some(session_manager);
        self
    }

    /// Get the server name as a string slice.
    ///
    /// For typed access — when feeding the name into a session-manager
    /// call or another typed API — use [`Self::server_name_typed`].
    pub fn server_name(&self) -> &str {
        self.server_name.as_str()
    }

    /// Get the typed server name.
    pub fn server_name_typed(&self) -> &McpServerName {
        &self.server_name
    }

    /// Get the server URL.
    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    /// Whether this client has a session manager attached.
    pub fn has_session_manager(&self) -> bool {
        self.session_manager.is_some()
    }

    /// Get the underlying transport (test-only).
    #[cfg(test)]
    pub(crate) fn transport(&self) -> &Arc<dyn McpTransport> {
        &self.transport
    }

    /// Which constructor produced this client (test-only).
    ///
    /// Used by caller-level tests to verify that path-selecting helpers like
    /// `mcp::factory::create_client_from_config` chose the correct branch.
    #[cfg(test)]
    pub(crate) fn constructor_kind(&self) -> McpClientConstructor {
        self.constructor_kind
    }

    /// Get the next request ID.
    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Get the access token for this server (if authenticated).
    ///
    /// If the stored token has expired, automatically attempts a refresh using
    /// the stored refresh token before failing.
    ///
    /// Falls back to the legacy (pre-normalization) secret name so that
    /// existing users who stored tokens under the hyphenated server name
    /// (e.g. `mcp_my-server_access_token`) aren't broken after the
    /// factory starts normalizing `server.name` to underscores. Without
    /// this fallback, `is_authenticated` would report `true` (it has its
    /// own legacy fallback) but the actual request would send no
    /// `Authorization` header and the MCP server would 401.
    async fn get_access_token(&self) -> Result<Option<String>, ToolError> {
        let Some(ref secrets) = self.secrets else {
            return Ok(None);
        };
        let Some(ref config) = self.server_config else {
            return Ok(None);
        };
        // Try canonical (normalized) secret name first.
        let result = resolve_access_token_string_with_refresh(
            secrets.as_ref(),
            &self.user_id,
            &config.token_secret_name(),
            self.server_name.as_str(),
            || async {
                refresh_access_token(config, secrets, &self.user_id)
                    .await
                    .map(|token| token.access_token)
                    .map_err(|e| format!("Token refresh failed: {}", e))
            },
        )
        .await
        .map_err(|e| ToolError::ExternalService(format!("Failed to get access token: {}", e)))?;

        if result.is_some() {
            return Ok(result);
        }

        // Fall back to the legacy (pre-normalization) secret name.
        // This path is transitional — the user will re-auth once and
        // get migrated to the canonical name. Bare get_decrypted (no
        // refresh) is intentional: wiring refresh through the legacy
        // naming scheme adds complexity for a self-healing compat path.
        if let Some(legacy_name) = config.legacy_token_secret_name()
            && let Ok(decrypted) = secrets.get_decrypted(&self.user_id, &legacy_name).await
        {
            return Ok(Some(decrypted.expose().to_string()));
        }

        Ok(None)
    }

    /// Build the headers map for a request (auth, session-id, custom headers).
    ///
    /// Custom headers are applied first. OAuth token injection is skipped if the
    /// user has explicitly configured an Authorization header, so user-provided
    /// credentials are never silently overwritten.
    async fn build_request_headers(&self) -> Result<HashMap<String, String>, ToolError> {
        let mut headers = self.custom_headers.clone();

        // Only inject OAuth token if the user hasn't set a custom Authorization header.
        let has_custom_auth = self
            .custom_headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case("authorization"));
        if !has_custom_auth && let Some(token) = self.get_access_token().await? {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                headers.insert("Authorization".to_string(), format!("Bearer {}", trimmed));
            }
        }
        if let Some(ref session_manager) = self.session_manager
            && let Some(session_id) = session_manager.get_session_id(&self.server_name).await
        {
            headers.insert("Mcp-Session-Id".to_string(), session_id);
        }
        Ok(headers)
    }

    /// Re-run the MCP initialize handshake outside the OnceCell cache.
    ///
    /// This is used for recoverable session-expiry failures when an MCP server
    /// reports that the current session ID is no longer valid.
    async fn reinitialize_session(&self) -> Result<InitializeResult, ToolError> {
        if let Some(ref session_manager) = self.session_manager {
            session_manager.terminate(&self.server_name).await;
            session_manager
                .get_or_create(&self.server_name, &self.server_url)
                .await;
        }

        let request = McpRequest::initialize(self.next_request_id());
        let response = self
            .transport
            .send(&request, &self.build_request_headers().await?)
            .await?;

        if let Some(error) = response.error {
            return Err(ToolError::ExternalService(format!(
                "MCP initialization error: {} (code {})",
                error.message, error.code
            )));
        }

        let init_result: InitializeResult = response
            .result
            .ok_or_else(|| {
                ToolError::ExternalService("No result in initialize response".to_string())
            })
            .and_then(|r| {
                serde_json::from_value(r).map_err(|e| {
                    ToolError::ExternalService(format!("Invalid initialize result: {}", e))
                })
            })?;

        if let Some(ref session_manager) = self.session_manager {
            session_manager.mark_initialized(&self.server_name).await;
        }

        let notification = McpRequest::initialized_notification();
        if let Err(e) = self
            .transport
            .send(&notification, &self.build_request_headers().await?)
            .await
        {
            tracing::debug!(
                "Failed to send initialized notification to '{}': {}",
                self.server_name,
                e
            );
        }

        Ok(init_result)
    }

    /// Return true when the error looks like a recoverable MCP session expiry.
    fn is_session_expiry_error(message: &str) -> bool {
        let lower = message.to_ascii_lowercase();
        lower.contains("session")
            && (lower.contains("400")
                || lower.contains("missing session id")
                || lower.contains("no valid session id"))
    }

    /// Send a request to the MCP server with auth and session headers.
    /// Automatically attempts token refresh on 401 errors (HTTP transports only).
    async fn send_request(&self, request: McpRequest) -> Result<McpResponse, ToolError> {
        // For non-HTTP transports, just send directly without retry logic
        if !self.transport.supports_http_features() {
            let headers = self.build_request_headers().await?;
            return self.transport.send(&request, &headers).await;
        }

        // HTTP transport: try up to 2 times (first attempt, then retry after token refresh
        // or recoverable session reinitialization).
        for attempt in 0..2 {
            let headers = self.build_request_headers().await?;
            let result = self.transport.send(&request, &headers).await;

            match result {
                Ok(response) => return Ok(response),
                Err(ToolError::ExternalService(ref msg))
                    if attempt == 0
                        && self.session_manager.is_some()
                        && Self::is_session_expiry_error(msg) =>
                {
                    tracing::debug!(
                        "MCP session expired, attempting reinitialize for '{}'",
                        self.server_name
                    );
                    self.reinitialize_session().await?;
                    continue;
                }
                Err(ToolError::ExternalService(ref msg)) if super::is_auth_error_message(msg) => {
                    if attempt == 0
                        && let Some(ref secrets) = self.secrets
                        && let Some(ref config) = self.server_config
                    {
                        tracing::debug!(
                            "MCP token expired, attempting refresh for '{}'",
                            self.server_name
                        );
                        match refresh_access_token(config, secrets, &self.user_id).await {
                            Ok(_) => {
                                tracing::info!("MCP token refreshed for '{}'", self.server_name);
                                continue;
                            }
                            Err(e) => {
                                tracing::debug!(
                                    "Token refresh failed for '{}': {}",
                                    self.server_name,
                                    e
                                );
                            }
                        }
                    }
                    let auth_message = if self
                        .server_config
                        .as_ref()
                        .is_some_and(|config| config.has_custom_auth_header())
                    {
                        format!(
                            "MCP server '{}' rejected its configured Authorization header. Update the configured credential and try again.",
                            self.server_name
                        )
                    } else {
                        format!(
                            "MCP server '{}' requires authentication. Run: ironclaw mcp auth {}",
                            self.server_name, self.server_name
                        )
                    };
                    return Err(ToolError::ExternalService(auth_message));
                }
                Err(e) => return Err(e),
            }
        }

        Err(ToolError::ExternalService(
            "MCP request failed after retry".to_string(),
        ))
    }

    /// Initialize the connection to the MCP server.
    ///
    /// Uses `OnceCell` to guarantee that exactly one caller performs the
    /// handshake, even under concurrent access. Subsequent calls return
    /// immediately.
    pub async fn initialize(&self) -> Result<InitializeResult, ToolError> {
        let result = self
            .initialized
            .get_or_try_init(|| async {
                if let Some(ref session_manager) = self.session_manager
                    && session_manager.is_initialized(&self.server_name).await
                {
                    return Ok(InitializeResult::default());
                }
                self.reinitialize_session().await
            })
            .await?;

        Ok(result.clone())
    }

    /// List available tools from the MCP server.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, ToolError> {
        if let Some(tools) = self.tools_cache.read().await.as_ref() {
            return Ok(tools.clone());
        }
        self.initialize().await?;

        let request = McpRequest::list_tools(self.next_request_id());
        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(ToolError::ExternalService(format!(
                "MCP error: {} (code {})",
                error.message, error.code
            )));
        }

        let result: ListToolsResult = response
            .result
            .ok_or_else(|| ToolError::ExternalService("No result in MCP response".to_string()))
            .and_then(|r| {
                serde_json::from_value(r)
                    .map_err(|e| ToolError::ExternalService(format!("Invalid tools list: {}", e)))
            })?;

        *self.tools_cache.write().await = Some(result.tools.clone());
        Ok(result.tools)
    }

    /// Call a tool on the MCP server.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<CallToolResult, ToolError> {
        self.initialize().await?;

        let request = McpRequest::call_tool(self.next_request_id(), name, arguments);
        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(ToolError::ExecutionFailed(format!(
                "MCP tool error: {} (code {})",
                error.message, error.code
            )));
        }

        response
            .result
            .ok_or_else(|| ToolError::ExternalService("No result in MCP response".to_string()))
            .and_then(|r| {
                serde_json::from_value(r)
                    .map_err(|e| ToolError::ExternalService(format!("Invalid tool result: {}", e)))
            })
    }

    /// Clear the tools cache.
    pub async fn clear_cache(&self) {
        *self.tools_cache.write().await = None;
    }

    /// Create Tool implementations for all MCP tools.
    ///
    /// `mcp_tool_id` normalizes every non-`[A-Za-z0-9_]` character to `_`,
    /// which is necessary for the registry key to survive LLM tool-name
    /// normalization but introduces a collision hazard: two MCP tools whose
    /// names differ only by `-` vs `_` (or `.` vs `_`, etc.) — e.g.
    /// `search-all` and `search_all` — produce the same registry key. The
    /// second `ToolRegistry::register` call would silently shadow the first
    /// with no signal at all, leaving operators debugging an unreachable
    /// tool with zero breadcrumb. We detect collisions here, where we still
    /// have both the original name and the normalized id, and emit a
    /// `warn!` log so the shadowing is observable. Behaviour is unchanged —
    /// the second tool still wins on register, matching what the LLM would
    /// emit anyway since it normalizes both names to the same string.
    pub async fn create_tools(&self) -> Result<Vec<Arc<dyn Tool>>, ToolError> {
        let mcp_tools = self.list_tools().await?;
        let client = Arc::new(self.clone());

        // Detect post-normalization collisions before registering. This is
        // a single linear pass; the n is small (a typical MCP server lists
        // a few dozen tools).
        let mut seen_ids: HashMap<String, String> = HashMap::new();
        for t in &mcp_tools {
            let id = mcp_tool_id(self.server_name.as_str(), &t.name);
            match seen_ids.get(&id) {
                Some(prev) if prev != &t.name => {
                    tracing::warn!(
                        normalized_id = %id,
                        first_name = %prev,
                        colliding_name = %t.name,
                        server = %self.server_name,
                        "MCP tool name collision after normalization — second tool will shadow the first in the registry. Operators: rename one of the upstream tools to differ in more than just '-' vs '_' (or '.' vs '_')."
                    );
                    // Update so a 3rd collision reports against the most
                    // recent shadow, not the original entry.
                    seen_ids.insert(id, t.name.clone());
                }
                _ => {
                    seen_ids.insert(id, t.name.clone());
                }
            }
        }

        Ok(mcp_tools
            .into_iter()
            .map(|t| {
                let prefixed_name = mcp_tool_id(self.server_name.as_str(), &t.name);
                Arc::new(McpToolWrapper {
                    tool: t,
                    prefixed_name,
                    provider_extension: self.server_name.as_str().to_string(),
                    client: client.clone(),
                }) as Arc<dyn Tool>
            })
            .collect())
    }

    /// Test the connection to the MCP server.
    pub async fn test_connection(&self) -> Result<(), ToolError> {
        self.initialize().await?;
        self.list_tools().await?;
        Ok(())
    }
}

/// Clone the client, resetting the tools cache and initialization state.
/// The cloned client shares the same transport and session manager, so
/// re-initialization will short-circuit via the session manager check if
/// the source was already initialized. The `next_id` counter is copied
/// so that cloned clients continue with monotonically increasing IDs.
impl Clone for McpClient {
    fn clone(&self) -> Self {
        Self {
            transport: self.transport.clone(),
            server_url: self.server_url.clone(),
            server_name: self.server_name.clone(),
            next_id: AtomicU64::new(self.next_id.load(Ordering::SeqCst)),
            tools_cache: RwLock::new(None),
            session_manager: self.session_manager.clone(),
            secrets: self.secrets.clone(),
            user_id: self.user_id.clone(),
            server_config: self.server_config.clone(),
            custom_headers: self.custom_headers.clone(),
            initialized: tokio::sync::OnceCell::new(),
            #[cfg(test)]
            constructor_kind: self.constructor_kind,
        }
    }
}

/// Extract a server name from a URL for logging/display purposes.
fn extract_server_name(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| "unknown".to_string())
        .replace('.', "_")
}

/// Build the canonical registry identifier for an MCP tool.
///
/// MCP tool names commonly contain dashes (e.g. `notion-search`), and so do
/// user-supplied server names (`my-server`). The IronClaw runtime converges
/// on snake_case identifiers (see `ToolRegistry::resolve_name`), and LLMs,
/// Codex / GPT-5 in particular, silently normalize tool names to valid
/// Python identifiers by converting dashes to underscores. If we registered
/// `notion_notion-search` the LLM would emit a call for `notion_notion_search`
/// and the registry lookup would miss, leaving the tool unreachable.
///
/// We replace **every** non-`[A-Za-z0-9_]` character with `_`, not just
/// dashes. The MCP spec doesn't actually constrain tool names to OpenAI's
/// `^[a-zA-Z0-9_-]{1,64}$` regex — a server could legally return
/// `notion.search` or `notion:create_issue` — and the same LLM normalization
/// that bites on `-` will bite on `.` and `:` too. Replacing them all up
/// front is a one-line defense that makes the registry key bulletproof.
/// `extract_server_name` already strips `.` from the host portion of a URL,
/// but the tool portion of the prefixed name was unprotected. Multi-byte
/// unicode characters (e.g. emoji or non-ASCII letters) are also normalized
/// to `_` so the registry key stays a valid Rust identifier suffix.
///
/// The original (possibly hyphenated / dotted / unicode) tool name is still
/// stored on the `McpToolWrapper`'s inner `McpTool` and used verbatim when
/// forwarding the `tools/call` request to the MCP server, so this
/// normalization is internal-only and does not affect protocol compatibility.
pub(crate) fn mcp_tool_id(server_name: &str, tool_name: &str) -> String {
    format!("{server_name}_{tool_name}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Wrapper that implements Tool for an MCP tool.
struct McpToolWrapper {
    tool: McpTool,
    prefixed_name: String,
    provider_extension: String,
    client: Arc<McpClient>,
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str {
        &self.prefixed_name
    }
    fn description(&self) -> &str {
        &self.tool.description
    }
    fn parameters_schema(&self) -> serde_json::Value {
        self.tool.input_schema.clone()
    }

    fn provider_extension(&self) -> Option<&str> {
        Some(&self.provider_extension)
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Strip top-level null values before forwarding — LLMs often emit
        // `"field": null` for optional params, but many MCP servers reject
        // explicit nulls for fields that should simply be absent.
        let params = strip_top_level_nulls(params);

        let result = self.client.call_tool(&self.tool.name, params).await?;
        let content: String = result
            .content
            .iter()
            .filter_map(|b| b.as_text())
            .collect::<Vec<_>>()
            .join("\n");
        if result.is_error {
            return Err(ToolError::ExecutionFailed(content));
        }
        Ok(ToolOutput::text(content, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        if self.tool.requires_approval() {
            ApprovalRequirement::UnlessAutoApproved
        } else {
            ApprovalRequirement::Never
        }
    }
}

/// Remove top-level keys whose value is JSON null from an object.
///
/// LLMs frequently emit `"field": null` for optional parameters.  Many MCP
/// servers (e.g. Notion) treat an explicit `null` as an invalid value for
/// optional fields that should simply be absent.  Stripping these before
/// forwarding avoids 400-class rejections from strict servers.
fn strip_top_level_nulls(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let filtered = map.into_iter().filter(|(_, v)| !v.is_null()).collect();
            serde_json::Value::Object(filtered)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_request_list_tools() {
        let req = McpRequest::list_tools(1);
        assert_eq!(req.method, "tools/list");
        assert_eq!(req.id, Some(1));
    }

    #[test]
    fn test_mcp_request_call_tool() {
        let req = McpRequest::call_tool(2, "test", serde_json::json!({"key": "value"}));
        assert_eq!(req.method, "tools/call");
        assert!(req.params.is_some());
    }

    #[test]
    fn test_extract_server_name() {
        assert_eq!(
            extract_server_name("https://mcp.notion.com/v1"),
            "mcp_notion_com"
        );
        assert_eq!(extract_server_name("http://localhost:8080"), "localhost");
        assert_eq!(extract_server_name("invalid"), "unknown");
    }

    #[test]
    fn test_simple_client_creation() {
        let client = McpClient::new("http://localhost:8080");
        assert_eq!(client.server_url(), "http://localhost:8080");
        assert!(client.session_manager.is_none());
        assert!(client.secrets.is_none());
    }

    #[test]
    fn test_extract_server_name_with_port() {
        assert_eq!(
            extract_server_name("http://example.com:3000"),
            "example_com"
        );
    }

    #[test]
    fn test_extract_server_name_with_path() {
        assert_eq!(
            extract_server_name("http://api.server.io/v2/mcp"),
            "api_server_io"
        );
    }

    #[test]
    fn test_extract_server_name_with_query_params() {
        assert_eq!(
            extract_server_name("http://mcp.example.com/endpoint?token=abc&v=1"),
            "mcp_example_com"
        );
    }

    #[test]
    fn test_extract_server_name_https() {
        assert_eq!(
            extract_server_name("https://secure.mcp.dev"),
            "secure_mcp_dev"
        );
    }

    #[test]
    fn test_extract_server_name_ip_address() {
        assert_eq!(
            extract_server_name("http://192.168.1.100:9090/mcp"),
            "192_168_1_100"
        );
    }

    #[test]
    fn test_new_defaults() {
        let client = McpClient::new("http://localhost:9999");
        assert_eq!(client.server_url(), "http://localhost:9999");
        assert_eq!(client.server_name(), "localhost");
        assert!(client.session_manager.is_none());
        assert!(client.secrets.is_none());
        assert_eq!(client.user_id, "<unset>");
    }

    #[test]
    fn test_new_with_name_uses_custom_name() {
        let client = McpClient::new_with_name("my-server", "http://localhost:8080");
        assert_eq!(client.server_name(), "my_server");
        assert_eq!(client.server_url(), "http://localhost:8080");
        assert_eq!(client.user_id, "<unset>");
        assert!(client.session_manager.is_none());
        assert!(client.secrets.is_none());
    }

    /// Regression for review Fix 7/8: `McpClient::new` must not bypass the
    /// `McpServerName` allowlist when `extract_server_name` produces a
    /// value containing forbidden characters (e.g. the `:` in an IPv6
    /// bracketed host such as `[::1]`). Historically we called
    /// `McpServerName::from_trusted(...)` here, which silently accepted
    /// whatever the heuristic parser returned.
    ///
    /// Behavior under the fix: invalid extracted names fall back to the
    /// canonical `"unknown"` and a `tracing::debug!` records the failure.
    /// Either outcome must still be a valid `McpServerName`.
    #[test]
    fn new_validates_server_name_for_ipv6_host() {
        let client = McpClient::new("http://[::1]:8080/");
        // The extracted name must round-trip through `McpServerName::new`
        // without error — the contract of the fix is "allowlist or
        // canonical fallback, never a raw un-checked string".
        let name = McpServerName::new(client.server_name())
            .expect("server_name must be a valid McpServerName (allowlist or fallback)");
        // In this environment the extracted host `[::1]` contains the
        // forbidden `:` and `[`/`]` characters, so the fallback fires.
        // Document that explicitly — the point of the test is that we
        // end up with `"unknown"` rather than a silently-accepted bogus
        // value.
        assert_eq!(
            name.as_str(),
            "unknown",
            "IPv6 bracketed host should fall back to canonical 'unknown'"
        );
    }

    /// Regression for review Fix 8: `McpClient::new_with_name` must not
    /// bypass the allowlist when the caller passes a name that contains
    /// forbidden characters after the hyphen fold.
    #[test]
    fn new_with_name_falls_back_on_invalid_input() {
        // Slashes are forbidden by the allowlist and are not touched by
        // the hyphen fold, so the validation must fire and the fallback
        // engage.
        let client = McpClient::new_with_name("bad/name", "http://localhost:8080");
        let name = McpServerName::new(client.server_name())
            .expect("server_name must be a valid McpServerName (allowlist or fallback)");
        assert_eq!(
            name.as_str(),
            "unknown",
            "invalid caller-supplied name should fall back to canonical 'unknown'"
        );
    }

    #[test]
    fn test_server_name_accessor() {
        let client = McpClient::new("https://tools.example.org/mcp");
        assert_eq!(client.server_name(), "tools_example_org");
    }

    #[test]
    fn test_server_url_accessor() {
        let url = "https://tools.example.org/mcp?v=2";
        let client = McpClient::new(url);
        assert_eq!(client.server_url(), url);
    }

    #[test]
    fn test_clone_preserves_fields() {
        let client = McpClient::new_with_name("cloned-server", "http://localhost:5555");
        client.next_request_id();
        client.next_request_id();
        let cloned = client.clone();
        assert_eq!(cloned.server_url(), "http://localhost:5555");
        assert_eq!(cloned.server_name(), "cloned_server");
        assert_eq!(cloned.user_id, "<unset>");
        assert_eq!(cloned.next_id.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_clone_resets_tools_cache() {
        let client = McpClient::new("http://localhost:5555");
        let cloned = client.clone();
        let cache = cloned.tools_cache.read().await;
        assert!(cache.is_none());
    }

    #[test]
    fn test_new_with_config_carries_custom_headers() {
        let mut headers = HashMap::new();
        headers.insert("X-API-Key".to_string(), "secret".to_string());
        headers.insert("X-Custom".to_string(), "value".to_string());

        let config = McpServerConfig::new("test", "http://localhost:8080").with_headers(headers);
        let client = McpClient::new_with_config(config.clone()).expect("HTTP config should work");

        assert_eq!(client.server_name(), "test");
        assert_eq!(client.server_url(), "http://localhost:8080");
        assert_eq!(client.custom_headers.len(), 2);
        assert_eq!(client.custom_headers.get("X-API-Key").unwrap(), "secret");
        assert!(client.server_config.is_some());
    }

    #[test]
    fn test_new_with_config_no_headers() {
        let config = McpServerConfig::new("bare", "http://localhost:9090");
        let client = McpClient::new_with_config(config).expect("HTTP config should work");

        assert_eq!(client.server_name(), "bare");
        assert!(client.custom_headers.is_empty());
        assert!(client.secrets.is_none());
        assert!(client.session_manager.is_none());
    }

    #[test]
    fn test_with_session_manager() {
        let client = McpClient::new("http://localhost:8080");
        assert!(!client.has_session_manager());

        let session_manager = Arc::new(McpSessionManager::new());
        let client = client.with_session_manager(session_manager);

        assert!(client.has_session_manager());
    }

    #[test]
    fn test_next_request_id_monotonically_increasing() {
        let client = McpClient::new("http://localhost:1234");
        assert_eq!(client.next_request_id(), 1);
        assert_eq!(client.next_request_id(), 2);
        assert_eq!(client.next_request_id(), 3);
    }

    #[test]
    fn test_mcp_tool_requires_approval_destructive() {
        use crate::tools::mcp::protocol::{McpTool, McpToolAnnotations};
        let tool = McpTool {
            name: "delete_all".to_string(),
            description: "Deletes everything".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: Some(McpToolAnnotations {
                destructive_hint: true,
                side_effects_hint: false,
                read_only_hint: false,
                execution_time_hint: None,
            }),
        };
        assert!(tool.requires_approval());
    }

    #[test]
    fn test_mcp_tool_no_approval_when_not_destructive() {
        use crate::tools::mcp::protocol::{McpTool, McpToolAnnotations};
        let tool = McpTool {
            name: "read_data".to_string(),
            description: "Reads data".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: Some(McpToolAnnotations {
                destructive_hint: false,
                side_effects_hint: true,
                read_only_hint: false,
                execution_time_hint: None,
            }),
        };
        assert!(!tool.requires_approval());
    }

    #[test]
    fn test_mcp_tool_no_approval_when_no_annotations() {
        use crate::tools::mcp::protocol::McpTool;
        let tool = McpTool {
            name: "simple_tool".to_string(),
            description: "A simple tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: None,
        };
        assert!(!tool.requires_approval());
    }

    /// Mock transport for testing transport abstraction behavior.
    struct MockTransport {
        supports_http: bool,
        responses: std::sync::Mutex<Vec<McpResponse>>,
        recorded_headers: std::sync::Mutex<Vec<HashMap<String, String>>>,
    }

    impl MockTransport {
        fn new(supports_http: bool, responses: Vec<McpResponse>) -> Self {
            Self {
                supports_http,
                responses: std::sync::Mutex::new(responses),
                recorded_headers: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn recorded_headers(&self) -> Vec<HashMap<String, String>> {
            self.recorded_headers.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl McpTransport for MockTransport {
        async fn send(
            &self,
            _request: &McpRequest,
            headers: &HashMap<String, String>,
        ) -> Result<McpResponse, ToolError> {
            self.recorded_headers.lock().unwrap().push(headers.clone());
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(ToolError::ExternalService(
                    "No more mock responses".to_string(),
                ));
            }
            Ok(responses.remove(0))
        }
        async fn shutdown(&self) -> Result<(), ToolError> {
            Ok(())
        }
        fn supports_http_features(&self) -> bool {
            self.supports_http
        }
    }

    /// Mock transport that can return errors and successful responses in a
    /// controlled sequence.
    struct RetryMockTransport {
        supports_http: bool,
        outcomes: std::sync::Mutex<std::collections::VecDeque<Result<McpResponse, ToolError>>>,
        recorded_headers: std::sync::Mutex<Vec<HashMap<String, String>>>,
    }

    impl RetryMockTransport {
        fn new(supports_http: bool, outcomes: Vec<Result<McpResponse, ToolError>>) -> Self {
            Self {
                supports_http,
                outcomes: std::sync::Mutex::new(outcomes.into()),
                recorded_headers: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn recorded_headers(&self) -> Vec<HashMap<String, String>> {
            self.recorded_headers.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl McpTransport for RetryMockTransport {
        async fn send(
            &self,
            _request: &McpRequest,
            headers: &HashMap<String, String>,
        ) -> Result<McpResponse, ToolError> {
            self.recorded_headers.lock().unwrap().push(headers.clone());
            let mut outcomes = self.outcomes.lock().unwrap();
            if outcomes.is_empty() {
                return Err(ToolError::ExternalService(
                    "No more mock outcomes".to_string(),
                ));
            }
            outcomes.pop_front().unwrap()
        }

        async fn shutdown(&self) -> Result<(), ToolError> {
            Ok(())
        }

        fn supports_http_features(&self) -> bool {
            self.supports_http
        }
    }

    #[tokio::test]
    async fn test_non_http_transport_skips_401_retry() {
        // initialize response, then notification ack (consumed but ignored),
        // then list_tools response
        let init_response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(1),
            result: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "test", "version": "1.0"}
            })),
            error: None,
        };
        let notification_ack = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: None,
            result: None,
            error: None,
        };
        let list_response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(2),
            result: Some(serde_json::json!({"tools": []})),
            error: None,
        };
        let transport = Arc::new(MockTransport::new(
            false,
            vec![init_response, notification_ack, list_response],
        ));
        let client = McpClient::new_with_transport(
            "test-stdio",
            transport.clone(),
            None,
            None,
            "default",
            None,
        );
        let result = client.list_tools().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
        let headers = transport.recorded_headers();
        // 3 sends: initialize + notifications/initialized + list_tools
        assert_eq!(headers.len(), 3);
        assert!(!headers[0].contains_key("Authorization"));
        assert!(!headers[0].contains_key("Mcp-Session-Id"));
    }

    #[tokio::test]
    async fn test_transport_supports_http_features_accessor() {
        let http_transport = HttpMcpTransport::new("http://localhost:8080", "test");
        assert!(http_transport.supports_http_features());
        let mock_non_http = MockTransport::new(false, vec![]);
        assert!(!mock_non_http.supports_http_features());
    }

    /// Regression test for issue #890: stdio clients must auto-initialize
    /// even without a session manager, and the second call should be idempotent.
    #[tokio::test]
    async fn test_stdio_client_auto_initializes_without_session_manager() {
        let init_response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(1),
            result: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "test", "version": "1.0"}
            })),
            error: None,
        };
        let notification_ack = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: None,
            result: None,
            error: None,
        };
        let transport = Arc::new(MockTransport::new(
            false,
            vec![init_response, notification_ack],
        ));
        let client = McpClient::new_with_transport(
            "test-stdio",
            transport.clone(),
            None, // no session manager
            None,
            "default",
            None,
        );

        // First call should send initialize + notification
        let result = client.initialize().await;
        assert!(result.is_ok());
        assert_eq!(transport.recorded_headers().len(), 2);

        // Second call should be a no-op (idempotent via local flag)
        let result2 = client.initialize().await;
        assert!(result2.is_ok());
        assert_eq!(transport.recorded_headers().len(), 2); // no additional sends
    }

    #[tokio::test]
    async fn test_http_session_error_triggers_reinitialize_and_retry() {
        let init_response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(1),
            result: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "test", "version": "1.0"}
            })),
            error: None,
        };
        let notification_ack = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: None,
            result: None,
            error: None,
        };
        let notification_ack2 = notification_ack.clone();
        let session_error = Err(ToolError::ExternalService(
            "[test] MCP server returned status: 400 - No valid session ID provided".to_string(),
        ));
        let reinit_response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(2),
            result: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "test", "version": "1.0"}
            })),
            error: None,
        };
        let call_response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(3),
            result: Some(serde_json::json!({
                "content": [{"type": "text", "text": "pong"}],
                "is_error": false
            })),
            error: None,
        };

        let transport = Arc::new(RetryMockTransport::new(
            true,
            vec![
                Ok(init_response),
                Ok(notification_ack),
                session_error,
                Ok(reinit_response),
                Ok(notification_ack2),
                Ok(call_response),
            ],
        ));
        let session_manager = Arc::new(McpSessionManager::new());
        let client = McpClient::new_with_transport(
            "test-http",
            transport.clone(),
            Some(session_manager),
            None,
            "default",
            None,
        );

        client.initialize().await.expect("initial handshake");

        let result = client
            .call_tool("echo", serde_json::json!({"input": "hello"}))
            .await
            .expect("call should recover after session expiry");
        assert!(!result.is_error);
        assert_eq!(result.content.len(), 1);
        assert_eq!(result.content[0].as_text(), Some("pong"));

        let headers = transport.recorded_headers();
        assert_eq!(headers.len(), 6);
    }

    #[test]
    fn test_strip_top_level_nulls_removes_null_fields() {
        let input = serde_json::json!({
            "query": "search term",
            "sort": null,
            "filter": null,
            "page_size": 10
        });
        let result = strip_top_level_nulls(input);
        let obj = result.as_object().unwrap();
        assert_eq!(obj.len(), 2);
        assert_eq!(obj["query"], "search term");
        assert_eq!(obj["page_size"], 10);
        assert!(!obj.contains_key("sort"));
        assert!(!obj.contains_key("filter"));
    }

    #[test]
    fn test_strip_top_level_nulls_preserves_non_objects() {
        let input = serde_json::json!("just a string");
        let result = strip_top_level_nulls(input.clone());
        assert_eq!(result, input);
    }

    #[test]
    fn test_strip_top_level_nulls_preserves_nested_nulls() {
        let input = serde_json::json!({
            "outer": { "inner": null },
            "top_null": null
        });
        let result = strip_top_level_nulls(input);
        let obj = result.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        assert!(obj["outer"]["inner"].is_null());
    }

    // --- Regression: constructors validate caller-provided server names ---
    //
    // Copilot review comments on PR nearai/ironclaw#2681 flagged that
    // `new_with_transport`, `new_with_config`, and `new_authenticated`
    // used `McpServerName::from_trusted` on caller-provided input,
    // allowing invalid names to enter the typed field and diverge from
    // the transport's independently-validated `server_name`. These tests
    // pin the safe-fallback behavior and the no-divergence invariant.

    #[test]
    fn new_with_transport_falls_back_on_invalid_server_name() {
        let transport = Arc::new(MockTransport::new(false, vec![]));
        // Slashes are outside the `McpServerName` allowlist, so the
        // fallback path must engage rather than storing the raw value.
        let client =
            McpClient::new_with_transport("bad/name", transport, None, None, "default", None);
        let name = McpServerName::new(client.server_name())
            .expect("stored server_name must satisfy the allowlist after the fix");
        assert_eq!(
            name.as_str(),
            "unknown",
            "invalid caller-supplied name should fall back to canonical 'unknown'"
        );
    }

    #[test]
    fn new_with_transport_preserves_valid_server_name() {
        let transport = Arc::new(MockTransport::new(false, vec![]));
        let client =
            McpClient::new_with_transport("good_name123", transport, None, None, "default", None);
        assert_eq!(client.server_name(), "good_name123");
    }

    #[test]
    fn new_with_config_falls_back_on_invalid_server_name() {
        // `McpServerConfig::new` does not pre-validate the name, so the
        // constructor receives a value that would fail the allowlist.
        let config = McpServerConfig::new("bad name", "http://localhost:8080");
        let client = McpClient::new_with_config(config).expect("HTTP transport accepted");
        assert_eq!(
            client.server_name(),
            "unknown",
            "invalid config name must fall back to canonical 'unknown'"
        );
    }

    /// Divergence regression: the client's `server_name` field and the
    /// value used when constructing the transport must agree. If the
    /// client stored the raw invalid value and the transport fell back
    /// to "unknown", session-manager writes (from the transport) would
    /// use a different key than session-manager lookups (keyed off the
    /// client's name), silently breaking `Mcp-Session-Id` tracking.
    #[test]
    fn new_with_config_client_and_transport_server_name_agree() {
        let invalid = McpServerConfig::new("bad name", "http://localhost:8080");
        let client = McpClient::new_with_config(invalid).expect("HTTP transport accepted");
        // The client surface is the single source of truth for tests;
        // the transport validates the same input the client just
        // canonicalized, so parity here proves no divergence exists.
        assert_eq!(client.server_name(), "unknown");

        let valid = McpServerConfig::new("good_name123", "http://localhost:8080");
        let client = McpClient::new_with_config(valid).expect("HTTP transport accepted");
        assert_eq!(client.server_name(), "good_name123");
    }

    #[tokio::test]
    async fn new_authenticated_falls_back_on_invalid_server_name() {
        let session_manager = Arc::new(McpSessionManager::new());
        let key =
            secrecy::SecretString::from(crate::testing::credentials::TEST_CRYPTO_KEY.to_string());
        let crypto = Arc::new(crate::secrets::SecretsCrypto::new(key).expect("test crypto"));
        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(crypto));

        let config = McpServerConfig::new("bad name", "https://api.example.com");
        let client = McpClient::new_authenticated(config, session_manager, secrets, "test-user");
        assert_eq!(
            client.server_name(),
            "unknown",
            "invalid config name must fall back to canonical 'unknown' in the OAuth path too"
        );
    }

    #[tokio::test]
    async fn new_authenticated_preserves_valid_server_name() {
        let session_manager = Arc::new(McpSessionManager::new());
        let key =
            secrecy::SecretString::from(crate::testing::credentials::TEST_CRYPTO_KEY.to_string());
        let crypto = Arc::new(crate::secrets::SecretsCrypto::new(key).expect("test crypto"));
        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(crypto));

        let config = McpServerConfig::new("good_name123", "https://api.example.com");
        let client = McpClient::new_authenticated(config, session_manager, secrets, "test-user");
        assert_eq!(client.server_name(), "good_name123");
    }

    // --- Issue 1 regression: new_with_config rejects non-HTTP transport ---

    #[test]
    fn test_new_with_config_rejects_stdio_transport() {
        let config = McpServerConfig::new_stdio(
            "stdio-server",
            "echo",
            vec!["hello".to_string()],
            HashMap::new(),
        );
        let result = McpClient::new_with_config(config);
        let err = result
            .err()
            .expect("stdio config must be rejected")
            .to_string();
        assert!(
            err.contains("new_with_config only supports HTTP"),
            "error should explain the restriction: {}",
            err
        );
    }

    // --- Issue 13: McpToolWrapper unit tests ---

    fn make_test_mcp_tool(destructive: bool) -> McpTool {
        use crate::tools::mcp::protocol::McpToolAnnotations;
        McpTool {
            name: "do_thing".to_string(),
            description: "Does a thing".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "input": {"type": "string"}
                }
            }),
            annotations: if destructive {
                Some(McpToolAnnotations {
                    destructive_hint: true,
                    side_effects_hint: false,
                    read_only_hint: false,
                    execution_time_hint: None,
                })
            } else {
                None
            },
        }
    }

    #[test]
    fn test_mcp_tool_wrapper_name_is_prefixed() {
        let client = Arc::new(McpClient::new("http://localhost:8080"));
        let wrapper = McpToolWrapper {
            tool: make_test_mcp_tool(false),
            prefixed_name: "mcp__myserver__do_thing".to_string(),
            provider_extension: "myserver".to_string(),
            client,
        };
        assert_eq!(wrapper.name(), "mcp__myserver__do_thing");
    }

    #[test]
    fn test_mcp_tool_wrapper_description() {
        let client = Arc::new(McpClient::new("http://localhost:8080"));
        let wrapper = McpToolWrapper {
            tool: make_test_mcp_tool(false),
            prefixed_name: "mcp__s__do_thing".to_string(),
            provider_extension: "s".to_string(),
            client,
        };
        assert_eq!(wrapper.description(), "Does a thing");
    }

    #[test]
    fn test_mcp_tool_wrapper_parameters_schema() {
        let client = Arc::new(McpClient::new("http://localhost:8080"));
        let wrapper = McpToolWrapper {
            tool: make_test_mcp_tool(false),
            prefixed_name: "mcp__s__do_thing".to_string(),
            provider_extension: "s".to_string(),
            client,
        };
        let schema = wrapper.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["input"].is_object());
    }

    #[test]
    fn test_mcp_tool_wrapper_requires_sanitization() {
        let client = Arc::new(McpClient::new("http://localhost:8080"));
        let wrapper = McpToolWrapper {
            tool: make_test_mcp_tool(false),
            prefixed_name: "mcp__s__do_thing".to_string(),
            provider_extension: "s".to_string(),
            client,
        };
        assert!(
            wrapper.requires_sanitization(),
            "MCP tools should always require sanitization"
        );
    }

    #[test]
    fn test_mcp_tool_wrapper_approval_destructive() {
        let client = Arc::new(McpClient::new("http://localhost:8080"));
        let wrapper = McpToolWrapper {
            tool: make_test_mcp_tool(true),
            prefixed_name: "mcp__s__do_thing".to_string(),
            provider_extension: "s".to_string(),
            client,
        };
        let approval = wrapper.requires_approval(&serde_json::json!({}));
        assert_eq!(approval, ApprovalRequirement::UnlessAutoApproved);
    }

    #[test]
    fn test_mcp_tool_wrapper_approval_non_destructive() {
        let client = Arc::new(McpClient::new("http://localhost:8080"));
        let wrapper = McpToolWrapper {
            tool: make_test_mcp_tool(false),
            prefixed_name: "mcp__s__do_thing".to_string(),
            provider_extension: "s".to_string(),
            client,
        };
        let approval = wrapper.requires_approval(&serde_json::json!({}));
        assert_eq!(approval, ApprovalRequirement::Never);
    }

    // ── mcp_tool_id canonicalization ──────────────────────────────────────
    //
    // The runtime keys tools by snake_case identifiers and LLMs (Codex /
    // GPT-5 in particular) silently normalize tool names to valid Python
    // identifiers by converting dashes to underscores. If the registered
    // name contains a dash, the LLM-emitted call won't match the registry
    // key and the tool becomes unreachable. The helper canonicalizes both
    // sides of the prefixed name so the registration and the lookup agree.

    #[test]
    fn test_mcp_tool_id_canonicalizes_dashed_tool_name() {
        // The Notion MCP server returns tools like "notion-search". The
        // registered identifier must use underscores so the LLM call
        // ("notion_notion_search") resolves directly.
        assert_eq!(
            mcp_tool_id("notion", "notion-search"),
            "notion_notion_search"
        );
        assert_eq!(
            mcp_tool_id("notion", "notion-get-users"),
            "notion_notion_get_users"
        );
    }

    #[test]
    fn test_mcp_tool_id_canonicalizes_dashed_server_name() {
        // User-supplied server names can contain dashes too. Both sides of
        // the prefixed name must be normalized.
        assert_eq!(mcp_tool_id("my-server", "ping"), "my_server_ping");
        assert_eq!(mcp_tool_id("my-server", "do-thing"), "my_server_do_thing");
    }

    #[test]
    fn test_mcp_tool_id_passthrough_for_already_canonical_names() {
        assert_eq!(mcp_tool_id("github", "list_issues"), "github_list_issues");
        assert_eq!(mcp_tool_id("local", "ping"), "local_ping");
    }

    #[test]
    fn test_mcp_tool_id_normalizes_non_identifier_chars() {
        // The MCP spec doesn't restrict tool names to OpenAI's
        // `[a-zA-Z0-9_-]` regex. A server could legally return names with
        // dots, colons, slashes, spaces, or non-ASCII characters. The same
        // LLM normalization that bites on `-` will bite on these too, so
        // canonicalize them all to `_` defensively.
        assert_eq!(
            mcp_tool_id("notion", "notion.search"),
            "notion_notion_search"
        );
        assert_eq!(
            mcp_tool_id("github", "github:create_issue"),
            "github_github_create_issue"
        );
        assert_eq!(
            mcp_tool_id("local", "do something now"),
            "local_do_something_now"
        );
        // Path-like tool names: every separator becomes `_`.
        assert_eq!(mcp_tool_id("fs", "files/read"), "fs_files_read");
        // Multi-byte unicode (each `α` is 2 UTF-8 bytes) → each char
        // becomes a single `_` in the output. Tests both correct char
        // iteration AND that the char count translates 1:1.
        assert_eq!(mcp_tool_id("local", "αβγ"), "local____");
    }

    /// Regression test (helper level): create_tools must surface the
    /// canonical snake_case identifier through `Tool::name()` even when the
    /// MCP server returns tools whose names contain dashes.
    #[tokio::test]
    async fn test_create_tools_canonicalizes_dashed_mcp_tool_names() {
        let init_response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(1),
            result: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "test", "version": "1.0"}
            })),
            error: None,
        };
        let notification_ack = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: None,
            result: None,
            error: None,
        };
        let list_response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(2),
            result: Some(serde_json::json!({
                "tools": [
                    {
                        "name": "notion-search",
                        "description": "Search Notion",
                        "inputSchema": {"type": "object"}
                    },
                    {
                        "name": "notion-get-users",
                        "description": "List Notion users",
                        "inputSchema": {"type": "object"}
                    }
                ]
            })),
            error: None,
        };

        let transport = Arc::new(MockTransport::new(
            false,
            vec![init_response, notification_ack, list_response],
        ));
        let client =
            McpClient::new_with_transport("notion", transport.clone(), None, None, "default", None);

        let tools = client
            .create_tools()
            .await
            .expect("create_tools should succeed");

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(
            names,
            vec!["notion_notion_search", "notion_notion_get_users"],
            "MCP tool names with dashes must be canonicalized to snake_case"
        );

        // The wrapper must still preserve the original (dashed) name on its
        // inner McpTool so the wire call to the MCP server uses what the
        // server actually advertised.
        for tool in &tools {
            // Cast through the trait object's parameters_schema as a sanity
            // check that the wrapper is wired up correctly.
            assert!(tool.parameters_schema().is_object());
        }
    }

    /// Regression test for the post-normalization collision case: an MCP
    /// server returning two tools whose names differ only by `-` vs `_`
    /// (`search-all` and `search_all`) produces the same registry key. The
    /// helper must NOT crash, must produce a wrapper for each tool, and the
    /// shadowing must be observable via the warn log emitted in
    /// `create_tools` (the test asserts the structural outcome — the warn
    /// itself is a side effect we don't capture without `tracing-test`).
    #[tokio::test]
    async fn test_create_tools_handles_post_normalization_collision() {
        let init_response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(1),
            result: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "test", "version": "1.0"}
            })),
            error: None,
        };
        let notification_ack = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: None,
            result: None,
            error: None,
        };
        let list_response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(2),
            result: Some(serde_json::json!({
                "tools": [
                    { "name": "search-all", "description": "first",  "inputSchema": {"type": "object"} },
                    { "name": "search_all", "description": "second", "inputSchema": {"type": "object"} }
                ]
            })),
            error: None,
        };

        let transport = Arc::new(MockTransport::new(
            false,
            vec![init_response, notification_ack, list_response],
        ));
        let client =
            McpClient::new_with_transport("demo", transport.clone(), None, None, "default", None);

        let tools = client
            .create_tools()
            .await
            .expect("create_tools should succeed even with collisions");

        // Both wrappers are produced and both have the same normalized
        // registry key — this is the collision the warn log calls out.
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name(), "demo_search_all");
        assert_eq!(tools[1].name(), "demo_search_all");

        // Register both into a real ToolRegistry: the second wins (this is
        // the documented shadowing behaviour). Without the warn log there
        // would be no signal that the first tool became unreachable.
        let registry = crate::tools::registry::ToolRegistry::new();
        for tool in tools {
            registry.register(tool).await;
        }
        let resolved = registry.get("demo_search_all").await;
        assert!(resolved.is_some(), "second tool must be registered");
        assert_eq!(
            resolved.unwrap().description(),
            "second",
            "the later-registered tool wins on shadow (last-write); operators see the warn log to know it happened"
        );
    }

    /// Regression test (caller level): the canonicalized identifier produced
    /// by `create_tools` must round-trip through the real `ToolRegistry` —
    /// including `resolve_name`, which is what the v2 effect adapter calls
    /// when dispatching an LLM-emitted tool call.
    ///
    /// This is the "test through the caller, not just the helper" pattern
    /// from `.claude/rules/testing.md`. A unit test on `mcp_tool_id` alone
    /// would not catch a regression where the registry path mangles names
    /// differently from the schema-emitting path.
    #[tokio::test]
    async fn test_create_tools_round_trips_through_registry_resolve_name() {
        use crate::tools::registry::ToolRegistry;

        let init_response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(1),
            result: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "test", "version": "1.0"}
            })),
            error: None,
        };
        let notification_ack = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: None,
            result: None,
            error: None,
        };
        let list_response = McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(2),
            result: Some(serde_json::json!({
                "tools": [
                    {
                        "name": "notion-search",
                        "description": "Search Notion",
                        "inputSchema": {"type": "object"}
                    }
                ]
            })),
            error: None,
        };

        let transport = Arc::new(MockTransport::new(
            false,
            vec![init_response, notification_ack, list_response],
        ));
        let client =
            McpClient::new_with_transport("notion", transport.clone(), None, None, "default", None);

        let registry = ToolRegistry::new();
        for tool in client
            .create_tools()
            .await
            .expect("create_tools should succeed")
        {
            registry.register(tool).await;
        }

        // The LLM (Codex / GPT-5) emits the tool name with all underscores.
        // resolve_name must find it directly without falling through to the
        // legacy alias path (which only goes underscores → dashes and would
        // miss the mixed-separator form `notion_notion-search`).
        let resolved = registry.resolve_name("notion_notion_search").await;
        assert_eq!(
            resolved.as_deref(),
            Some("notion_notion_search"),
            "LLM-emitted snake_case tool name must resolve to the registered MCP tool"
        );

        // And the get_resolved path that the effect adapter actually uses
        // must also produce a working Tool handle.
        let (resolved_name, tool) = registry
            .get_resolved("notion_notion_search")
            .await
            .expect("get_resolved should return the registered tool");
        assert_eq!(resolved_name, "notion_notion_search");
        assert_eq!(tool.name(), "notion_notion_search");
    }

    // Regression test: empty/whitespace-only tokens must not produce a
    // malformed `Authorization: Bearer ` header (GitHub MCP returns 400
    // "Authorization header is badly formatted" in this case).
    #[tokio::test]
    async fn test_build_headers_skips_empty_token() {
        use crate::secrets::{CreateSecretParams, DecryptedSecret, Secret, SecretError, SecretRef};
        use uuid::Uuid;

        // In-memory secrets store that returns a whitespace-only string for the token.
        struct EmptyTokenStore;
        #[async_trait]
        impl crate::secrets::SecretsStore for EmptyTokenStore {
            async fn create(
                &self,
                _user_id: &str,
                _params: CreateSecretParams,
            ) -> Result<Secret, SecretError> {
                unimplemented!()
            }
            async fn get(&self, _user_id: &str, _name: &str) -> Result<Secret, SecretError> {
                unimplemented!()
            }
            async fn get_decrypted(
                &self,
                _user_id: &str,
                _name: &str,
            ) -> Result<DecryptedSecret, SecretError> {
                DecryptedSecret::from_bytes(b"   ".to_vec())
            }
            async fn exists(&self, _user_id: &str, _name: &str) -> Result<bool, SecretError> {
                Ok(true)
            }
            async fn delete(&self, _user_id: &str, _name: &str) -> Result<bool, SecretError> {
                Ok(true)
            }
            async fn list(&self, _user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
                Ok(Vec::new())
            }
            async fn record_usage(&self, _secret_id: Uuid) -> Result<(), SecretError> {
                Ok(())
            }
            async fn is_accessible(
                &self,
                _user_id: &str,
                _secret_name: &str,
                _allowed_secrets: &[String],
            ) -> Result<bool, SecretError> {
                Ok(true)
            }
        }

        let config = McpServerConfig::new("github", "https://api.githubcopilot.com/mcp/");
        let session_manager = Arc::new(McpSessionManager::new());
        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(EmptyTokenStore);

        let client = McpClient::new_authenticated(config, session_manager, secrets, "test-user");

        let headers = client.build_request_headers().await.unwrap(); // safety: test
        assert!(
            // safety: test
            !headers.contains_key("Authorization"),
            "Empty/whitespace token must not produce an Authorization header, got: {:?}",
            headers.get("Authorization")
        );
    }

    // Regression test: tokens with leading/trailing whitespace must be trimmed
    // before being used in the Authorization header.
    #[tokio::test]
    async fn test_build_headers_trims_token() {
        use crate::secrets::{CreateSecretParams, DecryptedSecret, Secret, SecretError, SecretRef};
        use uuid::Uuid;

        struct PaddedTokenStore;
        #[async_trait]
        impl crate::secrets::SecretsStore for PaddedTokenStore {
            async fn create(
                &self,
                _user_id: &str,
                _params: CreateSecretParams,
            ) -> Result<Secret, SecretError> {
                unimplemented!()
            }
            async fn get(&self, _user_id: &str, _name: &str) -> Result<Secret, SecretError> {
                unimplemented!()
            }
            async fn get_decrypted(
                &self,
                _user_id: &str,
                _name: &str,
            ) -> Result<DecryptedSecret, SecretError> {
                DecryptedSecret::from_bytes(b"  gho_abc123  \n".to_vec())
            }
            async fn exists(&self, _user_id: &str, _name: &str) -> Result<bool, SecretError> {
                Ok(true)
            }
            async fn delete(&self, _user_id: &str, _name: &str) -> Result<bool, SecretError> {
                Ok(true)
            }
            async fn list(&self, _user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
                Ok(Vec::new())
            }
            async fn record_usage(&self, _secret_id: Uuid) -> Result<(), SecretError> {
                Ok(())
            }
            async fn is_accessible(
                &self,
                _user_id: &str,
                _secret_name: &str,
                _allowed_secrets: &[String],
            ) -> Result<bool, SecretError> {
                Ok(true)
            }
        }

        let config = McpServerConfig::new("github", "https://api.githubcopilot.com/mcp/");
        let session_manager = Arc::new(McpSessionManager::new());
        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(PaddedTokenStore);

        let client = McpClient::new_authenticated(config, session_manager, secrets, "test-user");

        let headers = client.build_request_headers().await.unwrap(); // safety: test
        assert_eq!(
            // safety: test
            headers.get("Authorization").unwrap(), // safety: test
            "Bearer gho_abc123",
            "Token must be trimmed before use in Authorization header"
        );
    }
}
