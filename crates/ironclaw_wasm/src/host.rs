use std::sync::{Arc, Mutex};

use ironclaw_host_api::{
    CapabilityId, NetworkMethod, NetworkPolicy, ResourceScope, RuntimeCredentialInjection,
    RuntimeCredentialSource, RuntimeCredentialTarget, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeKind, SecretHandle, is_sensitive_runtime_response_header,
};
use serde_json::{Map, Value};

use crate::WasmHostError;

/// HTTP request shape exposed through the WIT `host.http-request` import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmHttpRequest {
    pub method: String,
    pub url: String,
    pub headers_json: String,
    pub body: Option<Vec<u8>>,
    pub timeout_ms: Option<u32>,
}

/// HTTP response shape returned to a WASM guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmHttpResponse {
    pub status: u16,
    pub headers_json: String,
    pub body: Vec<u8>,
}

/// Host HTTP seam used by the WIT runtime.
///
/// Production composition should wire this to the shared Reborn runtime egress
/// service. Until that service exists, the default implementation denies every
/// request so WASM cannot perform direct network I/O. The runtime caps
/// `WasmHttpRequest::timeout_ms` to the smaller of the WIT HTTP default (when
/// omitted by the guest) and the remaining execution deadline before calling
/// this trait; implementations must honor that timeout because a
/// synchronous host call cannot be preempted safely once entered.
pub trait WasmHostHttp: Send + Sync {
    fn request(&self, request: WasmHttpRequest) -> Result<WasmHttpResponse, WasmHostError>;
}

/// Fail-closed HTTP host service.
#[derive(Debug, Default)]
pub struct DenyWasmHostHttp;

impl WasmHostHttp for DenyWasmHostHttp {
    fn request(&self, _request: WasmHttpRequest) -> Result<WasmHttpResponse, WasmHostError> {
        Err(WasmHostError::Unavailable(
            "WASM HTTP egress is not configured".to_string(),
        ))
    }
}

/// Recording HTTP host service for tests and development fixtures.
#[derive(Debug)]
pub struct RecordingWasmHostHttp {
    requests: Mutex<Vec<WasmHttpRequest>>,
    response: Result<WasmHttpResponse, WasmHostError>,
}

impl RecordingWasmHostHttp {
    pub fn ok(response: WasmHttpResponse) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            response: Ok(response),
        }
    }

    pub fn err(error: WasmHostError) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            response: Err(error),
        }
    }

    pub fn requests(&self) -> Result<Vec<WasmHttpRequest>, WasmHostError> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| WasmHostError::Failed("recording HTTP request log is poisoned".into()))
    }
}

impl WasmHostHttp for RecordingWasmHostHttp {
    fn request(&self, request: WasmHttpRequest) -> Result<WasmHttpResponse, WasmHostError> {
        self.requests
            .lock()
            .map_err(|_| WasmHostError::Failed("recording HTTP request log is poisoned".into()))?
            .push(request);
        self.response.clone()
    }
}

/// Thin adapter from the WASM WIT HTTP import to the shared Reborn runtime
/// egress service.
///
/// Host composition supplies scope, network policy, and a request-scoped
/// credential provider. The WASM guest supplies only native request fields; this
/// adapter does not create HTTP clients, resolve DNS, apply ad-hoc network
/// policy, or inject credentials itself.
#[derive(Debug, Clone)]
pub struct WasmRuntimeHttpAdapter<E> {
    egress: E,
    scope: ResourceScope,
    capability_id: CapabilityId,
    network_policy: NetworkPolicy,
    credential_provider: Arc<dyn WasmRuntimeCredentialProvider>,
    response_body_limit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmRuntimeCredentialRequest {
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
    pub method: NetworkMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub network_policy: NetworkPolicy,
}

pub trait WasmRuntimeCredentialProvider: Send + Sync + std::fmt::Debug {
    fn credential_injections(
        &self,
        request: &WasmRuntimeCredentialRequest,
    ) -> Result<Vec<RuntimeCredentialInjection>, WasmHostError>;
}

#[derive(Debug, Default)]
pub struct EmptyWasmRuntimeCredentials;

impl WasmRuntimeCredentialProvider for EmptyWasmRuntimeCredentials {
    fn credential_injections(
        &self,
        _request: &WasmRuntimeCredentialRequest,
    ) -> Result<Vec<RuntimeCredentialInjection>, WasmHostError> {
        Ok(Vec::new())
    }
}

/// Host-approved staged credential rule for one WASM HTTP request.
///
/// This type does not grant secret authority by itself. Host composition should
/// build it only from already-authorized `InjectSecretOnce` obligations and
/// destination/injection metadata that was validated outside the guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmStagedRuntimeCredential {
    pub handle: SecretHandle,
    pub target: RuntimeCredentialTarget,
    pub required: bool,
    exact_url: Option<String>,
}

