//! Network policy boundary for IronClaw Reborn.
//!
//! This crate evaluates host API [`NetworkPolicy`] values against scoped network
//! requests and provides a hardened HTTP egress client for runtime-owned network
//! calls. It does not inject secrets, reserve resources, or emit audit/events.

use std::{
    io::Read,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_host_api::{
    NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTarget, NetworkTargetPattern, ResourceScope,
};
use reqwest::blocking::Client;
use thiserror::Error;

/// One scoped network operation to authorize before a runtime performs I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkRequest {
    pub scope: ResourceScope,
    pub target: NetworkTarget,
    pub method: NetworkMethod,
    pub estimated_bytes: Option<u64>,
}

/// Metadata permit returned after policy evaluation succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkPermit {
    pub scope: ResourceScope,
    pub target: NetworkTarget,
    pub method: NetworkMethod,
    pub estimated_bytes: Option<u64>,
}

/// Network policy denial. Variants intentionally carry metadata only.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NetworkPolicyError {
    #[error("network target is not allowed by policy")]
    TargetDenied {
        scope: Box<ResourceScope>,
        target: NetworkTarget,
    },
    #[error(
        "network target is private, loopback, link-local, documentation, or otherwise host-local"
    )]
    PrivateTargetDenied {
        scope: Box<ResourceScope>,
        target: NetworkTarget,
    },
    #[error("network egress estimate {estimated} exceeds limit {limit}")]
    EgressLimitExceeded {
        scope: Box<ResourceScope>,
        estimated: u64,
        limit: u64,
    },
}

impl NetworkPolicyError {
    pub fn is_target_denied(&self) -> bool {
        matches!(self, Self::TargetDenied { .. })
    }

    pub fn is_private_target_denied(&self) -> bool {
        matches!(self, Self::PrivateTargetDenied { .. })
    }

    pub fn is_egress_limit_exceeded(&self) -> bool {
        matches!(self, Self::EgressLimitExceeded { .. })
    }
}

/// Scoped network policy evaluation contract.
#[async_trait]
pub trait NetworkPolicyEnforcer: Send + Sync {
    /// Authorizes one scoped network request without performing I/O.
    async fn authorize(&self, request: NetworkRequest)
    -> Result<NetworkPermit, NetworkPolicyError>;
}

/// Static policy enforcer for contract tests and composition scaffolding.
#[derive(Debug, Clone)]
pub struct StaticNetworkPolicyEnforcer {
    policy: NetworkPolicy,
}

impl StaticNetworkPolicyEnforcer {
    pub fn new(policy: NetworkPolicy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> &NetworkPolicy {
        &self.policy
    }
}

#[async_trait]
impl NetworkPolicyEnforcer for StaticNetworkPolicyEnforcer {
    async fn authorize(
        &self,
        request: NetworkRequest,
    ) -> Result<NetworkPermit, NetworkPolicyError> {
        if let Some(limit) = self.policy.max_egress_bytes
            && let Some(estimated) = request.estimated_bytes
            && estimated > limit
        {
            return Err(NetworkPolicyError::EgressLimitExceeded {
                scope: Box::new(request.scope),
                estimated,
                limit,
            });
        }

        if self.policy.deny_private_ip_ranges
            && let Ok(ip) = request.target.host.parse::<IpAddr>()
            && is_private_or_loopback_ip(ip)
        {
            return Err(NetworkPolicyError::PrivateTargetDenied {
                scope: Box::new(request.scope),
                target: request.target,
            });
        }

        if !network_policy_allows(&self.policy, &request.target) {
            return Err(NetworkPolicyError::TargetDenied {
                scope: Box::new(request.scope),
                target: request.target,
            });
        }

        Ok(NetworkPermit {
            scope: request.scope,
            target: request.target,
            method: request.method,
            estimated_bytes: request.estimated_bytes,
        })
    }
}

pub fn network_policy_allows(policy: &NetworkPolicy, target: &NetworkTarget) -> bool {
    if policy.allowed_targets.is_empty() {
        return false;
    }
    if policy.deny_private_ip_ranges
        && let Ok(ip) = target.host.parse::<IpAddr>()
        && is_private_or_loopback_ip(ip)
    {
        return false;
    }
    policy
        .allowed_targets
        .iter()
        .any(|pattern| target_matches_pattern(target, pattern))
}

pub fn target_matches_pattern(target: &NetworkTarget, pattern: &NetworkTargetPattern) -> bool {
    if let Some(scheme) = pattern.scheme
        && scheme != target.scheme
    {
        return false;
    }
    if let Some(port) = pattern.port
        && Some(port) != target.port
    {
        return false;
    }
    host_matches_pattern(&target.host.to_ascii_lowercase(), &pattern.host_pattern)
}

pub fn host_matches_pattern(host: &str, pattern: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host.ends_with(&format!(".{suffix}")) && host != suffix
    } else {
        host == pattern
    }
}

