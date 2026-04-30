//! Network policy and HTTP egress boundary for IronClaw Reborn.
//!
//! This crate evaluates host API [`NetworkPolicy`] values against scoped network
//! requests, resolves DNS, rejects private resolved targets when configured,
//! and owns outbound HTTP transport for host-mediated runtime requests. It does
//! not inject secrets, reserve resources, emit audit/events, or run product
//! workflow.

use std::{
    collections::HashMap,
    io::Read,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_host_api::{
    NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTarget, NetworkTargetPattern, ResourceScope,
};
use thiserror::Error;

pub const DEFAULT_RESPONSE_BODY_LIMIT: u64 = 5 * 1024 * 1024;
const MAX_RESPONSE_BODY_LIMIT: u64 = DEFAULT_RESPONSE_BODY_LIMIT;

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

/// Full host-mediated HTTP request handled by the network boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkHttpRequest {
    pub scope: ResourceScope,
    pub method: NetworkMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub policy: NetworkPolicy,
    pub response_body_limit: Option<u64>,
    pub timeout_ms: Option<u32>,
}

/// Transport request after policy, URL, DNS, and private-IP checks succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkTransportRequest {
    pub method: NetworkMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub resolved_ips: Vec<IpAddr>,
    pub response_body_limit: Option<u64>,
    pub timeout_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkHttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub usage: NetworkUsage,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkUsage {
    /// Outbound request body bytes. Response bytes are tracked separately.
    pub request_bytes: u64,
    pub response_bytes: u64,
    pub resolved_ip: Option<IpAddr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkHttpErrorKind {
    InvalidUrl,
    PolicyDenied,
    DnsFailed,
    TransportFailed,
    ResponseBodyLimitExceeded,
}

impl NetworkHttpErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUrl => "invalid_url",
            Self::PolicyDenied => "policy_denied",
            Self::DnsFailed => "dns_failed",
            Self::TransportFailed => "transport_failed",
            Self::ResponseBodyLimitExceeded => "response_body_limit_exceeded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NetworkHttpError {
    #[error("invalid network URL: {reason}")]
    InvalidUrl {
        reason: String,
        request_bytes: u64,
        response_bytes: u64,
    },
    #[error("network policy denied request: {reason}")]
    PolicyDenied {
        reason: String,
        request_bytes: u64,
        response_bytes: u64,
    },
    #[error("network DNS resolution failed: {reason}")]
    Dns {
        reason: String,
        request_bytes: u64,
        response_bytes: u64,
    },
    #[error("network transport failed: {reason}")]
    Transport {
        reason: String,
        request_bytes: u64,
        response_bytes: u64,
    },
    #[error("network response body exceeded limit {limit}")]
    ResponseBodyLimit {
        limit: u64,
        request_bytes: u64,
        response_bytes: u64,
    },
}

impl NetworkHttpError {
    pub fn kind(&self) -> NetworkHttpErrorKind {
        match self {
            Self::InvalidUrl { .. } => NetworkHttpErrorKind::InvalidUrl,
            Self::PolicyDenied { .. } => NetworkHttpErrorKind::PolicyDenied,
            Self::Dns { .. } => NetworkHttpErrorKind::DnsFailed,
            Self::Transport { .. } => NetworkHttpErrorKind::TransportFailed,
            Self::ResponseBodyLimit { .. } => NetworkHttpErrorKind::ResponseBodyLimitExceeded,
        }
    }

    pub fn stable_reason(&self) -> &'static str {
        self.kind().as_str()
    }

    pub fn request_bytes(&self) -> u64 {
        match self {
            Self::InvalidUrl { request_bytes, .. }
            | Self::PolicyDenied { request_bytes, .. }
            | Self::Dns { request_bytes, .. }
            | Self::Transport { request_bytes, .. }
            | Self::ResponseBodyLimit { request_bytes, .. } => *request_bytes,
        }
    }

    pub fn response_bytes(&self) -> u64 {
        match self {
            Self::InvalidUrl { response_bytes, .. }
            | Self::PolicyDenied { response_bytes, .. }
            | Self::Dns { response_bytes, .. }
            | Self::Transport { response_bytes, .. }
            | Self::ResponseBodyLimit { response_bytes, .. } => *response_bytes,
        }
    }
}

pub trait NetworkHttpEgress: Send + Sync {
    fn execute(&self, request: NetworkHttpRequest)
    -> Result<NetworkHttpResponse, NetworkHttpError>;
}

