//! Configurable, hermetic Streamable-HTTP MCP fixture for hosted-registration
//! journeys.  It deliberately records only redacted request facts so tests
//! can prove credential routing without retaining bearer material.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ironclaw_host_api::action::NetworkMethod;
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use serde_json::{Value, json};
use tokio::sync::{Notify, oneshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostedMcpAuthPolicy {
    NoAuth,
    ExactBearer { token: String },
    ExactBearerWithoutChallenge { token: String },
    OAuth { access_token: String },
    OAuthWithoutChallenge { access_token: String },
    OAuthWithoutChallengePathMetadata { access_token: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedHostedMcpRequest {
    pub method: String,
    pub path: String,
    pub authorization_present: bool,
    pub authorization_matches: bool,
    pub rpc_method: Option<String>,
}

#[derive(Clone, Debug)]
pub struct HostedMcpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub annotations: Option<Value>,
    pub result: Value,
}

impl HostedMcpTool {
    pub fn read_only(name: impl Into<String>, result: Value) -> Self {
        Self {
            name: name.into(),
            description: "hosted MCP fixture tool".to_string(),
            input_schema: json!({"type":"object"}),
            annotations: None,
            result,
        }
    }
}

pub struct HostedMcpRegistrationServer {
    base_url: String,
    state: Arc<StateData>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

/// Test-only gate that parks real MCP requests after the server has observed
/// them. It lets a caller-path test hold registration preflight network I/O
/// without introducing a production-only testing seam.
#[derive(Clone, Default)]
pub struct HostedMcpRegistrationProbeGate {
    entered: Arc<AtomicUsize>,
    entered_notify: Arc<Notify>,
    release: Arc<Notify>,
    released: Arc<std::sync::atomic::AtomicBool>,
}

impl HostedMcpRegistrationProbeGate {
    pub async fn wait_for_entries(&self, expected: usize) {
        loop {
            if self.entered.load(Ordering::SeqCst) >= expected {
                return;
            }
            let notified = self.entered_notify.notified();
            if self.entered.load(Ordering::SeqCst) >= expected {
                return;
            }
            notified.await;
        }
    }

    pub fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
        self.release.notify_waiters();
    }

    async fn park(&self) {
        self.entered.fetch_add(1, Ordering::SeqCst);
        self.entered_notify.notify_waiters();
        let notified = self.release.notified();
        if self.released.load(Ordering::SeqCst) {
            return;
        }
        notified.await;
    }
}

/// Hermetic public-origin adapter for the real product lifecycle. Admission
/// sees only the canonical HTTPS origins; this adapter alone maps those two
/// explicitly allowlisted origins to the loopback fixture.
pub struct HostedMcpRegistrationNetworkEgress {
    loopback_base: String,
    mcp_host: String,
    client: reqwest::Client,
    /// When set, any request whose path contains this substring fails at the
    /// transport layer instead of reaching the fixture server, scripting the
    /// `ports.runtime_http_egress().execute(...)` `Err` branch of
    /// `fetch_oauth_metadata`.
    fail_transport_for_path: Option<String>,
}

impl HostedMcpRegistrationNetworkEgress {
    pub fn for_server(server: &HostedMcpRegistrationServer) -> Self {
        Self::for_server_with_mcp_host(server, "mcp.example.test")
    }

    /// Map a specific production-shaped MCP host to the loopback fixture while
    /// retaining the fixture authorization-server origin.
    pub fn for_server_with_mcp_host(
        server: &HostedMcpRegistrationServer,
        mcp_host: impl Into<String>,
    ) -> Self {
        Self {
            loopback_base: server.base_url.clone(),
            mcp_host: mcp_host.into(),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("fixture client"),
            fail_transport_for_path: None,
        }
    }

    /// Same as [`Self::for_server`], but requests whose path contains
    /// `path_substr` fail with a transport error instead of reaching the
    /// server, for scripting `fetch_oauth_metadata`'s egress-`Err` branch.
    pub fn for_server_with_transport_failure_on(
        server: &HostedMcpRegistrationServer,
        path_substr: impl Into<String>,
    ) -> Self {
        Self {
            fail_transport_for_path: Some(path_substr.into()),
            ..Self::for_server(server)
        }
    }
}

#[async_trait]
impl NetworkHttpEgress for HostedMcpRegistrationNetworkEgress {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let request_bytes = request.body.len() as u64;
        let source =
            url::Url::parse(&request.url).map_err(|error| NetworkHttpError::InvalidUrl {
                reason: error.to_string(),
                request_bytes,
                response_bytes: 0,
            })?;
        let host = source.host_str().unwrap_or_default();
        if source.scheme() != "https"
            || source.port().is_some()
            || !source.username().is_empty()
            || source.password().is_some()
            || (host != self.mcp_host && host != "auth.example.test")
        {
            return Err(NetworkHttpError::PolicyDenied {
                reason: format!(
                    "hosted MCP fixture accepts only {} and auth.example.test HTTPS origins",
                    self.mcp_host
                ),
                request_bytes,
                response_bytes: 0,
            });
        }
        if let Some(path_substr) = &self.fail_transport_for_path
            && source.path().contains(path_substr.as_str())
        {
            return Err(NetworkHttpError::Transport {
                reason: "hosted MCP fixture scripted a transport failure".to_string(),
                request_bytes,
                response_bytes: 0,
            });
        }
        let mut target =
            url::Url::parse(&self.loopback_base).map_err(|error| NetworkHttpError::InvalidUrl {
                reason: error.to_string(),
                request_bytes,
                response_bytes: 0,
            })?;
        target.set_path(source.path());
        target.set_query(source.query());
        let method = match request.method {
            NetworkMethod::Get => reqwest::Method::GET,
            NetworkMethod::Post => reqwest::Method::POST,
            NetworkMethod::Put => reqwest::Method::PUT,
            NetworkMethod::Patch => reqwest::Method::PATCH,
            NetworkMethod::Delete => reqwest::Method::DELETE,
            NetworkMethod::Head => reqwest::Method::HEAD,
        };
        let mut builder = self.client.request(method, target);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body.clone());
        }
        let response = builder
            .send()
            .await
            .map_err(|error| NetworkHttpError::Transport {
                reason: error.to_string(),
                request_bytes,
                response_bytes: 0,
            })?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect::<Vec<_>>();
        let body = response
            .bytes()
            .await
            .map_err(|error| NetworkHttpError::Transport {
                reason: error.to_string(),
                request_bytes,
                response_bytes: 0,
            })?
            .to_vec();
        let response_bytes = body.len() as u64;
        Ok(NetworkHttpResponse {
            status,
            headers,
            body,
            usage: NetworkUsage {
                request_bytes,
                response_bytes,
                resolved_ip: Some(std::net::Ipv4Addr::LOCALHOST.into()),
            },
        })
    }
}

