use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionPackage, ExtensionRuntime, ManifestSource, McpHttpEndpoint, McpHttpScheme,
    SharedExtensionRegistry,
};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
    RuntimeCredentialInjection, RuntimeCredentialSource, RuntimeHttpEgress,
};
use ironclaw_mcp::{
    McpHostHttpClient, McpHostHttpEgressPlan, McpHostHttpEgressPlanRequest,
    McpHostHttpEgressPlanner, McpRuntime, McpRuntimeConfig, McpRuntimeHttpAdapter,
};

pub(crate) const MCP_RESPONSE_BODY_LIMIT: u64 = 2 * 1024 * 1024;
const MCP_NETWORK_EGRESS_LIMIT: u64 = 2 * 1024 * 1024;
const MCP_TIMEOUT_MS: u32 = 60_000;

pub(crate) fn hosted_http_mcp_runtime(
    registry: Arc<SharedExtensionRegistry>,
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
) -> McpRuntime<
    McpHostHttpClient<McpRuntimeHttpAdapter<Arc<dyn RuntimeHttpEgress>>, RegistryMcpEgressPlanner>,
> {
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(runtime_http_egress),
        RegistryMcpEgressPlanner::new(registry),
    );
    McpRuntime::new(McpRuntimeConfig::default(), client)
}

#[derive(Debug, Clone)]
pub(crate) struct RegistryMcpEgressPlanner {
    registry: Arc<SharedExtensionRegistry>,
}

impl RegistryMcpEgressPlanner {
    pub(crate) fn new(registry: Arc<SharedExtensionRegistry>) -> Self {
        Self { registry }
    }

