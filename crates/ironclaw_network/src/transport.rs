use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_host_api::NetworkMethod;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    egress::NetworkHttpTransport,
    error::NetworkHttpError,
    types::{
        DEFAULT_RESPONSE_BODY_LIMIT, MAX_RESPONSE_BODY_LIMIT, NetworkHttpResponse,
        NetworkTransportRequest, NetworkUsage,
    },
};

const MAX_REQWEST_CLIENT_CACHE_ENTRIES: usize = 128;

#[derive(Clone)]
pub struct ReqwestNetworkTransport {
    timeout: Duration,
    client_cache: Arc<Mutex<HashMap<ReqwestClientKey, reqwest::Client>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReqwestClientKey {
    host: String,
    port: u16,
    resolved_addrs: Vec<SocketAddr>,
    timeout: Duration,
}

impl std::fmt::Debug for ReqwestNetworkTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReqwestNetworkTransport")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl Default for ReqwestNetworkTransport {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

impl ReqwestNetworkTransport {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            client_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn client_for(
        &self,
        key: ReqwestClientKey,
        request_bytes: u64,
    ) -> Result<reqwest::Client, NetworkHttpError> {
        {
            let cache = self
                .client_cache
                .lock()
                .map_err(|_| NetworkHttpError::Transport {
                    reason: "reqwest client cache lock poisoned".to_string(),
                    request_bytes,
                    response_bytes: 0,
                })?;
            if let Some(client) = cache.get(&key).cloned() {
                return Ok(client);
            }
        }

        let build_key = key.clone();
        let client = tokio::task::spawn_blocking(move || build_reqwest_client(&build_key))
            .await
            .map_err(|error| NetworkHttpError::Transport {
                reason: format!("reqwest client builder task failed: {error}"),
                request_bytes,
                response_bytes: 0,
            })?
            .map_err(|error| NetworkHttpError::Transport {
                reason: reqwest_error_diagnostic(&error),
                request_bytes,
                response_bytes: 0,
            })?;

        let mut cache = self
            .client_cache
            .lock()
            .map_err(|_| NetworkHttpError::Transport {
                reason: "reqwest client cache lock poisoned".to_string(),
                request_bytes,
                response_bytes: 0,
            })?;
        if cache.len() >= MAX_REQWEST_CLIENT_CACHE_ENTRIES {
            cache.clear();
        }
        Ok(cache.entry(key).or_insert(client).clone())
    }
}

fn build_reqwest_client(key: &ReqwestClientKey) -> Result<reqwest::Client, reqwest::Error> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(key.timeout);
    if !key.resolved_addrs.is_empty() {
        builder = builder.resolve_to_addrs(&key.host, &key.resolved_addrs);
    }
    builder.build()
}

/// Parses the URL while scrubbing only the source request carrier copy.
///
/// The returned `url::Url` and reqwest internals still retain plaintext while
/// the request is dispatched. Parse failures use a fixed diagnostic rather
/// than `url::ParseError` formatting because those diagnostics may include
/// raw input that contains injected credentials.
/// The source request URL is consumed even on parse failure so callers cannot
/// later inspect a credential-bearing raw URL for diagnostics.
fn take_request_url(
    request: &mut NetworkTransportRequest,
    request_bytes: u64,
) -> Result<url::Url, NetworkHttpError> {
    let mut raw_url = std::mem::take(&mut request.url);
    let parsed = url::Url::parse(&raw_url).map_err(|_| NetworkHttpError::InvalidUrl {
        reason: "URL parse error: invalid format".to_string(),
        request_bytes,
        response_bytes: 0,
    });
    raw_url.zeroize();
    parsed
}

fn apply_request_headers(
    mut req: reqwest::RequestBuilder,
    headers: &mut [(String, String)],
) -> reqwest::RequestBuilder {
    for (name, value) in headers.iter() {
        req = req.header(name.as_str(), value.as_str());
    }
    for (name, value) in headers.iter_mut() {
        name.zeroize();
        value.zeroize();
    }
    // reqwest's internal HeaderMap retains its copied values until the request
    // builder is consumed and the response future completes.
    req
}