pub trait NetworkHttpTransport: Send + Sync {
    fn execute(
        &self,
        request: NetworkTransportRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError>;
}

pub trait NetworkResolver: Send + Sync {
    fn resolve_ips(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, NetworkHttpError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemNetworkResolver;

impl NetworkResolver for SystemNetworkResolver {
    fn resolve_ips(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, NetworkHttpError> {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![ip]);
        }
        (host, port)
            .to_socket_addrs()
            .map_err(|error| NetworkHttpError::Dns {
                reason: error.to_string(),
                request_bytes: 0,
                response_bytes: 0,
            })
            .map(|addrs| addrs.map(|addr| addr.ip()).collect())
    }
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

impl<T, R> NetworkHttpEgress for PolicyNetworkHttpEgress<T, R>
where
    T: NetworkHttpTransport,
    R: NetworkResolver,
{
    fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let request_body_bytes = request.body.len() as u64;
        let estimated_request_bytes = estimate_http_request_bytes(
            request.method,
            &request.url,
            &request.headers,
            &request.body,
        );
        reject_caller_host_header(&request.headers)?;
        let target = network_target_for_url(&request.url, estimated_request_bytes)?;
        let permit = StaticNetworkPolicyEnforcer::new(request.policy.clone())
            .authorize_blocking(NetworkRequest {
                scope: request.scope,
                target: target.clone(),
                method: request.method,
                estimated_bytes: Some(estimated_request_bytes),
            })
            .map_err(|error| NetworkHttpError::PolicyDenied {
                reason: error.to_string(),
                request_bytes: 0,
                response_bytes: 0,
            })?;
        let resolved_ips = resolve_public_ips(
            &target,
            &request.policy,
            &self.resolver,
            estimated_request_bytes,
        )?;
        let first_resolved_ip = resolved_ips.first().copied();

        let transport_request = NetworkTransportRequest {
            method: permit.method,
            url: request.url,
            headers: request.headers,
            body: request.body,
            resolved_ips,
            response_body_limit: request.response_body_limit,
            timeout_ms: request.timeout_ms,
        };
        let mut response = self.transport.execute(transport_request)?;
        response.usage.request_bytes = response.usage.request_bytes.max(request_body_bytes);
        response.usage.resolved_ip = response.usage.resolved_ip.or(first_resolved_ip);
        Ok(response)
    }
}

const MAX_REQWEST_CLIENT_CACHE_ENTRIES: usize = 128;

#[derive(Clone)]
pub struct ReqwestNetworkTransport {
    timeout: Duration,
    client_cache: Arc<Mutex<HashMap<ReqwestClientKey, reqwest::blocking::Client>>>,
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

