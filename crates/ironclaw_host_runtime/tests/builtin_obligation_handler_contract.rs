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
    RuntimeSecretInjectionStore,
};
use ironclaw_processes::{
    ProcessExecutionError, ProcessExecutionRequest, ProcessExecutionResult, ProcessExecutor,
    ProcessServices,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_secrets::{ExposeSecret, InMemorySecretStore, SecretMaterial, SecretStore};
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
async fn builtin_obligation_handler_emits_metadata_only_audit_after() {
    let audit = Arc::new(InMemoryAuditSink::new());
    let handler = BuiltinObligationHandler::new().with_audit_sink(audit.clone());
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::AuditAfter];
    let dispatch = sample_dispatch(&context.resource_scope, &capability_id, json!({"ok": true}));

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
    let completed = handler
        .complete_dispatch(CapabilityObligationCompletionRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
            dispatch: &dispatch,
        })
        .await
        .unwrap();

    assert_eq!(completed.output, json!({"ok": true}));
    let records = audit.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].stage, AuditStage::After);
    assert_eq!(records[0].action.kind, "capability_invoke");
    assert_eq!(records[0].action.target.as_deref(), Some("echo.say"));
    assert_eq!(
        records[0]
            .result
            .as_ref()
            .and_then(|result| result.output_bytes),
        Some(serde_json::to_vec(&dispatch.output).unwrap().len() as u64)
    );
    let serialized = serde_json::to_string(&records[0]).unwrap();
    assert!(!serialized.contains("raw output"));
    assert!(!serialized.contains("secret"));
}

#[tokio::test]
async fn builtin_obligation_handler_enforces_output_limit_after_dispatch() {
    let handler = BuiltinObligationHandler::new();
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::EnforceOutputLimit { bytes: 8 }];
    let dispatch = sample_dispatch(
        &context.resource_scope,
        &capability_id,
        json!({"message": "this output is too large"}),
    );

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
    let err = handler
        .complete_dispatch(CapabilityObligationCompletionRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
            dispatch: &dispatch,
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityObligationError::Failed {
            kind: CapabilityObligationFailureKind::Output
        }
    ));
}

#[tokio::test]
async fn builtin_obligation_handler_redacts_output_after_dispatch() {
    let handler = BuiltinObligationHandler::new();
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::RedactOutput];
    let leaked = "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let dispatch = sample_dispatch(
        &context.resource_scope,
        &capability_id,
        json!({"authorization": leaked}),
    );

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
    let completed = handler
        .complete_dispatch(CapabilityObligationCompletionRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
            dispatch: &dispatch,
        })
        .await
        .unwrap();

    let serialized = serde_json::to_string(&completed.output).unwrap();
    assert!(serialized.contains("[REDACTED]"));
    assert!(!serialized.contains(leaked));
}

#[tokio::test]
async fn builtin_obligation_handler_rejects_post_output_obligations_for_spawn() {
    let handler = BuiltinObligationHandler::new();
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::RedactOutput, Obligation::AuditAfter];

    let err = handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Spawn,
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
    assert_eq!(
        obligations,
        vec![Obligation::RedactOutput, Obligation::AuditAfter]
    );
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