impl WasmStagedRuntimeCredential {
    pub fn for_any_request(
        handle: SecretHandle,
        target: RuntimeCredentialTarget,
        required: bool,
    ) -> Self {
        Self {
            handle,
            target,
            required,
            exact_url: None,
        }
    }

    pub fn for_exact_url(
        handle: SecretHandle,
        target: RuntimeCredentialTarget,
        required: bool,
        exact_url: String,
    ) -> Self {
        Self {
            handle,
            target,
            required,
            exact_url: Some(exact_url),
        }
    }

    fn matches_request(&self, request: &WasmRuntimeCredentialRequest) -> bool {
        match &self.exact_url {
            Some(exact_url) => exact_url == &request.url,
            None => true,
        }
    }
}

/// Concrete WASM credential provider for `InjectSecretOnce` handoffs.
///
/// The provider converts host-approved rules into staged-obligation runtime
/// credential injections using the capability id attached to the adapter.
#[derive(Debug, Clone, Default)]
pub struct WasmStagedRuntimeCredentials {
    credentials: Vec<WasmStagedRuntimeCredential>,
}

impl WasmStagedRuntimeCredentials {
    pub fn new(credentials: Vec<WasmStagedRuntimeCredential>) -> Self {
        Self { credentials }
    }

    pub fn credentials(&self) -> &[WasmStagedRuntimeCredential] {
        &self.credentials
    }
}

impl WasmRuntimeCredentialProvider for WasmStagedRuntimeCredentials {
    fn credential_injections(
        &self,
        request: &WasmRuntimeCredentialRequest,
    ) -> Result<Vec<RuntimeCredentialInjection>, WasmHostError> {
        let matched = self
            .credentials
            .iter()
            .filter(|credential| credential.matches_request(request))
            .collect::<Vec<_>>();
        if matched.is_empty() {
            return Ok(Vec::new());
        }

        Ok(matched
            .into_iter()
            .map(|credential| RuntimeCredentialInjection {
                handle: credential.handle.clone(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: request.capability_id.clone(),
                },
                target: credential.target.clone(),
                required: credential.required,
            })
            .collect())
    }
}

impl<E> WasmRuntimeHttpAdapter<E>
where
    E: RuntimeHttpEgress,
{
    pub fn new(
        egress: E,
        scope: ResourceScope,
        capability_id: CapabilityId,
        network_policy: NetworkPolicy,
    ) -> Self {
        Self {
            egress,
            scope,
            capability_id,
            network_policy,
            credential_provider: Arc::new(EmptyWasmRuntimeCredentials),
            response_body_limit: None,
        }
    }

    pub fn with_capability_id(mut self, capability_id: CapabilityId) -> Self {
        self.capability_id = capability_id;
        self
    }

    pub fn with_credential_provider(
        mut self,
        credential_provider: Arc<dyn WasmRuntimeCredentialProvider>,
    ) -> Self {
        self.credential_provider = credential_provider;
        self
    }

    pub fn with_response_body_limit(mut self, response_body_limit: Option<u64>) -> Self {
        self.response_body_limit = response_body_limit;
        self
    }
}

