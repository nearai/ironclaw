use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::llm::recording::{HttpExchangeRequest, HttpExchangeResponse, HttpInterceptor};

#[derive(Debug)]
pub struct CompositeHttpInterceptor {
    interceptors: Vec<Arc<dyn HttpInterceptor>>,
}

impl CompositeHttpInterceptor {
    pub fn new(interceptors: Vec<Arc<dyn HttpInterceptor>>) -> Self {
        Self { interceptors }
    }
}

#[async_trait]
impl HttpInterceptor for CompositeHttpInterceptor {
    async fn before_request(&self, request: &HttpExchangeRequest) -> Option<HttpExchangeResponse> {
        for (producer_idx, interceptor) in self.interceptors.iter().enumerate() {
            if let Some(response) = interceptor.before_request(request).await {
                // Notify the *other* interceptors of the synthesized response.
                // The producing interceptor must not receive `after_response`
                // for its own fabricated response (it already knows it served
                // the request); interceptors before it returned `None` from
                // `before_request`, so they get the response notification only
                // — never their own short-circuit echoed back.
                for (j, recorder) in self.interceptors.iter().enumerate() {
                    if j != producer_idx {
                        recorder.after_response(request, &response).await;
                    }
                }
                return Some(response);
            }
        }
        None
    }

    async fn after_response(&self, request: &HttpExchangeRequest, response: &HttpExchangeResponse) {
        for interceptor in &self.interceptors {
            interceptor.after_response(request, response).await;
        }
    }
}

#[derive(Debug)]
struct HostRemapHttpInterceptor {
    mappings: HashMap<String, String>,
    client: reqwest::Client,
}

/// Restrict remap targets to loopback so that even if a stray
/// `IRONCLAW_TEST_HTTP_REMAP` env var sneaks into a debug build, the
/// remap can only forward to a local listener — never an external URL.
/// Combined with the `cfg(any(test, debug_assertions))` gate at the call
/// site in `app.rs`, this is the security boundary for the remap feature.
///
/// **Why we forward request headers (including `Authorization`) verbatim:**
/// the remap is a test affordance for integration scenarios that need to
/// verify the *full* outbound request — including bearer tokens — reached
/// the (mock) destination. The e2e auth/OAuth matrix relies on this to
/// assert that the right token was attached to the right request after
/// the OAuth flow completed. Stripping credential headers would defeat
/// the test affordance entirely.
///
/// The residual threat model is: "attacker has env var control on a
/// debug/test build of the binary AND a listener on the same loopback
/// interface". An attacker with both already has trivial ways to
/// exfiltrate credentials (process introspection, binary patching,
/// reading the secrets store directly), so the marginal risk from
/// loopback-only header forwarding is acceptable.
fn is_loopback_target(base_url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(base_url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    false
}

impl HostRemapHttpInterceptor {
    fn from_env() -> Option<Self> {
        let raw = std::env::var("IRONCLAW_TEST_HTTP_REMAP").ok()?;
        let mappings = raw
            .split(',')
            .filter_map(|entry| {
                let (host, base_url) = entry.split_once('=')?;
                let host = host.trim().to_lowercase();
                let base_url = base_url.trim().trim_end_matches('/').to_string();
                if host.is_empty() || base_url.is_empty() {
                    return None;
                }
                if !is_loopback_target(&base_url) {
                    tracing::warn!(
                        host = %host,
                        base_url = %base_url,
                        "IRONCLAW_TEST_HTTP_REMAP target is not loopback; refusing to register"
                    );
                    return None;
                }
                Some((host, base_url))
            })
            .collect::<HashMap<_, _>>();
        if mappings.is_empty() {
            return None;
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .ok()?;
        Some(Self { mappings, client })
    }

    fn rewrite_url(&self, url: &str) -> Option<String> {
        let parsed = reqwest::Url::parse(url).ok()?;
        let host = parsed.host_str()?.to_lowercase();
        let base = self.mappings.get(&host)?;
        let mut rewritten = format!("{base}{}", parsed.path());
        if let Some(query) = parsed.query() {
            rewritten.push('?');
            rewritten.push_str(query);
        }
        Some(rewritten)
    }
}

#[async_trait]
impl HttpInterceptor for HostRemapHttpInterceptor {
    async fn before_request(&self, request: &HttpExchangeRequest) -> Option<HttpExchangeResponse> {
        let rewritten_url = self.rewrite_url(&request.url)?;
        let method = reqwest::Method::from_bytes(request.method.as_bytes()).ok()?;
        let mut builder = self.client.request(method, rewritten_url);
        // Forward headers verbatim. Loopback-only target restriction +
        // cfg(test/debug_assertions) gating are the security boundary;
        // see the doc comment on `is_loopback_target`.
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = &request.body {
            builder = builder.body(body.clone());
        }
        let response = builder.send().await.ok()?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), value.to_string()))
            })
            .collect();
        let body = response.text().await.ok()?;
        Some(HttpExchangeResponse {
            status,
            headers,
            body,
        })
    }

    async fn after_response(
        &self,
        _request: &HttpExchangeRequest,
        _response: &HttpExchangeResponse,
    ) {
    }
}