pub fn is_private_or_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.octets()[0] == 0
                || ip.octets() == [169, 254, 169, 254]
                || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xC0) == 64)
        }
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return is_private_or_loopback_ip(IpAddr::V4(ip));
            }
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
        }
    }
}

pub fn scheme_label(scheme: NetworkScheme) -> &'static str {
    match scheme {
        NetworkScheme::Http => "http",
        NetworkScheme::Https => "https",
    }
}

/// Hardened HTTP egress request for runtime-owned network calls.
#[derive(Debug, Clone)]
pub struct HttpEgressRequest {
    pub scope: ResourceScope,
    pub policy: NetworkPolicy,
    pub method: NetworkMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Option<Duration>,
    pub max_response_bytes: Option<usize>,
}

/// HTTP egress response with sensitive headers removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpEgressResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Sanitized HTTP egress failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HttpEgressError {
    #[error("network URL is invalid")]
    InvalidUrl { scope: Box<ResourceScope> },
    #[error("network URL scheme is unsupported")]
    UnsupportedScheme { scope: Box<ResourceScope> },
    #[error("network target is not allowed by policy")]
    TargetDenied {
        scope: Box<ResourceScope>,
        target: NetworkTarget,
    },
    #[error("network target resolves to a private, loopback, link-local, or host-local address")]
    PrivateTargetDenied {
        scope: Box<ResourceScope>,
        target: NetworkTarget,
    },
    #[error("network request body exceeds configured egress limit")]
    RequestTooLarge {
        scope: Box<ResourceScope>,
        limit: u64,
    },
    #[error("network response body exceeds configured size limit")]
    ResponseTooLarge {
        scope: Box<ResourceScope>,
        limit: usize,
    },
    #[error("network redirect was denied")]
    RedirectDenied { scope: Box<ResourceScope> },
    #[error("network redirect limit exceeded")]
    TooManyRedirects {
        scope: Box<ResourceScope>,
        max: usize,
    },
    #[error("network request timed out")]
    Timeout { scope: Box<ResourceScope> },
    #[error("network transport failed")]
    Transport { scope: Box<ResourceScope> },
}

/// Synchronous HTTP egress contract for runtimes that expose sync host imports.
pub trait HttpEgressClient: Send + Sync {
    fn request(&self, request: HttpEgressRequest) -> Result<HttpEgressResponse, HttpEgressError>;
}

/// Config for the hardened HTTP egress client.
#[derive(Debug, Clone)]
pub struct HardenedHttpEgressConfig {
    pub default_timeout: Duration,
    pub max_timeout: Duration,
    pub default_max_response_bytes: usize,
    pub max_redirects: usize,
    pub user_agent: String,
}

impl Default for HardenedHttpEgressConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(30),
            max_timeout: Duration::from_secs(300),
            default_max_response_bytes: 5 * 1024 * 1024,
            max_redirects: 3,
            user_agent: "IronClaw-Reborn-Network/0.1".to_string(),
        }
    }
}

/// Hardened `reqwest` HTTP egress client adapted from IronClaw's existing HTTP tool defenses.
#[derive(Debug, Clone, Default)]
pub struct HardenedHttpEgressClient {
    config: HardenedHttpEgressConfig,
}

impl HardenedHttpEgressClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: HardenedHttpEgressConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &HardenedHttpEgressConfig {
        &self.config
    }

    pub fn request(
        &self,
        request: HttpEgressRequest,
    ) -> Result<HttpEgressResponse, HttpEgressError> {
        <Self as HttpEgressClient>::request(self, request)
    }
}