impl<E> WasmHostHttp for WasmRuntimeHttpAdapter<E>
where
    E: RuntimeHttpEgress,
{
    fn request(&self, request: WasmHttpRequest) -> Result<WasmHttpResponse, WasmHostError> {
        let method = wasm_network_method(&request.method)?;
        let headers = decode_wasm_headers(&request.headers_json)?;
        let body = request.body.unwrap_or_default();
        let credential_injections = self
            .credential_provider
            .credential_injections(&WasmRuntimeCredentialRequest {
                scope: self.scope.clone(),
                capability_id: self.capability_id.clone(),
                method,
                url: request.url.clone(),
                headers: headers.clone(),
                network_policy: self.network_policy.clone(),
            })
            .map_err(wasm_credential_provider_error)?;

        let response = self
            .egress
            .execute(RuntimeHttpEgressRequest {
                runtime: RuntimeKind::Wasm,
                scope: self.scope.clone(),
                capability_id: self.capability_id.clone(),
                method,
                url: request.url,
                headers,
                body,
                network_policy: self.network_policy.clone(),
                credential_injections,
                response_body_limit: self.response_body_limit,
                timeout_ms: request.timeout_ms,
            })
            .map_err(wasm_http_error)?;

        Ok(WasmHttpResponse {
            status: response.status,
            headers_json: encode_wasm_headers(response.headers)?,
            body: response.body,
        })
    }
}

fn wasm_network_method(method: &str) -> Result<NetworkMethod, WasmHostError> {
    match method.trim().to_ascii_uppercase().as_str() {
        "GET" => Ok(NetworkMethod::Get),
        "POST" => Ok(NetworkMethod::Post),
        "PUT" => Ok(NetworkMethod::Put),
        "PATCH" => Ok(NetworkMethod::Patch),
        "DELETE" => Ok(NetworkMethod::Delete),
        "HEAD" => Ok(NetworkMethod::Head),
        _ => Err(WasmHostError::Denied(
            "unsupported WASM HTTP method".to_string(),
        )),
    }
}

fn decode_wasm_headers(headers_json: &str) -> Result<Vec<(String, String)>, WasmHostError> {
    let value = serde_json::from_str::<Value>(headers_json).map_err(|_| {
        WasmHostError::Denied("WASM HTTP headers must be a JSON object".to_string())
    })?;
    let Some(headers) = value.as_object() else {
        return Err(WasmHostError::Denied(
            "WASM HTTP headers must be a JSON object".to_string(),
        ));
    };

    let mut decoded = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        let Some(value) = value.as_str() else {
            return Err(WasmHostError::Denied(
                "WASM HTTP header values must be strings".to_string(),
            ));
        };
        decoded.push((name.clone(), value.to_string()));
    }
    Ok(decoded)
}

fn encode_wasm_headers(headers: Vec<(String, String)>) -> Result<String, WasmHostError> {
    let mut encoded = Map::new();
    for (name, value) in headers {
        if is_sensitive_runtime_response_header(&name) {
            continue;
        }
        if let Some(existing_name) = encoded
            .keys()
            .find(|existing| existing.eq_ignore_ascii_case(&name))
            .cloned()
            && let Some(Value::String(existing_value)) = encoded.get_mut(&existing_name)
        {
            existing_value.push_str(", ");
            existing_value.push_str(&value);
        } else {
            encoded.insert(name, Value::String(value));
        }
    }
    serde_json::to_string(&encoded)
        .map_err(|_| WasmHostError::Failed("failed to encode WASM HTTP headers".to_string()))
}

fn wasm_http_error(error: RuntimeHttpEgressError) -> WasmHostError {
    let request_was_sent = wasm_http_request_was_sent(&error);
    let reason = wasm_http_error_reason(&error).to_string();
    if request_was_sent {
        return WasmHostError::FailedAfterRequestSent(reason);
    }

    match error {
        RuntimeHttpEgressError::Credential { .. } => WasmHostError::Unavailable(reason),
        RuntimeHttpEgressError::Request { .. } | RuntimeHttpEgressError::Network { .. } => {
            WasmHostError::Denied(reason)
        }
        RuntimeHttpEgressError::Response { .. } => WasmHostError::Failed(reason),
    }
}

