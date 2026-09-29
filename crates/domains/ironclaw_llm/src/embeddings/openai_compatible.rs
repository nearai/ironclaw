//! OpenAI-compatible `/v1/embeddings` provider.
//!
//! Works against OpenAI itself and any server that speaks the same wire shape
//! (vLLM, llama.cpp, Ollama's `/v1` surface, LM Studio, …). Transport is the
//! crate's hardened client: connect/keepalive/pool bounds from
//! [`crate::config`], the base-URL SSRF guard from [`crate::url_check`] with
//! the validated addresses pinned, no redirect following, and the loopback
//! proxy bypass.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use super::{EmbeddingError, EmbeddingProvider};

/// Base URL used by the `openai` provider id when none is configured.
pub const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Default maximum number of inputs sent in one request. OpenAI accepts up to
/// 2048; self-hosted servers are often far lower, so the default is modest and
/// larger batches are split into several requests.
pub const DEFAULT_EMBEDDING_MAX_BATCH_SIZE: usize = 64;

/// Default per-input budget in UTF-8 bytes (`str::len()`). The OpenAI
/// embedding family takes ~8191 tokens, budgeted here as ~32 000 bytes.
pub const DEFAULT_EMBEDDING_MAX_INPUT_BYTES: usize = 32_000;

/// Longest slice of an error response body kept in [`EmbeddingError::HttpStatus`].
const MAX_ERROR_BODY_CHARS: usize = 512;

/// Resolved settings for one [`OpenAiCompatibleEmbeddings`] instance.
///
/// Pure data. The API key is already resolved (the composition root reads it
/// from the environment variable the operator named); it is held as a
/// [`SecretString`] so `Debug` redacts it.
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleEmbeddingConfig {
    /// Provider id, used only to label errors and logs.
    pub provider_id: String,
    /// API base URL. A bare `scheme://host[:port]` gets `/v1` appended; a URL
    /// already ending in `/v1` (or `/v1/embeddings`) is not doubled; any other
    /// path (`/api/v4`, `/v1beta/openai`) is kept as given.
    pub base_url: String,
    /// Model identifier sent in the request body.
    pub model: String,
    /// Optional bearer key. `None` sends no `Authorization` header, for local
    /// servers that need none.
    pub api_key: Option<SecretString>,
    /// Expected vector dimension. `None` adopts the first dimension returned
    /// and holds every later response to it.
    pub dimension: Option<usize>,
    /// Maximum inputs per HTTP request; larger batches are split.
    pub max_batch_size: usize,
    /// Maximum UTF-8 bytes per input; a longer input fails the call before any
    /// request is sent.
    pub max_input_bytes: usize,
    /// Total timeout for one HTTP request (one sub-batch).
    pub request_timeout: Duration,
}

impl OpenAiCompatibleEmbeddingConfig {
    /// Settings with the crate defaults for everything but identity, endpoint
    /// and model.
    pub fn new(
        provider_id: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            base_url: base_url.into(),
            model: model.into(),
            api_key: None,
            dimension: None,
            max_batch_size: DEFAULT_EMBEDDING_MAX_BATCH_SIZE,
            max_input_bytes: DEFAULT_EMBEDDING_MAX_INPUT_BYTES,
            request_timeout: Duration::from_secs(crate::config::EMBEDDING_REQUEST_TIMEOUT_SECS),
        }
    }

    fn validate(&self) -> Result<(), EmbeddingError> {
        let invalid = |reason: &str| EmbeddingError::InvalidConfig {
            reason: format!("{} ({})", reason, self.provider_id),
        };
        if self.model.trim().is_empty() {
            return Err(invalid("model must not be empty"));
        }
        if self.base_url.trim().is_empty() {
            return Err(invalid("base URL must not be empty"));
        }
        if self.max_batch_size == 0 {
            return Err(invalid("max batch size must be greater than 0"));
        }
        if self.max_input_bytes == 0 {
            return Err(invalid("max input bytes must be greater than 0"));
        }
        if self.dimension == Some(0) {
            return Err(invalid("dimension must be greater than 0"));
        }
        if self.request_timeout.is_zero() {
            return Err(invalid("request timeout must be greater than 0"));
        }
        Ok(())
    }
}

