//! Network policy boundary for IronClaw Reborn.
//!
//! This crate evaluates host API [`NetworkPolicy`] values against scoped network
//! requests. It does not perform HTTP I/O, resolve DNS, inject secrets, reserve
//! resources, or emit audit/events. Actual egress enforcement, DNS/redirect
//! revalidation, and response limiting belong at the runtime/network execution
//! boundary.

use std::net::IpAddr;

use async_trait::async_trait;
use ironclaw_host_api::{
    NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTarget, NetworkTargetPattern, ResourceScope,
};
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
}

#[async_trait]
impl NetworkPolicyEnforcer for StaticNetworkPolicyEnforcer {
    async fn authorize(
        &self,
        request: NetworkRequest,
    ) -> Result<NetworkPermit, NetworkPolicyError> {
        if let Some(limit) = self.policy.max_egress_bytes {
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
