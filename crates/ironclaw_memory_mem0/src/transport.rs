//! The mem0 HTTP transport seam.
//!
//! The provider *logic* never owns an HTTP client directly. Instead it speaks to
//! mem0 through the small [`Mem0Transport`] trait. The production implementation
//! [`Mem0HttpTransport`] is a real `reqwest` client (built the same way the
//! embedding providers build theirs); tests substitute the in-memory
//! [`MockMem0Transport`]. This keeps the [`crate::Mem0MemoryService`] mapping
//! unit-testable without a live mem0 endpoint or network access, and keeps the
//! crate inside its narrow internal-dependency boundary.

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

use crate::error::Mem0Error;
use crate::url_check::check_base_url;

/// Upper bound on any single mem0 request. Context retrieval sits on the turn's
/// critical path, so a hung or unreachable mem0 must fail fast rather than stall
/// the turn; the provider degrades a retrieval timeout to "no memory context".
const REQUEST_TIMEOUT_SECS: u64 = 30;

/// HTTP method for a mem0 request. mem0's memory API uses only `POST` (add,
/// search) and `GET` (list).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mem0HttpMethod {
    Get,
    Post,
}

/// A mem0 REST request, expressed transport-neutrally. The transport supplies
/// the base URL, authentication header, and TLS; this struct carries only the
/// request shape the provider builds.
#[derive(Debug, Clone, PartialEq)]
pub struct Mem0HttpRequest {
    pub method: Mem0HttpMethod,
    /// Path under the mem0 base URL, e.g. `"/memories"`.
    pub path: String,
    /// Query parameters (e.g. `("user_id", "...")`), applied for `GET` listing.
    pub query: Vec<(String, String)>,
    /// JSON request body for `POST` requests, if any.
    pub body: Option<Value>,
}

impl Mem0HttpRequest {
    pub(crate) fn post(path: &str, body: Value) -> Self {
        Self {
            method: Mem0HttpMethod::Post,
            path: path.to_string(),
            query: Vec::new(),
            body: Some(body),
        }
    }

    pub(crate) fn get(path: &str, query: Vec<(String, String)>) -> Self {
        Self {
            method: Mem0HttpMethod::Get,
            path: path.to_string(),
            query,
            body: None,
        }
    }
}

/// A mem0 REST response: the HTTP status and the parsed JSON body.
#[derive(Debug, Clone, PartialEq)]
pub struct Mem0HttpResponse {
    pub status: u16,
    pub body: Value,
}

