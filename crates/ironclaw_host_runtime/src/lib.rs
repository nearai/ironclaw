//! Host runtime composition helpers for IronClaw Reborn.
//!
//! This crate is intentionally composition-only. It wires existing host services
//! together without moving authorization, dispatch, process lifecycle, approval,
//! or run-state responsibilities out of their owning crates.

use std::{
    collections::HashMap,
    fmt,
    net::IpAddr,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_approvals::ApprovalResolver;
use ironclaw_authorization::{CapabilityDispatchAuthorizer, CapabilityLeaseStore};
use ironclaw_capabilities::{
    CapabilityHost, CapabilityObligationCompletionRequest, CapabilityObligationError,
    CapabilityObligationFailureKind, CapabilityObligationHandler, CapabilityObligationPhase,
    CapabilityObligationRequest, DispatchProcessExecutor,
};
use ironclaw_dispatcher::{
    RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{AuditSink, EventSink};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    ActionResultSummary, ActionSummary, AuditEnvelope, AuditEventId, AuditStage,
    CapabilityDispatchResult, CapabilityDispatcher, CapabilityId, DecisionSummary, DispatchError,
    EffectKind, NetworkPolicy, Obligation, ResourceScope, RuntimeDispatchErrorKind, RuntimeKind,
    SecretHandle,
};
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutor, McpInvocation};
use ironclaw_network::{
    HardenedHttpEgressClient, HttpEgressClient, HttpEgressError, HttpEgressRequest,
    is_private_or_loopback_ip,
};
use ironclaw_processes::{
    ProcessExecutor, ProcessHost, ProcessResultStore, ProcessServices, ProcessStore,
};
use ironclaw_resources::ResourceGovernor;
use ironclaw_run_state::{ApprovalRequestStore, RunStateStore};
use ironclaw_safety::LeakDetector;
use ironclaw_scripts::{ScriptError, ScriptExecutionRequest, ScriptExecutor, ScriptInvocation};
use ironclaw_secrets::{
    CredentialLocation, CredentialMapping, ExposeSecret, SecretMaterial, SecretStore,
};
use ironclaw_wasm::{
    CapabilityInvocation, WasmError, WasmExecutionRequest, WasmHostHttp, WasmHttpRequest,
    WasmHttpResponse, WasmPolicyHttpClient, WasmRuntime,
};

/// Already-resolved credential material for runtime HTTP egress.
///
/// This type is intentionally configured by the host composition layer after
/// authorization/secret access. It does not fetch secrets by handle.
#[derive(Clone)]
pub struct RuntimeHttpCredential {
    pub mapping: CredentialMapping,
    pub material: SecretMaterial,
}

impl fmt::Debug for RuntimeHttpCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeHttpCredential")
            .field("mapping", &self.mapping)
            .field("material", &"[REDACTED]")
            .finish()
    }
}

/// One-shot runtime secret material staged after `InjectSecretOnce` lease consumption.
///
/// The store is keyed by scoped invocation, capability, and handle. Runtime adapters
/// must use `take(...)` so staged material is removed before it can be reused.
#[derive(Clone, Default)]
pub struct RuntimeSecretInjectionStore {
    secrets: Arc<Mutex<HashMap<RuntimeSecretInjectionKey, SecretMaterial>>>,
}

impl RuntimeSecretInjectionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
        material: SecretMaterial,
    ) -> Result<(), RuntimeSecretInjectionStoreError> {
        self.lock()?.insert(
            RuntimeSecretInjectionKey::new(scope, capability_id, handle),
            material,
        );
        Ok(())
    }

    pub fn take(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, RuntimeSecretInjectionStoreError> {
        Ok(self.lock()?.remove(&RuntimeSecretInjectionKey::new(
            scope,
            capability_id,
            handle,
        )))
    }

    fn lock(
        &self,
    ) -> Result<
        std::sync::MutexGuard<'_, HashMap<RuntimeSecretInjectionKey, SecretMaterial>>,
        RuntimeSecretInjectionStoreError,
    > {
        self.secrets
            .lock()
            .map_err(|_| RuntimeSecretInjectionStoreError::Unavailable)
    }
}

impl fmt::Debug for RuntimeSecretInjectionStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeSecretInjectionStore")
            .field("secrets", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeSecretInjectionStoreError {
    Unavailable,
}

impl fmt::Display for RuntimeSecretInjectionStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("runtime secret injection store unavailable"),
        }
    }
}

impl std::error::Error for RuntimeSecretInjectionStoreError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RuntimeSecretInjectionKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    invocation_id: String,
    capability_id: String,
    handle: String,
}

impl RuntimeSecretInjectionKey {
    fn new(scope: &ResourceScope, capability_id: &CapabilityId, handle: &SecretHandle) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            invocation_id: scope.invocation_id.to_string(),
            capability_id: capability_id.as_str().to_string(),
            handle: handle.as_str().to_string(),
        }
    }
}

/// Dispatcher adapter for the concrete WASM runtime crate.
pub struct WasmRuntimeAdapter {
    runtime: Arc<WasmRuntime>,
    network_policies: Option<Arc<NetworkObligationPolicyStore>>,
    http_client: Option<Arc<dyn WasmHostHttp>>,
    http_egress_client: Option<Arc<dyn HttpEgressClient>>,
    runtime_http_credentials: Vec<RuntimeHttpCredential>,
}

impl WasmRuntimeAdapter {
    pub fn new(runtime: Arc<WasmRuntime>) -> Self {
        Self {
            runtime,
            network_policies: None,
            http_client: None,
            http_egress_client: None,
            runtime_http_credentials: Vec::new(),
        }
    }

    pub fn with_network_policy_store(mut self, store: Arc<NetworkObligationPolicyStore>) -> Self {
        self.network_policies = Some(store);
        self
    }

    pub fn with_http_client<T>(mut self, client: Arc<T>) -> Self
    where
        T: WasmHostHttp + 'static,
    {
        let client: Arc<dyn WasmHostHttp> = client;
        self.http_client = Some(client);
        self
    }

    pub fn with_http_client_dyn(mut self, client: Arc<dyn WasmHostHttp>) -> Self {
        self.http_client = Some(client);
        self
    }

    pub fn with_http_egress_client<T>(mut self, client: Arc<T>) -> Self
    where
        T: HttpEgressClient + 'static,
    {
        let client: Arc<dyn HttpEgressClient> = client;
        self.http_egress_client = Some(client);
        self
    }

