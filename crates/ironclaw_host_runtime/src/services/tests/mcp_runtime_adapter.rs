use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    DispatchError, ExtensionId, ResourceEstimate, RuntimeCredentialAccountProviderId,
    RuntimeCredentialAuthRequirement, RuntimeKind,
};
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutionResult, McpExecutor};
use serde_json::json;

use super::*;

#[tokio::test]
async fn mcp_adapter_maps_executor_auth_required_to_dispatch_auth_required() {
    let requirement = RuntimeCredentialAuthRequirement {
        provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
        setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
            scopes: vec!["repo".to_string()],
        },
        requester_extension: ExtensionId::new("mcp").unwrap(),
        provider_scopes: vec!["repo".to_string()],
    };
    let adapter = McpRuntimeAdapter::from_executor(Arc::new(AuthRequiredMcpExecutor {
        requirement: requirement.clone(),
    }));
    let descriptor = test_descriptor(RuntimeKind::Mcp, Vec::new());
    let filesystem = DiskFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let package = test_package(MCP_MANIFEST, "test");
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::DirectLogged,
        SecretMode::ScrubbedEnv,
    );

    let result = adapter
        .dispatch_json(RuntimeAdapterRequest {
            run_id: None,
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope: sample_scope(),
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({"query": "auth through adapter"}),
        })
        .await;

    match result {
        Err(DispatchError::AuthRequired {
            capability,
            required_secrets,
            credential_requirements,
        }) => {
            assert_eq!(capability, descriptor.id);
            assert!(required_secrets.is_empty());
            assert_eq!(credential_requirements, vec![requirement]);
        }
        other => panic!("expected AuthRequired, got {other:?}"),
    }
}

const MCP_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "test"
name = "Test MCP"
version = "0.1.0"
description = "MCP adapter test extension"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.example.test/rpc"

[[capabilities]]
id = "test.capability"
description = "Search through MCP"
effects = ["network"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/test-mcp/search.input.v1.json"
output_schema_ref = "schemas/test-mcp/search.output.v1.json"
"#;

struct AuthRequiredMcpExecutor {
    requirement: RuntimeCredentialAuthRequirement,
}

#[async_trait]
impl McpExecutor for AuthRequiredMcpExecutor {
    async fn execute_extension_json(
        &self,
        _governor: &dyn ResourceGovernor,
        _request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError> {
        Err(McpError::AuthRequired {
            required_secrets: Vec::new(),
            credential_requirements: vec![self.requirement.clone()],
        })
    }
}