impl HttpEgressClient for HardenedHttpEgressClient {
    fn request(&self, request: HttpEgressRequest) -> Result<HttpEgressResponse, HttpEgressError> {
        let timeout = clamp_timeout(
            request.timeout.unwrap_or(self.config.default_timeout),
            self.config.max_timeout,
        );
        let response_limit =
            effective_response_limit(&request, self.config.default_max_response_bytes);
        if let Some(limit) = request.policy.max_egress_bytes {
            let request_bytes = u64::try_from(request.body.len()).unwrap_or(u64::MAX);
            if request_bytes > limit {
                return Err(HttpEgressError::RequestTooLarge {
                    scope: Box::new(request.scope),
                    limit,
                });
            }
        }

        let mut current_url = parse_http_url(&request.scope, &request.url)?;
        let mut redirects_remaining = self.config.max_redirects;
        let simple_get = request.method == NetworkMethod::Get
            && request.headers.is_empty()
            && request.body.is_empty();

        loop {
            let target = authorize_url_target(&request.scope, &request.policy, &current_url)?;
            let resolved = resolve_target(&request.scope, &request.policy, &target, &current_url)?;
            let client = build_client(&self.config, timeout, &resolved, &request.scope)?;
            let response = send_once(&client, &request, current_url.clone(), timeout)?;
            let status = response.status();

            if status.is_redirection() {
                if !simple_get {
                    return Err(HttpEgressError::RedirectDenied {
                        scope: Box::new(request.scope),
                    });
                }
                if redirects_remaining == 0 {
                    return Err(HttpEgressError::TooManyRedirects {
                        scope: Box::new(request.scope),
                        max: self.config.max_redirects,
                    });
                }
                let Some(next_url) = redirect_url(&request.scope, &current_url, &response)? else {
                    return Err(HttpEgressError::RedirectDenied {
                        scope: Box::new(request.scope),
                    });
                };
                current_url = next_url;
                redirects_remaining -= 1;
                continue;
            }

            return read_response(response, &request.scope, response_limit);
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedHttpTarget {
    host: String,
    addrs: Vec<SocketAddr>,
    pin_host_resolution: bool,
}

fn clamp_timeout(timeout: Duration, max: Duration) -> Duration {
    if timeout > max { max } else { timeout }
}

fn effective_response_limit(request: &HttpEgressRequest, default_limit: usize) -> usize {
    let mut limit = request.max_response_bytes.unwrap_or(default_limit);
    if let Some(policy_limit) = request.policy.max_egress_bytes
        && let Ok(policy_limit) = usize::try_from(policy_limit)
    {
        limit = limit.min(policy_limit);
    }
    limit
}

fn parse_http_url(scope: &ResourceScope, raw: &str) -> Result<url::Url, HttpEgressError> {
    let url = url::Url::parse(raw).map_err(|_| HttpEgressError::InvalidUrl {
        scope: Box::new(scope.clone()),
    })?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        _ => Err(HttpEgressError::UnsupportedScheme {
            scope: Box::new(scope.clone()),
        }),
    }
}

fn authorize_url_target(
    scope: &ResourceScope,
    policy: &NetworkPolicy,
    url: &url::Url,
) -> Result<NetworkTarget, HttpEgressError> {
    let target = network_target_for_url(scope, url)?;
    if policy.deny_private_ip_ranges
        && let Ok(ip) = target.host.parse::<IpAddr>()
        && is_private_or_loopback_ip(ip)
    {
        return Err(HttpEgressError::PrivateTargetDenied {
            scope: Box::new(scope.clone()),
            target,
        });
    }
    if !network_policy_allows(policy, &target) {
        return Err(HttpEgressError::TargetDenied {
            scope: Box::new(scope.clone()),
            target,
        });
    }
    Ok(target)
}

fn network_target_for_url(
    scope: &ResourceScope,
    url: &url::Url,
) -> Result<NetworkTarget, HttpEgressError> {
    let scheme = match url.scheme() {
        "http" => NetworkScheme::Http,
        "https" => NetworkScheme::Https,
        _ => {
            return Err(HttpEgressError::UnsupportedScheme {
                scope: Box::new(scope.clone()),
            });
        }
    };
    let host = url
        .host_str()
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| HttpEgressError::InvalidUrl {
            scope: Box::new(scope.clone()),
        })?
        .to_ascii_lowercase();
    Ok(NetworkTarget {
        scheme,
        host,
        port: url.port(),
    })
}

fn resolve_target(
    scope: &ResourceScope,
    policy: &NetworkPolicy,
    target: &NetworkTarget,
    url: &url::Url,
) -> Result<ResolvedHttpTarget, HttpEgressError> {
    let host = url
        .host_str()
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| HttpEgressError::InvalidUrl {
            scope: Box::new(scope.clone()),
        })?
        .to_string();
    let port = url.port_or_known_default().unwrap_or(match url.scheme() {
        "http" => 80,
        _ => 443,
    });

    if let Ok(ip) = host.parse::<IpAddr>() {
        if policy.deny_private_ip_ranges && is_private_or_loopback_ip(ip) {
            return Err(HttpEgressError::PrivateTargetDenied {
                scope: Box::new(scope.clone()),
                target: target.clone(),
            });
        }
        return Ok(ResolvedHttpTarget {
            host,
            addrs: Vec::new(),
            pin_host_resolution: false,
        });
    }