    pub fn with_http_egress_client_dyn(mut self, client: Arc<dyn HttpEgressClient>) -> Self {
        self.http_egress_client = Some(client);
        self
    }

    pub fn with_runtime_http_credentials(
        mut self,
        credentials: Vec<RuntimeHttpCredential>,
    ) -> Self {
        self.runtime_http_credentials = credentials;
        self
    }
}

#[async_trait]
impl<F, G> RuntimeAdapter<F, G> for WasmRuntimeAdapter
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let network_policy = self
            .network_policies
            .as_ref()
            .and_then(|store| store.take(&request.scope, request.capability_id));
        let scope = request.scope.clone();
        let execution_request = WasmExecutionRequest {
            package: request.package,
            capability_id: request.capability_id,
            scope: request.scope,
            estimate: request.estimate,
            invocation: CapabilityInvocation {
                input: request.input,
            },
        };
        let execution = if let Some(policy) = network_policy {
            let http: Arc<dyn WasmHostHttp> = if let Some(egress_client) = &self.http_egress_client
            {
                Arc::new(ScopedWasmHttpEgressClient::new(
                    scope,
                    policy,
                    Arc::clone(egress_client),
                    self.runtime_http_credentials.clone(),
                ))
            } else if let Some(http_client) = &self.http_client {
                Arc::new(WasmPolicyHttpClient::new(
                    SharedWasmHostHttp::new(Arc::clone(http_client)),
                    policy,
                ))
            } else {
                return Err(DispatchError::Wasm {
                    kind: RuntimeDispatchErrorKind::NetworkDenied,
                });
            };
            self.runtime
                .execute_extension_json_with_network(
                    request.filesystem,
                    request.governor,
                    execution_request,
                    http,
                )
                .await
        } else {
            self.runtime
                .execute_extension_json(request.filesystem, request.governor, execution_request)
                .await
        }
        .map_err(wasm_dispatch_error)?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
}

#[derive(Clone)]
struct SharedWasmHostHttp {
    inner: Arc<dyn WasmHostHttp>,
}

impl SharedWasmHostHttp {
    fn new(inner: Arc<dyn WasmHostHttp>) -> Self {
        Self { inner }
    }
}

impl WasmHostHttp for SharedWasmHostHttp {
    fn request_utf8(&self, request: WasmHttpRequest) -> Result<WasmHttpResponse, String> {
        self.inner.request_utf8(request)
    }
}

#[derive(Clone)]
struct ScopedWasmHttpEgressClient {
    scope: ResourceScope,
    policy: NetworkPolicy,
    client: Arc<dyn HttpEgressClient>,
    credentials: Vec<RuntimeHttpCredential>,
}

impl ScopedWasmHttpEgressClient {
    fn new(
        scope: ResourceScope,
        policy: NetworkPolicy,
        client: Arc<dyn HttpEgressClient>,
        credentials: Vec<RuntimeHttpCredential>,
    ) -> Self {
        Self {
            scope,
            policy,
            client,
            credentials,
        }
    }
}

impl WasmHostHttp for ScopedWasmHttpEgressClient {
    fn request_utf8(&self, request: WasmHttpRequest) -> Result<WasmHttpResponse, String> {
        let body = request.body.into_bytes();
        LeakDetector::new()
            .scan_http_request(&request.url, &[], Some(&body))
            .map_err(|_| "network request leak blocked".to_string())?;

        let (url, headers) =
            inject_runtime_http_credentials(request.url, Vec::new(), &self.credentials)?;

        let client = Arc::clone(&self.client);
        let request = HttpEgressRequest {
            scope: self.scope.clone(),
            policy: self.policy.clone(),
            method: request.method,
            url,
            headers,
            body,
            timeout: None,
            max_response_bytes: None,
        };
        let response = std::thread::spawn(move || client.request(request))
            .join()
            .map_err(|_| "network transport failed".to_string())?
            .map_err(|error| network_egress_error_label(&error).to_string())?;

        let body_text = String::from_utf8_lossy(&response.body).into_owned();
        let scan = LeakDetector::new().scan(&body_text);
        if scan.should_block {
            return Err("network response leak blocked".to_string());
        }
        let body = scan.redacted_content.unwrap_or(body_text);

        Ok(WasmHttpResponse {
            status: response.status,
            body,
        })
    }
}

fn inject_runtime_http_credentials(
    raw_url: String,
    headers: Vec<(String, String)>,
    credentials: &[RuntimeHttpCredential],
) -> Result<(String, Vec<(String, String)>), String> {
    let Some(host) = url::Url::parse(&raw_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
    else {
        return Ok((raw_url, headers));
    };

    let matching = credentials
        .iter()
        .filter(|credential| credential_matches_host(credential, &host))
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return Ok((raw_url, headers));
    }
    if headers.iter().any(|(name, _)| is_sensitive_header(name)) {
        return Err("manual credential header blocked".to_string());
    }

    let mut url = raw_url;
    let mut headers = headers;
    for credential in matching {
        match &credential.mapping.location {
            CredentialLocation::AuthorizationBearer => {
                headers.push((
                    "Authorization".to_string(),
                    format!("Bearer {}", credential.material.expose_secret()),
                ));
            }
            CredentialLocation::AuthorizationBasic { username } => {
                let encoded = base64_encode(
                    format!("{}:{}", username, credential.material.expose_secret()).as_bytes(),
                );
                headers.push(("Authorization".to_string(), format!("Basic {encoded}")));
            }
            CredentialLocation::Header { name, prefix } => {
                let value = match prefix {
                    Some(prefix) => format!("{}{}", prefix, credential.material.expose_secret()),
                    None => credential.material.expose_secret().to_string(),
                };
                headers.push((name.clone(), value));
            }
            CredentialLocation::QueryParam { name } => {
                let mut parsed = url::Url::parse(&url).map_err(|_| "invalid credential url")?;
                parsed
                    .query_pairs_mut()
                    .append_pair(name, credential.material.expose_secret());
                url = parsed.to_string();
            }
            CredentialLocation::UrlPath { placeholder } => {
                if !url.contains(placeholder) && !credential.mapping.optional {
                    return Err("credential path placeholder missing".to_string());
                }
                url = url.replace(placeholder, credential.material.expose_secret());
            }
        }
    }
    Ok((url, headers))
}

fn credential_matches_host(credential: &RuntimeHttpCredential, host: &str) -> bool {
    credential
        .mapping
        .host_patterns
        .iter()
        .any(|pattern| credential_host_matches_pattern(host, pattern))
}

