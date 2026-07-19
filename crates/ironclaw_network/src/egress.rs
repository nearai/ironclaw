use async_trait::async_trait;
use ironclaw_host_api::NetworkMethod;

use crate::{
    error::NetworkHttpError,
    policy::StaticNetworkPolicyEnforcer,
    resolver::{NetworkResolver, SystemNetworkResolver, resolve_public_ips},
    transport::reject_caller_host_header,
    types::{NetworkHttpRequest, NetworkHttpResponse, NetworkRequest, NetworkTransportRequest},
    url_target::network_target_for_http_url,
};

#[async_trait]
pub trait NetworkHttpEgress: Send + Sync {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError>;
}

#[async_trait]
pub trait NetworkHttpTransport: Send + Sync {
    async fn execute(
        &self,
        request: NetworkTransportRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError>;
}

#[derive(Debug, Clone)]
pub struct PolicyNetworkHttpEgress<T, R = SystemNetworkResolver> {
    transport: T,
    resolver: R,
}

impl<T> PolicyNetworkHttpEgress<T, SystemNetworkResolver> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            resolver: SystemNetworkResolver,
        }
    }
}

impl<T, R> PolicyNetworkHttpEgress<T, R> {
    pub fn new_with_resolver(transport: T, resolver: R) -> Self {
        Self {
            transport,
            resolver,
        }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }
}

#[async_trait]
impl<T, R> NetworkHttpEgress for PolicyNetworkHttpEgress<T, R>
where
    T: NetworkHttpTransport + Send + Sync,
    R: NetworkResolver + Clone + Send + Sync + 'static,
{
    async fn execute(
        &self,
        mut request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let mut redirects_remaining = MAX_REDIRECTS;
        loop {
            let estimated_request_bytes = estimate_http_request_bytes(
                request.method,
                &request.url,
                &request.headers,
                &request.body,
            );
            reject_caller_host_header(&request.headers)?;
            let target = network_target_for_http_url(&request.url, estimated_request_bytes)?;
            let permit = StaticNetworkPolicyEnforcer::new(request.policy.clone())
                .authorize_blocking(NetworkRequest {
                    // This clone only carries scoped identity metadata; the sensitive
                    // URL/header/body buffers are still moved out below with `mem::take`.
                    scope: request.scope.clone(),
                    target: target.clone(),
                    method: request.method,
                    estimated_bytes: Some(estimated_request_bytes),
                })
                .map_err(|error| NetworkHttpError::PolicyDenied {
                    reason: error.to_string(),
                    request_bytes: estimated_request_bytes,
                    response_bytes: 0,
                })?;
            let resolver = self.resolver.clone();
            let policy = request.policy.clone();
            let resolved_ips = tokio::task::spawn_blocking(move || {
                resolve_public_ips(&target, &policy, &resolver, estimated_request_bytes)
            })
            .await
            .map_err(|error| NetworkHttpError::Transport {
                reason: format!("network resolver worker failed: {error}"),
                request_bytes: estimated_request_bytes,
                response_bytes: 0,
            })??;
            // The URL is needed to resolve a relative `Location` after the
            // response, but it is scrubbed out of `request` by the `mem::take`
            // below, so capture it here before it is moved into the transport.
            let request_url = request.url.clone();
            let transport_request = NetworkTransportRequest {
                method: permit.method,
                url: std::mem::take(&mut request.url),
                headers: std::mem::take(&mut request.headers),
                body: std::mem::take(&mut request.body),
                resolved_ips,
                response_body_limit: request.response_body_limit,
                timeout_ms: request.timeout_ms,
            };
            let response = self.transport.execute(transport_request).await?;

            // Redirect following is host-mediated, not delegated to the HTTP
            // client (which is pinned to `Policy::none()`): every hop re-runs the
            // full policy authorization above against the redirect destination,
            // so an untrusted `Location` can never reach a host the caller's
            // network policy does not already allow. Credential-bearing headers
            // are stripped before the next hop so a host-injected token never
            // follows a redirect off its audience host.
            if redirects_remaining == 0 || !is_redirect_status(response.status) {
                return Ok(response);
            }
            let Some(location) = find_header(&response.headers, "location") else {
                return Ok(response);
            };
            let next_url = resolve_redirect_url(&request_url, location).ok_or_else(|| {
                NetworkHttpError::InvalidUrl {
                    reason: "redirect Location is not a valid URL".to_string(),
                    request_bytes: estimated_request_bytes,
                    response_bytes: 0,
                }
            })?;
            redirects_remaining -= 1;
            request.url = next_url;
            // 303 (and, following prevailing client behavior, 301/302) drop to a
            // bodyless GET; 307/308 preserve the method and body.
            if !preserves_method_on_redirect(response.status) {
                request.method = NetworkMethod::Get;
                request.body = Vec::new();
            }
            strip_credential_headers(&mut request.headers);
        }
    }
}

/// Maximum number of redirect hops the host-mediated egress will follow. Each
/// hop is independently policy-authorized, so this is a loop bound, not a trust
/// grant.
const MAX_REDIRECTS: u8 = 3;

fn is_redirect_status(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn preserves_method_on_redirect(status: u16) -> bool {
    matches!(status, 307 | 308)
}

/// Header names carrying host-injected or caller credential material that must
/// never follow a redirect to a different destination.
fn strip_credential_headers(headers: &mut Vec<(String, String)>) {
    headers.retain(|(name, _)| {
        !matches!(
            name.to_ascii_lowercase().as_str(),
            "authorization" | "cookie" | "proxy-authorization"
        )
    });
}

fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// Resolve a redirect `Location` (absolute or relative) against the URL that
/// produced it. Returns `None` if the base or resolved value is not a valid URL.
fn resolve_redirect_url(base: &str, location: &str) -> Option<String> {
    let base = url::Url::parse(base).ok()?;
    let resolved = base.join(location).ok()?;
    match resolved.scheme() {
        "http" | "https" => Some(resolved.to_string()),
        _ => None,
    }
}

fn estimate_http_request_bytes(
    method: NetworkMethod,
    url: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> u64 {
    let mut total = 0_u64;
    add_len(&mut total, method_label(method).len());
    add_len(&mut total, " ".len());
    add_len(&mut total, url.len());
    add_len(&mut total, " HTTP/1.1\r\n".len());
    for (name, value) in headers {
        add_len(&mut total, name.len());
        add_len(&mut total, ": ".len());
        add_len(&mut total, value.len());
        add_len(&mut total, "\r\n".len());
    }
    add_len(&mut total, "\r\n".len());
    add_len(&mut total, body.len());
    total
}

fn add_len(total: &mut u64, len: usize) {
    *total = total.saturating_add(u64::try_from(len).unwrap_or(u64::MAX));
}

fn method_label(method: NetworkMethod) -> &'static str {
    match method {
        NetworkMethod::Get => "GET",
        NetworkMethod::Post => "POST",
        NetworkMethod::Put => "PUT",
        NetworkMethod::Patch => "PATCH",
        NetworkMethod::Delete => "DELETE",
        NetworkMethod::Head => "HEAD",
    }
}