    fn client_for(
        &self,
        key: ReqwestClientKey,
        request_bytes: u64,
    ) -> Result<reqwest::blocking::Client, NetworkHttpError> {
        if let Some(client) = self
            .client_cache
            .lock()
            .map_err(|_| NetworkHttpError::Transport {
                reason: "reqwest client cache lock poisoned".to_string(),
                request_bytes,
                response_bytes: 0,
            })?
            .get(&key)
            .cloned()
        {
            return Ok(client);
        }

        let mut builder = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(key.timeout);
        if !key.resolved_addrs.is_empty() {
            builder = builder.resolve_to_addrs(&key.host, &key.resolved_addrs);
        }
        let client = builder
            .build()
            .map_err(|error| NetworkHttpError::Transport {
                reason: error.to_string(),
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

impl NetworkHttpTransport for ReqwestNetworkTransport {
    fn execute(
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
        let client = self.client_for(
            ReqwestClientKey {
                host,
                port,
                resolved_addrs,
                timeout: effective_request_timeout(request.timeout_ms, self.timeout),
            },
            request_bytes,
        )?;

        let mut req = client
            .request(reqwest_method(request.method), url)
            .body(request.body);
        for (name, value) in request.headers {
            req = req.header(name, value);
        }
        let response = req.send().map_err(|error| NetworkHttpError::Transport {
            reason: error.to_string(),
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
        let mut reader = response.take(limit.saturating_add(1));
        reader
            .read_to_end(&mut body)
            .map_err(|error| NetworkHttpError::Transport {
                reason: error.to_string(),
                request_bytes,
                response_bytes: body.len() as u64,
            })?;
        let response_bytes = body.len() as u64;
        if response_bytes > limit {
            return Err(NetworkHttpError::ResponseBodyLimit {
                limit,
                request_bytes,
                response_bytes,
            });
        }

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
    #[error("network egress estimate is required when limit {limit} is configured")]
    EgressEstimateRequired {
        scope: Box<ResourceScope>,
        limit: u64,
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

    pub fn is_egress_estimate_required(&self) -> bool {
        matches!(self, Self::EgressEstimateRequired { .. })
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

    pub fn authorize_blocking(
        &self,
        request: NetworkRequest,
    ) -> Result<NetworkPermit, NetworkPolicyError> {
        authorize_static_policy(&self.policy, request)
    }
}

#[async_trait]
impl NetworkPolicyEnforcer for StaticNetworkPolicyEnforcer {
    async fn authorize(
        &self,
        request: NetworkRequest,
    ) -> Result<NetworkPermit, NetworkPolicyError> {
        authorize_static_policy(&self.policy, request)
    }
}

fn authorize_static_policy(
    policy: &NetworkPolicy,
    request: NetworkRequest,
) -> Result<NetworkPermit, NetworkPolicyError> {
    if let Some(limit) = policy.max_egress_bytes {
        let Some(estimated) = request.estimated_bytes else {
            return Err(NetworkPolicyError::EgressEstimateRequired {
                scope: Box::new(request.scope),
                limit,
            });
        };
        if estimated > limit {
            return Err(NetworkPolicyError::EgressLimitExceeded {
                scope: Box::new(request.scope),
                estimated,
                limit,
            });
        }
    }

    if policy.deny_private_ip_ranges
        && let Ok(ip) = request.target.host.parse::<IpAddr>()
        && is_private_or_loopback_ip(ip)
    {
        return Err(NetworkPolicyError::PrivateTargetDenied {
            scope: Box::new(request.scope),
            target: request.target,
        });
    }

    if !network_policy_allows(policy, &request.target) {
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
        let suffix_with_dot = format!(".{suffix}");
        let Some(prefix) = host.strip_suffix(&suffix_with_dot) else {
            return false;
        };
        !prefix.is_empty() && !prefix.contains('.')
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
                || ip.octets()[0] == 0
                || is_carrier_grade_nat_v4(ip)
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_private_or_loopback_ip(IpAddr::V4(mapped));
            }
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
                || is_documentation_v6(ip)
        }
    }
}

fn is_carrier_grade_nat_v4(ip: std::net::Ipv4Addr) -> bool {
    let [first, second, ..] = ip.octets();
    first == 100 && (64..=127).contains(&second)
}

fn is_documentation_v6(ip: std::net::Ipv6Addr) -> bool {
    let segments = ip.segments();
    segments[0] == 0x2001 && segments[1] == 0x0db8
}

pub fn scheme_label(scheme: NetworkScheme) -> &'static str {
    match scheme {
        NetworkScheme::Http => "http",
        NetworkScheme::Https => "https",
    }
}

pub fn network_target_for_url(
    raw: &str,
    request_bytes: u64,
) -> Result<NetworkTarget, NetworkHttpError> {
    let url = url::Url::parse(raw).map_err(|error| NetworkHttpError::InvalidUrl {
        reason: error.to_string(),
        request_bytes,
        response_bytes: 0,
    })?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(NetworkHttpError::InvalidUrl {
            reason: "URL userinfo is not allowed".to_string(),
            request_bytes,
            response_bytes: 0,
        });
    }
    let scheme = match url.scheme() {
        "http" => NetworkScheme::Http,
        "https" => NetworkScheme::Https,
        other => {
            return Err(NetworkHttpError::InvalidUrl {
                reason: format!("unsupported URL scheme {other}"),
                request_bytes,
                response_bytes: 0,
            });
        }
    };
    let host = url
        .host_str()
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| NetworkHttpError::InvalidUrl {
            reason: "URL host is required".to_string(),
            request_bytes,
            response_bytes: 0,
        })?
        .to_ascii_lowercase();
    Ok(NetworkTarget {
        scheme,
        host,
        port: url.port_or_known_default(),
    })
}

pub fn default_port(scheme: NetworkScheme) -> u16 {
    match scheme {
        NetworkScheme::Http => 80,
        NetworkScheme::Https => 443,
    }
}

fn resolve_public_ips<R>(
    target: &NetworkTarget,
    policy: &NetworkPolicy,
    resolver: &R,
    _request_bytes: u64,
) -> Result<Vec<IpAddr>, NetworkHttpError>
where
    R: NetworkResolver,
{
    let resolved_ips = if let Ok(ip) = target.host.parse::<IpAddr>() {
        vec![ip]
    } else {
        let port = target.port.unwrap_or_else(|| default_port(target.scheme));
        resolver
            .resolve_ips(&target.host, port)
            .map_err(|error| NetworkHttpError::Dns {
                reason: error.to_string(),
                request_bytes: 0,
                response_bytes: error.response_bytes(),
            })?
    };
    if resolved_ips.is_empty() {
        return Err(NetworkHttpError::Dns {
            reason: "network target did not resolve to any IP addresses".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        });
    }
    if policy.deny_private_ip_ranges && resolved_ips.iter().copied().any(is_private_or_loopback_ip)
    {
        return Err(NetworkHttpError::PolicyDenied {
            reason: "network target resolves to a private or host-local IP".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        });
    }
    Ok(resolved_ips)
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

fn reject_caller_host_header(headers: &[(String, String)]) -> Result<(), NetworkHttpError> {
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

#[cfg(test)]
mod tests {
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

    #[test]
    fn reqwest_transport_caches_clients_by_resolution_key() {
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

        let _ = transport.client_for(key.clone(), 0).unwrap();
        let _ = transport.client_for(key, 0).unwrap();

        assert_eq!(transport.client_cache.lock().unwrap().len(), 1);
    }
}