/// The `…/embeddings` endpoint for an OpenAI-compatible base URL.
///
/// Reuses the chat providers' `/v1` rule ([`crate::normalize_openai_base_url`])
/// so a bare host gains `/v1` exactly once and `…/v1` is never doubled. A base
/// URL that already names the endpoint (`…/v1/embeddings`) is accepted as is.
pub(crate) fn embeddings_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    let base = trimmed
        .strip_suffix("/embeddings")
        .map(str::to_string)
        .unwrap_or_else(|| crate::normalize_openai_base_url(trimmed));
    format!("{base}/embeddings")
}

/// OpenAI-compatible embeddings client.
#[derive(Debug)]
pub struct OpenAiCompatibleEmbeddings {
    client: reqwest::Client,
    endpoint: String,
    provider_id: String,
    model: String,
    api_key: Option<SecretString>,
    /// Configured dimension, or the first one observed.
    dimension: OnceLock<usize>,
    max_batch_size: usize,
    max_input_bytes: usize,
    request_timeout: Duration,
}

impl OpenAiCompatibleEmbeddings {
    /// Validate `config` and build the client.
    ///
    /// Runs the crate's base-URL SSRF guard (scheme, blocked address classes,
    /// plaintext only to private hosts) and pins the HTTP client to the
    /// addresses it validated, so the endpoint cannot be re-resolved to a
    /// blocked address later. Redirects are not followed.
    pub async fn new(config: OpenAiCompatibleEmbeddingConfig) -> Result<Self, EmbeddingError> {
        config.validate()?;
        let endpoint = embeddings_endpoint(&config.base_url);

        let validated = crate::url_check::check_models_url(&config.provider_id, &endpoint)
            .await
            .map_err(|error| EmbeddingError::InvalidConfig {
                reason: format!("rejected embeddings base URL: {error}"),
            })?;
        let mut builder =
            crate::config::hardened_client_builder_with_timeout(config.request_timeout)
                .redirect(reqwest::redirect::Policy::none());
        if let Some((host, addrs)) = &validated.pin {
            builder = builder.resolve_to_addrs(host, addrs);
        }
        let client = crate::url_check::build_http_client(&config.provider_id, &endpoint, builder)
            .map_err(|error| EmbeddingError::InvalidConfig {
            reason: error.to_string(),
        })?;

        let dimension = config.dimension.map(OnceLock::from).unwrap_or_default();

        tracing::debug!(
            provider = %config.provider_id,
            model = %config.model,
            endpoint = %endpoint,
            "built OpenAI-compatible embeddings provider"
        );

        Ok(Self {
            client,
            endpoint,
            provider_id: config.provider_id,
            model: config.model,
            api_key: config.api_key,
            dimension,
            max_batch_size: config.max_batch_size,
            max_input_bytes: config.max_input_bytes,
            request_timeout: config.request_timeout,
        })
    }

