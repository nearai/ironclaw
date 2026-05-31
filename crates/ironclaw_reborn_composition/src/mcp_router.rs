/// Provider-keyed MCP executor router.
///
/// `HostRuntimeServices::with_mcp_runtime` is a single-slot setter. When
/// multiple hosted MCP runtimes coexist (e.g. the registry-driven Notion
/// adapter and the NEAR AI bespoke adapter), they cannot both be registered
/// directly. This router owns the slot and dispatches by
/// `request.package.id.as_str()` (the extension provider id).
///
/// An unknown provider returns [`McpError::Client`] with reason
/// `"request_denied"` — the same opaque error produced by url/capability
/// validation elsewhere in the MCP stack.
use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutionResult, McpExecutor};
use ironclaw_resources::ResourceGovernor;

const REQUEST_DENIED: &str = "request_denied";

/// Routes MCP execution requests to per-provider [`McpExecutor`]s by
/// `request.package.id`.
#[derive(Default)]
pub(crate) struct McpExecutorRouter {
    by_provider: HashMap<String, Arc<dyn McpExecutor>>,
}

impl McpExecutorRouter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Register an executor for `provider`. Overwrites any previous executor
    /// for the same provider id.
    pub(crate) fn insert(&mut self, provider: impl Into<String>, executor: Arc<dyn McpExecutor>) {
        self.by_provider.insert(provider.into(), executor);
    }

    /// Returns `true` when no executors have been registered.
    #[allow(dead_code)] // used in factory.rs after rebase onto reborn-integration
    pub(crate) fn is_empty(&self) -> bool {
        self.by_provider.is_empty()
    }
}