    fn credential_injections(
        &self,
        provider: &ExtensionId,
        capability_id: &CapabilityId,
        endpoint: &McpEgressEndpoint,
    ) -> Vec<RuntimeCredentialInjection> {
        self.registry
            .snapshot()
            .get_capability(capability_id)
            .filter(|descriptor| &descriptor.provider == provider)
            .map(|descriptor| {
                descriptor
                    .runtime_credentials
                    .iter()
                    .filter(|credential| endpoint.allows_target(&credential.audience))
                    .map(|credential| RuntimeCredentialInjection {
                        handle: credential.handle.clone(),
                        source: RuntimeCredentialSource::StagedObligation {
                            capability_id: capability_id.clone(),
                        },
                        target: credential.target.clone(),
                        required: credential.required,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn provider_endpoint(&self, provider: &ExtensionId) -> Option<McpEgressEndpoint> {
        let registry = self.registry.snapshot();
        registry.get_extension(provider).and_then(http_mcp_endpoint)
    }
}

impl McpHostHttpEgressPlanner for RegistryMcpEgressPlanner {
    fn plan(&self, request: McpHostHttpEgressPlanRequest<'_>) -> McpHostHttpEgressPlan {
        let Some(endpoint) = self.provider_endpoint(request.provider) else {
            return McpHostHttpEgressPlan::default();
        };
        if !mcp_url_allowed(request.url, &endpoint) {
            return McpHostHttpEgressPlan::default();
        }
        let credential_injections =
            self.credential_injections(request.provider, request.capability_id, &endpoint);
        McpHostHttpEgressPlan {
            // Credential-free hosted MCP providers are valid: the manifest may
            // expose a public/unauthenticated server, and host network policy
            // is still enforced below. Missing credentials for providers that
            // should authenticate are a manifest/catalog validation concern,
            // not an egress-planning reason to block the HTTP request.
            // Must match the bundled manifest's network policy
            // (deny_private_ip_ranges: true) or the dispatcher rejects the
            // request.
            network_policy: mcp_network_policy(&endpoint),
            credential_injections,
            response_body_limit: Some(MCP_RESPONSE_BODY_LIMIT),
            timeout_ms: Some(MCP_TIMEOUT_MS),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct McpEgressEndpoint {
    parsed: McpHttpEndpoint,
}

impl McpEgressEndpoint {
    fn parse(url: &str) -> Option<Self> {
        Some(Self {
            parsed: McpHttpEndpoint::parse(url)?,
        })
    }

    fn scheme(&self) -> NetworkScheme {
        match self.parsed.scheme() {
            McpHttpScheme::Http => NetworkScheme::Http,
            McpHttpScheme::Https => NetworkScheme::Https,
        }
    }

    fn is_loopback_http(&self) -> bool {
        self.parsed.is_literal_ipv4_loopback_http()
    }

    fn allows_target(&self, target: &NetworkTargetPattern) -> bool {
        target.scheme == Some(self.scheme())
            && target.host_pattern.eq_ignore_ascii_case(self.parsed.host())
            && target.port == self.parsed.port()
    }

    fn matches_url(&self, url: &str) -> bool {
        self.parsed.matches_url(url)
    }

    fn network_target(&self) -> NetworkTargetPattern {
        NetworkTargetPattern {
            scheme: Some(self.scheme()),
            host_pattern: self.parsed.host().to_string(),
            // NetworkTargetPattern::port = None matches any port. Pin the HTTP
            // default explicitly so a loopback exception never widens from
            // port 80 to every service on the host.
            port: if self.is_loopback_http() {
                Some(self.parsed.port().unwrap_or(80))
            } else {
                self.parsed.port()
            },
        }
    }
}

pub(crate) fn http_mcp_endpoint(package: &ExtensionPackage) -> Option<McpEgressEndpoint> {
    let ExtensionRuntime::Mcp {
        transport,
        command: None,
        args,
        url: Some(url),
    } = &package.manifest.runtime
    else {
        return None;
    };
    if transport != "http" || !args.is_empty() {
        return None;
    }
    let endpoint = McpEgressEndpoint::parse(url)?;
    match package.manifest.source {
        ManifestSource::HostBundled if !endpoint.is_loopback_http() => Some(endpoint),
        ManifestSource::InstalledLocal if endpoint.is_loopback_http() => Some(endpoint),
        ManifestSource::HostBundled
        | ManifestSource::InstalledLocal
        | ManifestSource::RegistryInstalled => None,
    }
}

pub(crate) fn installed_local_mcp_loopback_target(
    package: &ExtensionPackage,
) -> Option<NetworkTargetPattern> {
    (package.manifest.source == ManifestSource::InstalledLocal)
        .then(|| http_mcp_endpoint(package))
        .flatten()
        .filter(|endpoint| endpoint.is_loopback_http())
        .map(|endpoint| endpoint.network_target())
}

/// Returns `true` only when `url` exactly matches the endpoint scheme, host,
/// port, and normalized path. Plaintext HTTP endpoints are parseable only for
/// literal loopback IPs.
fn mcp_url_allowed(url: &str, endpoint: &McpEgressEndpoint) -> bool {
    endpoint.matches_url(url)
}

fn mcp_network_policy(endpoint: &McpEgressEndpoint) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![endpoint.network_target()],
        // The allowlist still pins one literal host and port. Waive the general
        // private-range guard only for an endpoint already proven to be a
        // literal loopback IP; remote providers retain the SSRF guard.
        deny_private_ip_ranges: !endpoint.is_loopback_http(),
        max_egress_bytes: Some(MCP_NETWORK_EGRESS_LIMIT),
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ManifestSource};
    use ironclaw_host_api::{
        CapabilityId, CapabilityProfileSchemaRef, ExtensionId, InvocationId, NetworkMethod,
        NetworkScheme, NetworkTargetPattern, PermissionMode, ProjectId, ResourceScope,
        RuntimeCredentialAccountProviderId, RuntimeCredentialRequirementSource,
        RuntimeCredentialTarget, SecretHandle, TenantId, TrustClass, UserId, VirtualPath,
    };

    use super::*;

    const NOTION_MCP_HOST: &str = "mcp.notion.com";
    const NOTION_MCP_URL: &str = "https://mcp.notion.com/mcp";

    // ── credential projection ──────────────────────────────────────────────

    #[test]
    fn mcp_planner_projects_manifest_runtime_credentials_to_staged_injections() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_notion()));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("notion").unwrap();
        let capability_id = CapabilityId::new("notion.notion-search").unwrap();
        let endpoint = McpEgressEndpoint::parse(NOTION_MCP_URL).unwrap();

        let injections = planner.credential_injections(&provider, &capability_id, &endpoint);

        assert_eq!(injections.len(), 1);
        assert_eq!(
            injections[0].handle,
            SecretHandle::new("mcp_notion_access_token").unwrap()
        );
        assert_eq!(
            injections[0].source,
            RuntimeCredentialSource::StagedObligation { capability_id }
        );
        assert!(matches!(
            injections[0].target,
            RuntimeCredentialTarget::Header { .. }
        ));
    }

    // ── provider scoping ───────────────────────────────────────────────────

    #[test]
    fn planner_denies_unknown_provider() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_notion()));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("not-notion").unwrap();
        let cap = CapabilityId::new("notion.notion-search").unwrap();

        let scope = sample_scope();
        let plan = planner.plan(sample_plan_request(
            &provider,
            &cap,
            "https://mcp.notion.com/mcp",
            &scope,
        ));

        assert!(plan.credential_injections.is_empty());
        assert!(plan.network_policy.allowed_targets.is_empty());
    }