fn credential_host_matches_pattern(host: &str, pattern: &str) -> bool {
    if host.eq_ignore_ascii_case(pattern) {
        return true;
    }
    if let Some(pattern_host) = pattern.split(':').next()
        && pattern.contains(':')
        && host.eq_ignore_ascii_case(pattern_host)
    {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        let host = host.to_ascii_lowercase();
        let suffix = suffix.to_ascii_lowercase();
        return host.ends_with(&format!(".{suffix}")) && host != suffix;
    }
    false
}

fn is_sensitive_header(name: &str) -> bool {
    const SENSITIVE_HEADERS: &[&str] = &["authorization", "x-api-key", "api-key", "x-auth-token"];
    SENSITIVE_HEADERS
        .iter()
        .any(|sensitive| name.eq_ignore_ascii_case(sensitive))
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i];
        let b1 = if i + 1 < input.len() { input[i + 1] } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] } else { 0 };

        result.push(ALPHABET[(b0 >> 2) as usize] as char);
        result.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < input.len() {
            result.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            result.push('=');
        }
        if i + 2 < input.len() {
            result.push(ALPHABET[(b2 & 0x3f) as usize] as char);
        } else {
            result.push('=');
        }
        i += 3;
    }
    result
}

fn network_egress_error_label(error: &HttpEgressError) -> &'static str {
    match error {
        HttpEgressError::InvalidUrl { .. } => "InvalidUrl",
        HttpEgressError::UnsupportedScheme { .. } => "UnsupportedScheme",
        HttpEgressError::TargetDenied { .. } => "TargetDenied",
        HttpEgressError::PrivateTargetDenied { .. } => "PrivateTargetDenied",
        HttpEgressError::RequestTooLarge { .. } => "RequestTooLarge",
        HttpEgressError::ResponseTooLarge { .. } => "ResponseTooLarge",
        HttpEgressError::RedirectDenied { .. } => "RedirectDenied",
        HttpEgressError::TooManyRedirects { .. } => "TooManyRedirects",
        HttpEgressError::Timeout { .. } => "Timeout",
        HttpEgressError::Transport { .. } => "Transport",
    }
}

/// Dispatcher adapter for the concrete script executor port.
pub struct ScriptRuntimeAdapter {
    runtime: Arc<dyn ScriptExecutor>,
    network_policies: Option<Arc<NetworkObligationPolicyStore>>,
}

impl ScriptRuntimeAdapter {
    pub fn new<T>(runtime: Arc<T>) -> Self
    where
        T: ScriptExecutor + 'static,
    {
        let runtime: Arc<dyn ScriptExecutor> = runtime;
        Self {
            runtime,
            network_policies: None,
        }
    }

    pub fn from_dyn(runtime: Arc<dyn ScriptExecutor>) -> Self {
        Self {
            runtime,
            network_policies: None,
        }
    }

    pub fn with_network_policy_store(mut self, store: Arc<NetworkObligationPolicyStore>) -> Self {
        self.network_policies = Some(store);
        self
    }
}

#[async_trait]
impl<F, G> RuntimeAdapter<F, G> for ScriptRuntimeAdapter
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        if self
            .network_policies
            .as_ref()
            .and_then(|store| store.take(&request.scope, request.capability_id))
            .is_some()
        {
            return Err(DispatchError::Script {
                kind: RuntimeDispatchErrorKind::NetworkDenied,
            });
        }

        let execution = self
            .runtime
            .execute_extension_json(
                request.governor,
                ScriptExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    invocation: ScriptInvocation {
                        input: request.input,
                    },
                },
            )
            .map_err(script_dispatch_error)?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
}

/// Dispatcher adapter for the concrete MCP executor port.
pub struct McpRuntimeAdapter {
    runtime: Arc<dyn McpExecutor>,
    network_policies: Option<Arc<NetworkObligationPolicyStore>>,
}

impl McpRuntimeAdapter {
    pub fn new<T>(runtime: Arc<T>) -> Self
    where
        T: McpExecutor + 'static,
    {
        let runtime: Arc<dyn McpExecutor> = runtime;
        Self {
            runtime,
            network_policies: None,
        }
    }

    pub fn from_dyn(runtime: Arc<dyn McpExecutor>) -> Self {
        Self {
            runtime,
            network_policies: None,
        }
    }

    pub fn with_network_policy_store(mut self, store: Arc<NetworkObligationPolicyStore>) -> Self {
        self.network_policies = Some(store);
        self
    }
}

#[async_trait]
impl<F, G> RuntimeAdapter<F, G> for McpRuntimeAdapter
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        if self
            .network_policies
            .as_ref()
            .and_then(|store| store.take(&request.scope, request.capability_id))
            .is_some()
        {
            return Err(DispatchError::Mcp {
                kind: RuntimeDispatchErrorKind::NetworkDenied,
            });
        }

        let execution = self
            .runtime
            .execute_extension_json(
                request.governor,
                McpExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    invocation: McpInvocation {
                        input: request.input,
                    },
                },
            )
            .await
            .map_err(mcp_dispatch_error)?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
}

/// In-memory policy handoff from obligation handling to runtime adapters.
///
/// Policies are keyed by tenant/user/project/mission/thread/invocation scope and
/// capability id, and are consumed by runtime adapters immediately before the
/// actual runtime dispatch.
#[derive(Debug, Clone, Default)]
pub struct NetworkObligationPolicyStore {
    policies: Arc<Mutex<HashMap<NetworkPolicyKey, NetworkPolicy>>>,
}

impl NetworkObligationPolicyStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        policy: NetworkPolicy,
    ) {
        self.policies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(NetworkPolicyKey::new(scope, capability_id), policy);
    }

    pub fn take(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Option<NetworkPolicy> {
        self.policies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&NetworkPolicyKey::new(scope, capability_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct NetworkPolicyKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    invocation_id: String,
    capability_id: String,
}

impl NetworkPolicyKey {
    fn new(scope: &ResourceScope, capability_id: &CapabilityId) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            invocation_id: scope.invocation_id.to_string(),
            capability_id: capability_id.as_str().to_string(),
        }
    }
}