#[async_trait]
impl NetworkHttpTransport for ReqwestNetworkTransport {
    async fn execute(
        &self,
        mut request: NetworkTransportRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let request_bytes = request.body.len() as u64;
        reject_caller_host_header(&request.headers)?;
        let url = take_request_url(&mut request, request_bytes)?;
        let host = url
            .host_str()
            .ok_or_else(|| NetworkHttpError::InvalidUrl {
                reason: "URL host is required".to_string(),
                request_bytes,
                response_bytes: 0,
            })?
            .to_string();
        let port = url
            .port_or_known_default()
            .ok_or_else(|| NetworkHttpError::InvalidUrl {
                reason: "URL port is required".to_string(),
                request_bytes,
                response_bytes: 0,
            })?;

        let resolved_addrs = request
            .resolved_ips
            .iter()
            .copied()
            .map(|resolved_ip| SocketAddr::new(resolved_ip, port))
            .collect::<Vec<_>>();
        let client = self
            .client_for(
                ReqwestClientKey {
                    host,
                    port,
                    resolved_addrs,
                    timeout: effective_request_timeout(request.timeout_ms, self.timeout),
                },
                request_bytes,
            )
            .await?;

        let mut headers = Zeroizing::new(std::mem::take(&mut request.headers));
        let mut req = client
            .request(reqwest_method(request.method), url)
            .body(std::mem::take(&mut request.body));
        req = apply_request_headers(req, &mut headers[..]);
        let mut response = req
            .send()
            .await
            .map_err(|error| NetworkHttpError::Transport {
                reason: reqwest_error_diagnostic(&error),
                request_bytes,
                response_bytes: 0,
            })?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_string())))
            .collect::<Vec<_>>();
        let limit = effective_response_body_limit(request.response_body_limit);
        let mut body = Vec::new();
        while let Some(chunk) =
            response
                .chunk()
                .await
                .map_err(|error| NetworkHttpError::Transport {
                    reason: error.to_string(),
                    request_bytes,
                    response_bytes: body.len() as u64,
                })?
        {
            let current_len = body.len() as u64;
            let remaining = limit.saturating_sub(current_len);
            if chunk.len() as u64 > remaining {
                let take = remaining as usize;
                body.extend_from_slice(&chunk[..take.min(chunk.len())]);
                return Err(NetworkHttpError::ResponseBodyLimit {
                    limit,
                    request_bytes,
                    response_bytes: limit.saturating_add(1),
                    partial_response: Some(NetworkHttpResponse {
                        status,
                        headers,
                        body,
                        usage: NetworkUsage {
                            request_bytes,
                            response_bytes: limit.saturating_add(1),
                            resolved_ip: request.resolved_ips.first().copied(),
                        },
                    }),
                });
            }
            body.extend_from_slice(&chunk);
            let response_bytes = body.len() as u64;
            if response_bytes > limit {
                body.truncate(limit as usize);
                return Err(NetworkHttpError::ResponseBodyLimit {
                    limit,
                    request_bytes,
                    response_bytes,
                    partial_response: Some(NetworkHttpResponse {
                        status,
                        headers,
                        body,
                        usage: NetworkUsage {
                            request_bytes,
                            response_bytes,
                            resolved_ip: request.resolved_ips.first().copied(),
                        },
                    }),
                });
            }
        }
        let response_bytes = body.len() as u64;
        Ok(NetworkHttpResponse {
            status,
            headers,
            body,
            usage: NetworkUsage {
                request_bytes,
                response_bytes,
                resolved_ip: request.resolved_ips.first().copied(),
            },
        })
    }
}

fn effective_response_body_limit(requested: Option<u64>) -> u64 {
    requested
        .unwrap_or(DEFAULT_RESPONSE_BODY_LIMIT)
        .min(MAX_RESPONSE_BODY_LIMIT)
}

fn effective_request_timeout(requested_ms: Option<u32>, default: Duration) -> Duration {
    requested_ms
        .map(|timeout_ms| Duration::from_millis(u64::from(timeout_ms.max(1))).min(default))
        .unwrap_or(default)
}

pub(crate) fn reject_caller_host_header(
    headers: &[(String, String)],
) -> Result<(), NetworkHttpError> {
    if headers
        .iter()
        .any(|(name, _)| name.trim().eq_ignore_ascii_case("host"))
    {
        return Err(NetworkHttpError::PolicyDenied {
            reason: "caller-provided Host header is not allowed".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        });
    }
    Ok(())
}