fn wasm_http_request_was_sent(error: &RuntimeHttpEgressError) -> bool {
    match error {
        RuntimeHttpEgressError::Credential { .. } | RuntimeHttpEgressError::Request { .. } => false,
        RuntimeHttpEgressError::Network {
            request_bytes,
            response_bytes,
            ..
        } => *request_bytes > 0 || *response_bytes > 0,
        RuntimeHttpEgressError::Response { .. } => true,
    }
}

fn wasm_http_error_reason(error: &RuntimeHttpEgressError) -> &'static str {
    error.stable_runtime_reason()
}

fn wasm_credential_provider_error(_error: WasmHostError) -> WasmHostError {
    WasmHostError::Unavailable("credential_unavailable".to_string())
}

pub trait WasmHostWorkspace: Send + Sync {
    fn read(&self, path: &str) -> Option<String>;
}

#[derive(Debug, Default)]
pub struct DenyWasmHostWorkspace;

impl WasmHostWorkspace for DenyWasmHostWorkspace {
    fn read(&self, _path: &str) -> Option<String> {
        None
    }
}

pub trait WasmHostSecrets: Send + Sync {
    fn exists(&self, name: &str) -> bool;
}

#[derive(Debug, Default)]
pub struct DenyWasmHostSecrets;

impl WasmHostSecrets for DenyWasmHostSecrets {
    fn exists(&self, _name: &str) -> bool {
        false
    }
}

pub trait WasmHostTools: Send + Sync {
    fn invoke(&self, alias: &str, params_json: &str) -> Result<String, WasmHostError>;
}

#[derive(Debug, Default)]
pub struct DenyWasmHostTools;

impl WasmHostTools for DenyWasmHostTools {
    fn invoke(&self, _alias: &str, _params_json: &str) -> Result<String, WasmHostError> {
        Err(WasmHostError::Unavailable(
            "WASM tool invocation is not configured".to_string(),
        ))
    }
}

pub trait WasmHostClock: Send + Sync {
    fn now_millis(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct SystemWasmHostClock;

impl WasmHostClock for SystemWasmHostClock {
    fn now_millis(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0)
    }
}

/// Host services made available to one WASM tool execution.
#[derive(Clone)]
pub struct WitToolHost {
    pub(crate) http: Arc<dyn WasmHostHttp>,
    pub(crate) workspace: Arc<dyn WasmHostWorkspace>,
    pub(crate) secrets: Arc<dyn WasmHostSecrets>,
    pub(crate) tools: Arc<dyn WasmHostTools>,
    pub(crate) clock: Arc<dyn WasmHostClock>,
}

impl WitToolHost {
    pub fn deny_all() -> Self {
        Self {
            http: Arc::new(DenyWasmHostHttp),
            workspace: Arc::new(DenyWasmHostWorkspace),
            secrets: Arc::new(DenyWasmHostSecrets),
            tools: Arc::new(DenyWasmHostTools),
            clock: Arc::new(SystemWasmHostClock),
        }
    }

    pub fn with_http<T>(mut self, http: Arc<T>) -> Self
    where
        T: WasmHostHttp + 'static,
    {
        self.http = http;
        self
    }

    pub fn with_workspace<T>(mut self, workspace: Arc<T>) -> Self
    where
        T: WasmHostWorkspace + 'static,
    {
        self.workspace = workspace;
        self
    }

    pub fn with_secrets<T>(mut self, secrets: Arc<T>) -> Self
    where
        T: WasmHostSecrets + 'static,
    {
        self.secrets = secrets;
        self
    }

    pub fn with_tools<T>(mut self, tools: Arc<T>) -> Self
    where
        T: WasmHostTools + 'static,
    {
        self.tools = tools;
        self
    }

    pub fn with_clock<T>(mut self, clock: Arc<T>) -> Self
    where
        T: WasmHostClock + 'static,
    {
        self.clock = clock;
        self
    }
}

impl Default for WitToolHost {
    fn default() -> Self {
        Self::deny_all()
    }
}