/// Built-in obligation handler for the current host-runtime slice.
///
/// Supported obligations:
///
/// - `AuditBefore`: emits one metadata-only audit record and fails closed if no
///   audit sink is configured or emission fails.
/// - `ApplyNetworkPolicy`: validates policy metadata and hands the scoped policy
///   to runtime adapters through a configured policy store.
/// - `InjectSecretOnce`: requires a configured [`SecretStore`] and
///   [`RuntimeSecretInjectionStore`], leases the scoped handle once, consumes it,
///   and stages the material for one runtime take.
/// - `AuditAfter`, `RedactOutput`, and `EnforceOutputLimit`: preflight before
///   dispatch, then complete against the dispatch result before returning output.
///
/// Remaining runtime/input/output plumbing obligations remain unsupported and
/// fail closed.
#[derive(Clone, Default)]
pub struct BuiltinObligationHandler {
    audit_sink: Option<Arc<dyn AuditSink>>,
    network_policies: Option<Arc<NetworkObligationPolicyStore>>,
    secret_store: Option<Arc<dyn SecretStore>>,
    secret_injections: Option<Arc<RuntimeSecretInjectionStore>>,
}

impl BuiltinObligationHandler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_audit_sink<T>(mut self, sink: Arc<T>) -> Self
    where
        T: AuditSink + 'static,
    {
        let sink: Arc<dyn AuditSink> = sink;
        self.audit_sink = Some(sink);
        self
    }

    pub fn with_audit_sink_dyn(mut self, sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = Some(sink);
        self
    }

    pub fn with_network_policy_store(mut self, store: Arc<NetworkObligationPolicyStore>) -> Self {
        self.network_policies = Some(store);
        self
    }

    pub fn with_secret_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: SecretStore + 'static,
    {
        let store: Arc<dyn SecretStore> = store;
        self.secret_store = Some(store);
        self
    }

    pub fn with_secret_store_dyn(mut self, store: Arc<dyn SecretStore>) -> Self {
        self.secret_store = Some(store);
        self
    }

    pub fn with_secret_injection_store(mut self, store: Arc<RuntimeSecretInjectionStore>) -> Self {
        self.secret_injections = Some(store);
        self
    }

    async fn emit_audit_before(
        &self,
        request: &CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        let Some(audit_sink) = &self.audit_sink else {
            return Err(CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Audit,
            });
        };

        audit_sink
            .emit_audit(audit_before_record(request))
            .await
            .map_err(|_| CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Audit,
            })
    }

    async fn preflight_secret_injection(
        &self,
        request: &CapabilityObligationRequest<'_>,
        handles: &[SecretHandle],
    ) -> Result<(), CapabilityObligationError> {
        if handles.is_empty() {
            return Ok(());
        }
        let Some(secret_store) = &self.secret_store else {
            return Err(secret_obligation_failed());
        };
        if self.secret_injections.is_none() {
            return Err(secret_obligation_failed());
        }
        for handle in handles {
            let exists = secret_store
                .metadata(&request.context.resource_scope, handle)
                .await
                .map_err(|_| secret_obligation_failed())?
                .is_some();
            if !exists {
                return Err(secret_obligation_failed());
            }
        }
        Ok(())
    }

    async fn inject_secrets(
        &self,
        request: &CapabilityObligationRequest<'_>,
        handles: &[SecretHandle],
    ) -> Result<(), CapabilityObligationError> {
        if handles.is_empty() {
            return Ok(());
        }
        let Some(secret_store) = &self.secret_store else {
            return Err(secret_obligation_failed());
        };
        let Some(secret_injections) = &self.secret_injections else {
            return Err(secret_obligation_failed());
        };

        let mut material = Vec::with_capacity(handles.len());
        for handle in handles {
            let lease = secret_store
                .lease_once(&request.context.resource_scope, handle)
                .await
                .map_err(|_| secret_obligation_failed())?;
            let secret = secret_store
                .consume(&request.context.resource_scope, lease.id)
                .await
                .map_err(|_| secret_obligation_failed())?;
            material.push((handle.clone(), secret));
        }

        for (handle, secret) in material {
            secret_injections
                .insert(
                    &request.context.resource_scope,
                    request.capability_id,
                    &handle,
                    secret,
                )
                .map_err(|_| secret_obligation_failed())?;
        }
        Ok(())
    }

    async fn emit_audit_after(
        &self,
        request: &CapabilityObligationCompletionRequest<'_>,
        output_bytes: u64,
    ) -> Result<(), CapabilityObligationError> {
        let Some(audit_sink) = &self.audit_sink else {
            return Err(CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Audit,
            });
        };

        audit_sink
            .emit_audit(audit_after_record(request, output_bytes))
            .await
            .map_err(|_| CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Audit,
            })
    }
}

#[async_trait]
impl CapabilityObligationHandler for BuiltinObligationHandler {
    async fn satisfy(
        &self,
        request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        let unsupported = unsupported_obligations(request.phase, request.obligations);
        if !unsupported.is_empty() {
            return Err(CapabilityObligationError::Unsupported {
                obligations: unsupported,
            });
        }

        let network_policy = network_policy_obligation(request.obligations)?;
        if network_policy.is_some() && self.network_policies.is_none() {
            return Err(network_obligation_failed());
        }
        let secret_handles = secret_injection_obligations(request.obligations);
        self.preflight_secret_injection(&request, &secret_handles)
            .await?;

        if request
            .obligations
            .iter()
            .any(|obligation| matches!(obligation, Obligation::AuditBefore))
        {
            self.emit_audit_before(&request).await?;
        }

        self.inject_secrets(&request, &secret_handles).await?;

        if let Some(policy) = network_policy {
            let Some(store) = &self.network_policies else {
                return Err(network_obligation_failed());
            };
            store.insert(
                &request.context.resource_scope,
                request.capability_id,
                policy,
            );
        }

        Ok(())
    }

    async fn complete_dispatch(
        &self,
        request: CapabilityObligationCompletionRequest<'_>,
    ) -> Result<CapabilityDispatchResult, CapabilityObligationError> {
        let unsupported = unsupported_completion_obligations(request.phase, request.obligations);
        if !unsupported.is_empty() {
            return Err(CapabilityObligationError::Unsupported {
                obligations: unsupported,
            });
        }

        let mut dispatch = request.dispatch.clone();
        if request
            .obligations
            .iter()
            .any(|obligation| matches!(obligation, Obligation::RedactOutput))
        {
            dispatch.output = redact_output(dispatch.output)?;
        }

        let output_bytes = dispatch_output_bytes(&dispatch.output)?;
        for obligation in request.obligations {
            if let Obligation::EnforceOutputLimit { bytes } = obligation
                && output_bytes > *bytes
            {
                return Err(output_obligation_failed());
            }
        }

        if request
            .obligations
            .iter()
            .any(|obligation| matches!(obligation, Obligation::AuditAfter))
        {
            self.emit_audit_after(&request, output_bytes).await?;
        }

        Ok(dispatch)
    }
}

