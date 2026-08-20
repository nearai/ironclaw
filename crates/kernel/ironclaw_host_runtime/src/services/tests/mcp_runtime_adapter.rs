use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    decision::RuntimeCredentialAuthRequirement,
    dispatch::DispatchError,
    ids::{ExtensionId, VendorId},
    resource::{ResourceEstimate, RuntimeResourceBudget},
    runtime::RuntimeKind,
};
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutionResult, McpExecutor};
use serde_json::json;

use super::*;

#[tokio::test]
async fn mcp_adapter_maps_executor_auth_required_to_dispatch_auth_required() {
    let requirement = RuntimeCredentialAuthRequirement {
        provider: VendorId::new("github").unwrap(),
        setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
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
            requirement: auth_requirement,
        }) => {
            assert_eq!(capability, descriptor.id);
            assert!(auth_requirement.required_secrets.is_empty());
            assert_eq!(auth_requirement.credential_requirements, vec![requirement]);
        }
        other => panic!("expected AuthRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn mcp_adapter_maps_provider_rejection_to_typed_dispatch_rejection() {
    let adapter = McpRuntimeAdapter::from_executor(Arc::new(ProviderRejectedMcpExecutor {
        rejection: ironclaw_mcp::McpProviderRejection {
            diagnostic: ironclaw_host_api::dispatch::ProviderDiagnostic {
                code: Some(ironclaw_host_api::dispatch::ProviderErrorCode::new(
                    "mcp_tool_rejected",
                )),
                message: Some(ironclaw_host_api::dispatch::UntrustedProviderMessage::new(
                    "provider says no",
                )),
                retry_after: None,
            },
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: sample_scope(),
                status: ReservationStatus::Released,
                estimate: ResourceEstimate::default(),
                actual: None,
            },
            usage: ResourceUsage::default(),
        },
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
            input: json!({"query": "provider rejection through adapter"}),
        })
        .await;

    match result {
        Err(DispatchError::Rejected {
            runtime: Some(RuntimeKind::Mcp),
            kind:
                ironclaw_host_api::dispatch::DispatchFailureKind::Runtime(
                    ironclaw_host_api::dispatch::RuntimeDispatchErrorKind::Client,
                ),
            diagnostic: Some(diagnostic),
            ..
        }) => {
            assert_eq!(
                diagnostic.code.as_ref().map(|code| code.as_str()),
                Some("mcp_tool_rejected")
            );
            assert_eq!(
                diagnostic.message.as_ref().map(|message| message.as_str()),
                Some("provider says no")
            );
        }
        other => panic!("expected typed MCP rejection, got {other:?}"),
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

struct ProviderRejectedMcpExecutor {
    rejection: ironclaw_mcp::McpProviderRejection,
}

struct FailingMcpExecutor {
    reason: String,
}

#[async_trait]
impl McpExecutor for FailingMcpExecutor {
    async fn execute_extension_json(
        &self,
        _budget: &dyn RuntimeResourceBudget,
        _request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError> {
        Err(McpError::Client {
            reason: self.reason.clone(),
        })
    }
}

#[async_trait]
impl McpExecutor for ProviderRejectedMcpExecutor {
    async fn execute_extension_json(
        &self,
        _budget: &dyn RuntimeResourceBudget,
        _request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError> {
        Err(McpError::ProviderRejected(Box::new(self.rejection.clone())))
    }
}

#[async_trait]
impl McpExecutor for AuthRequiredMcpExecutor {
    async fn execute_extension_json(
        &self,
        _budget: &dyn RuntimeResourceBudget,
        _request: McpExecutionRequest<'_>,
    ) -> Result<McpExecutionResult, McpError> {
        Err(McpError::AuthRequired {
            required_secrets: Vec::new(),
            credential_requirements: vec![self.requirement.clone()],
        })
    }
}
