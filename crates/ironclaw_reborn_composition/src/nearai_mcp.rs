use std::{net::IpAddr, sync::Arc};

use ironclaw_host_api::{
    CapabilityId, NetworkPolicy, NetworkScheme, NetworkTargetPattern, RuntimeCredentialInjection,
    RuntimeCredentialSource, RuntimeCredentialTarget, RuntimeHttpEgress, SecretHandle,
};
use ironclaw_mcp::{
    McpExecutor, McpHostHttpClient, McpHostHttpEgressPlan, McpHostHttpEgressPlanRequest,
    McpHostHttpEgressPlanner, McpRuntime, McpRuntimeConfig, McpRuntimeHttpAdapter,
};

const NEARAI_EXTENSION_ID: &str = "nearai";
const NEARAI_API_KEY_SECRET_HANDLE: &str = "llm_nearai_api_key";
const NEARAI_MCP_TIMEOUT_MS: u32 = 60_000;
const NEARAI_MCP_RESPONSE_BODY_LIMIT: u64 = 2 * 1024 * 1024;
const NEARAI_MCP_NETWORK_EGRESS_LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NearAiMcpEndpoint {
    pub(crate) url: String,
    pub(crate) host_pattern: String,
    pub(crate) port: Option<u16>,
    deny_private_ip_ranges: bool,
}

pub(crate) fn nearai_mcp_endpoint_from_env() -> Result<NearAiMcpEndpoint, String> {
    let configured_base = std::env::var("NEARAI_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    nearai_mcp_endpoint_from_base(configured_base.as_deref())
}

pub(crate) fn nearai_mcp_endpoint_from_base(
    configured_base: Option<&str>,
) -> Result<NearAiMcpEndpoint, String> {
    let base = configured_base.unwrap_or("https://private.near.ai");
    let mut url = url::Url::parse(base)
        .map_err(|error| format!("NEARAI_BASE_URL must be an absolute URL: {error}"))?;
    if url.scheme() != "https" {
        return Err("NEARAI_BASE_URL must use https".to_string());
    }
    if url.username() != "" || url.password().is_some() {
        return Err("NEARAI_BASE_URL must not include userinfo".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("NEARAI_BASE_URL must not include query or fragment components".to_string());
    }

    let host = url
        .host_str()
        .ok_or_else(|| "NEARAI_BASE_URL must include a host".to_string())?
        .to_ascii_lowercase();
    let ip = host.parse::<IpAddr>().ok();
    if ip.is_some_and(is_forbidden_endpoint_ip) {
        return Err("NEARAI_BASE_URL host is not allowed".to_string());
    }
    if matches!(ip, Some(IpAddr::V6(_))) {
        return Err("NEARAI_BASE_URL IPv6 hosts are not supported yet".to_string());
    }

    let mut path = url.path().trim_end_matches('/').to_string();
    if path.eq_ignore_ascii_case("/v1") {
        path = String::new();
    }
    if path.is_empty() {
        url.set_path("/mcp");
    } else if !path.eq_ignore_ascii_case("/mcp") {
        url.set_path(&format!("{path}/mcp"));
    } else {
        url.set_path("/mcp");
    }

    Ok(NearAiMcpEndpoint {
        url: url.to_string(),
        host_pattern: host,
        port: url.port(),
        deny_private_ip_ranges: !ip.is_some_and(is_private_or_loopback_ip),
    })
}

pub(crate) fn nearai_mcp_runtime(
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    endpoint: NearAiMcpEndpoint,
) -> Arc<impl McpExecutor> {
    let http = McpRuntimeHttpAdapter::new(runtime_http_egress);
    let client = McpHostHttpClient::new(http, NearAiMcpEgressPlanner { endpoint });
    Arc::new(McpRuntime::new(McpRuntimeConfig::default(), client))
}

#[derive(Debug, Clone)]
struct NearAiMcpEgressPlanner {
    endpoint: NearAiMcpEndpoint,
}

impl McpHostHttpEgressPlanner for NearAiMcpEgressPlanner {
    fn plan(&self, request: McpHostHttpEgressPlanRequest<'_>) -> McpHostHttpEgressPlan {
        // This is a narrow NEAR AI MCP adapter. Do not grow this into a generic
        // product-auth-to-MCP planner. Account selection and typed AuthRequired
        // recovery happen upstream: the bundled manifest declares the credential
        // with `source = product_auth_account(provider = "nearai")`, so the
        // authorization layer emits `Obligation::InjectCredentialAccountOnce`
        // before this planner runs. The obligation handler resolves the
        // configured nearai product-auth account via
        // `RuntimeCredentialAccountResolver`, stages the access secret into
        // `RuntimeSecretInjectionStore` under the `llm_nearai_api_key` slot, and
        // this planner only references that slot via the `StagedObligation`
        // source. Closes the MCP slice of nearai/ironclaw#4176.
        if request.provider.as_str() != NEARAI_EXTENSION_ID
            || !nearai_mcp_url_allowed(request.url, &self.endpoint)
        {
            return McpHostHttpEgressPlan::default();
        }
        let Some(credential_injection) = nearai_api_key_injection(request.capability_id) else {
            return McpHostHttpEgressPlan::default();
        };
        McpHostHttpEgressPlan {
            // Mirror the staged ApplyNetworkPolicy grant's deny_private_ip_ranges
            // default. The bundled manifest's network policy always denies private
            // IPs, so the planner must not accept endpoint-derived policies that
            // permit them — dispatch would reject the request anyway.
            network_policy: nearai_network_policy(
                &self.endpoint,
                /*deny_private_ip_ranges=*/ true,
            ),
            credential_injections: vec![credential_injection],
            response_body_limit: Some(NEARAI_MCP_RESPONSE_BODY_LIMIT),
            timeout_ms: Some(NEARAI_MCP_TIMEOUT_MS),
        }
    }
}

fn nearai_mcp_url_allowed(url: &str, endpoint: &NearAiMcpEndpoint) -> bool {
    url::Url::parse(url)
        .ok()
        .and_then(|url| {
            if url.scheme() != "https" || url.port() != endpoint.port {
                return None;
            }
            url.host_str().map(|host| host.to_ascii_lowercase())
        })
        .is_some_and(|host| host == endpoint.host_pattern)
}

fn nearai_network_policy(
    endpoint: &NearAiMcpEndpoint,
    deny_private_ip_ranges: bool,
) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: endpoint.host_pattern.clone(),
            port: endpoint.port,
        }],
        deny_private_ip_ranges,
        max_egress_bytes: Some(NEARAI_MCP_NETWORK_EGRESS_LIMIT),
    }
}