fn unsupported_obligations(
    phase: CapabilityObligationPhase,
    obligations: &[Obligation],
) -> Vec<Obligation> {
    obligations
        .iter()
        .filter(|obligation| !obligation_supported_before_dispatch(phase, obligation))
        .cloned()
        .collect()
}

fn obligation_supported_before_dispatch(
    phase: CapabilityObligationPhase,
    obligation: &Obligation,
) -> bool {
    match obligation {
        Obligation::AuditBefore
        | Obligation::ApplyNetworkPolicy { .. }
        | Obligation::InjectSecretOnce { .. } => true,
        Obligation::AuditAfter
        | Obligation::RedactOutput
        | Obligation::EnforceOutputLimit { .. } => {
            !matches!(phase, CapabilityObligationPhase::Spawn)
        }
        Obligation::ReserveResources { .. } | Obligation::UseScopedMounts { .. } => false,
    }
}

fn unsupported_completion_obligations(
    phase: CapabilityObligationPhase,
    obligations: &[Obligation],
) -> Vec<Obligation> {
    obligations
        .iter()
        .filter(|obligation| !obligation_supported_after_dispatch(phase, obligation))
        .cloned()
        .collect()
}

fn obligation_supported_after_dispatch(
    phase: CapabilityObligationPhase,
    obligation: &Obligation,
) -> bool {
    match obligation {
        Obligation::AuditBefore
        | Obligation::ApplyNetworkPolicy { .. }
        | Obligation::InjectSecretOnce { .. } => true,
        Obligation::AuditAfter
        | Obligation::RedactOutput
        | Obligation::EnforceOutputLimit { .. } => {
            !matches!(phase, CapabilityObligationPhase::Spawn)
        }
        Obligation::ReserveResources { .. } | Obligation::UseScopedMounts { .. } => false,
    }
}

fn secret_injection_obligations(obligations: &[Obligation]) -> Vec<SecretHandle> {
    obligations
        .iter()
        .filter_map(|obligation| match obligation {
            Obligation::InjectSecretOnce { handle } => Some(handle.clone()),
            _ => None,
        })
        .collect()
}

fn network_policy_obligation(
    obligations: &[Obligation],
) -> Result<Option<NetworkPolicy>, CapabilityObligationError> {
    let mut policy = None;
    for obligation in obligations {
        if let Obligation::ApplyNetworkPolicy { policy: next } = obligation {
            if policy.is_some() {
                return Err(network_obligation_failed());
            }
            validate_network_policy_metadata(next)?;
            policy = Some(next.clone());
        }
    }
    Ok(policy)
}

fn validate_network_policy_metadata(
    policy: &NetworkPolicy,
) -> Result<(), CapabilityObligationError> {
    if policy.allowed_targets.is_empty() {
        return Err(network_obligation_failed());
    }

    if policy.deny_private_ip_ranges {
        for target in &policy.allowed_targets {
            let host = target
                .host_pattern
                .strip_prefix("*.")
                .unwrap_or(target.host_pattern.as_str());
            if let Ok(ip) = host.parse::<IpAddr>()
                && is_private_or_loopback_ip(ip)
            {
                return Err(network_obligation_failed());
            }
        }
    }

    Ok(())
}

fn network_obligation_failed() -> CapabilityObligationError {
    CapabilityObligationError::Failed {
        kind: CapabilityObligationFailureKind::Network,
    }
}

fn secret_obligation_failed() -> CapabilityObligationError {
    CapabilityObligationError::Failed {
        kind: CapabilityObligationFailureKind::Secret,
    }
}

fn output_obligation_failed() -> CapabilityObligationError {
    CapabilityObligationError::Failed {
        kind: CapabilityObligationFailureKind::Output,
    }
}

fn dispatch_output_bytes(output: &serde_json::Value) -> Result<u64, CapabilityObligationError> {
    serde_json::to_vec(output)
        .map(|bytes| bytes.len() as u64)
        .map_err(|_| output_obligation_failed())
}

fn redact_output(
    output: serde_json::Value,
) -> Result<serde_json::Value, CapabilityObligationError> {
    match output {
        serde_json::Value::String(value) => LeakDetector::new()
            .scan_and_clean(&value)
            .map(serde_json::Value::String)
            .map_err(|_| output_obligation_failed()),
        serde_json::Value::Array(values) => values
            .into_iter()
            .map(redact_output)
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        serde_json::Value::Object(entries) => entries
            .into_iter()
            .map(|(key, value)| redact_output(value).map(|value| (key, value)))
            .collect::<Result<serde_json::Map<_, _>, _>>()
            .map(serde_json::Value::Object),
        value => Ok(value),
    }
}

fn audit_before_record(request: &CapabilityObligationRequest<'_>) -> AuditEnvelope {
    AuditEnvelope {
        event_id: AuditEventId::new(),
        correlation_id: request.context.correlation_id,
        stage: AuditStage::Before,
        timestamp: Utc::now(),
        tenant_id: request.context.tenant_id.clone(),
        user_id: request.context.user_id.clone(),
        agent_id: request.context.agent_id.clone(),
        project_id: request.context.project_id.clone(),
        mission_id: request.context.mission_id.clone(),
        thread_id: request.context.thread_id.clone(),
        invocation_id: request.context.invocation_id,
        process_id: request.context.process_id,
        approval_request_id: None,
        extension_id: Some(request.context.extension_id.clone()),
        action: ActionSummary {
            kind: capability_action_kind(request.phase).to_string(),
            target: Some(request.capability_id.as_str().to_string()),
            effects: capability_action_effects(request.phase),
        },
        decision: DecisionSummary {
            kind: "obligation_satisfied".to_string(),
            reason: None,
            actor: None,
        },
        result: Some(ActionResultSummary {
            success: true,
            status: Some(obligation_status(request.obligations)),
            output_bytes: None,
        }),
    }
}