fn reqwest_method(method: NetworkMethod) -> reqwest::Method {
    match method {
        NetworkMethod::Get => reqwest::Method::GET,
        NetworkMethod::Post => reqwest::Method::POST,
        NetworkMethod::Put => reqwest::Method::PUT,
        NetworkMethod::Patch => reqwest::Method::PATCH,
        NetworkMethod::Delete => reqwest::Method::DELETE,
        NetworkMethod::Head => reqwest::Method::HEAD,
    }
}

fn reqwest_error_diagnostic(error: &reqwest::Error) -> String {
    format!("{error:?}")
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::*;
    use reqwest::Method;

    #[test]
    fn take_request_url_scrubs_only_source_carrier_copy() {
        let mut request = NetworkTransportRequest {
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1?token=sk-query-secret".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            resolved_ips: Vec::new(),
            response_body_limit: None,
            timeout_ms: None,
        };

        let parsed = take_request_url(&mut request, 0).unwrap();

        assert_eq!(parsed.host_str(), Some("api.example.test"));
        assert_eq!(
            parsed.as_str(),
            "https://api.example.test/v1?token=sk-query-secret"
        );
        assert!(request.url.is_empty());
    }

    #[test]
    fn take_request_url_error_does_not_include_source_url() {
        let mut request = NetworkTransportRequest {
            method: NetworkMethod::Get,
            url: "https://api.example.test:bad-port/v1?token=sk-query-secret".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            resolved_ips: Vec::new(),
            response_body_limit: None,
            timeout_ms: None,
        };

        let error = take_request_url(&mut request, 0).unwrap_err();

        let NetworkHttpError::InvalidUrl { reason, .. } = error else {
            panic!("expected invalid URL error");
        };
        assert_eq!(reason, "URL parse error: invalid format");
        assert!(!reason.contains("api.example.test"));
        assert!(!reason.contains("sk-query-secret"));
        assert!(request.url.is_empty());
    }

    #[test]
    fn take_request_url_relative_error_does_not_include_source_url() {
        let mut request = NetworkTransportRequest {
            method: NetworkMethod::Get,
            url: "/relative/path?token=sk-query-secret".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            resolved_ips: Vec::new(),
            response_body_limit: None,
            timeout_ms: None,
        };

        let error = take_request_url(&mut request, 0).unwrap_err();

        let NetworkHttpError::InvalidUrl { reason, .. } = error else {
            panic!("expected invalid URL error");
        };
        assert_eq!(reason, "URL parse error: invalid format");
        assert!(!reason.contains("/relative/path"));
        assert!(!reason.contains("sk-query-secret"));
        assert!(request.url.is_empty());
    }

    #[test]
    fn apply_request_headers_zeroizes_source_carrier_copy_after_reqwest_build() {
        let client = reqwest::Client::new();
        let req = client.request(Method::GET, "http://example.com");
        let mut headers = vec![
            (
                "authorization".to_string(),
                "Bearer sk-header-secret".to_string(),
            ),
            (
                "x-api-key".to_string(),
                "sk-second-header-secret".to_string(),
            ),
        ];

        let req = apply_request_headers(req, &mut headers);

        assert_eq!(headers[0], (String::new(), String::new()));
        assert_eq!(headers[1], (String::new(), String::new()));
        let _ = req;
    }

    #[test]
    fn effective_request_timeout_clamps_requested_timeout_to_transport_default() {
        assert_eq!(
            effective_request_timeout(Some(60_000), Duration::from_secs(30)),
            Duration::from_secs(30)
        );
        assert_eq!(
            effective_request_timeout(Some(250), Duration::from_secs(30)),
            Duration::from_millis(250)
        );
        assert_eq!(
            effective_request_timeout(Some(0), Duration::from_secs(30)),
            Duration::from_millis(1)
        );
        assert_eq!(
            effective_request_timeout(None, Duration::from_secs(30)),
            Duration::from_secs(30)
        );
    }

    #[tokio::test]
    async fn reqwest_transport_caches_clients_by_resolution_key() {
        let transport = ReqwestNetworkTransport::new(Duration::from_secs(1));
        let key = ReqwestClientKey {
            host: "api.example.test".to_string(),
            port: 443,
            resolved_addrs: vec![SocketAddr::new(
                IpAddr::V4(std::net::Ipv4Addr::new(93, 184, 216, 34)),
                443,
            )],
            timeout: Duration::from_secs(1),
        };

        let _ = transport.client_for(key.clone(), 0).await.unwrap();
        let _ = transport.client_for(key, 0).await.unwrap();

        assert_eq!(transport.client_cache.lock().unwrap().len(), 1);
    }
}