#[async_trait]
impl McpExecutor for McpExecutorRouter {
    async fn execute_extension_json(
        &self,
        governor: &dyn ResourceGovernor,
        request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError> {
        let provider = request.package.id.as_str();
        match self.by_provider.get(provider) {
            Some(executor) => executor.execute_extension_json(governor, request).await,
            None => Err(McpError::Client {
                reason: REQUEST_DENIED.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ManifestSource};
    use ironclaw_host_api::{
        CapabilityId, CapabilityProfileSchemaRef, ExtensionId, InvocationId, PermissionMode,
        ProjectId, ResourceEstimate, ResourceScope, TenantId, TrustClass, UserId, VirtualPath,
    };
    use ironclaw_mcp::{
        McpError, McpExecutionRequest, McpExecutionResult, McpExecutor, McpInvocation,
    };
    use ironclaw_resources::{InMemoryResourceGovernor, ResourceGovernor};

    use super::*;

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

    fn stub_package(provider: &str) -> ExtensionPackage {
        ExtensionPackage::from_manifest(
            ExtensionManifest {
                schema_version: ironclaw_extensions::MANIFEST_SCHEMA_VERSION.to_string(),
                id: ExtensionId::new(provider).unwrap(),
                name: provider.to_string(),
                version: "0.1.0".to_string(),
                description: String::new(),
                source: ManifestSource::HostBundled,
                requested_trust: ironclaw_host_api::RequestedTrustClass::ThirdParty,
                descriptor_trust_default: TrustClass::Sandbox,
                runtime: ironclaw_extensions::ExtensionRuntime::Mcp {
                    transport: "http".to_string(),
                    command: None,
                    args: Vec::new(),
                    url: Some(format!("https://{provider}.example.com/mcp")),
                },
                host_apis: Vec::new(),
                capabilities: vec![ironclaw_extensions::CapabilityManifest {
                    id: CapabilityId::new(format!("{provider}.search")).unwrap(),
                    implements: Vec::new(),
                    description: String::new(),
                    effects: Vec::new(),
                    default_permission: PermissionMode::Ask,
                    visibility: ironclaw_extensions::CapabilityVisibility::Model,
                    input_schema_ref: CapabilityProfileSchemaRef::new(
                        "schemas/search.input.v1.json",
                    )
                    .unwrap(),
                    output_schema_ref: CapabilityProfileSchemaRef::new(
                        "schemas/search.output.v1.json",
                    )
                    .unwrap(),
                    prompt_doc_ref: None,
                    required_host_ports: Vec::new(),
                    runtime_credentials: Vec::new(),
                    resource_profile: None,
                }],
            },
            VirtualPath::new(format!("/system/extensions/{provider}")).unwrap(),
        )
        .unwrap()
    }

    fn stub_request(package: &ExtensionPackage) -> McpExecutionRequest<'_> {
        McpExecutionRequest {
            package,
            capability_id: &package.manifest.capabilities[0].id,
            scope: sample_scope(),
            estimate: ResourceEstimate::default(),
            resource_reservation: None,
            invocation: McpInvocation {
                input: serde_json::json!({}),
            },
        }
    }

    /// Executor that records calls and returns a provider-tagged `McpError::Client`
    /// error. Returning `Ok(McpExecutionResult)` requires constructing
    /// `ResourceReceipt` + `McpCapabilityResult` infrastructure types; using a
    /// recognisable `Err` is sufficient to assert routing correctness.
    struct RecordingExecutor {
        provider: String,
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingExecutor {
        fn new(provider: &str) -> Arc<Self> {
            Arc::new(Self {
                provider: provider.to_string(),
                calls: std::sync::Mutex::new(Vec::new()),
            })
        }

        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl McpExecutor for RecordingExecutor {
        async fn execute_extension_json(
            &self,
            _governor: &dyn ResourceGovernor,
            request: McpExecutionRequest<'_>,
        ) -> Result<McpExecutionResult, McpError> {
            self.calls
                .lock()
                .unwrap()
                .push(request.capability_id.as_str().to_string());
            // Return a provider-tagged error so the caller can assert routing.
            Err(McpError::Client {
                reason: format!("reached:{}", self.provider),
            })
        }
    }

    // ── tests ─────────────────────────────────────────────────────────────

    #[test]
    fn empty_router_reports_is_empty() {
        let router = McpExecutorRouter::new();
        assert!(router.is_empty());
    }

    #[test]
    fn router_not_empty_after_insert() {
        let mut router = McpExecutorRouter::new();
        router.insert("notion", RecordingExecutor::new("notion"));
        assert!(!router.is_empty());
    }

    #[tokio::test]
    async fn router_dispatches_to_registered_provider() {
        let notion_exec = RecordingExecutor::new("notion");
        let mut router = McpExecutorRouter::new();
        router.insert("notion", Arc::clone(&notion_exec) as Arc<dyn McpExecutor>);

        let pkg = stub_package("notion");
        let err = router
            .execute_extension_json(&InMemoryResourceGovernor::new(), stub_request(&pkg))
            .await
            .expect_err("recording executor always errors");

        // Routing reached the notion executor (provider-tagged reason).
        assert!(
            matches!(&err, McpError::Client { reason } if reason == "reached:notion"),
            "expected reached:notion, got: {err}"
        );
        assert_eq!(notion_exec.call_count(), 1);
    }

    #[tokio::test]
    async fn router_denies_unregistered_provider() {
        let router = McpExecutorRouter::new(); // empty

        let pkg = stub_package("unknown-provider");
        let err = router
            .execute_extension_json(&InMemoryResourceGovernor::new(), stub_request(&pkg))
            .await
            .expect_err("unknown provider should be denied");

        assert!(
            matches!(&err, McpError::Client { reason } if reason == REQUEST_DENIED),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn router_routes_to_correct_executor_among_multiple() {
        let notion_exec = RecordingExecutor::new("notion");
        let nearai_exec = RecordingExecutor::new("nearai");
        let mut router = McpExecutorRouter::new();
        router.insert("notion", Arc::clone(&notion_exec) as Arc<dyn McpExecutor>);
        router.insert("nearai", Arc::clone(&nearai_exec) as Arc<dyn McpExecutor>);

        let notion_pkg = stub_package("notion");
        let nearai_pkg = stub_package("nearai");

        // Each executor should receive exactly one call.
        let _ = router
            .execute_extension_json(&InMemoryResourceGovernor::new(), stub_request(&notion_pkg))
            .await;
        let _ = router
            .execute_extension_json(&InMemoryResourceGovernor::new(), stub_request(&nearai_pkg))
            .await;

        assert_eq!(notion_exec.call_count(), 1);
        assert_eq!(nearai_exec.call_count(), 1);
    }
}