    let addrs = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|_| HttpEgressError::Transport {
            scope: Box::new(scope.clone()),
        })?
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        return Err(HttpEgressError::Transport {
            scope: Box::new(scope.clone()),
        });
    }
    if policy.deny_private_ip_ranges
        && addrs
            .iter()
            .any(|addr| is_private_or_loopback_ip(addr.ip()))
    {
        return Err(HttpEgressError::PrivateTargetDenied {
            scope: Box::new(scope.clone()),
            target: target.clone(),
        });
    }

    Ok(ResolvedHttpTarget {
        host,
        addrs,
        pin_host_resolution: true,
    })
}

fn build_client(
    config: &HardenedHttpEgressConfig,
    timeout: Duration,
    resolved: &ResolvedHttpTarget,
    scope: &ResourceScope,
) -> Result<Client, HttpEgressError> {
    let mut builder = Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(config.user_agent.clone())
        .no_proxy();
    if resolved.pin_host_resolution {
        builder = builder.resolve_to_addrs(&resolved.host, &resolved.addrs);
    }
    builder.build().map_err(|_| HttpEgressError::Transport {
        scope: Box::new(scope.clone()),
    })
}

fn send_once(
    client: &Client,
    request: &HttpEgressRequest,
    url: url::Url,
    timeout: Duration,
) -> Result<reqwest::blocking::Response, HttpEgressError> {
    let mut builder = match request.method {
        NetworkMethod::Get => client.get(url),
        NetworkMethod::Post => client.post(url),
        NetworkMethod::Put => client.put(url),
        NetworkMethod::Patch => client.patch(url),
        NetworkMethod::Delete => client.delete(url),
        NetworkMethod::Head => client.head(url),
    }
    .timeout(timeout);

    for (name, value) in &request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    if !request.body.is_empty() {
        builder = builder.body(request.body.clone());
    }

    builder.send().map_err(|error| {
        if error.is_timeout() {
            HttpEgressError::Timeout {
                scope: Box::new(request.scope.clone()),
            }
        } else {
            HttpEgressError::Transport {
                scope: Box::new(request.scope.clone()),
            }
        }
    })
}

fn redirect_url(
    scope: &ResourceScope,
    current: &url::Url,
    response: &reqwest::blocking::Response,
) -> Result<Option<url::Url>, HttpEgressError> {
    let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
        return Ok(None);
    };
    let location = location
        .to_str()
        .map_err(|_| HttpEgressError::RedirectDenied {
            scope: Box::new(scope.clone()),
        })?;
    let next = if location.starts_with("http://") || location.starts_with("https://") {
        location.to_string()
    } else {
        current
            .join(location)
            .map_err(|_| HttpEgressError::RedirectDenied {
                scope: Box::new(scope.clone()),
            })?
            .to_string()
    };
    parse_http_url(scope, &next).map(Some)
}

fn read_response(
    mut response: reqwest::blocking::Response,
    scope: &ResourceScope,
    max_response_bytes: usize,
) -> Result<HttpEgressResponse, HttpEgressError> {
    if let Some(content_length) = response.content_length()
        && let Ok(content_length) = usize::try_from(content_length)
        && content_length > max_response_bytes
    {
        return Err(HttpEgressError::ResponseTooLarge {
            scope: Box::new(scope.clone()),
            limit: max_response_bytes,
        });
    }

    let status = response.status();
    let headers = redacted_headers(response.headers());
    let mut body = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = response
            .read(&mut chunk)
            .map_err(|_| HttpEgressError::Transport {
                scope: Box::new(scope.clone()),
            })?;
        if read == 0 {
            break;
        }
        if body.len().saturating_add(read) > max_response_bytes {
            return Err(HttpEgressError::ResponseTooLarge {
                scope: Box::new(scope.clone()),
                limit: max_response_bytes,
            });
        }
        body.extend_from_slice(&chunk[..read]);
    }

    Ok(HttpEgressResponse {
        status: status.as_u16(),
        headers,
        body,
    })
}

fn redacted_headers(headers: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
    const REDACTED_RESPONSE_HEADERS: &[&str] = &[
        "authorization",
        "www-authenticate",
        "set-cookie",
        "x-api-key",
        "x-auth-token",
        "proxy-authenticate",
        "proxy-authorization",
    ];

    headers
        .iter()
        .filter_map(|(name, value)| {
            if REDACTED_RESPONSE_HEADERS
                .iter()
                .any(|redacted| name.as_str().eq_ignore_ascii_case(redacted))
            {
                None
            } else {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_string(), value.to_string()))
            }
        })
        .collect()
}