fn audit_after_record(
    request: &CapabilityObligationCompletionRequest<'_>,
    output_bytes: u64,
) -> AuditEnvelope {
    AuditEnvelope {
        event_id: AuditEventId::new(),
        correlation_id: request.context.correlation_id,
        stage: AuditStage::After,
        timestamp: Utc::now(),
        tenant_id: request.context.tenant_id.clone(),
        user_id: request.context.user_id.clone(),
        agent_id: request.context.agent_id.clone(),
        project_id: request.context.project_id.clone(),
        mission_id: request.context.mission_id.clone(),
        thread_id: request.context.thread_id.clone(),
        invocation_id: request.context.invocation_id,
        process_id: request.context.process_id,
        approval_request_id: None,
        extension_id: Some(request.context.extension_id.clone()),
        action: ActionSummary {
            kind: capability_action_kind(request.phase).to_string(),
            target: Some(request.capability_id.as_str().to_string()),
            effects: capability_action_effects(request.phase),
        },
        decision: DecisionSummary {
            kind: "obligation_satisfied".to_string(),
            reason: None,
            actor: None,
        },
        result: Some(ActionResultSummary {
            success: true,
            status: Some(obligation_status(request.obligations)),
            output_bytes: Some(output_bytes),
        }),
    }
}

fn capability_action_kind(phase: CapabilityObligationPhase) -> &'static str {
    match phase {
        CapabilityObligationPhase::Invoke => "capability_invoke",
        CapabilityObligationPhase::Resume => "capability_resume",
        CapabilityObligationPhase::Spawn => "capability_spawn",
    }
}

fn capability_action_effects(phase: CapabilityObligationPhase) -> Vec<EffectKind> {
    match phase {
        CapabilityObligationPhase::Invoke | CapabilityObligationPhase::Resume => {
            vec![EffectKind::DispatchCapability]
        }
        CapabilityObligationPhase::Spawn => {
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess]
        }
    }
}

fn obligation_status(obligations: &[Obligation]) -> String {
    obligations
        .iter()
        .filter_map(obligation_label)
        .collect::<Vec<_>>()
        .join(",")
}

fn obligation_label(obligation: &Obligation) -> Option<&'static str> {
    match obligation {
        Obligation::AuditBefore => Some("audit_before"),
        Obligation::AuditAfter => Some("audit_after"),
        Obligation::RedactOutput => Some("redact_output"),
        Obligation::ApplyNetworkPolicy { .. } => Some("apply_network_policy"),
        Obligation::InjectSecretOnce { .. } => Some("inject_secret_once"),
        Obligation::EnforceOutputLimit { .. } => Some("enforce_output_limit"),
        _ => None,
    }
}

/// Composition root for the Reborn host/runtime vertical slice.
///
/// `HostRuntimeServices` owns shared service handles and can build the narrow
/// service facades used by callers:
///
/// - `RuntimeDispatcher` for already-authorized runtime dispatch
/// - `CapabilityHost` for caller-facing invocation/spawn workflows
/// - `ProcessHost` for lifecycle/result/output/cancellation operations
///
/// It is deliberately not an authority engine, dispatcher, process manager, or
/// lifecycle store. Those responsibilities remain in their owning crates.
pub struct HostRuntimeServices<F, G, S, R, A>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
    A: CapabilityDispatchAuthorizer + 'static,
{
    registry: Arc<ExtensionRegistry>,
    filesystem: Arc<F>,
    governor: Arc<G>,
    authorizer: Arc<A>,
    process_services: ProcessServices<S, R>,
    run_state: Option<Arc<dyn RunStateStore>>,
    approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
    capability_leases: Option<Arc<dyn CapabilityLeaseStore>>,
    wasm_runtime: Option<Arc<WasmRuntime>>,
    wasm_http_client: Option<Arc<dyn WasmHostHttp>>,
    http_egress_client: Option<Arc<dyn HttpEgressClient>>,
    runtime_http_credentials: Vec<RuntimeHttpCredential>,
    script_runtime: Option<Arc<dyn ScriptExecutor>>,
    mcp_runtime: Option<Arc<dyn McpExecutor>>,
    event_sink: Option<Arc<dyn EventSink>>,
    audit_sink: Option<Arc<dyn AuditSink>>,
    obligation_handler: Option<Arc<dyn CapabilityObligationHandler>>,
    network_obligation_policies: Arc<NetworkObligationPolicyStore>,
    secret_store: Option<Arc<dyn SecretStore>>,
    runtime_secret_injections: Arc<RuntimeSecretInjectionStore>,
}

