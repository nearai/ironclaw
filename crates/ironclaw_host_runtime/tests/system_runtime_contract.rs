use std::{
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::Utc;
use futures_util::FutureExt;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_events::{
    AuditSink, EventError, InMemoryAuditSink, InMemoryEventSink, RuntimeEventKind,
};
use ironclaw_extensions::{
    CapabilityManifest, ExtensionManifest, ExtensionPackage, ExtensionRegistry, ExtensionRuntime,
};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CapabilitySurfacePolicy, CapabilitySurfaceVersion, HostRuntime, HostRuntimeError,
    HostRuntimeServices, ProductionWiringComponent, ProductionWiringConfig,
    ProductionWiringIssueKind, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeFailureKind, SurfaceKind, SystemCapabilityError, SystemCapabilityHandler,
    SystemCapabilityInvocationRequest, SystemCapabilityRegistry, SystemCapabilityRequest,
    SystemCapabilityResult, SystemInvocationAuthority, SystemInvocationAuthorityVerifier,
    SystemOperationId, VisibleCapabilityRequest,
};
use ironclaw_resources::{InMemoryResourceGovernor, ResourceAccount, ResourceTally};
use ironclaw_run_state::{InMemoryRunStateStore, RunStateStore, RunStatus};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use serde_json::{Value, json};

#[tokio::test]
async fn system_host_invokes_system_handler_through_capability_host() {
    let handler = Arc::new(RecordingSystemHandler::new(json!({"via":"system"})));
    let system_registry =
        SystemCapabilityRegistry::new().with_handler(capability_id(), Arc::clone(&handler));
    let events = InMemoryEventSink::new();
    let audits = InMemoryAuditSink::new();
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let system_host = HostRuntimeServices::new(
        Arc::new(system_extension_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::clone(&governor),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_system_capabilities(Arc::new(system_registry))
    .with_system_authority_verifier(Arc::new(AllowSystemAuthorityVerifier))
    .with_trust_policy(Arc::new(system_trust_policy()))
    .with_run_state(Arc::clone(&run_state))
    .with_event_sink(Arc::new(events.clone()))
    .with_audit_sink(Arc::new(audits.clone()))
    .system_host_for_local_testing()
    .expect("system host requires system registry, verifier, and audit sink");
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate {
        output_bytes: Some(1024),
        ..ResourceEstimate::default()
    };
    let input = json!({"repair":"state"});
    let authority = system_authority("repair-op-1", &scope);

    let outcome = system_host
        .invoke_system_capability(SystemCapabilityInvocationRequest::new(
            authority.clone(),
            context,
            capability_id(),
            estimate.clone(),
            input.clone(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed system invocation, got {outcome:?}");
    };
    assert_eq!(completed.capability_id, capability_id());
    assert_eq!(completed.output, json!({"via":"system"}));

    let recorded = handler.take_request();
    assert_eq!(recorded.capability_id, capability_id());
    assert_eq!(recorded.scope, scope);
    assert_eq!(recorded.estimate, estimate);
    assert_eq!(recorded.input, input);
    assert_eq!(recorded.authority.operation_id(), authority.operation_id());

    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(
        governor.reserved_for(&ResourceAccount::tenant(scope.tenant_id.clone())),
        ResourceTally::default()
    );
    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ],
    );
    let records = audits.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].stage, AuditStage::After);
    assert_eq!(records[0].action.kind, "system_capability");
    assert_eq!(
        records[0].action.target.as_deref(),
        Some("system.repair:repair-op-1")
    );
    assert_eq!(
        records[0].decision.actor,
        Some(Principal::System(
            SystemServiceId::new("scheduler").unwrap()
        ))
    );
    assert!(records[0].result.as_ref().unwrap().success);
}