impl Mem0HttpResponse {
    /// Whether the status is in the 2xx success range.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// A transport-level failure (connection, TLS, timeout, or reading the response
/// body bytes). Opaque on purpose: the provider maps it to a sanitized
/// `MemoryServiceError` and the host logs the cause.
///
/// A non-JSON or empty 2xx body is deliberately *not* a transport failure:
/// `execute` degrades it to `Value::Null` (the HTTP status is the authoritative
/// success signal and the service tolerates a null/unrecognized body), so an
/// un-parseable body never surfaces here.
#[derive(Debug, Error)]
#[error("mem0 transport failure: {message}")]
pub struct Mem0TransportError {
    message: String,
    #[source]
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl Mem0TransportError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

/// The swappable HTTP seam for mem0. The production implementation
/// ([`Mem0HttpTransport`]) wraps a real `reqwest` client and injects the mem0
/// API key; a test implementation answers in memory.
#[async_trait]
pub trait Mem0Transport: Send + Sync {
    async fn execute(
        &self,
        request: Mem0HttpRequest,
    ) -> Result<Mem0HttpResponse, Mem0TransportError>;
}

/// The production mem0 transport: a real `reqwest` client.
///
/// Built the same way the embedding providers build their outbound clients — a
/// direct `reqwest::Client` guarded at construction by the baseline
/// `check_base_url` SSRF check, with a bounded request timeout and redirects
/// disabled. The API key is **optional**: a self-hosted mem0 OSS server with
/// `AUTH_DISABLED=true` needs none; when one is supplied it is baked into a
/// sensitive `Authorization: Token <key>` default header, consumed at
/// construction and never stored on the struct.
pub struct Mem0HttpTransport {
    client: reqwest::Client,
    /// Base URL with any trailing slash trimmed so `base_url + path` (where
    /// `path` begins with `/`) never produces a double slash.
    base_url: String,
}

impl std::fmt::Debug for Mem0HttpTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never render the client (it holds the auth header).
        formatter
            .debug_struct("Mem0HttpTransport")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl Mem0HttpTransport {
    /// Build the real transport for `base_url`, optionally authenticating with
    /// `api_key`.
    ///
    /// `api_key` is `None` for a self-hosted mem0 OSS server running with
    /// `AUTH_DISABLED=true` (the default local deployment); `Some` for the hosted
    /// cloud or a self-hosted server with auth enabled.
    ///
    /// Fails closed with [`Mem0Error::InvalidUrl`] when `base_url` does not pass
    /// the baseline SSRF check, or [`Mem0Error::Client`] when a supplied API key is
    /// not a valid HTTP header value or the TLS backend cannot be built.
    pub fn new(base_url: &str, api_key: Option<&str>) -> Result<Self, Mem0Error> {
        check_base_url(base_url)?;

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(api_key) = api_key {
            let mut authorization = reqwest::header::HeaderValue::from_str(&format!(
                "Token {api_key}"
            ))
            .map_err(|error| Mem0Error::Client {
                reason: format!("API key is not a valid Authorization header value: {error}"),
            })?;
            // Redact the API key from any reqwest header logging.
            authorization.set_sensitive(true);
            headers.insert(reqwest::header::AUTHORIZATION, authorization);
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            // Bound every request so an unreachable/hung mem0 fails fast.
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            // Never follow redirects: a compromised or misconfigured mem0 must not
            // be able to bounce a request to an internal endpoint (SSRF defense in
            // depth, on top of the construction-time base-URL check).
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| Mem0Error::Client {
                reason: error.to_string(),
            })?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }
}

#[async_trait]
impl Mem0Transport for Mem0HttpTransport {
    async fn execute(
        &self,
        request: Mem0HttpRequest,
    ) -> Result<Mem0HttpResponse, Mem0TransportError> {
        let url = format!("{}{}", self.base_url, request.path);
        let mut builder = match request.method {
            Mem0HttpMethod::Get => self.client.get(&url),
            Mem0HttpMethod::Post => self.client.post(&url),
        };
        if !request.query.is_empty() {
            builder = builder.query(&request.query);
        }
        if let Some(body) = &request.body {
            builder = builder.json(body);
        }

        let response = builder
            .send()
            .await
            .map_err(|error| Mem0TransportError::with_source("mem0 request failed", error))?;
        let status = response.status().as_u16();
        // Read the body as text first so a non-JSON body (an empty 204, an HTML
        // error page) degrades to `Value::Null` rather than a transport error —
        // the HTTP status is the authoritative success/failure signal, and the
        // service's body parsing already tolerates a null/unrecognized shape.
        let text = response.text().await.map_err(|error| {
            Mem0TransportError::with_source("reading mem0 response body failed", error)
        })?;
        let body = if text.trim().is_empty() {
            Value::Null
        } else {
            // silent-ok: the HTTP status is the authoritative success/failure
            // signal and the service tolerates a null/unrecognized body, so a
            // non-JSON body degrades to `Value::Null` rather than erroring.
            serde_json::from_str(&text).unwrap_or_default()
        };
        tracing::debug!(target: "ironclaw_memory_mem0", status, path = %request.path, "mem0 response");
        Ok(Mem0HttpResponse { status, body })
    }
}

#[cfg(any(test, feature = "test-support"))]
mod mock {
    use std::sync::Mutex;

    use super::*;

    /// Decides the canned response for a recorded request. Returning `None`
    /// yields a default `404` so an unexpected call surfaces as an API error
    /// rather than a panic.
    pub type Mem0MockHandler =
        Box<dyn Fn(&Mem0HttpRequest) -> Option<Mem0HttpResponse> + Send + Sync>;