impl<F, G, S, R, A> HostRuntimeServices<F, G, S, R, A>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
    A: CapabilityDispatchAuthorizer + 'static,
{
    pub fn new(
        registry: Arc<ExtensionRegistry>,
        filesystem: Arc<F>,
        governor: Arc<G>,
        authorizer: Arc<A>,
        process_services: ProcessServices<S, R>,
    ) -> Self {
        Self {
            registry,
            filesystem,
            governor,
            authorizer,
            process_services,
            run_state: None,
            approval_requests: None,
            capability_leases: None,
            wasm_runtime: None,
            wasm_http_client: None,
            http_egress_client: None,
            runtime_http_credentials: Vec::new(),
            script_runtime: None,
            mcp_runtime: None,
            event_sink: None,
            audit_sink: None,
            obligation_handler: None,
            network_obligation_policies: Arc::new(NetworkObligationPolicyStore::new()),
            secret_store: None,
            runtime_secret_injections: Arc::new(RuntimeSecretInjectionStore::new()),
        }
    }

    pub fn registry(&self) -> Arc<ExtensionRegistry> {
        Arc::clone(&self.registry)
    }

    pub fn filesystem(&self) -> Arc<F> {
        Arc::clone(&self.filesystem)
    }

    pub fn governor(&self) -> Arc<G> {
        Arc::clone(&self.governor)
    }

    pub fn authorizer(&self) -> Arc<A> {
        Arc::clone(&self.authorizer)
    }

    pub fn process_services(&self) -> &ProcessServices<S, R> {
        &self.process_services
    }

    pub fn runtime_secret_injections(&self) -> Arc<RuntimeSecretInjectionStore> {
        Arc::clone(&self.runtime_secret_injections)
    }

    pub fn process_host(&self) -> ProcessHost<'_> {
        self.process_services.host()
    }

    pub fn with_run_state<T>(mut self, run_state: Arc<T>) -> Self
    where
        T: RunStateStore + 'static,
    {
        self.run_state = Some(run_state);
        self
    }

    pub fn with_approval_requests<T>(mut self, approval_requests: Arc<T>) -> Self
    where
        T: ApprovalRequestStore + 'static,
    {
        self.approval_requests = Some(approval_requests);
        self
    }

    pub fn with_capability_leases<T>(mut self, capability_leases: Arc<T>) -> Self
    where
        T: CapabilityLeaseStore + 'static,
    {
        self.capability_leases = Some(capability_leases);
        self
    }

    pub fn with_wasm_runtime(mut self, runtime: Arc<WasmRuntime>) -> Self {
        self.wasm_runtime = Some(runtime);
        self
    }

    pub fn with_wasm_http_client<T>(mut self, client: Arc<T>) -> Self
    where
        T: WasmHostHttp + 'static,
    {
        let client: Arc<dyn WasmHostHttp> = client;
        self.wasm_http_client = Some(client);
        self
    }

    pub fn with_wasm_http_client_dyn(mut self, client: Arc<dyn WasmHostHttp>) -> Self {
        self.wasm_http_client = Some(client);
        self
    }

    pub fn with_http_egress_client<T>(mut self, client: Arc<T>) -> Self
    where
        T: HttpEgressClient + 'static,
    {
        let client: Arc<dyn HttpEgressClient> = client;
        self.http_egress_client = Some(client);
        self
    }

    pub fn with_http_egress_client_dyn(mut self, client: Arc<dyn HttpEgressClient>) -> Self {
        self.http_egress_client = Some(client);
        self
    }

    pub fn with_hardened_network_egress(mut self) -> Self {
        self.http_egress_client = Some(Arc::new(HardenedHttpEgressClient::new()));
        self
    }

    pub fn with_runtime_http_credentials(
        mut self,
        credentials: Vec<RuntimeHttpCredential>,
    ) -> Self {
        self.runtime_http_credentials = credentials;
        self
    }

    pub fn with_secret_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: SecretStore + 'static,
    {
        let store: Arc<dyn SecretStore> = store;
        self.secret_store = Some(store);
        self
    }

    pub fn with_secret_store_dyn(mut self, store: Arc<dyn SecretStore>) -> Self {
        self.secret_store = Some(store);
        self
    }

    pub fn with_runtime_secret_injection_store(
        mut self,
        store: Arc<RuntimeSecretInjectionStore>,
    ) -> Self {
        self.runtime_secret_injections = store;
        self
    }

    pub fn with_script_runtime<T>(mut self, runtime: Arc<T>) -> Self
    where
        T: ScriptExecutor + 'static,
    {
        let runtime: Arc<dyn ScriptExecutor> = runtime;
        self.script_runtime = Some(runtime);
        self
    }

    pub fn with_mcp_runtime<T>(mut self, runtime: Arc<T>) -> Self
    where
        T: McpExecutor + 'static,
    {
        let runtime: Arc<dyn McpExecutor> = runtime;
        self.mcp_runtime = Some(runtime);
        self
    }

    pub fn with_event_sink<T>(mut self, sink: Arc<T>) -> Self
    where
        T: EventSink + 'static,
    {
        let sink: Arc<dyn EventSink> = sink;
        self.event_sink = Some(sink);
        self
    }

    pub fn with_audit_sink<T>(mut self, sink: Arc<T>) -> Self
    where
        T: AuditSink + 'static,
    {
        let sink: Arc<dyn AuditSink> = sink;
        self.audit_sink = Some(sink);
        self
    }

    pub fn with_obligation_handler<T>(mut self, handler: Arc<T>) -> Self
    where
        T: CapabilityObligationHandler + 'static,
    {
        let handler: Arc<dyn CapabilityObligationHandler> = handler;
        self.obligation_handler = Some(handler);
        self
    }

    pub fn with_builtin_obligation_handler(mut self) -> Self {
        let mut handler = BuiltinObligationHandler::new()
            .with_network_policy_store(Arc::clone(&self.network_obligation_policies))
            .with_secret_injection_store(Arc::clone(&self.runtime_secret_injections));
        if let Some(audit_sink) = &self.audit_sink {
            handler = handler.with_audit_sink_dyn(Arc::clone(audit_sink));
        }
        if let Some(secret_store) = &self.secret_store {
            handler = handler.with_secret_store_dyn(Arc::clone(secret_store));
        }
        self.obligation_handler = Some(Arc::new(handler));
        self
    }

    pub fn approval_resolver(
        &self,
    ) -> Option<ApprovalResolver<'_, dyn ApprovalRequestStore, dyn CapabilityLeaseStore>> {
        let approval_requests = self.approval_requests.as_deref()?;
        let capability_leases = self.capability_leases.as_deref()?;
        let mut resolver = ApprovalResolver::new(approval_requests, capability_leases);
        if let Some(audit_sink) = &self.audit_sink {
            resolver = resolver.with_audit_sink(audit_sink.as_ref());
        }
        Some(resolver)
    }

    pub fn runtime_dispatcher(&self) -> RuntimeDispatcher<'static, F, G> {
        let mut dispatcher = RuntimeDispatcher::from_arcs(
            Arc::clone(&self.registry),
            Arc::clone(&self.filesystem),
            Arc::clone(&self.governor),
        );

        if let Some(runtime) = &self.wasm_runtime {
            let mut adapter = WasmRuntimeAdapter::new(Arc::clone(runtime))
                .with_network_policy_store(Arc::clone(&self.network_obligation_policies));
            if let Some(http_client) = &self.wasm_http_client {
                adapter = adapter.with_http_client_dyn(Arc::clone(http_client));
            }
            if let Some(egress_client) = &self.http_egress_client {
                adapter = adapter.with_http_egress_client_dyn(Arc::clone(egress_client));
            }
            adapter = adapter.with_runtime_http_credentials(self.runtime_http_credentials.clone());
            dispatcher = dispatcher.with_runtime_adapter_arc(RuntimeKind::Wasm, Arc::new(adapter));
        }
        if let Some(runtime) = &self.script_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::Script,
                Arc::new(
                    ScriptRuntimeAdapter::from_dyn(Arc::clone(runtime))
                        .with_network_policy_store(Arc::clone(&self.network_obligation_policies)),
                ),
            );
        }
        if let Some(runtime) = &self.mcp_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::Mcp,
                Arc::new(
                    McpRuntimeAdapter::from_dyn(Arc::clone(runtime))
                        .with_network_policy_store(Arc::clone(&self.network_obligation_policies)),
                ),
            );
        }
        if let Some(sink) = &self.event_sink {
            dispatcher = dispatcher.with_event_sink_arc(Arc::clone(sink));
        }

        dispatcher
    }

    pub fn runtime_dispatcher_arc(&self) -> Arc<RuntimeDispatcher<'static, F, G>> {
        Arc::new(self.runtime_dispatcher())
    }

    pub fn capability_host<'a, D, E>(
        &'a self,
        dispatcher: &'a D,
        executor: Arc<E>,
    ) -> CapabilityHost<'a, D>
    where
        D: CapabilityDispatcher + ?Sized,
        E: ProcessExecutor + 'static,
    {
        self.configure_capability_host(
            CapabilityHost::new(self.registry.as_ref(), dispatcher, self.authorizer.as_ref())
                .with_process_services(&self.process_services, executor),
        )
    }

    pub fn capability_host_for_runtime_dispatcher<'a>(
        &'a self,
        dispatcher: &'a Arc<RuntimeDispatcher<'static, F, G>>,
    ) -> CapabilityHost<'a, RuntimeDispatcher<'static, F, G>> {
        let executor = Arc::new(DispatchProcessExecutor::new(Arc::clone(dispatcher)));
        self.capability_host(dispatcher.as_ref(), executor)
    }

    fn configure_capability_host<'a, D>(
        &'a self,
        host: CapabilityHost<'a, D>,
    ) -> CapabilityHost<'a, D>
    where
        D: CapabilityDispatcher + ?Sized,
    {
        let mut host = host;
        if let Some(run_state) = &self.run_state {
            host = host.with_run_state(run_state.as_ref());
        }
        if let Some(approval_requests) = &self.approval_requests {
            host = host.with_approval_requests(approval_requests.as_ref());
        }
        if let Some(capability_leases) = &self.capability_leases {
            host = host.with_capability_leases(capability_leases.as_ref());
        }
        if let Some(obligation_handler) = &self.obligation_handler {
            host = host.with_obligation_handler(obligation_handler.as_ref());
        }
        host
    }
}

