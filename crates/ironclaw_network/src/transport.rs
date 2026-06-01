use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_host_api::NetworkMethod;

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

#[async_trait]
impl NetworkHttpTransport for ReqwestNetworkTransport {
    async fn execute(
        &self,
        request: NetworkTransportRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let request_bytes = request.body.len() as u64;
        reject_caller_host_header(&request.headers)?;
        let url = url::Url::parse(&request.url).map_err(|error| NetworkHttpError::InvalidUrl {
            reason: error.to_string(),
            request_bytes,
            response_bytes: 0,
        })?;
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

        let mut req = client
            .request(reqwest_method(request.method), url)
            .body(request.body);
        for (name, value) in request.headers {
            req = req.header(name, value);
        }
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
                let take = remaining.saturating_add(1) as usize;
                body.extend_from_slice(&chunk[..take.min(chunk.len())]);
                return Err(NetworkHttpError::ResponseBodyLimit {
                    limit,
                    request_bytes,
                    response_bytes: body.len() as u64,
                });
            }
            body.extend_from_slice(&chunk);
            let response_bytes = body.len() as u64;
            if response_bytes > limit {
                return Err(NetworkHttpError::ResponseBodyLimit {
                    limit,
                    request_bytes,
                    response_bytes,
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