    /// The resolved `…/embeddings` URL requests are sent to.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn transport_error(&self, error: reqwest::Error) -> EmbeddingError {
        if error.is_timeout() {
            EmbeddingError::Timeout {
                timeout: self.request_timeout,
            }
        } else {
            EmbeddingError::RequestFailed {
                reason: format!("{}: {error}", self.provider_id),
            }
        }
    }

    async fn embed_chunk(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let request = EmbeddingRequest {
            model: &self.model,
            input: inputs,
        };
        let mut builder = self.client.post(&self.endpoint).json(&request);
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key.expose_secret());
        }
        let response = builder
            .send()
            .await
            .map_err(|error| self.transport_error(error))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(EmbeddingError::AuthFailed {
                status: status.as_u16(),
            });
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(EmbeddingError::RateLimited {
                retry_after: crate::retry::parse_retry_after(
                    response.headers().get(reqwest::header::RETRY_AFTER),
                ),
            });
        }
        let body = response
            .bytes()
            .await
            .map_err(|error| self.transport_error(error))?;
        if !status.is_success() {
            let text = String::from_utf8_lossy(&body);
            return Err(EmbeddingError::HttpStatus {
                status: status.as_u16(),
                body: text.chars().take(MAX_ERROR_BODY_CHARS).collect(),
            });
        }

        let parsed: EmbeddingResponse =
            serde_json::from_slice(&body).map_err(|error| EmbeddingError::InvalidResponse {
                reason: format!("unparseable response body: {error}"),
            })?;
        order_by_index(parsed.data, inputs.len())
    }

    fn check_dimensions(&self, vectors: &[Vec<f32>]) -> Result<(), EmbeddingError> {
        let Some(first) = vectors.first() else {
            return Ok(());
        };
        let expected = *self.dimension.get_or_init(|| first.len());
        for vector in vectors {
            if vector.len() != expected {
                return Err(EmbeddingError::DimensionMismatch {
                    expected,
                    actual: vector.len(),
                });
            }
        }
        Ok(())
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiCompatibleEmbeddings {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn dimension(&self) -> Option<usize> {
        self.dimension.get().copied()
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if let Some((index, text)) = texts
            .iter()
            .enumerate()
            .find(|(_, text)| text.len() > self.max_input_bytes)
        {
            return Err(EmbeddingError::InputTooLong {
                index,
                length: text.len(),
                max: self.max_input_bytes,
            });
        }

        let mut vectors = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(self.max_batch_size) {
            let chunk_vectors = self.embed_chunk(chunk).await?;
            self.check_dimensions(&chunk_vectors)?;
            vectors.extend(chunk_vectors);
        }
        Ok(vectors)
    }
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingDatum>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingDatum {
    index: usize,
    embedding: Vec<f32>,
}

/// Put response vectors back in input order. The wire format carries an
/// `index` per vector and does not promise the array is sorted.
fn order_by_index(
    data: Vec<EmbeddingDatum>,
    expected: usize,
) -> Result<Vec<Vec<f32>>, EmbeddingError> {
    if data.len() != expected {
        return Err(EmbeddingError::InvalidResponse {
            reason: format!("expected {expected} embeddings, got {}", data.len()),
        });
    }
    let mut slots: Vec<Option<Vec<f32>>> = vec![None; expected];
    for datum in data {
        let slot = slots
            .get_mut(datum.index)
            .ok_or_else(|| EmbeddingError::InvalidResponse {
                reason: format!("embedding index {} is out of range", datum.index),
            })?;
        if slot.is_some() {
            return Err(EmbeddingError::InvalidResponse {
                reason: format!("embedding index {} appears twice", datum.index),
            });
        }
        if datum.embedding.is_empty() {
            return Err(EmbeddingError::InvalidResponse {
                reason: format!("embedding {} is empty", datum.index),
            });
        }
        *slot = Some(datum.embedding);
    }
    // Every slot is filled: `expected` distinct in-range indices were placed.
    Ok(slots.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    /// One request the stub saw.
    #[derive(Debug, Clone)]
    struct Captured {
        request_line: String,
        headers: String,
        body: serde_json::Value,
    }

    /// What the stub answers with: status, extra header lines, JSON body, and
    /// a delay before answering.
    #[derive(Clone)]
    struct Reply {
        status: u16,
        headers: &'static str,
        body: serde_json::Value,
        delay: Duration,
    }

    impl Reply {
        fn ok(body: serde_json::Value) -> Self {
            Self {
                status: 200,
                headers: "",
                body,
                delay: Duration::ZERO,
            }
        }
    }

    /// A loopback HTTP stub. `respond` maps each parsed request body to a
    /// reply; every request is recorded.
    struct Stub {
        base_url: String,
        captured: Arc<Mutex<Vec<Captured>>>,
    }

    impl Stub {
        async fn start<F>(respond: F) -> Self
        where
            F: Fn(&serde_json::Value) -> Reply + Send + Sync + 'static,
        {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
            let addr = listener.local_addr().expect("stub addr");
            let captured = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&captured);
            let respond = Arc::new(respond);
            tokio::spawn(async move {
                loop {
                    let Ok((mut socket, _)) = listener.accept().await else {
                        return;
                    };
                    let sink = Arc::clone(&sink);
                    let respond = Arc::clone(&respond);
                    tokio::spawn(async move {
                        let (request_line, headers, body) = read_request(&mut socket).await;
                        let body: serde_json::Value =
                            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
                        let reply = respond(&body);
                        sink.lock().await.push(Captured {
                            request_line,
                            headers,
                            body,
                        });
                        tokio::time::sleep(reply.delay).await;
                        let payload = reply.body.to_string();
                        let response = format!(
                            "HTTP/1.1 {} Stub\r\ncontent-type: application/json\r\n{}connection: close\r\ncontent-length: {}\r\n\r\n{}",
                            reply.status,
                            reply.headers,
                            payload.len(),
                            payload
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.shutdown().await;
                    });
                }
            });
            Self {
                base_url: format!("http://{addr}"),
                captured,
            }
        }

        async fn requests(&self) -> Vec<Captured> {
            self.captured.lock().await.clone()
        }
    }

    async fn read_request(socket: &mut tokio::net::TcpStream) -> (String, String, String) {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        let header_end = loop {
            let n = socket.read(&mut chunk).await.expect("read request");
            assert!(n > 0, "connection closed before headers");
            buffer.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
        let content_length = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while buffer.len() < header_end + content_length {
            let n = socket.read(&mut chunk).await.expect("read body");
            assert!(n > 0, "connection closed before body");
            buffer.extend_from_slice(&chunk[..n]);
        }
        let body =
            String::from_utf8_lossy(&buffer[header_end..header_end + content_length]).to_string();
        let (request_line, headers) = head.split_once("\r\n").unwrap_or((&head, ""));
        (request_line.to_string(), headers.to_string(), body)
    }

    /// Answer every request with one `dim`-length vector per input, the value
    /// of each vector's elements being its input's position in the batch. The
    /// `data` array is returned reversed so ordering by `index` is exercised.
    fn echo_vectors(dim: usize) -> impl Fn(&serde_json::Value) -> Reply + Send + Sync {
        move |body| {
            let count = body["input"].as_array().map_or(0, Vec::len);
            let data: Vec<_> = (0..count)
                .rev()
                .map(|i| json!({"object": "embedding", "index": i, "embedding": vec![i as f32; dim]}))
                .collect();
            Reply::ok(json!({"object": "list", "data": data, "model": "m"}))
        }
    }

    fn config(base_url: &str) -> OpenAiCompatibleEmbeddingConfig {
        OpenAiCompatibleEmbeddingConfig::new("test_embed", base_url, "embed-model")
    }

    fn texts(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn embeds_a_batch_with_the_openai_request_and_response_shape() {
        let stub = Stub::start(echo_vectors(3)).await;
        let mut cfg = config(&stub.base_url);
        cfg.api_key = Some(SecretString::from("sk-test-key".to_string()));
        let provider = OpenAiCompatibleEmbeddings::new(cfg)
            .await
            .expect("provider builds");

        let vectors = provider
            .embed(&texts(&["alpha", "beta"]))
            .await
            .expect("embed succeeds");

        // Reversed on the wire, restored to input order here.
        assert_eq!(vectors, vec![vec![0.0; 3], vec![1.0; 3]]);
        assert_eq!(provider.dimension(), Some(3));
        assert_eq!(provider.model_name(), "embed-model");

        let requests = stub.requests().await;
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.request_line, "POST /v1/embeddings HTTP/1.1");
        assert!(
            request
                .headers
                .lines()
                .any(|l| l.eq_ignore_ascii_case("authorization: Bearer sk-test-key")),
            "bearer key must be sent: {}",
            request.headers
        );
        assert!(
            request
                .headers
                .lines()
                .any(|l| l.eq_ignore_ascii_case("content-type: application/json")),
            "{}",
            request.headers
        );
        assert_eq!(
            request.body,
            json!({"model": "embed-model", "input": ["alpha", "beta"]})
        );
    }

    #[tokio::test]
    async fn sends_no_authorization_header_without_a_key() {
        let stub = Stub::start(echo_vectors(2)).await;
        let provider = OpenAiCompatibleEmbeddings::new(config(&stub.base_url))
            .await
            .expect("provider builds");
        provider.embed(&texts(&["x"])).await.expect("embed");
        let requests = stub.requests().await;
        assert!(
            !requests[0]
                .headers
                .to_ascii_lowercase()
                .contains("authorization:"),
            "{}",
            requests[0].headers
        );
    }

    #[tokio::test]
    async fn splits_batches_over_the_limit_and_keeps_input_order() {
        let stub = Stub::start(echo_vectors(2)).await;
        let mut cfg = config(&format!("{}/v1", stub.base_url));
        cfg.max_batch_size = 2;
        let provider = OpenAiCompatibleEmbeddings::new(cfg)
            .await
            .expect("provider builds");

        let vectors = provider
            .embed(&texts(&["a", "b", "c", "d", "e"]))
            .await
            .expect("embed");

        // Each sub-batch numbers its own inputs from 0.
        assert_eq!(
            vectors,
            vec![
                vec![0.0; 2],
                vec![1.0; 2],
                vec![0.0; 2],
                vec![1.0; 2],
                vec![0.0; 2]
            ]
        );
        let inputs: Vec<_> = stub
            .requests()
            .await
            .into_iter()
            .map(|r| r.body["input"].clone())
            .collect();
        assert_eq!(
            inputs,
            vec![json!(["a", "b"]), json!(["c", "d"]), json!(["e"])]
        );
    }

    #[tokio::test]
    async fn empty_input_makes_no_request() {
        let stub = Stub::start(echo_vectors(2)).await;
        let provider = OpenAiCompatibleEmbeddings::new(config(&stub.base_url))
            .await
            .expect("provider builds");
        assert!(provider.embed(&[]).await.expect("embed").is_empty());
        assert!(stub.requests().await.is_empty());
    }

    #[tokio::test]
    async fn a_slow_endpoint_times_out() {
        let stub = Stub::start(|_| Reply {
            delay: Duration::from_secs(5),
            ..Reply::ok(json!({"data": []}))
        })
        .await;
        let mut cfg = config(&stub.base_url);
        cfg.request_timeout = Duration::from_millis(200);
        let provider = OpenAiCompatibleEmbeddings::new(cfg)
            .await
            .expect("provider builds");

        let error = provider
            .embed(&texts(&["slow"]))
            .await
            .expect_err("must time out");
        assert!(
            matches!(error, EmbeddingError::Timeout { timeout } if timeout == Duration::from_millis(200)),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_response_off_the_configured_dimension_is_rejected() {
        let stub = Stub::start(echo_vectors(2)).await;
        let mut cfg = config(&stub.base_url);
        cfg.dimension = Some(3);
        let provider = OpenAiCompatibleEmbeddings::new(cfg)
            .await
            .expect("provider builds");
        assert_eq!(provider.dimension(), Some(3));

        let error = provider
            .embed(&texts(&["x"]))
            .await
            .expect_err("dimension mismatch");
        assert!(
            matches!(
                error,
                EmbeddingError::DimensionMismatch {
                    expected: 3,
                    actual: 2
                }
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn the_first_seen_dimension_binds_later_calls() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let stub = Stub::start(move |body| {
            // First call answers with 3 dimensions, every later one with 4.
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let dim = if n == 0 { 3 } else { 4 };
            echo_vectors(dim)(body)
        })
        .await;
        let provider = OpenAiCompatibleEmbeddings::new(config(&stub.base_url))
            .await
            .expect("provider builds");
        assert_eq!(provider.dimension(), None);

        provider.embed(&texts(&["first"])).await.expect("first");
        assert_eq!(provider.dimension(), Some(3));

        let error = provider
            .embed(&texts(&["second"]))
            .await
            .expect_err("second call drifts");
        assert!(
            matches!(
                error,
                EmbeddingError::DimensionMismatch {
                    expected: 3,
                    actual: 4
                }
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn mixed_dimensions_within_one_response_are_rejected() {
        let stub = Stub::start(|_| {
            Reply::ok(json!({"data": [
                {"index": 0, "embedding": [0.1, 0.2]},
                {"index": 1, "embedding": [0.1, 0.2, 0.3]}
            ]}))
        })
        .await;
        let provider = OpenAiCompatibleEmbeddings::new(config(&stub.base_url))
            .await
            .expect("provider builds");
        let error = provider
            .embed(&texts(&["a", "b"]))
            .await
            .expect_err("mixed dims");
        assert!(
            matches!(error, EmbeddingError::DimensionMismatch { .. }),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_wrong_vector_count_or_bad_index_is_an_invalid_response() {
        for data in [
            json!([{"index": 0, "embedding": [1.0]}]),
            json!([{"index": 0, "embedding": [1.0]}, {"index": 0, "embedding": [1.0]}]),
            json!([{"index": 0, "embedding": [1.0]}, {"index": 7, "embedding": [1.0]}]),
            json!([{"index": 0, "embedding": []}, {"index": 1, "embedding": []}]),
        ] {
            let stub = Stub::start(move |_| Reply::ok(json!({ "data": data.clone() }))).await;
            let provider = OpenAiCompatibleEmbeddings::new(config(&stub.base_url))
                .await
                .expect("provider builds");
            let error = provider
                .embed(&texts(&["a", "b"]))
                .await
                .expect_err("invalid response");
            assert!(
                matches!(error, EmbeddingError::InvalidResponse { .. }),
                "{error:?}"
            );
        }
    }

    #[tokio::test]
    async fn http_failures_map_to_typed_errors() {
        type Case = (u16, &'static str, fn(&EmbeddingError) -> bool);
        let cases: [Case; 3] = [
            (401, "", |e| {
                matches!(e, EmbeddingError::AuthFailed { status: 401 })
            }),
            (
                429,
                "retry-after: 7\r\n",
                |e| matches!(e, EmbeddingError::RateLimited { retry_after } if *retry_after == Duration::from_secs(7)),
            ),
            (
                500,
                "",
                |e| matches!(e, EmbeddingError::HttpStatus { status: 500, body } if body.contains("boom")),
            ),
        ];
        for (status, headers, check) in cases {
            let stub = Stub::start(move |_| Reply {
                status,
                headers,
                ..Reply::ok(json!({"error": "boom"}))
            })
            .await;
            let provider = OpenAiCompatibleEmbeddings::new(config(&stub.base_url))
                .await
                .expect("provider builds");
            let error = provider.embed(&texts(&["x"])).await.expect_err("fails");
            assert!(check(&error), "status {status}: {error:?}");
        }
    }

    #[tokio::test]
    async fn an_input_over_the_byte_budget_fails_before_any_request() {
        let stub = Stub::start(echo_vectors(2)).await;
        let mut cfg = config(&stub.base_url);
        cfg.max_input_bytes = 4;
        let provider = OpenAiCompatibleEmbeddings::new(cfg)
            .await
            .expect("provider builds");
        let error = provider
            .embed(&texts(&["ok", "too long"]))
            .await
            .expect_err("too long");
        assert!(
            matches!(
                error,
                EmbeddingError::InputTooLong {
                    index: 1,
                    length: 8,
                    max: 4
                }
            ),
            "{error:?}"
        );
        assert!(stub.requests().await.is_empty());
    }

    #[test]
    fn endpoint_never_doubles_v1() {
        for (base, expected) in [
            (
                "http://localhost:8080",
                "http://localhost:8080/v1/embeddings",
            ),
            (
                "http://localhost:8080/",
                "http://localhost:8080/v1/embeddings",
            ),
            (
                "http://localhost:8080/v1",
                "http://localhost:8080/v1/embeddings",
            ),
            (
                "http://localhost:8080/v1/",
                "http://localhost:8080/v1/embeddings",
            ),
            (
                "https://api.openai.com/V1",
                "https://api.openai.com/V1/embeddings",
            ),
            (
                "http://localhost:8080/v1/embeddings",
                "http://localhost:8080/v1/embeddings",
            ),
            (
                "https://open.bigmodel.cn/api/paas/v4",
                "https://open.bigmodel.cn/api/paas/v4/embeddings",
            ),
            (
                OPENAI_DEFAULT_BASE_URL,
                "https://api.openai.com/v1/embeddings",
            ),
        ] {
            assert_eq!(embeddings_endpoint(base), expected, "base {base}");
        }
    }

    #[tokio::test]
    async fn construction_rejects_unsafe_or_empty_settings() {
        let blocked = OpenAiCompatibleEmbeddings::new(config("http://169.254.169.254")).await;
        assert!(
            matches!(blocked, Err(EmbeddingError::InvalidConfig { .. })),
            "{blocked:?}"
        );

        let scheme = OpenAiCompatibleEmbeddings::new(config("file:///etc/passwd")).await;
        assert!(
            matches!(scheme, Err(EmbeddingError::InvalidConfig { .. })),
            "{scheme:?}"
        );

        let mut empty_model = config("http://127.0.0.1:9");
        empty_model.model = "  ".to_string();
        assert!(matches!(
            OpenAiCompatibleEmbeddings::new(empty_model).await,
            Err(EmbeddingError::InvalidConfig { .. })
        ));

        let mut zero_batch = config("http://127.0.0.1:9");
        zero_batch.max_batch_size = 0;
        assert!(matches!(
            OpenAiCompatibleEmbeddings::new(zero_batch).await,
            Err(EmbeddingError::InvalidConfig { .. })
        ));

        let mut zero_dim = config("http://127.0.0.1:9");
        zero_dim.dimension = Some(0);
        assert!(matches!(
            OpenAiCompatibleEmbeddings::new(zero_dim).await,
            Err(EmbeddingError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn config_debug_redacts_the_key() {
        let mut cfg = config("http://127.0.0.1:9");
        cfg.api_key = Some(SecretString::from("sk-very-secret".to_string()));
        assert!(!format!("{cfg:?}").contains("sk-very-secret"));
    }
}
