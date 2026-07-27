use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    DispatchError, ExtensionId, ResourceEstimate, RuntimeCredentialAuthRequirement, RuntimeKind,
    VendorId,
};
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutionResult, McpExecutor};
use serde_json::json;

use super::*;

#[tokio::test]
async fn mcp_adapter_maps_executor_auth_required_to_dispatch_auth_required() {
    let requirement = RuntimeCredentialAuthRequirement {
        provider: VendorId::new("github").unwrap(),
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
        .dispatch_json(RuntimeLaneRequest {
            run_id: None,
            origin: None,
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

#[tokio::test]
async fn mcp_adapter_preserves_executor_failure_cause() {
    // Regression (Phase 1): an MCP dispatch failure's raw cause — including
    // path/JSON delimiters — must ride the model-visible-cause channel so the
    // model-visible Diagnostic downstream keeps it instead of collapsing to a
    // bare failure category.
    let raw = "MCP client failed at /tmp/{socket}";
    let adapter = McpRuntimeAdapter::from_executor(Arc::new(FailingMcpExecutor {
        reason: raw.to_string(),
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
        .dispatch_json(RuntimeLaneRequest {
            run_id: None,
            origin: None,
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
            input: json!({"query": "fail through adapter"}),
        })
        .await;

    match result {
        Err(DispatchError::Mcp {
            model_visible_cause,
            ..
        }) => {
            let summary = model_visible_cause.expect("MCP cause should be retained");
            assert!(summary.contains(raw), "unexpected cause: {summary}");
        }
        other => panic!("expected MCP dispatch failure, got {other:?}"),
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

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
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

struct FailingMcpExecutor {
    reason: String,
}

#[async_trait]
impl McpExecutor for FailingMcpExecutor {
    async fn execute_extension_json(
        &self,
        _governor: &dyn ResourceGovernor,
        _request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError> {
        Err(McpError::Client {
            reason: self.reason.clone(),
        })
    }
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
