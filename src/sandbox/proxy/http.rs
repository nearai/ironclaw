//! HTTP proxy server for sandboxed network access.
//!
//! This proxy runs on the host and handles all network requests from containers.
//! It validates requests against the allowlist and injects credentials when needed.
//!
//! ```text
//! Container ──► http_proxy=host.docker.internal:PORT ──► This Proxy ──► Internet
//!                                                             │
//!                                                             ├─► Validate domain
//!                                                             ├─► Inject credentials
//!                                                             └─► Log requests
//! ```

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Full, combinators::BoxBody};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::RwLock;

use crate::sandbox::config::CredentialLocation;
use crate::sandbox::error::{Result, SandboxError};
use crate::sandbox::proxy::policy::{NetworkDecision, NetworkPolicyDecider, NetworkRequest};

/// State shared across proxy connections.
struct ProxyState {
    /// Policy decider for network requests.
    decider: Arc<dyn NetworkPolicyDecider>,
    /// Credential resolver (maps secret names to values).
    credential_resolver: Arc<dyn CredentialResolver>,
    /// Request counter for logging.
    request_count: std::sync::atomic::AtomicU64,
    /// Whether the proxy is running.
    running: std::sync::atomic::AtomicBool,
}

/// Resolves secret names to their values.
#[async_trait::async_trait]
pub trait CredentialResolver: Send + Sync {
    /// Get the value of a secret by name.
    async fn resolve(&self, name: &str) -> Option<String>;
}

/// A credential resolver that uses environment variables.
pub struct EnvCredentialResolver;

#[async_trait::async_trait]
impl CredentialResolver for EnvCredentialResolver {
    async fn resolve(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// A credential resolver that returns nothing (for testing).
pub struct NoCredentialResolver;

#[async_trait::async_trait]
impl CredentialResolver for NoCredentialResolver {
    async fn resolve(&self, _name: &str) -> Option<String> {
        None
    }
}

/// HTTP proxy server.
pub struct HttpProxy {
    state: Arc<ProxyState>,
    addr: RwLock<Option<SocketAddr>>,
    shutdown_tx: RwLock<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl HttpProxy {
    /// Create a new HTTP proxy.
    pub fn new(
        decider: Arc<dyn NetworkPolicyDecider>,
        credential_resolver: Arc<dyn CredentialResolver>,
    ) -> Self {
        Self {
            state: Arc::new(ProxyState {
                decider,
                credential_resolver,
                request_count: std::sync::atomic::AtomicU64::new(0),
                running: std::sync::atomic::AtomicBool::new(false),
            }),
            addr: RwLock::new(None),
            shutdown_tx: RwLock::new(None),
        }
    }

    /// Start the proxy server on the given port (0 for auto-assign).
    pub async fn start(&self, port: u16) -> Result<SocketAddr> {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .map_err(|e| SandboxError::ProxyError {
                reason: format!("failed to bind: {}", e),
            })?;

        let addr = listener
            .local_addr()
            .map_err(|e| SandboxError::ProxyError {
                reason: format!("failed to get local addr: {}", e),
            })?;

        *self.addr.write().await = Some(addr);

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        *self.shutdown_tx.write().await = Some(shutdown_tx);

        self.state
            .running
            .store(true, std::sync::atomic::Ordering::SeqCst);

        let state = self.state.clone();

        tokio::spawn(async move {
            tracing::info!("Sandbox proxy started on {}", addr);

            loop {
                tokio::select! {
                    accept_result = listener.accept() => {
                        match accept_result {
                            Ok((stream, _)) => {
                                let io = TokioIo::new(stream);
                                let state = state.clone();

                                tokio::spawn(async move {
                                    let service = service_fn(move |req| {
                                        let state = state.clone();
                                        async move { handle_request(req, state).await }
                                    });

                                    if let Err(e) = http1::Builder::new()
                                        .preserve_header_case(true)
                                        .title_case_headers(true)
                                        .serve_connection(io, service)
                                        .with_upgrades()
                                        .await
                                    {
                                        tracing::debug!("Proxy connection error: {}", e);
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::error!("Proxy accept error: {}", e);
                            }
                        }
                    }
                    _ = &mut shutdown_rx => {
                        tracing::info!("Sandbox proxy shutting down");
                        break;
                    }
                }
            }

            state
                .running
                .store(false, std::sync::atomic::Ordering::SeqCst);
        });

        Ok(addr)
    }

    /// Stop the proxy server.
    pub async fn stop(&self) {
        if let Some(tx) = self.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }
    }

    /// Get the address the proxy is listening on.
    pub async fn addr(&self) -> Option<SocketAddr> {
        *self.addr.read().await
    }

    /// Check if the proxy is running.
    pub fn is_running(&self) -> bool {
        self.state.running.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Get the number of requests handled.
    pub fn request_count(&self) -> u64 {
        self.state
            .request_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Handle an incoming proxy request.
async fn handle_request(
    req: Request<hyper::body::Incoming>,
    state: Arc<ProxyState>,
) -> std::result::Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    state
        .request_count
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    // Handle CONNECT method for HTTPS tunneling
    if req.method() == Method::CONNECT {
        return Ok(handle_connect(req, state).await);
    }

    // For HTTP requests, validate and forward
    let uri = req.uri().to_string();
    let method = req.method().to_string();

    let network_req = match NetworkRequest::from_url(&method, &uri) {
        Some(r) => r,
        None => {
            tracing::warn!("Proxy: invalid URL: {}", uri);
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                "Invalid URL".to_string(),
            ));
        }
    };

    // Make policy decision
    let decision = state.decider.decide(&network_req).await;

    match decision {
        NetworkDecision::Deny { reason } => {
            tracing::info!("Proxy: blocked {} {} - {}", method, uri, reason);
            Ok(error_response(StatusCode::FORBIDDEN, reason))
        }
        NetworkDecision::Allow | NetworkDecision::AllowWithCredentials { .. } => {
            // Forward the request
            forward_request(req, decision, state).await
        }
    }
}

/// Handle CONNECT method for HTTPS tunneling.
async fn handle_connect(
    req: Request<hyper::body::Incoming>,
    state: Arc<ProxyState>,
) -> Response<BoxBody<Bytes, Infallible>> {
    // Extract host from CONNECT target
    let host = req.uri().authority().map(|a| a.host().to_string());

    let host = match host {
        Some(h) => h,
        None => {
            return error_response(StatusCode::BAD_REQUEST, "Missing host".to_string());
        }
    };

    // Check if host is allowed
    let network_req = NetworkRequest {
        method: "CONNECT".to_string(),
        url: format!("https://{}", host),
        host: host.clone(),
        path: "/".to_string(),
    };

    let decision = state.decider.decide(&network_req).await;

    if !decision.is_allowed()
        && let NetworkDecision::Deny { reason } = decision
    {
        tracing::info!("Proxy: blocked CONNECT {} - {}", host, reason);
        return error_response(StatusCode::FORBIDDEN, reason);
    }

    tracing::debug!("Proxy: allowing CONNECT to {}", host);

    // For CONNECT, we return 200 OK and the client will upgrade to TLS
    // The actual TLS connection goes directly to the target, we just act as a tunnel
    Response::builder()
        .status(StatusCode::OK)
        .body(empty_body())
        .unwrap()
}

/// Forward a request to the target server.
async fn forward_request(
    req: Request<hyper::body::Incoming>,
    decision: NetworkDecision,
    state: Arc<ProxyState>,
) -> std::result::Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    let method = req.method().clone();
    let uri = req.uri().clone();

    // Build the forwarded request
    let client = reqwest::Client::new();
    let mut builder = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
        uri.to_string(),
    );

    // Copy headers (except hop-by-hop headers)
    for (name, value) in req.headers() {
        if !is_hop_by_hop_header(name.as_str())
            && let Ok(v) = value.to_str()
        {
            builder = builder.header(name.as_str(), v);
        }
    }

    // Inject credentials if needed
    if let NetworkDecision::AllowWithCredentials {
        secret_name,
        location,
    } = decision
    {
        if let Some(credential) = state.credential_resolver.resolve(&secret_name).await {
            builder = match location {
                CredentialLocation::AuthorizationBearer => {
                    builder.header("Authorization", format!("Bearer {}", credential))
                }
                CredentialLocation::Header(header_name) => builder.header(header_name, credential),
                CredentialLocation::QueryParam(param_name) => {
                    builder.query(&[(param_name, credential)])
                }
            };
            tracing::debug!("Proxy: injected credential for {}", secret_name);
        } else {
            tracing::warn!("Proxy: credential {} not found", secret_name);
        }
    }

    // Copy body
    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            tracing::error!("Proxy: failed to read request body: {}", e);
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to read body".to_string(),
            ));
        }
    };

    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes.to_vec());
    }

    // Send the request
    match builder.send().await {
        Ok(response) => {
            let status = response.status();
            let headers = response.headers().clone();

            match response.bytes().await {
                Ok(body) => {
                    let mut builder = Response::builder().status(status.as_u16());

                    for (name, value) in headers.iter() {
                        if !is_hop_by_hop_header(name.as_str()) {
                            builder = builder.header(name.as_str(), value.as_bytes());
                        }
                    }

                    Ok(builder.body(full_body(body)).unwrap())
                }
                Err(e) => {
                    tracing::error!("Proxy: failed to read response body: {}", e);
                    Ok(error_response(
                        StatusCode::BAD_GATEWAY,
                        "Failed to read response".to_string(),
                    ))
                }
            }
        }
        Err(e) => {
            tracing::error!("Proxy: request failed: {}", e);
            Ok(error_response(
                StatusCode::BAD_GATEWAY,
                format!("Request failed: {}", e),
            ))
        }
    }
}

