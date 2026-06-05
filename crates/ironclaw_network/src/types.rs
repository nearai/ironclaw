use std::net::IpAddr;

use ironclaw_host_api::{NetworkMethod, NetworkPolicy, NetworkTarget, ResourceScope};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const DEFAULT_RESPONSE_BODY_LIMIT: u64 = 10 * 1024 * 1024;
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
///
/// URL and header values may contain host-injected credential material. The
/// carrier keeps the public field shape stable but scrubs those buffers on drop.
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

impl Drop for NetworkHttpRequest {
    fn drop(&mut self) {
        self.scrub_sensitive_url_and_headers();
    }
}

impl NetworkHttpRequest {
    fn scrub_sensitive_url_and_headers(&mut self) {
        // Host credential injection currently writes secrets into URL components
        // and header values. Header names and body payloads are separate
        // caller-controlled data and need an explicit threat-model decision
        // before broadening this carrier scrub scope.
        self.url.zeroize();
        for (_, value) in &mut self.headers {
            value.zeroize();
        }
    }
}

impl ZeroizeOnDrop for NetworkHttpRequest {}

const _: fn(&NetworkHttpRequest) = |request| {
    fn require_zeroize_on_drop<T: ?Sized + ZeroizeOnDrop>(_: &T) {}
    require_zeroize_on_drop(request);
};

/// Transport request after policy, URL, DNS, and private-IP checks succeed.
///
/// URL/header buffers are scrubbed when this carrier drops. Transport code must
/// still hand plaintext to URL parsing and the HTTP client while dispatching.
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

impl Drop for NetworkTransportRequest {
    fn drop(&mut self) {
        self.scrub_sensitive_url_and_headers();
    }
}

impl NetworkTransportRequest {
    fn scrub_sensitive_url_and_headers(&mut self) {
        // Host credential injection currently writes secrets into URL components
        // and header values. Header names and body payloads are separate
        // caller-controlled data and need an explicit threat-model decision
        // before broadening this carrier scrub scope.
        self.url.zeroize();
        for (_, value) in &mut self.headers {
            value.zeroize();
        }
    }
}

impl ZeroizeOnDrop for NetworkTransportRequest {}

const _: fn(&NetworkTransportRequest) = |request| {
    fn require_zeroize_on_drop<T: ?Sized + ZeroizeOnDrop>(_: &T) {}
    require_zeroize_on_drop(request);
};

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

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{InvocationId, NetworkScheme, NetworkTargetPattern, TenantId, UserId};

    #[test]
    fn network_http_request_scrubs_url_and_header_values() {
        let mut request = NetworkHttpRequest {
            scope: sample_scope(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1?token=sk-query-secret".to_string(),
            headers: vec![(
                "authorization".to_string(),
                "Bearer sk-header-secret".to_string(),
            )],
            body: Vec::new(),
            policy: sample_policy(),
            response_body_limit: Some(4096),
            timeout_ms: None,
        };

        request.scrub_sensitive_url_and_headers();

        assert!(request.url.is_empty());
        assert_eq!(request.headers[0].0, "authorization");
        assert!(request.headers[0].1.is_empty());
    }

    #[test]
    fn network_http_request_scrubs_url_with_empty_headers() {
        let mut request = NetworkHttpRequest {
            scope: sample_scope(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1?token=sk-query-secret".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            policy: sample_policy(),
            response_body_limit: Some(4096),
            timeout_ms: None,
        };

        request.scrub_sensitive_url_and_headers();

        assert!(request.url.is_empty());
        assert!(request.headers.is_empty());
    }

    #[test]
    fn network_transport_request_scrubs_url_and_header_values() {
        let mut request = NetworkTransportRequest {
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1?token=sk-query-secret".to_string(),
            headers: vec![(
                "authorization".to_string(),
                "Bearer sk-header-secret".to_string(),
            )],
            body: b"hello".to_vec(),
            resolved_ips: vec![],
            response_body_limit: Some(4096),
            timeout_ms: None,
        };

        request.scrub_sensitive_url_and_headers();

        assert!(request.url.is_empty());
        assert_eq!(request.headers[0].0, "authorization");
        assert!(request.headers[0].1.is_empty());
    }

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant1").unwrap(),
            user_id: UserId::new("user1").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn sample_policy() -> NetworkPolicy {
        NetworkPolicy {
            allowed_targets: vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "api.example.test".to_string(),
                port: Some(443),
            }],
            deny_private_ip_ranges: true,
            max_egress_bytes: Some(4096),
        }
    }
}
