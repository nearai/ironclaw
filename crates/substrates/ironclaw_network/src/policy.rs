use std::net::IpAddr;

use ironclaw_host_api::{
    action::{NetworkPolicy, NetworkTarget, NetworkTargetPattern},
    resource::ResourceScope,
};
use thiserror::Error;

use crate::types::{NetworkPermit, NetworkRequest};

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
    #[error("invalid network host pattern {pattern:?}: {reason}")]
    InvalidHostPattern { pattern: String, reason: String },
    #[error("invalid network egress limit {raw:?}: {reason}")]
    InvalidEgressLimit { raw: String, reason: String },
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

    pub fn is_invalid_host_pattern(&self) -> bool {
        matches!(self, Self::InvalidHostPattern { .. })
    }

    pub fn is_invalid_egress_limit(&self) -> bool {
        matches!(self, Self::InvalidEgressLimit { .. })
    }
}

/// Parses one operator-supplied egress byte-limit string (e.g. from an env
/// var override) into a positive `u64`. Rejects non-numeric input and zero —
/// `max_egress_bytes: Some(0)` would deny every request with an egress
/// estimate and is never what an operator setting a volume cap means; if
/// they want to disable the cap, unsetting the override (falling back to the
/// caller's own default) is the way to do that, not `0`.
pub fn parse_egress_limit(raw: &str) -> Result<u64, NetworkPolicyError> {
    let trimmed = raw.trim();
    let parsed: u64 = trimmed
        .parse()
        .map_err(|_| NetworkPolicyError::InvalidEgressLimit {
            raw: raw.to_string(),
            reason: "must be a positive integer number of bytes".to_string(),
        })?;
    if parsed == 0 {
        return Err(NetworkPolicyError::InvalidEgressLimit {
            raw: raw.to_string(),
            reason: "must not be zero".to_string(),
        });
    }
    Ok(parsed)
}

/// Parses one operator- or config-supplied hostname string into a validated
/// [`NetworkTargetPattern`] with `scheme: None, port: None`.
///
/// This is the chokepoint for turning untrusted "extra allowed domain"
/// strings into policy the enforcer will actually honor. It is stricter than
/// [`NetworkTargetPattern::validate_declaration`] (`crates/contracts/ironclaw_host_api/
/// src/action.rs`): that method accepts a bare `*` because some
/// host-authored grants — e.g. `CapabilityNetworkProfile::DevWildcard`'s
/// local-dev shell profile — legitimately mean "every host" and are reviewed
/// as such at declaration time. A hostname typed into an env var was never
/// reviewed — `host_matches_pattern` (this module) treats `*` as "match
/// every host", so a typo here silently turns a package-registry allowlist
/// into allow-all egress for the one profile whose entire purpose is holding
/// untrusted code. Reject it instead of trusting it.
///
/// The two validators diverge INTENTIONALLY and this direction must not be
/// collapsed either: `validate_declaration` must not be tightened to match
/// this function's wildcard rejection, since that would break
/// `DevWildcard`'s declared full-access grant. (Extension manifest
/// credential audiences are already stricter than either of these — they
/// reject a wildcard host outright via `ManifestV3Error::WildcardAudienceHost`
/// — so they are not an example of a legitimate bare-`*` consumer either.)
///
/// Accepts: a bare hostname (`example.com`) or a single `*.`-prefixed
/// wildcard label (`*.example.com`). Rejects: empty/whitespace-only input,
/// the bare wildcard `*`, and anything containing characters outside
/// `[A-Za-z0-9.-]` or with an empty/malformed label.
pub fn parse_host_pattern(raw: &str) -> Result<NetworkTargetPattern, NetworkPolicyError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(invalid_host_pattern(raw, "must not be empty"));
    }
    // Mirror `validate_declaration`'s 253-byte cap (the DNS name length
    // limit) so this validator is never laxer than the sibling it claims to
    // be stricter than — see the doc comment above.
    if trimmed.len() > 253 {
        return Err(invalid_host_pattern(raw, "must be at most 253 bytes"));
    }
    if trimmed == "*" {
        return Err(invalid_host_pattern(
            raw,
            "bare `*` would match every host; use a specific hostname or a \
             `*.`-prefixed wildcard label",
        ));
    }

    let label_source = trimmed.strip_prefix("*.").unwrap_or(trimmed);
    let shape_ok = !label_source.is_empty()
        && !label_source.starts_with('.')
        && !label_source.ends_with('.')
        && !label_source.contains("..")
        && label_source
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'));
    if !shape_ok {
        return Err(invalid_host_pattern(
            raw,
            "must be a valid hostname or a `*.`-prefixed wildcard label",
        ));
    }

    Ok(NetworkTargetPattern {
        scheme: None,
        host_pattern: trimmed.to_string(),
        port: None,
    })
}

fn invalid_host_pattern(raw: &str, reason: &str) -> NetworkPolicyError {
    NetworkPolicyError::InvalidHostPattern {
        pattern: raw.to_string(),
        reason: reason.to_string(),
    }
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

    /// Authorizes one scoped network request without performing I/O.
    pub async fn authorize(
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

pub(crate) fn network_policy_allows(policy: &NetworkPolicy, target: &NetworkTarget) -> bool {
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
    host_matches_pattern(&target.host, &pattern.host_pattern)
}

fn host_matches_pattern(host: &str, pattern: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    if pattern == "*" {
        return true;
    }
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

pub(crate) fn is_private_or_loopback_ip(ip: IpAddr) -> bool {
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