fn nearai_api_key_injection(capability_id: &CapabilityId) -> Option<RuntimeCredentialInjection> {
    Some(RuntimeCredentialInjection {
        handle: nearai_api_key_handle()?,
        source: RuntimeCredentialSource::StagedObligation {
            capability_id: capability_id.clone(),
        },
        target: RuntimeCredentialTarget::Header {
            name: "authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        },
        required: true,
    })
}

fn nearai_api_key_handle() -> Option<SecretHandle> {
    SecretHandle::new(NEARAI_API_KEY_SECRET_HANDLE).ok()
}

fn is_private_or_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local(),
    }
}

fn is_forbidden_endpoint_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_multicast()
                || ip.octets()[0] == 0
        }
        IpAddr::V6(ip) => {
            ip.is_unspecified()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
                || is_documentation_v6(ip)
        }
    }
}

fn is_documentation_v6(ip: std::net::Ipv6Addr) -> bool {
    let segments = ip.segments();
    segments[0] == 0x2001 && segments[1] == 0x0db8
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, InvocationId, ProjectId, ResourceScope, TenantId, UserId};

    #[test]
    fn planner_allows_only_nearai_https_targets() {
        let endpoint = nearai_mcp_endpoint_from_base(None).unwrap();
        let planner = NearAiMcpEgressPlanner {
            endpoint: endpoint.clone(),
        };
        let provider = ironclaw_host_api::ExtensionId::new("nearai").unwrap();
        let capability_id = CapabilityId::new("nearai.search").unwrap();
        let scope = scope();
        let allowed_url = "https://private.near.ai/mcp";
        let denied_url = "https://attacker.example/mcp";
        let allowed = McpHostHttpEgressPlanRequest {
            provider: &provider,
            capability_id: &capability_id,
            scope: &scope,
            transport: "http",
            method: ironclaw_host_api::NetworkMethod::Post,
            url: allowed_url,
            headers: &[],
            body: &[],
        };
        let denied = McpHostHttpEgressPlanRequest {
            url: denied_url,
            ..allowed
        };

        assert_eq!(
            planner.plan(allowed).network_policy.allowed_targets,
            nearai_network_policy(&endpoint, endpoint.deny_private_ip_ranges).allowed_targets
        );
        assert!(
            planner
                .plan(denied)
                .network_policy
                .allowed_targets
                .is_empty()
        );
    }

    #[test]
    fn endpoint_validation_normalizes_custom_https_base() {
        let endpoint = nearai_mcp_endpoint_from_base(Some("https://search.example.test/v1/"))
            .expect("custom endpoint");

        assert_eq!(endpoint.url, "https://search.example.test/mcp");
        assert_eq!(endpoint.host_pattern, "search.example.test");
        assert_eq!(endpoint.port, None);
        assert!(endpoint.deny_private_ip_ranges);
    }

    #[test]
    fn endpoint_validation_rejects_http_and_forbidden_ips() {
        assert!(nearai_mcp_endpoint_from_base(Some("http://search.example.test")).is_err());
        assert!(nearai_mcp_endpoint_from_base(Some("https://169.254.169.254")).is_err());
        assert!(nearai_mcp_endpoint_from_base(Some("https://224.0.0.1")).is_err());
    }

    #[test]
    fn endpoint_validation_allows_private_loopback_https_targets() {
        let private =
            nearai_mcp_endpoint_from_base(Some("https://10.0.0.12:8443")).expect("private IP");
        let loopback =
            nearai_mcp_endpoint_from_base(Some("https://127.0.0.1")).expect("loopback IP");

        assert_eq!(private.host_pattern, "10.0.0.12");
        assert_eq!(private.port, Some(8443));
        assert!(!private.deny_private_ip_ranges);
        assert!(!loopback.deny_private_ip_ranges);
    }

    #[test]
    fn planner_rejects_wrong_provider_and_http_scheme() {
        let endpoint = nearai_mcp_endpoint_from_base(Some("https://search.example.test"))
            .expect("custom endpoint");
        let planner = NearAiMcpEgressPlanner { endpoint };
        let nearai_provider = ironclaw_host_api::ExtensionId::new("nearai").unwrap();
        let other_provider = ironclaw_host_api::ExtensionId::new("other").unwrap();
        let capability_id = CapabilityId::new("nearai.search").unwrap();
        let scope = scope();
        let allowed = McpHostHttpEgressPlanRequest {
            provider: &nearai_provider,
            capability_id: &capability_id,
            scope: &scope,
            transport: "http",
            method: ironclaw_host_api::NetworkMethod::Post,
            url: "https://search.example.test/mcp",
            headers: &[],
            body: &[],
        };

        assert_eq!(planner.plan(allowed).credential_injections.len(), 1);
        assert!(
            planner
                .plan(McpHostHttpEgressPlanRequest {
                    provider: &other_provider,
                    ..allowed
                })
                .network_policy
                .allowed_targets
                .is_empty()
        );
        assert!(
            planner
                .plan(McpHostHttpEgressPlanRequest {
                    url: "http://search.example.test/mcp",
                    ..allowed
                })
                .network_policy
                .allowed_targets
                .is_empty()
        );
    }

    #[test]
    fn planner_denies_nearai_url_for_wrong_provider() {
        let endpoint = nearai_mcp_endpoint_from_base(None).unwrap();
        let planner = NearAiMcpEgressPlanner {
            endpoint: endpoint.clone(),
        };
        let other_provider = ironclaw_host_api::ExtensionId::new("not-nearai").unwrap();
        let capability_id = CapabilityId::new("nearai.search").unwrap();
        let scope = scope();
        let request = McpHostHttpEgressPlanRequest {
            provider: &other_provider,
            capability_id: &capability_id,
            scope: &scope,
            transport: "http",
            method: ironclaw_host_api::NetworkMethod::Post,
            url: "https://private.near.ai/mcp",
            headers: &[],
            body: &[],
        };
        assert!(
            planner
                .plan(request)
                .network_policy
                .allowed_targets
                .is_empty(),
            "wrong provider must produce empty plan"
        );
    }

    #[test]
    fn planner_denies_http_nearai_url() {
        let endpoint = nearai_mcp_endpoint_from_base(None).unwrap();
        let planner = NearAiMcpEgressPlanner {
            endpoint: endpoint.clone(),
        };
        let provider = ironclaw_host_api::ExtensionId::new("nearai").unwrap();
        let capability_id = CapabilityId::new("nearai.search").unwrap();
        let scope = scope();
        let request = McpHostHttpEgressPlanRequest {
            provider: &provider,
            capability_id: &capability_id,
            scope: &scope,
            transport: "http",
            method: ironclaw_host_api::NetworkMethod::Post,
            url: "http://private.near.ai/mcp",
            headers: &[],
            body: &[],
        };
        assert!(
            planner
                .plan(request)
                .network_policy
                .allowed_targets
                .is_empty(),
            "http scheme must produce empty plan"
        );
    }

    fn scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("user-a").unwrap(),
            agent_id: Some(AgentId::new("agent-a").unwrap()),
            project_id: Some(ProjectId::new("project-a").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }
}
