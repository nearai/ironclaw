use std::sync::Arc;

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

pub(crate) fn nearai_mcp_runtime(
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
) -> Arc<impl McpExecutor> {
    let http = McpRuntimeHttpAdapter::new(runtime_http_egress);
    let client = McpHostHttpClient::new(http, NearAiMcpEgressPlanner);
    Arc::new(McpRuntime::new(McpRuntimeConfig::default(), client))
}

#[derive(Debug, Clone)]
struct NearAiMcpEgressPlanner;

impl McpHostHttpEgressPlanner for NearAiMcpEgressPlanner {
    fn plan(&self, request: McpHostHttpEgressPlanRequest<'_>) -> McpHostHttpEgressPlan {
        if request.provider.as_str() != NEARAI_EXTENSION_ID || !nearai_mcp_url_allowed(request.url)
        {
            return McpHostHttpEgressPlan::default();
        }
        McpHostHttpEgressPlan {
            network_policy: nearai_network_policy(),
            credential_injections: vec![nearai_api_key_injection(request.capability_id)],
            response_body_limit: Some(NEARAI_MCP_RESPONSE_BODY_LIMIT),
            timeout_ms: Some(NEARAI_MCP_TIMEOUT_MS),
        }
    }
}

fn nearai_mcp_url_allowed(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .and_then(|url| {
            (url.scheme() == "https")
                .then(|| url.host_str().map(|host| host.to_ascii_lowercase()))?
        })
        .is_some_and(|host| host == "near.ai" || host.ends_with(".near.ai"))
}

fn nearai_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "*.near.ai".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(NEARAI_MCP_NETWORK_EGRESS_LIMIT),
    }
}

fn nearai_api_key_injection(capability_id: &CapabilityId) -> RuntimeCredentialInjection {
    RuntimeCredentialInjection {
        handle: nearai_api_key_handle(),
        source: RuntimeCredentialSource::StagedObligation {
            capability_id: capability_id.clone(),
        },
        target: RuntimeCredentialTarget::Header {
            name: "authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        },
        required: true,
    }
}

fn nearai_api_key_handle() -> SecretHandle {
    SecretHandle::new(NEARAI_API_KEY_SECRET_HANDLE).expect("valid NEAR AI secret handle")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, InvocationId, ProjectId, ResourceScope, TenantId, UserId};

    #[test]
    fn planner_allows_only_nearai_https_targets() {
        let planner = NearAiMcpEgressPlanner;
        let provider = ironclaw_host_api::ExtensionId::new("nearai").unwrap();
        let capability_id = CapabilityId::new("nearai.search").unwrap();
        let scope = scope();
        let allowed_url = "https://cloud-api.near.ai/mcp";
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
            nearai_network_policy().allowed_targets
        );
        assert!(
            planner
                .plan(denied)
                .network_policy
                .allowed_targets
                .is_empty()
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
