/// Host-mediated HTTP request issued by a WASM network import.
pub struct WasmHttpRequest {
    pub method: NetworkMethod,
    pub url: String,
    pub body: String,
    /// IP address selected by the policy layer after allowlist/private-IP checks.
    /// Host clients must connect to this pinned address instead of resolving the
    /// hostname again.
    pub resolved_ip: Option<IpAddr>,
    /// Maximum response body bytes the host client may read before failing.
    pub max_response_bytes: Option<u64>,
}

/// Host-mediated HTTP response returned to a WASM network import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmHttpResponse {
    pub status: u16,
    pub body: String,
}

/// Host-mediated HTTP failure with optional byte accounting for partial reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmHostHttpError {
    pub reason: String,
    pub bytes_received: u64,
}

impl WasmHostHttpError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            bytes_received: 0,
        }
    }

    pub fn with_bytes_received(mut self, bytes_received: u64) -> Self {
        self.bytes_received = bytes_received;
        self
    }
}

impl std::fmt::Display for WasmHostHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for WasmHostHttpError {}

impl From<String> for WasmHostHttpError {
    fn from(reason: String) -> Self {
        Self::new(reason)
    }
}

impl From<&str> for WasmHostHttpError {
    fn from(reason: &str) -> Self {
        Self::new(reason)
    }
}

/// Synchronous host HTTP surface exposed to WASM network imports.
///
/// Implementations used behind [`WasmPolicyHttpClient`] must not follow
/// redirects transparently or bypass the validated host/port. The policy wrapper
/// validates the URL before dispatch, pins `resolved_ip`, and passes
/// `max_response_bytes` so clients can enforce response bounds while streaming.
pub trait WasmHostHttp: Send + Sync {
    fn request_utf8(&self, request: WasmHttpRequest)
    -> Result<WasmHttpResponse, WasmHostHttpError>;
}

/// Invocation-scoped host import context for WASM JSON execution.
#[derive(Clone, Default)]
pub struct WasmHostImportContext {
    filesystem: Option<Arc<dyn WasmHostFilesystem>>,
    http: Option<Arc<dyn WasmHostHttp>>,
}

impl std::fmt::Debug for WasmHostImportContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmHostImportContext")
            .field("filesystem", &self.filesystem.is_some())
            .field("http", &self.http.is_some())
            .finish()
    }
}

impl WasmHostImportContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_filesystem(mut self, filesystem: Arc<dyn WasmHostFilesystem>) -> Self {
        self.filesystem = Some(filesystem);
        self
    }

    pub fn with_http(mut self, http: Arc<dyn WasmHostHttp>) -> Self {
        self.http = Some(http);
        self
    }

    pub fn filesystem(&self) -> Option<&Arc<dyn WasmHostFilesystem>> {
        self.filesystem.as_ref()
    }

    pub fn http(&self) -> Option<&Arc<dyn WasmHostHttp>> {
        self.http.as_ref()
    }
}

/// DNS resolver used by the policy wrapper before host HTTP dispatch.
pub trait WasmNetworkResolver: Send + Sync {
    fn resolve_ips(&self, host: &str, port: Option<u16>) -> Result<Vec<IpAddr>, String>;
}

/// System DNS resolver for production host HTTP policy checks.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemNetworkResolver;

impl WasmNetworkResolver for SystemNetworkResolver {
    fn resolve_ips(&self, host: &str, port: Option<u16>) -> Result<Vec<IpAddr>, String> {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![ip]);
        }
        let port = port.unwrap_or(443);
        (host, port)
            .to_socket_addrs()
            .map_err(|error| error.to_string())
            .map(|addrs| addrs.map(|addr| addr.ip()).collect())
    }
}

/// Network-policy enforcing wrapper around a host-provided HTTP client.
#[derive(Debug, Clone)]
pub struct WasmPolicyHttpClient<C, R = SystemNetworkResolver> {
    client: C,
    policy: NetworkPolicy,
    resolver: R,
    max_response_bytes: Option<u64>,
}

impl<C> WasmPolicyHttpClient<C, SystemNetworkResolver> {
    pub fn new(client: C, policy: NetworkPolicy) -> Self {
        Self {
            client,
            policy,
            resolver: SystemNetworkResolver,
            max_response_bytes: Some(DEFAULT_WASM_HTTP_RESPONSE_BYTES),
        }
    }
}

impl<C, R> WasmPolicyHttpClient<C, R> {
    pub fn new_with_resolver(client: C, policy: NetworkPolicy, resolver: R) -> Self {
        Self {
            client,
            policy,
            resolver,
            max_response_bytes: Some(DEFAULT_WASM_HTTP_RESPONSE_BYTES),
        }
    }

    pub fn with_response_body_limit(mut self, max_response_bytes: Option<u64>) -> Self {
        self.max_response_bytes = max_response_bytes;
        self
    }

    pub fn policy(&self) -> &NetworkPolicy {
        &self.policy
    }

    pub fn response_body_limit(&self) -> Option<u64> {
        self.max_response_bytes
    }
}

impl<C, R> WasmHostHttp for WasmPolicyHttpClient<C, R>
where
    C: WasmHostHttp,
    R: WasmNetworkResolver,
{
    fn request_utf8(
        &self,
        mut request: WasmHttpRequest,
    ) -> Result<WasmHttpResponse, WasmHostHttpError> {
        let target = network_target_for_url(&request.url).map_err(WasmHostHttpError::new)?;
        let resolved_ip = validate_network_target(&self.policy, &target, &self.resolver)
            .map_err(WasmHostHttpError::new)?;
        if let Some(max) = self.policy.max_egress_bytes
            && request.body.len() as u64 > max
        {
            return Err(WasmHostHttpError::new(
                "network request exceeds egress limit",
            ));
        }
        request.resolved_ip = Some(resolved_ip);
        request.max_response_bytes = self.max_response_bytes;
        let response = self.client.request_utf8(request)?;
        if let Some(max) = self.max_response_bytes
            && response.body.len() as u64 > max
        {
            return Err(
                WasmHostHttpError::new("network response exceeds body limit")
                    .with_bytes_received(response.body.len() as u64),
            );
        }
        Ok(response)
    }
}