#[tokio::test]
async fn public_host_runtime_rejects_system_capability_even_with_normal_grant() {
    let handler = Arc::new(RecordingSystemHandler::new(json!({"via":"system"})));
    let system_registry =
        SystemCapabilityRegistry::new().with_handler(capability_id(), Arc::clone(&handler));
    let runtime = HostRuntimeServices::new(
        Arc::new(system_extension_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_system_capabilities(Arc::new(system_registry))
    .with_system_authority_verifier(Arc::new(AllowSystemAuthorityVerifier))
    .with_trust_policy(Arc::new(system_trust_policy()))
    .host_runtime_for_local_testing();

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context(CapabilitySet {
                grants: vec![dispatch_grant()],
            }),
            capability_id(),
            ResourceEstimate::default(),
            json!({"repair":"state"}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected public system invocation to fail closed, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
    assert!(handler.take_requests().is_empty());
}

#[tokio::test]
async fn visible_capabilities_omits_system_capabilities_even_when_policy_allows_all() {
    let runtime = HostRuntimeServices::new(
        Arc::new(system_extension_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_system_capabilities(Arc::new(SystemCapabilityRegistry::new().with_handler(
        capability_id(),
        Arc::new(RecordingSystemHandler::new(json!(1))),
    )))
    .with_system_authority_verifier(Arc::new(AllowSystemAuthorityVerifier))
    .with_trust_policy(Arc::new(system_trust_policy()))
    .host_runtime_for_local_testing();
    let mut provider_trust = std::collections::BTreeMap::new();
    provider_trust.insert(provider_id(), trust_decision());

    let surface = runtime
        .visible_capabilities(
            VisibleCapabilityRequest::new(
                execution_context(CapabilitySet {
                    grants: vec![dispatch_grant()],
                }),
                SurfaceKind::new("model").unwrap(),
            )
            .with_policy(CapabilitySurfacePolicy::allow_all())
            .with_provider_trust(provider_trust),
        )
        .await
        .unwrap();

    assert!(surface.capabilities.is_empty());
}

#[tokio::test]
async fn production_wiring_requires_system_registry_coverage_and_authority_verifier() {
    let services = HostRuntimeServices::new(
        Arc::new(system_extension_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_system_capabilities(Arc::new(SystemCapabilityRegistry::new()));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([RuntimeKind::System]))
        .expect_err("system production wiring requires handlers and authority verifier");

    assert!(report.contains(
        ProductionWiringComponent::SystemRuntime,
        ProductionWiringIssueKind::Missing
    ));
    assert!(report.contains(
        ProductionWiringComponent::SystemAuthorityVerifier,
        ProductionWiringIssueKind::Missing
    ));
}

#[tokio::test]
async fn system_first_party_registries_do_not_satisfy_each_other() {
    let services = HostRuntimeServices::new(
        Arc::new(system_extension_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        ironclaw_host_runtime::FirstPartyCapabilityRegistry::new()
            .with_handler(capability_id(), Arc::new(NoopFirstPartyHandler)),
    ))
    .with_system_authority_verifier(Arc::new(AllowSystemAuthorityVerifier));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([RuntimeKind::System]))
        .expect_err("first-party handlers must not satisfy system runtime coverage");

    assert!(report.contains(
        ProductionWiringComponent::SystemRuntime,
        ProductionWiringIssueKind::Missing
    ));
}

#[tokio::test]
async fn system_handler_error_reconciles_reported_usage_after_side_effect() {
    let handler = Arc::new(FailingSystemHandler::new(ResourceUsage {
        network_egress_bytes: 55,
        ..ResourceUsage::default()
    }));
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let system_host = system_host_with_handler(Arc::clone(&handler), Arc::clone(&governor));
    let context = execution_context(CapabilitySet::default());
    let account = ResourceAccount::tenant(context.resource_scope.tenant_id.clone());

    let authority = system_authority("repair-op-usage", &context.resource_scope);
    let outcome = system_host
        .invoke_system_capability(SystemCapabilityInvocationRequest::new(
            authority,
            context,
            capability_id(),
            ResourceEstimate {
                network_egress_bytes: Some(100),
                ..ResourceEstimate::default()
            },
            json!({"repair":"state"}),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected failed system invocation, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Backend);
    assert_eq!(governor.usage_for(&account).network_egress_bytes, 55);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn system_handler_panic_fails_closed_and_releases_reservation() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let system_host =
        system_host_with_handler(Arc::new(PanickingSystemHandler), Arc::clone(&governor));
    let context = execution_context(CapabilitySet::default());
    let account = ResourceAccount::tenant(context.resource_scope.tenant_id.clone());

    let authority = system_authority("repair-op-panic", &context.resource_scope);
    let outcome = AssertUnwindSafe(system_host.invoke_system_capability(
        SystemCapabilityInvocationRequest::new(
            authority,
            context,
            capability_id(),
            ResourceEstimate {
                network_egress_bytes: Some(100),
                ..ResourceEstimate::default()
            },
            json!({"repair":"state"}),
        ),
    ))
    .catch_unwind()
    .await
    .expect("system handler panic must be translated into stable failed outcome")
    .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected handler panic to fail closed, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Backend);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn system_authority_denial_fails_before_handler_side_effects() {
    let handler = Arc::new(RecordingSystemHandler::new(json!(1)));
    let registry =
        SystemCapabilityRegistry::new().with_handler(capability_id(), Arc::clone(&handler));
    let system_host = HostRuntimeServices::new(
        Arc::new(system_extension_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_system_capabilities(Arc::new(registry))
    .with_system_authority_verifier(Arc::new(DenySystemAuthorityVerifier))
    .with_trust_policy(Arc::new(system_trust_policy()))
    .with_audit_sink(Arc::new(InMemoryAuditSink::new()))
    .system_host_for_local_testing()
    .unwrap();

    let context = execution_context(CapabilitySet::default());
    let authority = system_authority("repair-op-denied", &context.resource_scope);
    let outcome = system_host
        .invoke_system_capability(SystemCapabilityInvocationRequest::new(
            authority,
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"repair":"state"}),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected authority denial failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
    assert!(handler.take_requests().is_empty());
}

#[tokio::test]
async fn system_authorizer_require_approval_fails_closed_before_handler_side_effects() {
    let handler = Arc::new(RecordingSystemHandler::new(json!(1)));
    let registry =
        SystemCapabilityRegistry::new().with_handler(capability_id(), Arc::clone(&handler));
    let system_host = HostRuntimeServices::new(
        Arc::new(system_extension_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(RequireApprovalAuthorizer),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_system_capabilities(Arc::new(registry))
    .with_system_authority_verifier(Arc::new(AllowSystemAuthorityVerifier))
    .with_trust_policy(Arc::new(system_trust_policy()))
    .with_audit_sink(Arc::new(InMemoryAuditSink::new()))
    .system_host_for_local_testing()
    .unwrap();

    let context = execution_context(CapabilitySet::default());
    let authority = system_authority("repair-op-approval", &context.resource_scope);
    let outcome = system_host
        .invoke_system_capability(SystemCapabilityInvocationRequest::new(
            authority,
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"repair":"state"}),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected require-approval failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
    assert!(handler.take_requests().is_empty());
}

#[tokio::test]
async fn system_audit_failure_fails_closed() {
    let handler = Arc::new(RecordingSystemHandler::new(json!(1)));
    let registry =
        SystemCapabilityRegistry::new().with_handler(capability_id(), Arc::clone(&handler));
    let system_host = HostRuntimeServices::new(
        Arc::new(system_extension_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_system_capabilities(Arc::new(registry))
    .with_system_authority_verifier(Arc::new(AllowSystemAuthorityVerifier))
    .with_trust_policy(Arc::new(system_trust_policy()))
    .with_audit_sink(Arc::new(FailingAuditSink))
    .system_host_for_local_testing()
    .unwrap();
    let context = execution_context(CapabilitySet::default());
    let authority = system_authority("repair-op-audit", &context.resource_scope);

    let error = system_host
        .invoke_system_capability(SystemCapabilityInvocationRequest::new(
            authority,
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"repair":"state"}),
        ))
        .await
        .expect_err("system audit failure must fail closed");

    assert_eq!(
        error,
        HostRuntimeError::unavailable("system audit sink failed")
    );
}

#[tokio::test]
async fn system_authority_scope_mismatch_fails_before_handler_side_effects() {
    let handler = Arc::new(RecordingSystemHandler::new(json!(1)));
    let system_host = system_host_with_handler(
        Arc::clone(&handler),
        Arc::new(InMemoryResourceGovernor::new()),
    );
    let context = execution_context(CapabilitySet::default());
    let other_context = execution_context(CapabilitySet::default());
    let authority = system_authority("repair-op-wrong-scope", &other_context.resource_scope);

    let outcome = system_host
        .invoke_system_capability(SystemCapabilityInvocationRequest::new(
            authority,
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"repair":"state"}),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected scope mismatch failure, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
    assert!(handler.take_requests().is_empty());
}

#[derive(Clone)]
struct RecordedSystemRequest {
    capability_id: CapabilityId,
    scope: ResourceScope,
    estimate: ResourceEstimate,
    input: Value,
    authority: SystemInvocationAuthority,
}

struct RecordingSystemHandler {
    output: Value,
    requests: Mutex<Vec<RecordedSystemRequest>>,
}

impl RecordingSystemHandler {
    fn new(output: Value) -> Self {
        Self {
            output,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn take_request(&self) -> RecordedSystemRequest {
        self.requests.lock().unwrap().remove(0)
    }

    fn take_requests(&self) -> Vec<RecordedSystemRequest> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

struct FailingSystemHandler {
    usage: ResourceUsage,
}

impl FailingSystemHandler {
    fn new(usage: ResourceUsage) -> Self {
        Self { usage }
    }
}

struct PanickingSystemHandler;
struct NoopFirstPartyHandler;

#[async_trait]
impl SystemCapabilityHandler for RecordingSystemHandler {
    async fn dispatch(
        &self,
        request: SystemCapabilityRequest,
    ) -> Result<SystemCapabilityResult, SystemCapabilityError> {
        self.requests.lock().unwrap().push(RecordedSystemRequest {
            capability_id: request.capability_id.clone(),
            scope: request.scope.clone(),
            estimate: request.estimate.clone(),
            input: request.input.clone(),
            authority: request.authority.clone(),
        });
        Ok(SystemCapabilityResult::new(
            self.output.clone(),
            ResourceUsage::default(),
        ))
    }
}

#[async_trait]
impl SystemCapabilityHandler for FailingSystemHandler {
    async fn dispatch(
        &self,
        _request: SystemCapabilityRequest,
    ) -> Result<SystemCapabilityResult, SystemCapabilityError> {
        Err(
            SystemCapabilityError::new(RuntimeDispatchErrorKind::Backend)
                .with_usage(self.usage.clone()),
        )
    }
}

#[async_trait]
impl SystemCapabilityHandler for PanickingSystemHandler {
    async fn dispatch(
        &self,
        _request: SystemCapabilityRequest,
    ) -> Result<SystemCapabilityResult, SystemCapabilityError> {
        panic!("system handler panic")
    }
}

#[async_trait]
impl ironclaw_host_runtime::FirstPartyCapabilityHandler for NoopFirstPartyHandler {
    async fn dispatch(
        &self,
        _request: ironclaw_host_runtime::FirstPartyCapabilityRequest,
    ) -> Result<
        ironclaw_host_runtime::FirstPartyCapabilityResult,
        ironclaw_host_runtime::FirstPartyCapabilityError,
    > {
        Ok(ironclaw_host_runtime::FirstPartyCapabilityResult::new(
            json!(null),
            ResourceUsage::default(),
        ))
    }
}

struct AllowSystemAuthorityVerifier;
struct DenySystemAuthorityVerifier;
struct RequireApprovalAuthorizer;
struct FailingAuditSink;

#[async_trait]
impl AuditSink for FailingAuditSink {
    async fn emit_audit(&self, _record: AuditEnvelope) -> Result<(), EventError> {
        Err(EventError::Sink {
            reason: "audit unavailable".to_string(),
        })
    }
}

#[async_trait]
impl ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer for RequireApprovalAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::Dispatch {
                    capability: capability_id(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: None,
                reason: "approval required".to_string(),
                reusable_scope: None,
            },
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::SpawnCapability {
                    capability: capability_id(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: None,
                reason: "approval required".to_string(),
                reusable_scope: None,
            },
        }
    }
}

#[async_trait]
impl SystemInvocationAuthorityVerifier for AllowSystemAuthorityVerifier {
    async fn verify_system_authority(
        &self,
        _authority: &SystemInvocationAuthority,
        _scope: &ResourceScope,
        _capability_id: &CapabilityId,
    ) -> Result<(), HostRuntimeError> {
        Ok(())
    }
}

#[async_trait]
impl SystemInvocationAuthorityVerifier for DenySystemAuthorityVerifier {
    async fn verify_system_authority(
        &self,
        _authority: &SystemInvocationAuthority,
        _scope: &ResourceScope,
        _capability_id: &CapabilityId,
    ) -> Result<(), HostRuntimeError> {
        Err(HostRuntimeError::invalid_request("denied system authority"))
    }
}

fn system_host_with_handler<T>(
    handler: Arc<T>,
    governor: Arc<InMemoryResourceGovernor>,
) -> ironclaw_host_runtime::SystemHost
where
    T: SystemCapabilityHandler + 'static,
{
    HostRuntimeServices::new(
        Arc::new(system_extension_registry()),
        Arc::new(LocalFilesystem::new()),
        governor,
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_system_capabilities(Arc::new(
        SystemCapabilityRegistry::new().with_handler(capability_id(), handler),
    ))
    .with_system_authority_verifier(Arc::new(AllowSystemAuthorityVerifier))
    .with_trust_policy(Arc::new(system_trust_policy()))
    .with_audit_sink(Arc::new(InMemoryAuditSink::new()))
    .system_host_for_local_testing()
    .unwrap()
}

fn system_extension_registry() -> ExtensionRegistry {
    let package = ExtensionPackage::from_manifest(
        ExtensionManifest {
            id: provider_id(),
            name: "System".to_string(),
            version: "0.1.0".to_string(),
            description: "Host-owned system capabilities".to_string(),
            requested_trust: RequestedTrustClass::SystemRequested,
            trust: TrustClass::Sandbox,
            runtime: ExtensionRuntime::System {
                service: "kernel".to_string(),
            },
            capabilities: vec![CapabilityManifest {
                id: capability_id(),
                description: "Repairs host-owned state".to_string(),
                effects: vec![EffectKind::DispatchCapability],
                default_permission: PermissionMode::Allow,
                parameters_schema: json!({"type":"object"}),
                resource_profile: None,
            }],
        },
        VirtualPath::new("/system/extensions/system").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn execution_context(grants: CapabilitySet) -> ExecutionContext {
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        provider_id(),
        RuntimeKind::System,
        TrustClass::System,
        grants,
        MountView::default(),
    )
    .unwrap()
}

fn dispatch_grant() -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(),
        grantee: Principal::Extension(provider_id()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::DispatchCapability],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn system_trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("system").unwrap(),
            "/system/extensions/system/manifest.toml".to_string(),
            None,
            HostTrustAssignment::system(),
            vec![EffectKind::DispatchCapability],
            None,
        ),
    ]))])
    .unwrap()
}

fn trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![EffectKind::DispatchCapability],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

fn system_authority(operation_id: &str, scope: &ResourceScope) -> SystemInvocationAuthority {
    SystemInvocationAuthority::host_minted(
        SystemServiceId::new("scheduler").unwrap(),
        "recovery",
        SystemOperationId::new(operation_id).unwrap(),
        scope.clone(),
    )
    .unwrap()
}

fn provider_id() -> ExtensionId {
    ExtensionId::new("system").unwrap()
}

fn capability_id() -> CapabilityId {
    CapabilityId::new("system.repair").unwrap()
}

fn assert_event_kinds(events: &InMemoryEventSink, expected: &[RuntimeEventKind]) {
    let actual = events
        .events()
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}
