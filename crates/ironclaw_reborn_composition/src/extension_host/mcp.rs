use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionPackage, ExtensionRuntime, ManifestSource, SharedExtensionRegistry,
};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
    RuntimeCredentialInjection, RuntimeCredentialSource, RuntimeHttpEgress,
};
use ironclaw_mcp::{
    McpHostHttpClient, McpHostHttpEgressPlan, McpHostHttpEgressPlanRequest,
    McpHostHttpEgressPlanner, McpRequestAuthority, McpRuntime, McpRuntimeConfig,
    McpRuntimeHttpAdapter,
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
        endpoint: &HostedMcpEndpoint,
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

    fn provider_endpoint(&self, provider: &ExtensionId) -> Option<HostedMcpEndpoint> {
        let registry = self.registry.snapshot();
        registry
            .get_extension(provider)
            .and_then(hosted_http_mcp_endpoint)
    }
}

impl McpHostHttpEgressPlanner for RegistryMcpEgressPlanner {
    fn plan(&self, request: McpHostHttpEgressPlanRequest<'_>) -> McpHostHttpEgressPlan {
        let Some(endpoint) = self.provider_endpoint(request.provider) else {
            return McpHostHttpEgressPlan::default();
        };
        if !hosted_mcp_url_allowed(request.url, &endpoint) {
            return McpHostHttpEgressPlan::default();
        }
        let credential_injections = match request.authority {
            McpRequestAuthority::Capability(capability_id) => {
                self.credential_injections(request.provider, capability_id, &endpoint)
            }
            McpRequestAuthority::ProviderDiscovery { .. } => Vec::new(),
        };
        McpHostHttpEgressPlan {
            // Credential-free hosted MCP providers are valid: the manifest may
            // expose a public/unauthenticated server, and host network policy
            // is still enforced below. Missing credentials for providers that
            // should authenticate are a manifest/catalog validation concern,
            // not an egress-planning reason to block the HTTP request.
            // Must match the bundled manifest's network policy
            // (deny_private_ip_ranges: true) or the dispatcher rejects the
            // request.
            network_policy: hosted_mcp_network_policy(&endpoint),
            credential_injections,
            response_body_limit: Some(MCP_RESPONSE_BODY_LIMIT),
            timeout_ms: Some(MCP_TIMEOUT_MS),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostedMcpEndpoint {
    host_pattern: String,
    port: Option<u16>,
    path: String,
}

impl HostedMcpEndpoint {
    pub(crate) fn parse(url: &str) -> Option<Self> {
        let parsed = url::Url::parse(url).ok()?;
        if parsed.scheme() != "https"
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return None;
        }
        Some(Self {
            host_pattern: parsed.host_str()?.to_ascii_lowercase(),
            port: parsed.port(),
            path: normalize_mcp_path(parsed.path()),
        })
    }

    fn allows_target(&self, target: &NetworkTargetPattern) -> bool {
        target.scheme == Some(NetworkScheme::Https)
            && target.host_pattern.eq_ignore_ascii_case(&self.host_pattern)
            && target.port == self.port
    }

    fn matches_url(&self, url: &str) -> bool {
        Self::parse(url).is_some_and(|request_endpoint| request_endpoint == *self)
    }
}

/// Hosted-egress-eligible sources: `HostBundled` (first-party) and
/// `UserRegistered` (T2, guarded by `CapabilitiesForbiddenForRegisteredSource`
/// so a registered manifest can never carry a capability to dispatch here).
/// The discovery twin gate (`hosted_http_mcp_url`, hosted_mcp_discovery.rs)
/// stays closed for `UserRegistered` until T3 lands live-discovery
/// capabilities — do not widen it here to "fix" the asymmetry.
pub(crate) fn hosted_http_mcp_endpoint(package: &ExtensionPackage) -> Option<HostedMcpEndpoint> {
    if !matches!(
        package.manifest.source,
        ManifestSource::HostBundled | ManifestSource::UserRegistered { .. }
    ) {
        return None;
    }
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
    HostedMcpEndpoint::parse(url)
}

/// Returns `true` only when `url` has scheme `https`, a host that
/// case-insensitively matches `endpoint.host_pattern`, and a path that
/// (ignoring trailing slashes) matches `endpoint.path`.
fn hosted_mcp_url_allowed(url: &str, endpoint: &HostedMcpEndpoint) -> bool {
    endpoint.matches_url(url)
}

fn normalize_mcp_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn hosted_mcp_network_policy(endpoint: &HostedMcpEndpoint) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: endpoint.host_pattern.clone(),
            port: endpoint.port,
        }],
        // Matches the bundled manifest's deny_private_ip_ranges default.
        // Dispatcher would reject anyway, but the plan must agree.
        deny_private_ip_ranges: true,
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
        let endpoint = HostedMcpEndpoint::parse(NOTION_MCP_URL).unwrap();

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
            &McpRequestAuthority::Capability(cap.clone()),
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
            &McpRequestAuthority::Capability(cap.clone()),
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
    fn planner_denies_wrong_host_for_notion_provider() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_notion()));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("notion").unwrap();
        let cap = CapabilityId::new("notion.notion-search").unwrap();
        let scope = sample_scope();

        let plan = planner.plan(sample_plan_request(
            &provider,
            &McpRequestAuthority::Capability(cap.clone()),
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
            &McpRequestAuthority::Capability(cap.clone()),
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
            &McpRequestAuthority::Capability(cap.clone()),
            "https://mcp.notion.com/other",
            &scope,
        ));

        assert!(plan.credential_injections.is_empty());
    }

    // ── UserRegistered egress arm (T2) ──────────────────────────────────────

    #[test]
    fn planner_accepts_user_registered_provider_to_its_registered_url() {
        // Egress arm parity: a UserRegistered provider must be dispatchable to
        // exactly its registered URL, same as HostBundled.
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_provider_source(
            "user-notion",
            NOTION_MCP_URL,
            "user-notion.notion-search",
            "mcp_user_notion_access_token",
            ManifestSource::UserRegistered {
                owner: UserId::new("owner-1").unwrap(),
            },
        )));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("user-notion").unwrap();
        let cap = CapabilityId::new("user-notion.notion-search").unwrap();
        let scope = sample_scope();

        let plan = planner.plan(sample_plan_request(
            &provider,
            &McpRequestAuthority::Capability(cap.clone()),
            NOTION_MCP_URL,
            &scope,
        ));

        assert_eq!(plan.credential_injections.len(), 1);
        assert_eq!(plan.network_policy.allowed_targets.len(), 1);
        assert_eq!(
            plan.network_policy.allowed_targets[0].host_pattern,
            NOTION_MCP_HOST
        );
    }

    #[test]
    fn provider_discovery_keeps_locked_policy_without_capability_credentials() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_provider_source(
            "user-notion",
            NOTION_MCP_URL,
            "user-notion.notion-search",
            "mcp_user_notion_access_token",
            ManifestSource::UserRegistered {
                owner: UserId::new("owner-1").unwrap(),
            },
        )));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("user-notion").unwrap();
        let scope = sample_scope();

        let plan = planner.plan(sample_plan_request(
            &provider,
            &McpRequestAuthority::ProviderDiscovery {
                network_policy_authority: CapabilityId::new("builtin.extension_activate").unwrap(),
            },
            NOTION_MCP_URL,
            &scope,
        ));

        assert!(plan.credential_injections.is_empty());
        assert!(plan.network_policy.deny_private_ip_ranges);
        assert_eq!(plan.network_policy.allowed_targets.len(), 1);
        assert_eq!(plan.response_body_limit, Some(MCP_RESPONSE_BODY_LIMIT));
        assert_eq!(plan.timeout_ms, Some(MCP_TIMEOUT_MS));
    }

    #[test]
    fn hosted_http_mcp_endpoint_excludes_installed_local_and_registry_installed() {
        // Scope-creep guard for the `!=` -> `matches!` swap: only HostBundled
        // and UserRegistered are hosted-egress-eligible. Other sources must
        // keep returning None.
        for source in [
            ManifestSource::InstalledLocal,
            ManifestSource::RegistryInstalled,
        ] {
            let registry = registry_with_provider_source(
                "notion",
                NOTION_MCP_URL,
                "notion.notion-search",
                "mcp_notion_access_token",
                source.clone(),
            );
            let package = registry
                .get_extension(&ExtensionId::new("notion").unwrap())
                .unwrap();

            assert!(
                hosted_http_mcp_endpoint(package).is_none(),
                "expected None for source {source:?}"
            );
        }
    }

    #[test]
    fn user_registered_provider_gets_locked_policy_parity_with_host_bundled() {
        let registry = Arc::new(SharedExtensionRegistry::new(registry_with_provider_source(
            "user-notion",
            NOTION_MCP_URL,
            "user-notion.notion-search",
            "mcp_user_notion_access_token",
            ManifestSource::UserRegistered {
                owner: UserId::new("owner-1").unwrap(),
            },
        )));
        let planner = RegistryMcpEgressPlanner::new(registry);
        let provider = ExtensionId::new("user-notion").unwrap();
        let cap = CapabilityId::new("user-notion.notion-search").unwrap();
        let scope = sample_scope();

        let plan = planner.plan(sample_plan_request(
            &provider,
            &McpRequestAuthority::Capability(cap.clone()),
            NOTION_MCP_URL,
            &scope,
        ));

        assert!(plan.network_policy.deny_private_ip_ranges);
        assert_eq!(
            plan.network_policy.max_egress_bytes,
            Some(MCP_NETWORK_EGRESS_LIMIT)
        );
        assert_eq!(plan.network_policy.allowed_targets.len(), 1);
        assert_eq!(
            plan.network_policy.allowed_targets[0].scheme,
            Some(NetworkScheme::Https)
        );
    }

    #[test]
    fn user_registered_provider_gets_same_credential_injection_as_host_bundled() {
        // Credential parity: a UserRegistered provider must receive exactly
        // the injections a HostBundled provider with the same descriptor
        // would receive — no more, no less.
        let scope = sample_scope();
        let cap = CapabilityId::new("notion.notion-search").unwrap();

        let host_bundled_registry = Arc::new(SharedExtensionRegistry::new(registry_with_notion()));
        let host_bundled_planner = RegistryMcpEgressPlanner::new(host_bundled_registry);
        let host_bundled_provider = ExtensionId::new("notion").unwrap();
        let host_bundled_plan = host_bundled_planner.plan(sample_plan_request(
            &host_bundled_provider,
            &McpRequestAuthority::Capability(cap.clone()),
            NOTION_MCP_URL,
            &scope,
        ));

        let user_registered_registry =
            Arc::new(SharedExtensionRegistry::new(registry_with_provider_source(
                "notion",
                NOTION_MCP_URL,
                "notion.notion-search",
                "mcp_notion_access_token",
                ManifestSource::UserRegistered {
                    owner: UserId::new("owner-1").unwrap(),
                },
            )));
        let user_registered_planner = RegistryMcpEgressPlanner::new(user_registered_registry);
        let user_registered_plan = user_registered_planner.plan(sample_plan_request(
            &host_bundled_provider,
            &McpRequestAuthority::Capability(cap.clone()),
            NOTION_MCP_URL,
            &scope,
        ));

        assert_eq!(
            host_bundled_plan.credential_injections.len(),
            user_registered_plan.credential_injections.len()
        );
        assert_eq!(
            host_bundled_plan.credential_injections[0].handle,
            user_registered_plan.credential_injections[0].handle
        );
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
            &McpRequestAuthority::Capability(cap.clone()),
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
    fn hosted_mcp_url_allowed_accepts_canonical_notion_url() {
        let endpoint = HostedMcpEndpoint::parse(NOTION_MCP_URL).unwrap();
        assert!(hosted_mcp_url_allowed(NOTION_MCP_URL, &endpoint));
    }

    #[test]
    fn hosted_mcp_url_allowed_rejects_http_scheme() {
        let endpoint = HostedMcpEndpoint::parse(NOTION_MCP_URL).unwrap();
        assert!(!hosted_mcp_url_allowed(
            "http://mcp.notion.com/mcp",
            &endpoint
        ));
    }

    #[test]
    fn hosted_mcp_url_allowed_rejects_wrong_host() {
        let endpoint = HostedMcpEndpoint::parse(NOTION_MCP_URL).unwrap();
        assert!(!hosted_mcp_url_allowed(
            "https://evil.example.com/mcp",
            &endpoint
        ));
    }

    #[test]
    fn hosted_mcp_url_allowed_rejects_wrong_path() {
        let endpoint = HostedMcpEndpoint::parse(NOTION_MCP_URL).unwrap();
        assert!(!hosted_mcp_url_allowed(
            "https://mcp.notion.com/other",
            &endpoint
        ));
    }

    #[test]
    fn hosted_mcp_url_allowed_accepts_trailing_slash() {
        let endpoint = HostedMcpEndpoint::parse(NOTION_MCP_URL).unwrap();
        assert!(hosted_mcp_url_allowed(
            "https://mcp.notion.com/mcp/",
            &endpoint
        ));
    }

    #[test]
    fn hosted_mcp_url_allowed_rejects_extra_url_components() {
        let endpoint = HostedMcpEndpoint::parse(NOTION_MCP_URL).unwrap();

        assert!(!hosted_mcp_url_allowed(
            "https://mcp.notion.com/mcp?token=shadow",
            &endpoint
        ));
        assert!(!hosted_mcp_url_allowed(
            "https://mcp.notion.com/mcp#fragment",
            &endpoint
        ));
        assert!(!hosted_mcp_url_allowed(
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
        authority: &'a McpRequestAuthority,
        url: &'a str,
        scope: &'a ResourceScope,
    ) -> McpHostHttpEgressPlanRequest<'a> {
        McpHostHttpEgressPlanRequest {
            provider,
            authority,
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

    /// Same fixture as [`registry_with_provider`] but with a caller-supplied
    /// [`ManifestSource`] — used to pin egress-arm parity between
    /// `HostBundled` and `UserRegistered` providers.
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
