//! MCP client for connecting to MCP servers.
//!
//! Supports both local (unauthenticated) and hosted (OAuth-authenticated) servers.
//! Uses pluggable transports (HTTP, stdio, Unix) via the `McpTransport` trait.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
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
    server_name: String,

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
        let name = extract_server_name(&url);
        let transport = Arc::new(HttpMcpTransport::new(url.clone(), name.clone()));

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
        let name: String = server_name.into().replace('-', "_");
        let url: String = server_url.into();
        let transport = Arc::new(HttpMcpTransport::new(url.clone(), name.clone()));

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
        let transport = Arc::new(HttpMcpTransport::new(
            config.url.clone(),
            config.name.clone(),
        ));

        Ok(Self {
            transport,
            server_url: config.url.clone(),
            server_name: config.name.clone(),
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
        let transport = Arc::new(
            HttpMcpTransport::new(config.url.clone(), config.name.clone())
                .with_session_manager(session_manager.clone()),
        );

        let custom_headers = config.headers.clone();

        Self {
            transport,
            server_url: config.url.clone(),
            server_name: config.name.clone(),
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
        let name: String = server_name.into();
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

    /// Get the server name.
    pub fn server_name(&self) -> &str {
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
            &self.server_name,
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
                            "MCP server '{}' requires authentication. Run: bastionclaw mcp auth {}",
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

    /// Inject the Trinity delegation credential into `arguments` for `t3n-mcp` tool calls.
    ///
    /// Reads the per-user `t3n_delegation_token` secret (a JSON object with
    /// `credential_jcs`, `user_sig`, and `agent_pubkey` produced by the Trinity FE),
    /// then merges `credential_jcs_b64u` and `user_sig_b64u` into `arguments` so the
    /// sidecar can forward them to the Trinity node via `runPayroll`'s schema fields.
    ///
    /// Fails closed — returns an error when the secret is absent or malformed rather
    /// than forwarding the call without a credential.
    async fn inject_t3n_delegation_credential(
        &self,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let secrets = self.secrets.as_ref().ok_or_else(|| {
            ToolError::ExternalService(
                "t3n-mcp: no secrets store available; delegation token cannot be read".to_string(),
            )
        })?;

        let raw = secrets
            .get_decrypted(
                &self.user_id,
                crate::tools::mcp::config::T3N_DELEGATION_TOKEN_SECRET,
            )
            .await
            .map_err(|_| {
                ToolError::ExternalService(
                    "t3n-mcp: no delegation token configured — paste your Trinity delegation \
                     credential into the t3n-mcp extension setup form before invoking payroll tools"
                        .to_string(),
                )
            })?;

        let token_str = raw.expose();

        let token: serde_json::Value = serde_json::from_str(token_str).map_err(|e| {
            ToolError::ExternalService(format!(
                "t3n-mcp: stored delegation token is not valid JSON: {e}"
            ))
        })?;

        let credential_jcs = token
            .get("credential_jcs")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::ExternalService(
                    "t3n-mcp: delegation token missing required field 'credential_jcs'".to_string(),
                )
            })?
            .to_string();

        let user_sig = token
            .get("user_sig")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::ExternalService(
                    "t3n-mcp: delegation token missing required field 'user_sig'".to_string(),
                )
            })?
            .to_string();

        // Decode the inner credential JCS to extract org_did, which is injected
        // server-side so the LLM never needs to supply it (and can't substitute
        // the wrong value from session context).
        let cred_bytes = {
            use base64::Engine as _;
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(credential_jcs.trim_end_matches('='))
                .map_err(|e| {
                    ToolError::ExternalService(format!(
                        "t3n-mcp: stored credential_jcs is not valid base64url: {e}"
                    ))
                })?
        };
        let cred_json: serde_json::Value =
            serde_json::from_slice(&cred_bytes).map_err(|e| {
                ToolError::ExternalService(format!(
                    "t3n-mcp: stored credential_jcs does not decode to JSON: {e}"
                ))
            })?;
        let credential_org_did = cred_json
            .get("org_did")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::ExternalService(
                    "t3n-mcp: stored credential is missing 'org_did'".to_string(),
                )
            })?
            .to_string();

        let mut merged = match arguments {
            serde_json::Value::Object(map) => map,
            other => {
                // Caller passed a non-object value; wrap it so the credential fields
                // can be merged in without losing the original payload.
                let mut m = serde_json::Map::new();
                m.insert("__args".to_string(), other);
                m
            }
        };

        // Cross-check if the LLM supplied org_did; the credential's value is authoritative.
        if let Some(caller_org_did) = merged.get("org_did").and_then(|v| v.as_str()) {
            if caller_org_did != credential_org_did {
                return Err(ToolError::ExternalService(format!(
                    "t3n-mcp: org_did mismatch — caller supplied '{caller_org_did}', \
                     stored credential is for '{credential_org_did}'. \
                     The credential's org_did is authoritative."
                )));
            }
        }
        merged.insert(
            "org_did".to_string(),
            serde_json::Value::String(credential_org_did),
        );

        merged.insert(
            "credential_jcs_b64u".to_string(),
            serde_json::Value::String(credential_jcs),
        );
        merged.insert(
            "user_sig_b64u".to_string(),
            serde_json::Value::String(user_sig),
        );

        Ok(serde_json::Value::Object(merged))
    }

    /// Call a tool on the MCP server.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<CallToolResult, ToolError> {
        self.initialize().await?;

        // Inject the per-user Trinity delegation credential for every t3n-mcp call.
        // The sidecar's runPayroll tool accepts `credential_jcs_b64u` and `user_sig_b64u`
        // as schema fields, so riding the credential in the JSON-RPC params requires no
        // unix-transport changes (phase-0 approach; header plumbing is phase 1).
        let arguments =
            if self.server_name == crate::tools::mcp::config::T3N_MCP_SERVER_NAME_NORMALISED {
                self.inject_t3n_delegation_credential(arguments).await?
            } else {
                arguments
            };

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
            let id = mcp_tool_id(&self.server_name, &t.name);
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
                let prefixed_name = mcp_tool_id(&self.server_name, &t.name);
                Arc::new(McpToolWrapper {
                    tool: t,
                    prefixed_name,
                    provider_extension: self.server_name.clone(),
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
/// user-supplied server names (`my-server`). The BastionClaw runtime converges
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

        // Check the MCP protocol-level error flag first.
        let content: String = result
            .content
            .iter()
            .filter_map(|b| b.as_text())
            .collect::<Vec<_>>()
            .join("\n");
        if result.is_error {
            return Err(ToolError::ExecutionFailed(content));
        }

        // Also check for handler-reported errors (followup #3): a handler that
        // returned `buildToolResponse("error", …)` sets `status: "error"` in
        // the content but leaves `is_error` unset because the JSON-RPC call
        // itself succeeded.  Without this check, such responses reach the
        // caller as `Ok(…)` and the chat-history entry gets `has_error=false`,
        // showing a spurious ✓ in the UI.
        if let Some(details) = inspect_handler_error(&result) {
            let mut msg = format!("MCP handler error: {}", details.message);
            if let Some(reason) = details.reason {
                msg.push_str(&format!(" (reason: {reason})"));
            }
            return Err(ToolError::ExecutionFailed(msg));
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

/// Details extracted from a handler-reported error response.
pub(crate) struct HandlerErrorDetails {
    pub message: String,
    pub reason: Option<String>,
}

/// Extract error details from a JSON object that carries `status: "error"`.
fn pick_error_details(payload: &serde_json::Value) -> Option<HandlerErrorDetails> {
    if payload.get("status").and_then(|v| v.as_str()) != Some("error") {
        return None;
    }
    let message = payload
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let reason = payload
        .get("reason")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(HandlerErrorDetails { message, reason })
}

/// Inspect a `CallToolResult` for a handler-reported error discriminant.
///
/// Mirrors the TypeScript `inspectErrorResponse` from
/// `client/mcp/t3n-mcp/src/server/tools/shared/responseUtils.ts` (followup #3).
///
/// `buildToolResponse("error", …)` always writes both `structuredContent` and
/// `content[0].text` (JSON-serialised).  We prefer `structuredContent` as the
/// authoritative source: if it is present we use it exclusively and do not fall
/// through to the text block.  The text-block path covers handlers that produce
/// the same shape without going through `buildToolResponse`.
///
/// Returns `None` for success responses, non-matching shapes, or parse errors
/// — callers treat `None` as "no handler-reported failure detected".
pub(crate) fn inspect_handler_error(result: &CallToolResult) -> Option<HandlerErrorDetails> {
    // When structuredContent is present it is authoritative — check it and
    // return immediately regardless of whether it signals an error or not.
    if let Some(structured) = &result.structured_content {
        return pick_error_details(structured);
    }

    // Fall back to the first text content block's JSON payload for handlers
    // that do not populate structuredContent.
    let text = result.content.first().and_then(|b| b.as_text())?;
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    pick_error_details(&parsed)
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
    use crate::tools::mcp::protocol::ContentBlock;

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

    // ── t3n-mcp delegation credential injection ─────────────────────────────
    //
    // Tests for `inject_t3n_delegation_credential` covering present/absent/
    // malformed sentinel cases plus the org_did cross-check behaviour.
    // The caller-level integration test below drives `call_tool` itself
    // (the "Test Through the Caller, Not Just the Helper" rule from
    // `.claude/rules/testing.md`).

    fn make_test_secrets_store() -> std::sync::Arc<dyn crate::secrets::SecretsStore + Send + Sync> {
        use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
        let key =
            secrecy::SecretString::from(crate::testing::credentials::TEST_CRYPTO_KEY.to_string());
        let crypto = std::sync::Arc::new(SecretsCrypto::new(key).expect("test crypto"));
        std::sync::Arc::new(InMemorySecretsStore::new(crypto))
    }

    fn t3n_mcp_client_with_secrets(
        secrets: std::sync::Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
    ) -> McpClient {
        McpClient::new_with_transport(
            crate::tools::mcp::config::T3N_MCP_SERVER_NAME_NORMALISED,
            Arc::new(MockTransport::new(false, vec![])),
            None,
            Some(secrets),
            "test-user",
            None,
        )
    }

    /// Build a base64url-encoded credential JCS JSON string for use in tests.
    fn make_credential_jcs_b64u(org_did: &str) -> String {
        use base64::Engine as _;
        let inner = serde_json::json!({
            "vc_id": "test-vc-id",
            "user_did": "did:t3n:aaaa000000000000000000000000000000000000",
            "org_did": org_did,
            "not_before_secs": 1700000000u64,
            "not_after_secs": 1800000000u64,
        });
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(inner.to_string().as_bytes())
    }

    /// Build a minimal delegation token JSON string with a valid credential_jcs.
    fn make_token_json(org_did: &str) -> String {
        serde_json::json!({
            "credential_jcs": make_credential_jcs_b64u(org_did),
            "user_sig": "usig-value",
            "agent_pubkey": "pubkey-value",
        })
        .to_string()
    }

    const TEST_ORG_DID: &str = "did:t3n:a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";

    #[tokio::test]
    async fn inject_t3n_delegation_credential_merges_fields_when_secret_present() {
        use crate::secrets::CreateSecretParams;

        let store = make_test_secrets_store();
        let token_json = make_token_json(TEST_ORG_DID);
        store
            .create(
                "test-user",
                CreateSecretParams::new(
                    crate::tools::mcp::config::T3N_DELEGATION_TOKEN_SECRET,
                    &token_json,
                ),
            )
            .await
            .expect("store delegation token");

        let client = t3n_mcp_client_with_secrets(store);
        let original_args = serde_json::json!({"cycle_id": "2025-01"});

        let merged = client
            .inject_t3n_delegation_credential(original_args)
            .await
            .expect("inject should succeed when secret is present");

        assert_eq!(merged["credential_jcs_b64u"], make_credential_jcs_b64u(TEST_ORG_DID));
        assert_eq!(merged["user_sig_b64u"], "usig-value");
        assert_eq!(merged["org_did"], TEST_ORG_DID);
        assert_eq!(merged["cycle_id"], "2025-01");
    }

    #[tokio::test]
    async fn inject_injects_org_did_when_caller_omits_it() {
        use crate::secrets::CreateSecretParams;

        let store = make_test_secrets_store();
        store
            .create(
                "test-user",
                CreateSecretParams::new(
                    crate::tools::mcp::config::T3N_DELEGATION_TOKEN_SECRET,
                    &make_token_json(TEST_ORG_DID),
                ),
            )
            .await
            .expect("store delegation token");

        let client = t3n_mcp_client_with_secrets(store);
        let args = serde_json::json!({"cycle_id": "2025-06"});

        let merged = client
            .inject_t3n_delegation_credential(args)
            .await
            .expect("inject should succeed");

        assert_eq!(
            merged["org_did"], TEST_ORG_DID,
            "org_did from the stored credential must be injected when caller omits it"
        );
    }

    #[tokio::test]
    async fn inject_accepts_matching_caller_org_did() {
        use crate::secrets::CreateSecretParams;

        let store = make_test_secrets_store();
        store
            .create(
                "test-user",
                CreateSecretParams::new(
                    crate::tools::mcp::config::T3N_DELEGATION_TOKEN_SECRET,
                    &make_token_json(TEST_ORG_DID),
                ),
            )
            .await
            .expect("store delegation token");

        let client = t3n_mcp_client_with_secrets(store);
        let args = serde_json::json!({"org_did": TEST_ORG_DID, "cycle_id": "2025-06"});

        let merged = client
            .inject_t3n_delegation_credential(args)
            .await
            .expect("matching org_did must be accepted");

        assert_eq!(merged["org_did"], TEST_ORG_DID);
    }

    #[tokio::test]
    async fn inject_rejects_mismatching_caller_org_did() {
        use crate::secrets::CreateSecretParams;

        let store = make_test_secrets_store();
        store
            .create(
                "test-user",
                CreateSecretParams::new(
                    crate::tools::mcp::config::T3N_DELEGATION_TOKEN_SECRET,
                    &make_token_json(TEST_ORG_DID),
                ),
            )
            .await
            .expect("store delegation token");

        let client = t3n_mcp_client_with_secrets(store);
        let wrong_did = "did:t3n:0000000000000000000000000000000000000000";
        let args = serde_json::json!({"org_did": wrong_did});

        let err = client
            .inject_t3n_delegation_credential(args)
            .await
            .expect_err("mismatching org_did must be rejected");

        let msg = err.to_string();
        assert!(
            msg.contains("org_did mismatch"),
            "error must name the mismatch: {msg}"
        );
        assert!(msg.contains(wrong_did), "error must include caller's value: {msg}");
        assert!(msg.contains(TEST_ORG_DID), "error must include credential's value: {msg}");
    }

    #[tokio::test]
    async fn inject_t3n_delegation_credential_fails_closed_when_secret_absent() {
        let store = make_test_secrets_store();
        let client = t3n_mcp_client_with_secrets(store);

        let err = client
            .inject_t3n_delegation_credential(serde_json::json!({}))
            .await
            .expect_err("must fail when delegation token not configured");

        let msg = err.to_string();
        assert!(
            msg.contains("no delegation token configured"),
            "error must guide the user to the setup form: {msg}"
        );
    }

    #[tokio::test]
    async fn inject_t3n_delegation_credential_fails_closed_when_secret_malformed() {
        use crate::secrets::CreateSecretParams;

        let store = make_test_secrets_store();
        store
            .create(
                "test-user",
                CreateSecretParams::new(
                    crate::tools::mcp::config::T3N_DELEGATION_TOKEN_SECRET,
                    "not-valid-json{{{",
                ),
            )
            .await
            .expect("store bad token");

        let client = t3n_mcp_client_with_secrets(store);

        let err = client
            .inject_t3n_delegation_credential(serde_json::json!({}))
            .await
            .expect_err("must fail when delegation token is not valid JSON");

        let msg = err.to_string();
        assert!(
            msg.contains("not valid JSON"),
            "error must mention JSON parse failure: {msg}"
        );
    }

    /// Caller-level integration test: drives `McpClient::call_tool` for a
    /// `t3n_mcp` server and asserts that the JSON-RPC params forwarded to the
    /// transport contain the merged `credential_jcs_b64u` and `user_sig_b64u`
    /// fields from the stored delegation token.
    ///
    /// This is the "Test Through the Caller, Not Just the Helper" pattern from
    /// `.claude/rules/testing.md`.  The three unit tests above exercise the
    /// `inject_t3n_delegation_credential` helper in isolation; this test
    /// exercises `call_tool` — the caller that invokes the helper — so that a
    /// future refactor that bypasses the helper (or changes when it is called)
    /// would be caught here rather than only at runtime.
    #[tokio::test]
    async fn call_tool_injects_delegation_credential_for_t3n_mcp() {
        use crate::secrets::CreateSecretParams;

        // ── Build a mock transport that captures the full request ──────
        struct CapturingTransport {
            requests: std::sync::Mutex<Vec<McpRequest>>,
        }
        #[async_trait]
        impl McpTransport for CapturingTransport {
            async fn send(
                &self,
                request: &McpRequest,
                _headers: &HashMap<String, String>,
            ) -> Result<McpResponse, crate::tools::tool::ToolError> {
                self.requests.lock().unwrap().push(request.clone());
                // Return appropriate responses depending on the method so
                // that `initialize()` completes before `call_tool` runs.
                let result = match request.method.as_str() {
                    "initialize" => serde_json::json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {},
                        "serverInfo": {"name": "t3n-mcp", "version": "1.0"}
                    }),
                    "notifications/initialized" => {
                        // Notification — return empty response (id is None).
                        return Ok(McpResponse {
                            jsonrpc: "2.0".to_string(),
                            id: None,
                            result: None,
                            error: None,
                        });
                    }
                    _ => serde_json::json!({
                        "content": [{"type": "text", "text": "ok"}],
                        "is_error": false
                    }),
                };
                Ok(McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: Some(result),
                    error: None,
                })
            }
            async fn shutdown(&self) -> Result<(), crate::tools::tool::ToolError> {
                Ok(())
            }
            fn supports_http_features(&self) -> bool {
                false
            }
        }

        // ── Seed the delegation token in an in-memory store ───────────
        let store = make_test_secrets_store();
        store
            .create(
                "test-user",
                CreateSecretParams::new(
                    crate::tools::mcp::config::T3N_DELEGATION_TOKEN_SECRET,
                    &make_token_json(TEST_ORG_DID),
                ),
            )
            .await
            .expect("store delegation token");

        // ── Build a t3n_mcp client wired to the capturing transport ───
        let transport = Arc::new(CapturingTransport {
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let client = McpClient::new_with_transport(
            crate::tools::mcp::config::T3N_MCP_SERVER_NAME_NORMALISED,
            Arc::clone(&transport) as Arc<dyn McpTransport>,
            None,
            Some(store),
            "test-user",
            None,
        );

        // ── Drive call_tool (the caller) ──────────────────────────────
        let result = client
            .call_tool("runPayroll", serde_json::json!({"cycle_id": "2025-01"}))
            .await
            .expect("call_tool should succeed with delegation token present");
        assert!(!result.is_error);

        // ── Assert the forwarded params contain the injected fields ───
        let requests = transport.requests.lock().unwrap();
        let call_req = requests
            .iter()
            .find(|r| r.method == "tools/call")
            .expect("a tools/call request must have been forwarded");

        let params = call_req
            .params
            .as_ref()
            .expect("tools/call must have params");
        let args = &params["arguments"];

        assert_eq!(
            args["credential_jcs_b64u"],
            make_credential_jcs_b64u(TEST_ORG_DID),
            "credential_jcs_b64u must be injected into the forwarded arguments"
        );
        assert_eq!(
            args["user_sig_b64u"], "usig-value",
            "user_sig_b64u must be injected into the forwarded arguments"
        );
        assert_eq!(
            args["org_did"], TEST_ORG_DID,
            "org_did must be injected from the stored credential"
        );
        assert_eq!(
            args["cycle_id"], "2025-01",
            "caller-supplied params must be preserved alongside injected fields"
        );
    }

    // ── inspect_handler_error ────────────────────────────────────────────────

    fn make_result(content: Vec<ContentBlock>, structured: Option<serde_json::Value>) -> CallToolResult {
        CallToolResult {
            content,
            is_error: false,
            structured_content: structured,
        }
    }

    #[test]
    fn inspect_handler_error_success_structured_returns_none() {
        let result = make_result(
            vec![ContentBlock::Text {
                text: r#"{"status":"success","message":"ok"}"#.to_string(),
            }],
            Some(serde_json::json!({"status": "success", "message": "ok"})),
        );
        assert!(inspect_handler_error(&result).is_none());
    }

    #[test]
    fn inspect_handler_error_error_via_structured_content() {
        let result = make_result(
            vec![ContentBlock::Text {
                text: r#"{"status":"error","message":"bad thing","reason":"vc_id missing"}"#
                    .to_string(),
            }],
            Some(serde_json::json!({
                "status": "error",
                "message": "bad thing",
                "reason": "vc_id missing"
            })),
        );
        let details = inspect_handler_error(&result).expect("should detect error");
        assert_eq!(details.message, "bad thing");
        assert_eq!(details.reason.as_deref(), Some("vc_id missing"));
    }

    #[test]
    fn inspect_handler_error_error_via_text_content_fallback() {
        // No structuredContent — error carried only in text block.
        let result = make_result(
            vec![ContentBlock::Text {
                text: r#"{"status":"error","message":"expired credential"}"#.to_string(),
            }],
            None,
        );
        let details = inspect_handler_error(&result).expect("should detect error via text");
        assert_eq!(details.message, "expired credential");
        assert!(details.reason.is_none());
    }

    #[test]
    fn inspect_handler_error_empty_content_returns_none() {
        let result = make_result(vec![], None);
        assert!(inspect_handler_error(&result).is_none());
    }

    #[test]
    fn inspect_handler_error_non_text_content_returns_none() {
        // Image block — no text to parse.
        let result = make_result(
            vec![ContentBlock::Image {
                data: "base64data".to_string(),
                mime_type: "image/png".to_string(),
            }],
            None,
        );
        assert!(inspect_handler_error(&result).is_none());
    }

    #[test]
    fn inspect_handler_error_invalid_json_in_text_returns_none() {
        let result = make_result(
            vec![ContentBlock::Text {
                text: "not valid json at all".to_string(),
            }],
            None,
        );
        assert!(inspect_handler_error(&result).is_none());
    }

    #[test]
    fn inspect_handler_error_non_object_json_returns_none() {
        let result = make_result(
            vec![ContentBlock::Text {
                text: r#"["status","error"]"#.to_string(),
            }],
            None,
        );
        assert!(inspect_handler_error(&result).is_none());
    }

    #[test]
    fn inspect_handler_error_error_without_reason_field() {
        // reason is optional — absence should not prevent detection.
        let result = make_result(
            vec![ContentBlock::Text {
                text: r#"{"status":"error","message":"something failed"}"#.to_string(),
            }],
            None,
        );
        let details = inspect_handler_error(&result).expect("should detect error");
        assert_eq!(details.message, "something failed");
        assert!(details.reason.is_none());
    }

    #[test]
    fn inspect_handler_error_structured_content_takes_precedence_over_text() {
        // structuredContent says success, text says error — structured wins.
        let result = make_result(
            vec![ContentBlock::Text {
                text: r#"{"status":"error","message":"text says error"}"#.to_string(),
            }],
            Some(serde_json::json!({"status": "success", "message": "structured says ok"})),
        );
        // structured content is checked first and returns None (not an error)
        assert!(inspect_handler_error(&result).is_none());
    }
}