/// Check if a header is hop-by-hop (should not be forwarded).
fn is_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// Create an error response.
fn error_response(status: StatusCode, message: String) -> Response<BoxBody<Bytes, Infallible>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .body(full_body(Bytes::from(message)))
        .unwrap()
}

/// Create an empty body.
fn empty_body() -> BoxBody<Bytes, Infallible> {
    Empty::<Bytes>::new().map_err(|_| unreachable!()).boxed()
}

/// Create a body from bytes.
fn full_body(bytes: Bytes) -> BoxBody<Bytes, Infallible> {
    Full::new(bytes).map_err(|_| unreachable!()).boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::proxy::allowlist::DomainAllowlist;
    use crate::sandbox::proxy::policy::DefaultPolicyDecider;

    #[tokio::test]
    async fn test_proxy_starts_and_stops() {
        let allowlist = DomainAllowlist::new(&["example.com".to_string()]);
        let decider = Arc::new(DefaultPolicyDecider::new(allowlist, vec![]));
        let resolver = Arc::new(NoCredentialResolver);

        let proxy = HttpProxy::new(decider, resolver);

        let addr = proxy.start(0).await.unwrap();
        assert!(proxy.is_running());
        assert!(addr.port() > 0);

        proxy.stop().await;
        // Give it a moment to shut down
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    #[test]
    fn test_hop_by_hop_headers() {
        assert!(is_hop_by_hop_header("connection"));
        assert!(is_hop_by_hop_header("Connection"));
        assert!(is_hop_by_hop_header("transfer-encoding"));
        assert!(!is_hop_by_hop_header("content-type"));
        assert!(!is_hop_by_hop_header("authorization"));
    }
}
