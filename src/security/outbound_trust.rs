use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// Runtime configuration for outbound trust policy evaluation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutboundTrustConfig {
    /// Global kill switch for all outbound trust decisions.
    #[serde(default)]
    pub enabled: bool,

    /// Configured operator-managed trust policies.
    #[serde(default)]
    pub policies: Vec<OutboundTrustPolicy>,
}

/// Execution surface that wants to consume an outbound trust policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundTrustSurface {
    WasmTool,
    WasmChannel,
    McpServer,
}

/// Explicit risk bit granted by a policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundTrustRisk {
    AllowInvalidTls,
    AllowPrivateNetwork,
}

/// Operator-managed outbound trust policy entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundTrustPolicy {
    /// Stable policy identifier referenced by extensions.
    pub id: String,

    /// Human-friendly policy name.
    pub display_name: String,

    /// Optional operator note.
    #[serde(default)]
    pub description: Option<String>,

    /// Whether this policy is active.
    #[serde(default)]
    pub enabled: bool,

    /// Surfaces this policy may be used from.
    #[serde(default)]
    pub allowed_surfaces: Vec<OutboundTrustSurface>,

    /// Explicit risk flags granted by this policy.
    #[serde(default)]
    pub allowed_risks: Vec<OutboundTrustRisk>,

    /// Target tuples the request URL must match.
    #[serde(default)]
    pub targets: Vec<OutboundTrustTarget>,
}

/// Host/port/path tuple matched against a request URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundTrustTarget {
    /// Exact hostname or IP literal.
    pub host: String,

    /// Optional port restriction.
    #[serde(default)]
    pub port: Option<u16>,

    /// Optional path prefix restriction.
    #[serde(default)]
    pub path_prefix: Option<String>,
}

/// Shared request-scoped decision context for outbound trust resolution.
#[derive(Debug, Clone, Copy)]
pub struct OutboundTrustRequestContext<'a> {
    pub surface: OutboundTrustSurface,
    pub extension_name: &'a str,
    pub url: &'a str,
    pub declared_policy_ids: &'a [String],
}

/// Decision returned to runtime consumers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutboundTrustDecision {
    pub matched_policy_id: Option<String>,
    pub allow_invalid_tls: bool,
    pub allow_private_network: bool,
}

/// Shared resolver for operator-managed outbound trust policies.
#[derive(Debug, Clone)]
pub struct OutboundTrustResolver {
    config: OutboundTrustConfig,
}

impl Default for OutboundTrustResolver {
    fn default() -> Self {
        Self::new(OutboundTrustConfig::default())
    }
}

impl OutboundTrustResolver {
    pub fn new(config: OutboundTrustConfig) -> Self {
        Self { config }
    }

    pub fn resolve(&self, ctx: &OutboundTrustRequestContext<'_>) -> OutboundTrustDecision {
        if !self.config.enabled {
            return OutboundTrustDecision::default();
        }

        let target = match NormalizedRequestTarget::parse(ctx.url) {
            Some(target) => target,
            None => return OutboundTrustDecision::default(),
        };

        let mut decision = OutboundTrustDecision::default();

        for policy in &self.config.policies {
            if !policy.enabled
                || !ctx
                    .declared_policy_ids
                    .iter()
                    .any(|declared| declared == &policy.id)
                || !policy.allowed_surfaces.contains(&ctx.surface)
                || !policy
                    .targets
                    .iter()
                    .any(|candidate| target.matches(candidate))
            {
                continue;
            }

            if decision.matched_policy_id.is_none() {
                decision.matched_policy_id = Some(policy.id.clone());
            }

            for risk in &policy.allowed_risks {
                match risk {
                    OutboundTrustRisk::AllowInvalidTls => decision.allow_invalid_tls = true,
                    OutboundTrustRisk::AllowPrivateNetwork => {
                        decision.allow_private_network = true;
                    }
                }
            }
        }

        decision
    }
}

pub(crate) fn is_dangerous_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || (v4.octets()[0] == 169 && v4.octets()[1] == 254)
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
        }
        IpAddr::V6(v6) => {
            let segs = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || (segs[0] & 0xffc0) == 0xfe80
                || (segs[0] & 0xffc0) == 0xfec0
                || (segs[0] & 0xfe00) == 0xfc00
                || (segs[0] == 0x2001 && segs[1] == 0x0db8)
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_dangerous_ip(IpAddr::V4(v4)))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedRequestTarget {
    host: String,
    port: Option<u16>,
    path: String,
}