fn mcp_dispatch_error(error: McpError) -> DispatchError {
    DispatchError::Mcp {
        kind: mcp_error_kind(&error),
    }
}

fn script_dispatch_error(error: ScriptError) -> DispatchError {
    DispatchError::Script {
        kind: script_error_kind(&error),
    }
}

fn wasm_dispatch_error(error: WasmError) -> DispatchError {
    DispatchError::Wasm {
        kind: wasm_error_kind(&error),
    }
}

fn mcp_error_kind(error: &McpError) -> RuntimeDispatchErrorKind {
    match error {
        McpError::Resource(_) => RuntimeDispatchErrorKind::Resource,
        McpError::Client { .. } => RuntimeDispatchErrorKind::Client,
        McpError::UnsupportedTransport { .. } => RuntimeDispatchErrorKind::UnsupportedRunner,
        McpError::ExtensionRuntimeMismatch { .. } => {
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
        }
        McpError::CapabilityNotDeclared { .. } => RuntimeDispatchErrorKind::UndeclaredCapability,
        McpError::DescriptorMismatch { .. } => RuntimeDispatchErrorKind::ExtensionRuntimeMismatch,
        McpError::InvalidInvocation { .. } => RuntimeDispatchErrorKind::InputEncode,
        McpError::OutputLimitExceeded { .. } => RuntimeDispatchErrorKind::OutputTooLarge,
    }
}

fn script_error_kind(error: &ScriptError) -> RuntimeDispatchErrorKind {
    match error {
        ScriptError::Resource(_) => RuntimeDispatchErrorKind::Resource,
        ScriptError::Backend { .. } => RuntimeDispatchErrorKind::Backend,
        ScriptError::UnsupportedRunner { .. } => RuntimeDispatchErrorKind::UnsupportedRunner,
        ScriptError::ExtensionRuntimeMismatch { .. } => {
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
        }
        ScriptError::CapabilityNotDeclared { .. } => RuntimeDispatchErrorKind::UndeclaredCapability,
        ScriptError::DescriptorMismatch { .. } => {
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
        }
        ScriptError::InvalidInvocation { .. } => RuntimeDispatchErrorKind::InputEncode,
        ScriptError::ExitFailure { .. } => RuntimeDispatchErrorKind::ExitFailure,
        ScriptError::OutputLimitExceeded { .. } => RuntimeDispatchErrorKind::OutputTooLarge,
        ScriptError::InvalidOutput { .. } => RuntimeDispatchErrorKind::OutputDecode,
    }
}

fn wasm_error_kind(error: &WasmError) -> RuntimeDispatchErrorKind {
    match error {
        WasmError::Engine { .. } | WasmError::Cache { .. } => RuntimeDispatchErrorKind::Executor,
        WasmError::Extension(_) => RuntimeDispatchErrorKind::Manifest,
        WasmError::Filesystem(_) => RuntimeDispatchErrorKind::FilesystemDenied,
        WasmError::Resource(_) => RuntimeDispatchErrorKind::Resource,
        WasmError::InvalidModule { .. } => RuntimeDispatchErrorKind::Manifest,
        WasmError::UnsupportedImport { .. } => RuntimeDispatchErrorKind::Executor,
        WasmError::DescriptorMismatch { .. } => RuntimeDispatchErrorKind::ExtensionRuntimeMismatch,
        WasmError::ExtensionRuntimeMismatch { .. } => {
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
        }
        WasmError::CapabilityNotDeclared { .. } => RuntimeDispatchErrorKind::UndeclaredCapability,
        WasmError::InvalidInvocation { .. } => RuntimeDispatchErrorKind::InputEncode,
        WasmError::MissingReservation => RuntimeDispatchErrorKind::Resource,
        WasmError::MissingExport { .. } => RuntimeDispatchErrorKind::Executor,
        WasmError::MissingMemory => RuntimeDispatchErrorKind::Memory,
        WasmError::GuestAllocation { .. } => RuntimeDispatchErrorKind::Memory,
        WasmError::GuestError { .. } => RuntimeDispatchErrorKind::Guest,
        WasmError::InvalidGuestOutput { .. } => RuntimeDispatchErrorKind::OutputDecode,
        WasmError::FuelExhausted { .. } => RuntimeDispatchErrorKind::Resource,
        WasmError::MemoryExceeded { .. } => RuntimeDispatchErrorKind::Memory,
        WasmError::Timeout { .. } => RuntimeDispatchErrorKind::Resource,
        WasmError::OutputLimitExceeded { .. } => RuntimeDispatchErrorKind::OutputTooLarge,
        WasmError::Trap { .. } => RuntimeDispatchErrorKind::Guest,
    }
}