impl HostedMcpRegistrationServer {
    pub async fn start(policy: HostedMcpAuthPolicy, tools: Vec<HostedMcpTool>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind hosted MCP loopback fixture");
        let address = listener.local_addr().expect("fixture listener address");
        let base_url = format!("http://{address}");
        let state = Arc::new(StateData {
            policy,
            tools,
            requests: Mutex::new(Vec::new()),
            protected_resource_override: Mutex::new(None),
            authorization_server_override: Mutex::new(None),
            mcp_probe_gate: Mutex::new(None),
        });
        let (shutdown, receiver) = oneshot::channel();
        let app = Router::new()
            .route("/mcp", post(mcp))
            .route(
                "/.well-known/oauth-protected-resource",
                get(protected_resource),
            )
            .route(
                "/.well-known/oauth-protected-resource/mcp",
                get(path_protected_resource),
            )
            .route(
                "/.well-known/oauth-authorization-server",
                get(authorization_server),
            )
            .route("/register", post(dynamic_client_registration))
            .route("/token", post(token))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = receiver.await;
                })
                .await
                .expect("hosted MCP fixture serves");
        });
        Self {
            base_url,
            state,
            shutdown: Some(shutdown),
            task: Some(task),
        }
    }

    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url)
    }
    pub fn requests(&self) -> Vec<RecordedHostedMcpRequest> {
        self.state.requests.lock().expect("request lock").clone()
    }

    /// Parks every currently configured MCP preflight request until the
    /// returned gate is released.
    pub fn block_mcp_preflight_requests(&self) -> HostedMcpRegistrationProbeGate {
        let gate = HostedMcpRegistrationProbeGate::default();
        *self
            .state
            .mcp_probe_gate
            .lock()
            .expect("MCP preflight gate lock") = Some(gate.clone());
        gate
    }

    /// Scripts the next `/.well-known/oauth-protected-resource` response,
    /// replacing the default valid JSON document.
    pub fn script_protected_resource_response(&self, response: ScriptedMetadataResponse) {
        *self
            .state
            .protected_resource_override
            .lock()
            .expect("protected-resource override lock") = Some(response);
    }

    /// Scripts the next `/.well-known/oauth-authorization-server` response,
    /// replacing the default valid JSON document.
    pub fn script_authorization_server_response(&self, response: ScriptedMetadataResponse) {
        *self
            .state
            .authorization_server_override
            .lock()
            .expect("authorization-server override lock") = Some(response);
    }
}