pub fn remap_from_env() -> Option<Arc<dyn HttpInterceptor>> {
    HostRemapHttpInterceptor::from_env().map(|interceptor| Arc::new(interceptor) as Arc<_>)
}

pub fn chain(
    interceptors: impl IntoIterator<Item = Arc<dyn HttpInterceptor>>,
) -> Option<Arc<dyn HttpInterceptor>> {
    let interceptors = interceptors.into_iter().collect::<Vec<_>>();
    match interceptors.len() {
        0 => None,
        1 => interceptors.into_iter().next(),
        _ => Some(Arc::new(CompositeHttpInterceptor::new(interceptors))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn loopback_target_validation() {
        assert!(is_loopback_target("http://localhost:8080"));
        assert!(is_loopback_target("http://127.0.0.1"));
        assert!(is_loopback_target("http://127.0.0.1:8080"));
        assert!(!is_loopback_target("https://api.anthropic.com"));
        assert!(!is_loopback_target("http://192.168.1.1"));
        assert!(!is_loopback_target("http://10.0.0.1"));
        assert!(!is_loopback_target("not-a-url"));
    }

    /// Records every `(request, response)` pair the interceptor is notified
    /// about, plus a label so we can tell who got called when.
    #[derive(Debug)]
    struct RecordingInterceptor {
        label: &'static str,
        produce: Option<HttpExchangeResponse>,
        log: Arc<Mutex<Vec<(&'static str, &'static str)>>>,
    }

    #[async_trait]
    impl HttpInterceptor for RecordingInterceptor {
        async fn before_request(
            &self,
            _request: &HttpExchangeRequest,
        ) -> Option<HttpExchangeResponse> {
            self.log.lock().unwrap().push((self.label, "before"));
            self.produce.clone()
        }

        async fn after_response(
            &self,
            _request: &HttpExchangeRequest,
            _response: &HttpExchangeResponse,
        ) {
            self.log.lock().unwrap().push((self.label, "after"));
        }
    }

    /// Regression: when one interceptor short-circuits via `before_request`,
    /// the producing interceptor must NOT receive `after_response` for its own
    /// fabricated response.
    #[tokio::test]
    async fn composite_skips_producer_in_after_response() {
        let log: Arc<Mutex<Vec<(&'static str, &'static str)>>> = Arc::new(Mutex::new(Vec::new()));
        let response = HttpExchangeResponse {
            status: 200,
            headers: vec![],
            body: "fake".to_string(),
        };
        let a = Arc::new(RecordingInterceptor {
            label: "a",
            produce: None,
            log: Arc::clone(&log),
        }) as Arc<dyn HttpInterceptor>;
        let b = Arc::new(RecordingInterceptor {
            label: "b",
            produce: Some(response.clone()),
            log: Arc::clone(&log),
        }) as Arc<dyn HttpInterceptor>;
        let c = Arc::new(RecordingInterceptor {
            label: "c",
            produce: None,
            log: Arc::clone(&log),
        }) as Arc<dyn HttpInterceptor>;
        let composite = CompositeHttpInterceptor::new(vec![a, b, c]);

        let request = HttpExchangeRequest {
            method: "GET".to_string(),
            url: "https://example.test/".to_string(),
            headers: vec![],
            body: None,
        };
        let result = composite.before_request(&request).await;
        assert!(result.is_some(), "producer should short-circuit");

        let events = log.lock().unwrap().clone();
        // 'a' and 'b' run before_request (a returns None, b produces).
        // After short-circuit: a and c get after_response (NOT b).
        assert!(events.contains(&("a", "before")));
        assert!(events.contains(&("b", "before")));
        assert!(
            !events.contains(&("c", "before")),
            "interceptors after the producer should not see before_request",
        );
        assert!(events.contains(&("a", "after")));
        assert!(events.contains(&("c", "after")));
        assert!(
            !events.contains(&("b", "after")),
            "producer must NOT receive after_response for its own response",
        );
    }
}