impl NormalizedRequestTarget {
    fn parse(url: &str) -> Option<Self> {
        let parsed = url::Url::parse(url).ok()?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return None;
        }

        let host = parsed.host_str()?.to_ascii_lowercase();
        Some(Self {
            host,
            port: parsed.port_or_known_default(),
            path: parsed.path().to_string(),
        })
    }

    fn matches(&self, target: &OutboundTrustTarget) -> bool {
        if self.host != target.host.to_ascii_lowercase() {
            return false;
        }

        if let Some(expected_port) = target.port
            && self.port != Some(expected_port)
        {
            return false;
        }

        if let Some(path_prefix) = &target.path_prefix
            && !self.path.starts_with(path_prefix)
        {
            return false;
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::{
        OutboundTrustConfig, OutboundTrustPolicy, OutboundTrustRequestContext,
        OutboundTrustResolver, OutboundTrustRisk, OutboundTrustSurface, OutboundTrustTarget,
    };

    fn policy(
        id: &str,
        surfaces: Vec<OutboundTrustSurface>,
        risks: Vec<OutboundTrustRisk>,
        targets: Vec<OutboundTrustTarget>,
    ) -> OutboundTrustPolicy {
        OutboundTrustPolicy {
            id: id.to_string(),
            display_name: id.to_string(),
            description: None,
            enabled: true,
            allowed_surfaces: surfaces,
            allowed_risks: risks,
            targets,
        }
    }

    fn target(host: &str) -> OutboundTrustTarget {
        OutboundTrustTarget {
            host: host.to_string(),
            port: None,
            path_prefix: None,
        }
    }

    fn resolve(
        resolver: &OutboundTrustResolver,
        surface: OutboundTrustSurface,
        url: &str,
        declared_policy_ids: &[String],
    ) -> super::OutboundTrustDecision {
        resolver.resolve(&OutboundTrustRequestContext {
            surface,
            extension_name: "test-extension",
            url,
            declared_policy_ids,
        })
    }

    #[test]
    fn dangerous_ip_blocks_ipv4_mapped_private_ipv6() {
        let ip: IpAddr = "::ffff:10.0.0.1".parse().unwrap();
        assert!(super::is_dangerous_ip(ip));
    }

    #[test]
    fn dangerous_ip_allows_public_ipv6() {
        let ip: IpAddr = "2606:4700::1111".parse().unwrap();
        assert!(!super::is_dangerous_ip(ip));
    }

    #[test]
    fn matches_exact_host() {
        let resolver = OutboundTrustResolver::new(OutboundTrustConfig {
            enabled: true,
            policies: vec![policy(
                "corp-internal-api",
                vec![OutboundTrustSurface::WasmTool],
                vec![OutboundTrustRisk::AllowInvalidTls],
                vec![target("internal-api.example.test")],
            )],
        });

        let result = resolve(
            &resolver,
            OutboundTrustSurface::WasmTool,
            "https://internal-api.example.test/api/status",
            &["corp-internal-api".to_string()],
        );

        assert_eq!(
            result.matched_policy_id.as_deref(),
            Some("corp-internal-api")
        );
        assert!(result.allow_invalid_tls);
        assert!(!result.allow_private_network);
    }

    #[test]
    fn respects_port_and_path_prefix() {
        let resolver = OutboundTrustResolver::new(OutboundTrustConfig {
            enabled: true,
            policies: vec![OutboundTrustPolicy {
                id: "corp-gateway".to_string(),
                display_name: "corp-gateway".to_string(),
                description: None,
                enabled: true,
                allowed_surfaces: vec![OutboundTrustSurface::McpServer],
                allowed_risks: vec![OutboundTrustRisk::AllowPrivateNetwork],
                targets: vec![OutboundTrustTarget {
                    host: "10.0.0.25".to_string(),
                    port: Some(8443),
                    path_prefix: Some("/rpc".to_string()),
                }],
            }],
        });

        let allowed = resolve(
            &resolver,
            OutboundTrustSurface::McpServer,
            "https://10.0.0.25:8443/rpc/tools/list",
            &["corp-gateway".to_string()],
        );
        assert!(allowed.allow_private_network);

        let wrong_port = resolve(
            &resolver,
            OutboundTrustSurface::McpServer,
            "https://10.0.0.25:443/rpc/tools/list",
            &["corp-gateway".to_string()],
        );
        assert_eq!(wrong_port.matched_policy_id, None);

        let wrong_path = resolve(
            &resolver,
            OutboundTrustSurface::McpServer,
            "https://10.0.0.25:8443/other",
            &["corp-gateway".to_string()],
        );
        assert_eq!(wrong_path.matched_policy_id, None);
    }

    #[test]
    fn requires_declared_policy_id() {
        let resolver = OutboundTrustResolver::new(OutboundTrustConfig {
            enabled: true,
            policies: vec![policy(
                "corp-internal-api",
                vec![OutboundTrustSurface::WasmTool],
                vec![OutboundTrustRisk::AllowInvalidTls],
                vec![target("internal-api.example.test")],
            )],
        });

        let result = resolve(
            &resolver,
            OutboundTrustSurface::WasmTool,
            "https://internal-api.example.test/api/status",
            &[],
        );

        assert_eq!(result.matched_policy_id, None);
        assert!(!result.allow_invalid_tls);
        assert!(!result.allow_private_network);
    }

    #[test]
    fn rejects_surface_mismatch() {
        let resolver = OutboundTrustResolver::new(OutboundTrustConfig {
            enabled: true,
            policies: vec![policy(
                "corp-channel",
                vec![OutboundTrustSurface::WasmChannel],
                vec![OutboundTrustRisk::AllowInvalidTls],
                vec![target("hooks.internal.example")],
            )],
        });

        let result = resolve(
            &resolver,
            OutboundTrustSurface::WasmTool,
            "https://hooks.internal.example/webhook",
            &["corp-channel".to_string()],
        );

        assert_eq!(result.matched_policy_id, None);
    }

    #[test]
    fn disabled_config_or_policy_grants_nothing() {
        let disabled_globally = OutboundTrustResolver::new(OutboundTrustConfig {
            enabled: false,
            policies: vec![policy(
                "corp-internal-api",
                vec![OutboundTrustSurface::WasmTool],
                vec![
                    OutboundTrustRisk::AllowInvalidTls,
                    OutboundTrustRisk::AllowPrivateNetwork,
                ],
                vec![target("10.42.0.15")],
            )],
        });
        let global_result = resolve(
            &disabled_globally,
            OutboundTrustSurface::WasmTool,
            "https://10.42.0.15/api/status",
            &["corp-internal-api".to_string()],
        );
        assert_eq!(global_result.matched_policy_id, None);

        let disabled_policy = OutboundTrustResolver::new(OutboundTrustConfig {
            enabled: true,
            policies: vec![OutboundTrustPolicy {
                enabled: false,
                ..policy(
                    "corp-internal-api",
                    vec![OutboundTrustSurface::WasmTool],
                    vec![OutboundTrustRisk::AllowPrivateNetwork],
                    vec![target("10.42.0.15")],
                )
            }],
        });
        let policy_result = resolve(
            &disabled_policy,
            OutboundTrustSurface::WasmTool,
            "https://10.42.0.15/api/status",
            &["corp-internal-api".to_string()],
        );
        assert_eq!(policy_result.matched_policy_id, None);
    }

    #[test]
    fn unions_risks_across_matching_policies() {
        let resolver = OutboundTrustResolver::new(OutboundTrustConfig {
            enabled: true,
            policies: vec![
                policy(
                    "tls",
                    vec![OutboundTrustSurface::WasmTool],
                    vec![OutboundTrustRisk::AllowInvalidTls],
                    vec![target("internal-api.example.test")],
                ),
                policy(
                    "private",
                    vec![OutboundTrustSurface::WasmTool],
                    vec![OutboundTrustRisk::AllowPrivateNetwork],
                    vec![target("internal-api.example.test")],
                ),
            ],
        });

        let result = resolve(
            &resolver,
            OutboundTrustSurface::WasmTool,
            "https://internal-api.example.test/api/status",
            &["tls".to_string(), "private".to_string()],
        );

        assert_eq!(result.matched_policy_id.as_deref(), Some("tls"));
        assert!(result.allow_invalid_tls);
        assert!(result.allow_private_network);
    }
}