impl Drop for HostedMcpRegistrationServer {
    fn drop(&mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Scripts a raw status/body pair for the protected-resource or
/// authorization-server metadata endpoints, in place of their default valid
/// JSON document, for driving `fetch_oauth_metadata`'s non-200 / oversized /
/// malformed-JSON failure branches.
#[derive(Clone, Debug)]
pub struct ScriptedMetadataResponse {
    pub status: StatusCode,
    pub body: Vec<u8>,
}

impl ScriptedMetadataResponse {
    pub fn new(status: StatusCode, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
        }
    }
}

struct StateData {
    policy: HostedMcpAuthPolicy,
    tools: Vec<HostedMcpTool>,
    requests: Mutex<Vec<RecordedHostedMcpRequest>>,
    protected_resource_override: Mutex<Option<ScriptedMetadataResponse>>,
    authorization_server_override: Mutex<Option<ScriptedMetadataResponse>>,
    mcp_probe_gate: Mutex<Option<HostedMcpRegistrationProbeGate>>,
}

async fn mcp(
    State(state): State<Arc<StateData>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    let expected = match &state.policy {
        HostedMcpAuthPolicy::NoAuth => None,
        HostedMcpAuthPolicy::ExactBearer { token }
        | HostedMcpAuthPolicy::ExactBearerWithoutChallenge { token }
        | HostedMcpAuthPolicy::OAuth {
            access_token: token,
        }
        | HostedMcpAuthPolicy::OAuthWithoutChallenge {
            access_token: token,
        }
        | HostedMcpAuthPolicy::OAuthWithoutChallengePathMetadata {
            access_token: token,
        } => Some(format!("Bearer {token}")),
    };
    let matches = expected
        .as_deref()
        .is_none_or(|value| authorization == Some(value));
    state
        .requests
        .lock()
        .expect("request lock")
        .push(RecordedHostedMcpRequest {
            method: "POST".to_string(),
            path: "/mcp".to_string(),
            authorization_present: authorization.is_some(),
            authorization_matches: matches,
            rpc_method: request
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    let gate = state
        .mcp_probe_gate
        .lock()
        .expect("MCP preflight gate lock")
        .clone();
    if let Some(gate) = gate {
        gate.park().await;
    }
    if !matches {
        return match state.policy {
            HostedMcpAuthPolicy::OAuth { .. } => oauth_challenge(StatusCode::UNAUTHORIZED),
            HostedMcpAuthPolicy::OAuthWithoutChallenge { .. }
            | HostedMcpAuthPolicy::OAuthWithoutChallengePathMetadata { .. } => {
                StatusCode::UNAUTHORIZED.into_response()
            }
            HostedMcpAuthPolicy::ExactBearer { .. } => bearer_challenge(StatusCode::UNAUTHORIZED),
            HostedMcpAuthPolicy::ExactBearerWithoutChallenge { .. } => {
                StatusCode::UNAUTHORIZED.into_response()
            }
            HostedMcpAuthPolicy::NoAuth => StatusCode::UNAUTHORIZED.into_response(),
        };
    }
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let result = match request.get("method").and_then(Value::as_str) {
        Some("initialize") => {
            json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"hosted-mcp-fixture","version":"1"}})
        }
        // Streamable HTTP clients send this JSON-RPC notification after the
        // initialization response and before tools/list. It has no result
        // semantics, but accepting it keeps the fixture protocol-complete.
        Some("notifications/initialized") => json!({}),
        Some("tools/list") => {
            json!({"tools": state.tools.iter().map(tool_wire).collect::<Vec<_>>()})
        }
        Some("tools/call") => {
            let name = request
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match state.tools.iter().find(|tool| tool.name == name) {
                Some(tool) => json!({"content":[{"type":"text","text":tool.result.to_string()}]}),
                None => {
                    return streamable_json_rpc(
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"unknown tool"}}),
                    );
                }
            }
        }
        _ => {
            return streamable_json_rpc(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"unknown method"}}),
            );
        }
    };
    streamable_json_rpc(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

/// The real MRC trace recorded on 2026-07-29 uses this Streamable-HTTP SSE
/// envelope. Keeping it at the fixture boundary exercises the production MCP
/// client's framing parser rather than a JSON-only happy path.
fn streamable_json_rpc(payload: Value) -> Response {
    (
        [("content-type", "text/event-stream")],
        format!("event: message\ndata: {payload}\n\n"),
    )
        .into_response()
}

fn tool_wire(tool: &HostedMcpTool) -> Value {
    let mut value =
        json!({"name":tool.name,"description":tool.description,"inputSchema":tool.input_schema});
    if let Some(annotations) = &tool.annotations {
        value["annotations"] = annotations.clone();
    }
    value
}

fn oauth_challenge(status: StatusCode) -> Response {
    // OAuth recipe admission intentionally rejects relative metadata locations:
    // the challenge must name the exact HTTPS document subsequently fetched
    // through the policy-mediated metadata lane.
    let mut response = status.into_response();
    response.headers_mut().insert(
        "www-authenticate",
        HeaderValue::from_static(
            "Bearer resource_metadata=\"https://mcp.example.test/.well-known/oauth-protected-resource\"",
        ),
    );
    response
}

fn bearer_challenge(status: StatusCode) -> Response {
    let mut response = status.into_response();
    response.headers_mut().insert(
        "www-authenticate",
        HeaderValue::from_static("Bearer realm=\"Hosted MCP fixture\""),
    );
    response
}

fn record_metadata_request(state: &StateData, path: &str) {
    state
        .requests
        .lock()
        .expect("request lock")
        .push(RecordedHostedMcpRequest {
            method: "GET".to_string(),
            path: path.to_string(),
            authorization_present: false,
            authorization_matches: false,
            rpc_method: None,
        });
}

async fn protected_resource(State(state): State<Arc<StateData>>) -> Response {
    record_metadata_request(&state, "/.well-known/oauth-protected-resource");
    if let Some(scripted) = state
        .protected_resource_override
        .lock()
        .expect("protected-resource override lock")
        .take()
    {
        return (scripted.status, scripted.body).into_response();
    }
    if !matches!(
        state.policy,
        HostedMcpAuthPolicy::OAuth { .. }
            | HostedMcpAuthPolicy::OAuthWithoutChallenge { .. }
            | HostedMcpAuthPolicy::OAuthWithoutChallengePathMetadata { .. }
    ) {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(
        // Representative protected-resource response: admission consumes only
        // its security-critical URLs and tolerates unrelated standard fields.
        json!({"resource":"https://mcp.example.test/mcp","authorization_servers":["https://auth.example.test"],"scopes_supported":["default"],"bearer_methods_supported":["header"],"resource_name":"Hosted MCP fixture"}),
    )
    .into_response()
}

async fn path_protected_resource(State(state): State<Arc<StateData>>) -> Response {
    record_metadata_request(&state, "/.well-known/oauth-protected-resource/mcp");
    if matches!(
        state.policy,
        HostedMcpAuthPolicy::OAuthWithoutChallengePathMetadata { .. }
    ) {
        return Json(
            json!({"resource":"https://mcp.example.test/mcp","authorization_servers":["https://auth.example.test"]}),
        )
        .into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}
async fn authorization_server(State(state): State<Arc<StateData>>) -> Response {
    record_metadata_request(&state, "/.well-known/oauth-authorization-server");
    if let Some(scripted) = state
        .authorization_server_override
        .lock()
        .expect("authorization-server override lock")
        .take()
    {
        return (scripted.status, scripted.body).into_response();
    }
    Json(
        json!({"issuer":"https://auth.example.test","authorization_endpoint":"https://auth.example.test/authorize","token_endpoint":"https://auth.example.test/token","registration_endpoint":"https://auth.example.test/register","scopes_supported":["default"],"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token"],"token_endpoint_auth_methods_supported":["none"],"code_challenge_methods_supported":["S256"]}),
    )
    .into_response()
}
async fn dynamic_client_registration() -> Json<Value> {
    Json(
        json!({"client_id":"fixture-client","client_secret":"fixture-secret","token_endpoint_auth_method":"none"}),
    )
}
async fn token() -> Json<Value> {
    Json(json!({"access_token":"oauth-token","token_type":"Bearer","expires_in":3600}))
}