    #[test]
    fn planner_accepts_any_host_bundled_http_mcp_provider() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_provider(
            "fixture",
            "https://fixture.example.com/mcp",
            "fixture.search",
            "fixture_token",
        )));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("fixture").unwrap();
        let cap = CapabilityId::new("fixture.search").unwrap();
        let scope = sample_scope();

        let plan = planner.plan(sample_plan_request(
            &provider,
            &cap,
            "https://fixture.example.com/mcp",
            &scope,
        ));

        assert_eq!(plan.credential_injections.len(), 1);
        assert_eq!(
            plan.network_policy.allowed_targets[0].host_pattern,
            "fixture.example.com"
        );
    }

    #[test]
    fn planner_allows_installed_local_mcp_only_at_literal_loopback_endpoint() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_provider_source(
            "local-mcp",
            "http://127.0.0.2:4321/mcp",
            "local-mcp.search",
            "local_mcp_token",
            ManifestSource::InstalledLocal,
        )));
        let planner = RegistryMcpEgressPlanner::new(Arc::clone(&registry));
        let provider = ExtensionId::new("local-mcp").unwrap();
        let cap = CapabilityId::new("local-mcp.search").unwrap();
        let scope = sample_scope();

        let plan = planner.plan(sample_plan_request(
            &provider,
            &cap,
            "http://127.0.0.2:4321/mcp",
            &scope,
        ));

        assert_eq!(
            plan.network_policy.allowed_targets,
            vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Http),
                host_pattern: "127.0.0.2".to_string(),
                port: Some(4321),
            }]
        );
        assert!(!plan.network_policy.deny_private_ip_ranges);
        assert_eq!(plan.response_body_limit, Some(MCP_RESPONSE_BODY_LIMIT));
        let package = registry
            .snapshot()
            .get_extension(&provider)
            .expect("local provider")
            .clone();
        assert_eq!(
            installed_local_mcp_loopback_target(&package),
            plan.network_policy.allowed_targets.first().cloned()
        );
    }

    #[test]
    fn planner_rejects_installed_local_mcp_for_non_loopback_or_hostname_http() {
        let provider = ExtensionId::new("local-mcp").unwrap();
        let cap = CapabilityId::new("local-mcp.search").unwrap();
        let scope = sample_scope();

        for url in [
            "http://localhost:4321/mcp",
            "http://[::1]:4321/mcp",
            "http://192.168.1.10:4321/mcp",
            "https://example.com/mcp",
        ] {
            let registry = Arc::new(SharedExtensionRegistry::new(registry_with_provider_source(
                "local-mcp",
                url,
                "local-mcp.search",
                "local_mcp_token",
                ManifestSource::InstalledLocal,
            )));
            let planner = RegistryMcpEgressPlanner::new(registry);
            let plan = planner.plan(sample_plan_request(&provider, &cap, url, &scope));

            assert!(
                plan.network_policy.allowed_targets.is_empty(),
                "installed-local endpoint {url} must remain denied"
            );
        }
    }

    #[test]
    fn planner_pins_installed_local_default_http_port() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_provider_source(
            "local-mcp",
            "http://127.0.0.1/mcp",
            "local-mcp.search",
            "local_mcp_token",
            ManifestSource::InstalledLocal,
        )));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("local-mcp").unwrap();
        let capability = CapabilityId::new("local-mcp.search").unwrap();
        let scope = sample_scope();

        let plan = planner.plan(sample_plan_request(
            &provider,
            &capability,
            "http://127.0.0.1/mcp",
            &scope,
        ));

        assert_eq!(plan.network_policy.allowed_targets[0].port, Some(80));
        assert!(!plan.network_policy.deny_private_ip_ranges);
    }

    #[test]
    fn planner_denies_registry_installed_loopback_mcp() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_provider_source(
            "registry-mcp",
            "http://127.0.0.1:4321/mcp",
            "registry-mcp.search",
            "registry_mcp_token",
            ManifestSource::RegistryInstalled,
        )));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("registry-mcp").unwrap();
        let capability = CapabilityId::new("registry-mcp.search").unwrap();
        let scope = sample_scope();

        let plan = planner.plan(sample_plan_request(
            &provider,
            &capability,
            "http://127.0.0.1:4321/mcp",
            &scope,
        ));

        assert!(plan.network_policy.allowed_targets.is_empty());
    }

    #[test]
    fn planner_denies_host_bundled_loopback_http_mcp() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_provider(
            "bundled-mcp",
            "http://127.0.0.1:4321/mcp",
            "bundled-mcp.search",
            "bundled_mcp_token",
        )));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("bundled-mcp").unwrap();
        let capability = CapabilityId::new("bundled-mcp.search").unwrap();
        let scope = sample_scope();

        let plan = planner.plan(sample_plan_request(
            &provider,
            &capability,
            "http://127.0.0.1:4321/mcp",
            &scope,
        ));

        assert!(plan.network_policy.allowed_targets.is_empty());
    }

    #[test]
    fn planner_denies_wrong_host_for_notion_provider() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_notion()));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("notion").unwrap();
        let cap = CapabilityId::new("notion.notion-search").unwrap();
        let scope = sample_scope();

        let plan = planner.plan(sample_plan_request(
            &provider,
            &cap,
            "https://evil.example.com/mcp",
            &scope,
        ));

        assert!(plan.credential_injections.is_empty());
        assert!(plan.network_policy.allowed_targets.is_empty());
    }

    #[test]
    fn planner_denies_http_scheme_for_notion_provider() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_notion()));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("notion").unwrap();
        let cap = CapabilityId::new("notion.notion-search").unwrap();
        let scope = sample_scope();

        let plan = planner.plan(sample_plan_request(
            &provider,
            &cap,
            "http://mcp.notion.com/mcp",
            &scope,
        ));

        assert!(plan.credential_injections.is_empty());
    }

    #[test]
    fn planner_denies_wrong_path_for_notion_provider() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_notion()));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("notion").unwrap();
        let cap = CapabilityId::new("notion.notion-search").unwrap();
        let scope = sample_scope();

        let plan = planner.plan(sample_plan_request(
            &provider,
            &cap,
            "https://mcp.notion.com/other",
            &scope,
        ));

        assert!(plan.credential_injections.is_empty());
    }

    // ── happy path policy ─────────────────────────────────────────────────

    #[test]
    fn planner_emits_locked_policy_for_notion_provider() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_notion()));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("notion").unwrap();
        let cap = CapabilityId::new("notion.notion-search").unwrap();
        let scope = sample_scope();

        let plan = planner.plan(sample_plan_request(
            &provider,
            &cap,
            "https://mcp.notion.com/mcp",
            &scope,
        ));

        assert_eq!(plan.credential_injections.len(), 1);
        assert!(plan.network_policy.deny_private_ip_ranges);
        assert_eq!(plan.network_policy.allowed_targets.len(), 1);
        assert_eq!(
            plan.network_policy.allowed_targets[0].host_pattern,
            NOTION_MCP_HOST.to_string()
        );
        assert_eq!(
            plan.network_policy.allowed_targets[0].scheme,
            Some(NetworkScheme::Https)
        );
        assert_eq!(plan.response_body_limit, Some(MCP_RESPONSE_BODY_LIMIT));
        assert_eq!(plan.timeout_ms, Some(MCP_TIMEOUT_MS));
    }

    // ── URL allowlist unit tests ───────────────────────────────────────────

    #[test]
    fn mcp_url_allowed_accepts_canonical_notion_url() {
        let endpoint = McpEgressEndpoint::parse(NOTION_MCP_URL).unwrap();
        assert!(mcp_url_allowed(NOTION_MCP_URL, &endpoint));
    }

    #[test]
    fn mcp_url_allowed_rejects_http_scheme() {
        let endpoint = McpEgressEndpoint::parse(NOTION_MCP_URL).unwrap();
        assert!(!mcp_url_allowed("http://mcp.notion.com/mcp", &endpoint));
    }

    #[test]
    fn mcp_url_allowed_rejects_wrong_host() {
        let endpoint = McpEgressEndpoint::parse(NOTION_MCP_URL).unwrap();
        assert!(!mcp_url_allowed("https://evil.example.com/mcp", &endpoint));
    }

    #[test]
    fn mcp_url_allowed_rejects_wrong_path() {
        let endpoint = McpEgressEndpoint::parse(NOTION_MCP_URL).unwrap();
        assert!(!mcp_url_allowed("https://mcp.notion.com/other", &endpoint));
    }

    #[test]
    fn mcp_url_allowed_accepts_trailing_slash() {
        let endpoint = McpEgressEndpoint::parse(NOTION_MCP_URL).unwrap();
        assert!(mcp_url_allowed("https://mcp.notion.com/mcp/", &endpoint));
    }

    #[test]
    fn mcp_url_allowed_rejects_extra_url_components() {
        let endpoint = McpEgressEndpoint::parse(NOTION_MCP_URL).unwrap();

        assert!(!mcp_url_allowed(
            "https://mcp.notion.com/mcp?token=shadow",
            &endpoint
        ));
        assert!(!mcp_url_allowed(
            "https://mcp.notion.com/mcp#fragment",
            &endpoint
        ));
        assert!(!mcp_url_allowed(
            "https://user@mcp.notion.com/mcp",
            &endpoint
        ));
    }

    // ── helpers ───────────────────────────────────────────────────────────

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("test-tenant").unwrap(),
            user_id: UserId::new("test-user").unwrap(),
            agent_id: None,
            project_id: Some(ProjectId::new("test-project").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn sample_plan_request<'a>(
        provider: &'a ExtensionId,
        capability_id: &'a CapabilityId,
        url: &'a str,
        scope: &'a ResourceScope,
    ) -> McpHostHttpEgressPlanRequest<'a> {
        McpHostHttpEgressPlanRequest {
            provider,
            capability_id,
            scope,
            transport: "http",
            method: NetworkMethod::Post,
            url,
            headers: &[],
            body: &[],
        }
    }

    fn registry_with_notion() -> ironclaw_extensions::ExtensionRegistry {
        registry_with_provider(
            "notion",
            NOTION_MCP_URL,
            "notion.notion-search",
            "mcp_notion_access_token",
        )
    }

    fn registry_with_provider(
        provider: &str,
        url: &str,
        capability_id: &str,
        credential_handle: &str,
    ) -> ironclaw_extensions::ExtensionRegistry {
        registry_with_provider_source(
            provider,
            url,
            capability_id,
            credential_handle,
            ManifestSource::HostBundled,
        )
    }

    fn registry_with_provider_source(
        provider: &str,
        url: &str,
        capability_id: &str,
        credential_handle: &str,
        source: ManifestSource,
    ) -> ironclaw_extensions::ExtensionRegistry {
        let mut registry = ironclaw_extensions::ExtensionRegistry::new();
        let host = url::Url::parse(url)
            .unwrap()
            .host_str()
            .unwrap()
            .to_string();
        registry
            .insert(
                ExtensionPackage::from_manifest(
                    ExtensionManifest {
                        schema_version: ironclaw_extensions::MANIFEST_SCHEMA_VERSION.to_string(),
                        id: ExtensionId::new(provider).unwrap(),
                        name: provider.to_string(),
                        version: "0.1.0".to_string(),
                        description: "Hosted MCP".to_string(),
                        source,
                        requested_trust: ironclaw_host_api::RequestedTrustClass::ThirdParty,
                        descriptor_trust_default: TrustClass::Sandbox,
                        runtime: ironclaw_extensions::ExtensionRuntime::Mcp {
                            transport: "http".to_string(),
                            command: None,
                            args: Vec::new(),
                            url: Some(url.to_string()),
                        },
                        host_apis: Vec::new(),
                        hooks: Vec::new(),
                        capabilities: vec![ironclaw_extensions::CapabilityManifest {
                            id: CapabilityId::new(capability_id).unwrap(),
                            implements: Vec::new(),
                            description: "Search".to_string(),
                            effects: vec![
                                ironclaw_host_api::EffectKind::DispatchCapability,
                                ironclaw_host_api::EffectKind::Network,
                                ironclaw_host_api::EffectKind::UseSecret,
                            ],
                            default_permission: PermissionMode::Allow,
                            visibility: ironclaw_extensions::CapabilityVisibility::Model,
                            input_schema_ref: CapabilityProfileSchemaRef::new(
                                "schemas/notion/notion-search.input.v1.json",
                            )
                            .unwrap(),
                            output_schema_ref: CapabilityProfileSchemaRef::new(
                                "schemas/notion/notion-search.output.v1.json",
                            )
                            .unwrap(),
                            prompt_doc_ref: None,
                            required_host_ports: Vec::new(),
                            runtime_credentials: vec![
                                ironclaw_host_api::RuntimeCredentialRequirement {
                                    handle: SecretHandle::new(credential_handle).unwrap(),
                                    source:
                                        RuntimeCredentialRequirementSource::ProductAuthAccount {
                                            provider: RuntimeCredentialAccountProviderId::new(
                                                provider,
                                            )
                                            .unwrap(),
                                            setup: Default::default(),
                                        },
                                    provider_scopes: Vec::new(),
                                    audience: NetworkTargetPattern {
                                        scheme: Some(NetworkScheme::Https),
                                        host_pattern: host,
                                        port: None,
                                    },
                                    target: RuntimeCredentialTarget::Header {
                                        name: "authorization".to_string(),
                                        prefix: Some("Bearer ".to_string()),
                                    },
                                    required: true,
                                },
                            ],
                            network_targets: Vec::new(),
                            resource_profile: None,
                        }],
                    },
                    VirtualPath::new(format!("/system/extensions/{provider}")).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        registry
    }
}