    /// An in-memory [`Mem0Transport`] that records every request and answers
    /// from a handler closure. Panic-free so it is safe to expose outside
    /// `cfg(test)` behind the `test-support` feature.
    pub struct MockMem0Transport {
        recorded: Mutex<Vec<Mem0HttpRequest>>,
        handler: Mem0MockHandler,
    }

    impl MockMem0Transport {
        /// Build a mock whose `handler` maps each request to a response.
        pub fn new(handler: Mem0MockHandler) -> Self {
            Self {
                recorded: Mutex::new(Vec::new()),
                handler,
            }
        }

        /// Convenience constructor: always answer `200` with the same body,
        /// regardless of the request. Useful for write-only assertions.
        pub fn always_ok(body: Value) -> Self {
            Self::new(Box::new(move |_request| {
                Some(Mem0HttpResponse {
                    status: 200,
                    body: body.clone(),
                })
            }))
        }

        /// A snapshot of every request the provider has issued so far.
        pub fn recorded(&self) -> Vec<Mem0HttpRequest> {
            match self.recorded.lock() {
                Ok(guard) => guard.clone(),
                // Poison can only occur if a holder panicked; recover the data
                // rather than propagating a panic from a test helper.
                Err(poisoned) => poisoned.into_inner().clone(),
            }
        }

        /// Number of requests issued whose path equals `path`.
        pub fn count_path(&self, path: &str) -> usize {
            self.recorded()
                .iter()
                .filter(|request| request.path == path)
                .count()
        }
    }

    #[async_trait]
    impl Mem0Transport for MockMem0Transport {
        async fn execute(
            &self,
            request: Mem0HttpRequest,
        ) -> Result<Mem0HttpResponse, Mem0TransportError> {
            let response = (self.handler)(&request).unwrap_or(Mem0HttpResponse {
                status: 404,
                body: Value::Null,
            });
            if let Ok(mut guard) = self.recorded.lock() {
                guard.push(request);
            }
            Ok(response)
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use mock::{Mem0MockHandler, MockMem0Transport};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_transport_builds_for_a_local_endpoint_without_a_key() {
        // The default self-hosted mem0 OSS deployment: localhost, no API key.
        let transport = Mem0HttpTransport::new("http://localhost:8888", None)
            .expect("a local http endpoint with no key builds");
        assert_eq!(transport.base_url, "http://localhost:8888");
    }

    #[test]
    fn real_transport_builds_with_an_api_key() {
        let transport = Mem0HttpTransport::new("https://mem0.example.com", Some("m0-secret"))
            .expect("a normal https endpoint + key builds");
        assert_eq!(transport.base_url, "https://mem0.example.com");
    }

    #[test]
    fn real_transport_trims_trailing_slash() {
        let transport = Mem0HttpTransport::new("http://localhost:8888/", None)
            .expect("builds with a trailing slash");
        // Trimmed so `base_url + "/memories"` is not double-slashed.
        assert_eq!(transport.base_url, "http://localhost:8888");
    }

    #[test]
    fn real_transport_rejects_a_blocked_url() {
        let error = Mem0HttpTransport::new("https://169.254.169.254", None)
            .expect_err("cloud-metadata IP must be refused at construction");
        assert!(matches!(error, Mem0Error::InvalidUrl { .. }));
    }

    #[test]
    fn real_transport_rejects_a_non_http_scheme() {
        let error = Mem0HttpTransport::new("file:///etc/passwd", None)
            .expect_err("non-http scheme refused");
        assert!(matches!(error, Mem0Error::InvalidUrl { .. }));
    }

    #[test]
    fn real_transport_rejects_an_invalid_header_api_key() {
        // A control character cannot be encoded into an Authorization header.
        let error = Mem0HttpTransport::new("https://mem0.example.com", Some("bad\nkey"))
            .expect_err("an un-encodable API key is refused");
        assert!(matches!(error, Mem0Error::Client { .. }));
    }
}
