use std::net::IpAddr;

use ironclaw_host_api::{NetworkMethod, NetworkPolicy, NetworkTarget, ResourceScope};

pub const DEFAULT_RESPONSE_BODY_LIMIT: u64 = 5 * 1024 * 1024;
pub(crate) const MAX_RESPONSE_BODY_LIMIT: u64 = DEFAULT_RESPONSE_BODY_LIMIT;

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
