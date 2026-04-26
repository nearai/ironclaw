use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_events::InMemoryAuditSink;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    BuiltinObligationHandler, HostRuntimeServices, NetworkObligationPolicyStore,
};
use ironclaw_processes::{
    ProcessExecutionError, ProcessExecutionRequest, ProcessExecutionResult, ProcessExecutor,
    ProcessServices,
};
use ironclaw_resources::InMemoryResourceGovernor;
use serde_json::json;

#[tokio::test]
async fn builtin_obligation_handler_emits_metadata_only_audit_before() {
    let audit = Arc::new(InMemoryAuditSink::new());
    let handler = BuiltinObligationHandler::new().with_audit_sink(audit.clone());
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::AuditBefore];

    handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .unwrap();

    let records = audit.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].stage, AuditStage::Before);
    assert_eq!(records[0].tenant_id, context.tenant_id);
    assert_eq!(records[0].user_id, context.user_id);
    assert_eq!(records[0].invocation_id, context.invocation_id);
    assert_eq!(records[0].action.kind, "capability_invoke");
    assert_eq!(records[0].action.target.as_deref(), Some("echo.say"));
    assert_eq!(records[0].decision.kind, "obligation_satisfied");
    assert_eq!(
        records[0]
            .result
            .as_ref()
            .and_then(|result| result.status.as_deref()),
        Some("audit_before")
    );
    let serialized = serde_json::to_string(&records[0]).unwrap();
    assert!(!serialized.contains("raw input"));
    assert!(!serialized.contains("secret"));
}

#[tokio::test]
async fn builtin_obligation_handler_stores_network_policy_for_runtime_handoff() {
    let policy_store = Arc::new(NetworkObligationPolicyStore::new());
    let handler = BuiltinObligationHandler::new().with_network_policy_store(policy_store.clone());
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::ApplyNetworkPolicy {
        policy: allowed_network_policy(),
    }];

    handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .unwrap();

    assert!(
        policy_store
            .take(&context.resource_scope, &capability_id)
            .is_some(),
        "accepted network obligations must be handed to runtime adapters"
    );
}

#[tokio::test]
async fn builtin_obligation_handler_fails_closed_without_network_policy_store() {
    let handler = BuiltinObligationHandler::new();
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::ApplyNetworkPolicy {
        policy: allowed_network_policy(),
    }];

    let err = handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityObligationError::Failed {
            kind: CapabilityObligationFailureKind::Network
        }
    ));
}

#[tokio::test]
async fn builtin_obligation_handler_rejects_invalid_network_policy_before_dispatch() {
    let handler = BuiltinObligationHandler::new();
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::ApplyNetworkPolicy {
        policy: NetworkPolicy::default(),
    }];

    let err = handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityObligationError::Failed {
            kind: CapabilityObligationFailureKind::Network
        }
    ));
}

#[tokio::test]
async fn builtin_obligation_handler_rejects_non_public_literal_network_targets() {
    let handler = BuiltinObligationHandler::new();
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::ApplyNetworkPolicy {
        policy: NetworkPolicy {
            allowed_targets: vec![
                NetworkTargetPattern::new(Some(NetworkScheme::Https), "100.64.0.1", None).unwrap(),
            ],
            deny_private_ip_ranges: true,
            max_egress_bytes: Some(1024),
        },
    }];

    let err = handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityObligationError::Failed {
            kind: CapabilityObligationFailureKind::Network
        }
    ));
}

#[tokio::test]
async fn builtin_obligation_handler_keeps_runtime_plumbing_obligations_fail_closed() {
    let audit = Arc::new(InMemoryAuditSink::new());
    let handler = BuiltinObligationHandler::new().with_audit_sink(audit.clone());
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let obligations = vec![
        Obligation::AuditBefore,
        Obligation::InjectSecretOnce {
            handle: SecretHandle::new("api_token").unwrap(),
        },
        Obligation::AuditAfter,
        Obligation::RedactOutput,
        Obligation::EnforceOutputLimit { bytes: 1024 },
    ];

    let err = handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .unwrap_err();

    let CapabilityObligationError::Unsupported { obligations } = err else {
        panic!("expected unsupported obligations");
    };
    assert_eq!(obligations.len(), 4);
    assert!(audit.records().is_empty());
}

#[tokio::test]
async fn host_runtime_services_can_install_builtin_obligation_handler() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer = Arc::new(AuditBeforeAuthorizer);
    let process_services = ProcessServices::in_memory();
    let audit = Arc::new(InMemoryAuditSink::new());
    let services =
        HostRuntimeServices::new(registry, filesystem, governor, authorizer, process_services)
            .with_audit_sink(audit.clone())
            .with_builtin_obligation_handler();
    let dispatcher = NoopDispatcher;
    let capability_host = services.capability_host(&dispatcher, Arc::new(ImmediateExecutor));
    let context = execution_context(CapabilitySet::default());

    capability_host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "obligated"}),
        })
        .await
        .unwrap();

    let records = audit.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].action.kind, "capability_spawn");
    assert_eq!(records[0].action.target.as_deref(), Some("echo-script.say"));
}

fn allowed_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![
            NetworkTargetPattern::new(Some(NetworkScheme::Https), "api.example.test", None)
                .unwrap(),
        ],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(1024),
    }
}

struct AuditBeforeAuthorizer;

#[async_trait]
impl CapabilityDispatchAuthorizer for AuditBeforeAuthorizer {
    async fn authorize_dispatch(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Allow {
            obligations: vec![Obligation::AuditBefore],
        }
    }

    async fn authorize_spawn(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Allow {
            obligations: vec![Obligation::AuditBefore],
        }
    }
}

struct ImmediateExecutor;

#[async_trait]
impl ProcessExecutor for ImmediateExecutor {
    async fn execute(
        &self,
        _request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        Ok(ProcessExecutionResult {
            output: json!({"ok": true}),
        })
    }
}

struct NoopDispatcher;

#[async_trait]
impl CapabilityDispatcher for NoopDispatcher {
    async fn dispatch_json(
        &self,
        _request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, CapabilityDispatchError> {
        panic!("test process executor should not dispatch")
    }
}

fn registry_with_manifest(manifest: &str) -> ExtensionRegistry {
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    let package = ExtensionPackage::from_manifest(manifest, root).unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn execution_context(grants: CapabilitySet) -> ExecutionContext {
    let invocation_id = InvocationId::new();
    let resource_scope = ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::Sandbox,
        grants,
        mounts: MountView::default(),
        resource_scope,
    }
}

const SCRIPT_MANIFEST: &str = r#"
id = "echo-script"
name = "Echo Script"
version = "0.1.0"
description = "Echo script demo extension"
trust = "sandbox"

[runtime]
kind = "script"
backend = "docker"
image = "alpine:latest"
command = "sh"
args = ["-c", "cat"]

[[capabilities]]
id = "echo-script.say"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