#[test]
fn network_obligation_policy_store_isolates_agent_scope() {
    let store = NetworkObligationPolicyStore::new();
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let mut agent_a = execution_context(CapabilitySet::default()).resource_scope;
    agent_a.agent_id = Some(AgentId::new("agent-a").unwrap());
    let mut agent_b = agent_a.clone();
    agent_b.agent_id = Some(AgentId::new("agent-b").unwrap());

    store.insert(&agent_a, &capability_id, allowed_network_policy());

    assert!(
        store.take(&agent_b, &capability_id).is_none(),
        "network policies must not cross agent scope"
    );
    assert!(store.take(&agent_a, &capability_id).is_some());
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
async fn builtin_obligation_handler_leases_consumes_and_stages_secret_once() {
    let secret_store = Arc::new(InMemorySecretStore::new());
    let injection_store = Arc::new(RuntimeSecretInjectionStore::new());
    let handler = BuiltinObligationHandler::new()
        .with_secret_store(secret_store.clone())
        .with_secret_injection_store(injection_store.clone());
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let handle = SecretHandle::new("api_token").unwrap();
    secret_store
        .put(
            context.resource_scope.clone(),
            handle.clone(),
            SecretMaterial::from("runtime-secret"),
        )
        .await
        .unwrap();
    let obligations = vec![Obligation::InjectSecretOnce {
        handle: handle.clone(),
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

    let leases = secret_store
        .leases_for_scope(&context.resource_scope)
        .await
        .unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(
        leases[0].status,
        ironclaw_secrets::SecretLeaseStatus::Consumed
    );
    let material = injection_store
        .take(&context.resource_scope, &capability_id, &handle)
        .unwrap()
        .expect("secret material should be staged exactly once");
    assert_eq!(material.expose_secret(), "runtime-secret");
    assert!(
        injection_store
            .take(&context.resource_scope, &capability_id, &handle)
            .unwrap()
            .is_none(),
        "runtime secret injection store must be one-shot"
    );
}

#[test]
fn runtime_secret_injection_store_isolates_agent_and_project_scope() {
    let store = RuntimeSecretInjectionStore::new();
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let handle = SecretHandle::new("api_token").unwrap();
    let mut agent_a = execution_context(CapabilitySet::default()).resource_scope;
    agent_a.agent_id = Some(AgentId::new("agent-a").unwrap());
    agent_a.project_id = Some(ProjectId::new("project-a").unwrap());
    let mut agent_b = agent_a.clone();
    agent_b.agent_id = Some(AgentId::new("agent-b").unwrap());
    let mut project_b = agent_a.clone();
    project_b.project_id = Some(ProjectId::new("project-b").unwrap());

    store
        .insert(
            &agent_a,
            &capability_id,
            &handle,
            SecretMaterial::from("agent-a-project-a"),
        )
        .unwrap();

    assert!(
        store
            .take(&agent_b, &capability_id, &handle)
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .take(&project_b, &capability_id, &handle)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .take(&agent_a, &capability_id, &handle)
            .unwrap()
            .unwrap()
            .expose_secret(),
        "agent-a-project-a"
    );
}

#[tokio::test]
async fn builtin_obligation_handler_fails_closed_without_secret_store() {
    let injection_store = Arc::new(RuntimeSecretInjectionStore::new());
    let handler = BuiltinObligationHandler::new().with_secret_injection_store(injection_store);
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let handle = SecretHandle::new("api_token").unwrap();
    let obligations = vec![Obligation::InjectSecretOnce {
        handle: handle.clone(),
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
            kind: CapabilityObligationFailureKind::Secret
        }
    ));
}

#[tokio::test]
async fn builtin_obligation_handler_fails_closed_when_secret_is_missing() {
    let secret_store = Arc::new(InMemorySecretStore::new());
    let injection_store = Arc::new(RuntimeSecretInjectionStore::new());
    let handler = BuiltinObligationHandler::new()
        .with_secret_store(secret_store)
        .with_secret_injection_store(injection_store.clone());
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let handle = SecretHandle::new("missing_token").unwrap();
    let obligations = vec![Obligation::InjectSecretOnce {
        handle: handle.clone(),
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
            kind: CapabilityObligationFailureKind::Secret
        }
    ));
    assert!(
        injection_store
            .take(&context.resource_scope, &capability_id, &handle)
            .unwrap()
            .is_none(),
        "missing secrets must not stage runtime material"
    );
}

#[tokio::test]
async fn host_runtime_services_can_wire_builtin_secret_obligation_handler() {
    let registry = Arc::new(registry_with_manifest(SCRIPT_MANIFEST));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer = Arc::new(SecretInjectionAuthorizer);
    let process_services = ProcessServices::in_memory();
    let secret_store = Arc::new(InMemorySecretStore::new());
    let services =
        HostRuntimeServices::new(registry, filesystem, governor, authorizer, process_services)
            .with_secret_store(secret_store.clone())
            .with_builtin_obligation_handler();
    let dispatcher = NoopDispatcher;
    let capability_host = services.capability_host(&dispatcher, Arc::new(ImmediateExecutor));
    let context = execution_context(CapabilitySet::default());
    let handle = SecretHandle::new("api_token").unwrap();
    secret_store
        .put(
            context.resource_scope.clone(),
            handle.clone(),
            SecretMaterial::from("service-secret"),
        )
        .await
        .unwrap();

    capability_host
        .spawn_json(CapabilitySpawnRequest {
            context: context.clone(),
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "secret obligation"}),
        })
        .await
        .unwrap();

    let material = services
        .runtime_secret_injections()
        .take(
            &context.resource_scope,
            &CapabilityId::new("echo-script.say").unwrap(),
            &handle,
        )
        .unwrap()
        .expect("shared services should stage consumed secret material");
    assert_eq!(material.expose_secret(), "service-secret");
}

#[tokio::test]
async fn builtin_obligation_handler_keeps_other_runtime_plumbing_obligations_fail_closed() {
    let audit = Arc::new(InMemoryAuditSink::new());
    let handler = BuiltinObligationHandler::new().with_audit_sink(audit.clone());
    let context = execution_context(CapabilitySet::default());
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let obligations = vec![
        Obligation::AuditBefore,
        Obligation::ReserveResources {
            reservation_id: ResourceReservationId::new(),
        },
        Obligation::UseScopedMounts {
            mounts: MountView::default(),
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
    assert_eq!(obligations.len(), 2);
    assert!(
        obligations
            .iter()
            .any(|obligation| matches!(obligation, Obligation::ReserveResources { .. }))
    );
    assert!(
        obligations
            .iter()
            .any(|obligation| matches!(obligation, Obligation::UseScopedMounts { .. }))
    );
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

fn sample_dispatch(
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    output: serde_json::Value,
) -> CapabilityDispatchResult {
    CapabilityDispatchResult {
        capability_id: capability_id.clone(),
        provider: ExtensionId::new("echo").unwrap(),
        runtime: RuntimeKind::Wasm,
        output,
        usage: ResourceUsage::default(),
        receipt: ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: scope.clone(),
            status: ReservationStatus::Reconciled,
            estimate: ResourceEstimate::default(),
            actual: Some(ResourceUsage::default()),
        },
    }
}

fn allowed_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(1024),
    }
}

struct AuditBeforeAuthorizer;

struct SecretInjectionAuthorizer;

#[async_trait]
impl CapabilityDispatchAuthorizer for SecretInjectionAuthorizer {
    async fn authorize_dispatch(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Allow {
            obligations: vec![Obligation::InjectSecretOnce {
                handle: SecretHandle::new("api_token").unwrap(),
            }],
        }
    }

    async fn authorize_spawn(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Allow {
            obligations: vec![Obligation::InjectSecretOnce {
                handle: SecretHandle::new("api_token").unwrap(),
            }],
        }
    }
}

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
    ) -> Result<CapabilityDispatchResult, DispatchError> {
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
        agent_id: None,
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
        agent_id: resource_scope.agent_id.clone(),
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
runner = "sandboxed_process"
command = "echo"

[[capabilities]]
id = "echo-script.say"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
