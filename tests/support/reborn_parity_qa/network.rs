use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use axum::{Router, http::Uri};
use ironclaw_host_api::{NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern};
use ironclaw_network::{
    NetworkHttpError, NetworkHttpResponse, NetworkHttpTransport, NetworkTransportRequest,
    PolicyNetworkHttpEgress,
};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct RecordingNetworkHttpTransport {
    inner: Arc<Mutex<RecordingNetworkState>>,
}

#[derive(Debug, Default)]
struct RecordingNetworkState {
    recorded: Vec<SanitizedNetworkTransportRequest>,
    scripted: VecDeque<Result<NetworkHttpResponse, NetworkHttpError>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedNetworkTransportRequest {
    pub method: NetworkMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body_len: usize,
    pub body_sha256: String,
}

impl RecordingNetworkHttpTransport {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecordingNetworkState::default())),
        }
    }

    pub fn push_response(&self, response: NetworkHttpResponse) {
        self.push_result(Ok(response));
    }

    pub fn push_error(&self, error: NetworkHttpError) {
        self.push_result(Err(error));
    }

    pub fn push_result(&self, result: Result<NetworkHttpResponse, NetworkHttpError>) {
        self.inner
            .lock()
            .expect("network transport lock poisoned")
            .scripted
            .push_back(result);
    }

    pub fn requests(&self) -> Vec<SanitizedNetworkTransportRequest> {
        self.inner
            .lock()
            .expect("network transport lock poisoned")
            .recorded
            .clone()
    }

    pub fn policy_egress(&self) -> PolicyNetworkHttpEgress<Self> {
        PolicyNetworkHttpEgress::new(self.clone())
    }
}

impl Default for RecordingNetworkHttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl NetworkHttpTransport for RecordingNetworkHttpTransport {
    async fn execute(
        &self,
        request: NetworkTransportRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let mut state = self.inner.lock().map_err(|_| NetworkHttpError::Transport {
            reason: "network transport lock poisoned".to_string(),
            request_bytes: request.body.len() as u64,
            response_bytes: 0,
        })?;
        state.recorded.push(sanitize_request(&request));
        state.scripted.pop_front().unwrap_or_else(|| {
            Err(NetworkHttpError::Transport {
                reason: "unexpected HTTP request".to_string(),
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
            })
        })
    }
}

fn sanitize_request(request: &NetworkTransportRequest) -> SanitizedNetworkTransportRequest {
    SanitizedNetworkTransportRequest {
        method: request.method,
        url: sanitize_url(&request.url),
        headers: request
            .headers
            .iter()
            .map(|(name, value)| {
                if is_sensitive_header(name) {
                    (name.clone(), "<redacted>".to_string())
                } else {
                    (name.clone(), value.clone())
                }
            })
            .collect(),
        body_len: request.body.len(),
        body_sha256: hex::encode(Sha256::digest(&request.body)),
    }
}

fn sanitize_url(url: &str) -> String {
    url.split_once('?')
        .map_or_else(|| url.to_string(), |(base, _)| format!("{base}?<redacted>"))
}

fn is_sensitive_header(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "authorization"
            | "cookie"
            | "set-cookie"
            | "proxy-authorization"
            | "api-key"
            | "apikey"
            | "x-api-key"
            | "x-auth-token"
            | "x-access-token"
            | "x-csrf-token"
            | "csrf-token"
            | "secret"
            | "x-secret"
            | "client-secret"
            | "x-client-secret"
    )
}

#[allow(dead_code)] // Shared test support; not every root test target uses live loopback HTTP.
#[derive(Clone)]
pub struct LiveLoopbackHttpState {
    requests: Arc<Mutex<Vec<String>>>,
}

#[allow(dead_code)]
impl LiveLoopbackHttpState {
    pub fn record(&self, uri: &Uri) {
        self.requests
            .lock()
            .expect("live loopback HTTP request log lock poisoned")
            .push(
                uri.path_and_query()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| uri.path().to_string()),
            );
    }
}

#[allow(dead_code)] // Shared test support; instantiated by QA web/doc tests.
pub struct LiveLoopbackHttpServer {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

#[allow(dead_code)]
impl LiveLoopbackHttpServer {
    pub async fn start(routes: Router<LiveLoopbackHttpState>) -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind live loopback HTTP test server");
        let port = listener.local_addr().expect("local addr").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let app = routes.with_state(LiveLoopbackHttpState {
            requests: Arc::clone(&requests),
        });
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            port,
            requests,
            task,
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn url(&self, path_and_query: &str) -> String {
        format!("http://127.0.0.1:{}{path_and_query}", self.port)
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("live loopback HTTP request log lock poisoned")
            .clone()
    }
}

impl Drop for LiveLoopbackHttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[allow(dead_code)] // Shared test support; consumed outside support_unit_tests.
pub fn loopback_http_policy(port: u16) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Http),
            host_pattern: "127.0.0.1".to_string(),
            port: Some(port),
        }],
        deny_private_ip_ranges: false,
        max_egress_bytes: Some(10_000),
    }
}
