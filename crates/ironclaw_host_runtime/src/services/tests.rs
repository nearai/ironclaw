// arch-exempt: large_file, mechanical approval-store migration only — InMemory*Store deleted, guard test repointed to Filesystem*Store<InMemoryBackend> (arch-simplification §4.3), no new test logic, plan #6168
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_approvals::test_support::in_memory_backed_persistent_approval_policy_store;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_capabilities::{
    CapabilityObligationCompletionRequest, CapabilityObligationError,
    CapabilityObligationFailureKind, CapabilityObligationHandler, CapabilityObligationPhase,
    CapabilityObligationRequest,
};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_filesystem::{DiskFilesystem, InMemoryBackend};
use ironclaw_host_api::{
    AgentId, CapabilityDescriptor, CapabilityDispatchResult, CapabilityId, CapabilitySet,
    CorrelationId, DispatchError, EffectKind, ExecutionContext, ExtensionId, HostPortCatalog,
    InvocationId, MountView, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
    Obligation, PermissionMode, ProjectId, ReservationStatus, ResourceEstimate, ResourceReceipt,
    ResourceReservationId, ResourceScope, ResourceUsage, RuntimeCredentialInjection,
    RuntimeCredentialSource, RuntimeCredentialTarget, RuntimeDispatchErrorKind, RuntimeHttpEgress,
    RuntimeHttpEgressRequest, RuntimeKind, SecretHandle, TenantId, TrustClass, UserId, VirtualPath,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_processes::{FilesystemProcessResultStore, FilesystemProcessStore};
use ironclaw_resources::{
    InMemoryResourceGovernor, ResourceAccount, ResourceGovernor, ResourceTally,
};
use ironclaw_secrets::{
    InMemorySecretStore, SecretLeaseId, SecretMaterial, SecretStore, SecretStoreError,
};
use secrecy::ExposeSecret;
use serde_json::{Value, json};

use super::{
    CapabilitySurfaceVersion, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    FirstPartyCapabilityRegistry, FirstPartyRuntimeAdapter, HostProcessPort,
    HostRuntimeHttpEgressPort, HostRuntimeServices, LocalInvocationServicesResolver,
    McpRuntimeAdapter, NetworkMode, ProcessBackendKind, ProcessResultStore, ProcessStore,
    ProductionWiringComponent, ProductionWiringConfig, ProductionWiringIssueKind, RootFilesystem,
    RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeProfile, SecretMode,
    ServiceResolvedRuntimeAdapter,
};
#[cfg(unix)]
use crate::CommandExecutionRequest;
use crate::obligations::{NetworkObligationPolicyStore, RuntimeSecretInjectionStore};
use crate::{HostRuntimeCredentialMaterial, HostRuntimeHttpEgressRequest};

mod first_party_runtime_adapter;
mod mcp_runtime_adapter;

#[tokio::test]
async fn shared_extension_registry_returns_same_instance() {
    let services = test_services();
    let left = services.shared_extension_registry();
    let right = services.shared_extension_registry();

    assert_eq!(Arc::as_ptr(&left), Arc::as_ptr(&right)); // safety: test assertion only; verifies both accessors expose the same shared registry.
}

#[tokio::test]
async fn production_wiring_reports_missing_persistent_approval_policies() {
    let report = test_services()
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("minimal local graph should not satisfy production wiring");

    assert!(report.contains(
        ProductionWiringComponent::PersistentApprovalPolicies,
        ProductionWiringIssueKind::Missing
    ));
}

// The volatile, in-memory-backed persistent-approval store must never satisfy
// production wiring. `in_memory_backed_persistent_approval_policy_store()` returns
// `FilesystemPersistentApprovalPolicyStore<InMemoryBackend>` — the exact concrete
// type the no-durable-features composition wires (factory.rs
// `ComposedPersistentApprovalPolicyStore`), so this guards the real shape, not a
// synthetic one. The durable libSQL/Postgres monomorphizations are distinct types
// and fall through to `ProductionCandidate`.
#[tokio::test]
async fn production_wiring_reports_local_only_persistent_approval_policies() {
    let services = test_services().with_persistent_approval_policies(Arc::new(
        in_memory_backed_persistent_approval_policy_store(),
    ));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]))
        .expect_err("local-only persistent approval policies should not satisfy production");

    assert!(report.contains(
        ProductionWiringComponent::PersistentApprovalPolicies,
        ProductionWiringIssueKind::LocalOnlyImplementation
    ));
}

#[tokio::test]
async fn product_auth_provider_runtime_ports_returns_none_without_egress() {
    let services = test_services();

    assert!(services.product_auth_provider_runtime_ports().is_none());
}

#[tokio::test]
async fn product_auth_provider_runtime_ports_returns_configured_egress_and_obligation_handler() {
    let secret_store = Arc::new(InMemorySecretStore::new());
    let services = test_services()
        .with_secret_store(Arc::clone(&secret_store))
        .try_with_host_http_egress(RecordingNetwork::ok())
        .expect("host HTTP egress should wire with graph secret store");
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("product-auth-secret").unwrap();
    secret_store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("product-auth-material"),
            None,
        )
        .await
        .expect("test secret should store");

    let ports = services
        .product_auth_provider_runtime_ports()
        .expect("runtime ports should be configured");
    assert!(Arc::ptr_eq(
        &ports.runtime_http_egress(),
        &configured_egress(&services)
    ));
    let _handler = ports.obligation_handler();
    ports
        .stage_secret_once(&scope, &capability_id, &handle)
        .await
        .expect("runtime ports should stage product auth secret");
    assert!(
        services
            .secret_injection_store
            .take(&scope, &capability_id, &handle)
            .expect("staged secret should be readable for assertion")
            .is_some()
    );
}

#[tokio::test]
async fn product_auth_ports_stage_secret_from_source_scope_into_target_scope() {
    let secret_store = Arc::new(InMemorySecretStore::new());
    let services = test_services()
        .with_secret_store(Arc::clone(&secret_store))
        .try_with_host_http_egress(RecordingNetwork::ok())
        .expect("host HTTP egress should wire with graph secret store");
    let source_scope = sample_scope();
    let mut target_scope = source_scope.clone();
    target_scope.invocation_id = InvocationId::new();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("product-auth-secret").unwrap();
    secret_store
        .put(
            source_scope.clone(),
            handle.clone(),
            SecretMaterial::from("product-auth-material"),
            None,
        )
        .await
        .expect("test secret should store");

    services
        .product_auth_provider_runtime_ports()
        .expect("runtime ports should be configured")
        .stage_secret_from_scope_once(&source_scope, &target_scope, &capability_id, &handle)
        .await
        .expect("runtime ports should stage product auth secret across scopes");

    assert!(
        services
            .secret_injection_store
            .take(&target_scope, &capability_id, &handle)
            .expect("staged secret should be readable for target scope")
            .is_some()
    );
    assert!(
        services
            .secret_injection_store
            .take(&source_scope, &capability_id, &handle)
            .expect("source scope should remain readable")
            .is_none()
    );
}

#[tokio::test]
async fn runtime_secret_material_stager_stages_secret_material_into_target_scope() {
    let services = test_services()
        .with_secret_store(Arc::new(InMemorySecretStore::new()))
        .try_with_host_http_egress(RecordingNetwork::ok())
        .expect("host HTTP egress should wire with graph secret store");
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("config-secret").unwrap();

    services
        .runtime_secret_material_stager()
        .stage_secret_material_once(
            &scope,
            &capability_id,
            &handle,
            SecretMaterial::from("config-secret-material"),
        )
        .await
        .expect("runtime stager should stage provided secret material");

    let staged = services
        .secret_injection_store
        .take(&scope, &capability_id, &handle)
        .expect("staged secret should be readable for target scope")
        .expect("provided material should be staged");
    assert_eq!(staged.expose_secret(), "config-secret-material");
}

#[tokio::test]
async fn host_runtime_http_egress_port_executes_with_host_staged_credentials() {
    let network = RecordingNetwork::ok();
    let recorded_requests = Arc::clone(&network.requests);
    let services = test_services()
        .with_secret_store(Arc::new(InMemorySecretStore::new()))
        .try_with_host_http_egress(network)
        .expect("host HTTP egress should wire with graph secret store");
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("host-token").unwrap();
    let port = services
        .host_runtime_http_egress_port()
        .expect("host runtime egress port should be configured");

    port.execute(HostRuntimeHttpEgressRequest {
        extension_id: ExtensionId::new("test-extension").unwrap(),
        trust: TrustClass::System,
        request: request_without_credentials(scope.clone(), capability_id.clone()),
        credentials: vec![HostRuntimeCredentialMaterial {
            handle: handle.clone(),
            material: SecretMaterial::from("host-staged-token"),
            target: RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            required: true,
        }],
    })
    .await
    .expect("host egress port should stage credentials and execute");

    {
        let requests = recorded_requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .headers
                .iter()
                .find(|(name, _)| name == "authorization"),
            Some(&(
                "authorization".to_string(),
                "Bearer host-staged-token".to_string()
            ))
        );
    }

    let error = port
        .execute(HostRuntimeHttpEgressRequest {
            extension_id: ExtensionId::new("test-extension").unwrap(),
            trust: TrustClass::System,
            request: request_with_staged_credential(scope, capability_id, handle),
            credentials: Vec::new(),
        })
        .await
        .expect_err("caller-provided credential injections must be rejected");
    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
    ));
    assert_eq!(recorded_requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn host_runtime_http_egress_port_denies_before_network_when_obligation_fails() {
    let network = RecordingNetwork::ok();
    let recorded_requests = Arc::clone(&network.requests);
    let runtime_http_egress = Arc::new(crate::HostHttpEgressService::production(
        network,
        InMemorySecretStore::new(),
        Arc::new(NetworkObligationPolicyStore::new()),
        Arc::new(RuntimeSecretInjectionStore::new()),
        Arc::new(crate::http_body::UnsupportedRuntimeHttpBodyStore),
    ));
    let secret_stager =
        super::RuntimeSecretMaterialStager::new(Arc::new(RuntimeSecretInjectionStore::new()));
    let port = HostRuntimeHttpEgressPort::new(
        runtime_http_egress,
        Arc::new(DenyingObligationHandler),
        secret_stager,
    );

    let error = port
        .execute(HostRuntimeHttpEgressRequest {
            extension_id: ExtensionId::new("test-extension").unwrap(),
            trust: TrustClass::System,
            request: request_without_credentials(sample_scope(), sample_capability_id()),
            credentials: Vec::new(),
        })
        .await
        .expect_err("obligation denial should reject before network dispatch");

    assert_eq!(
        error.reason_code(),
        ironclaw_host_api::RuntimeHttpEgressReasonCode::RequestDenied
    );
    assert!(recorded_requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_borrows_staged_policy_for_repeated_invocation_requests() {
    let network = RecordingNetwork::ok();
    let recorded_requests = Arc::clone(&network.requests);
    let services = test_services()
        .with_secret_store(Arc::new(InMemorySecretStore::new()))
        .try_with_host_http_egress(network)
        .expect("host HTTP egress should wire with graph secret store");
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let staged_policy = staged_policy();
    services
        .network_policy_store
        .insert(&scope, &capability_id, staged_policy.clone());
    let egress = configured_egress(&services);

    egress
        .execute(request_without_credentials(
            scope.clone(),
            capability_id.clone(),
        ))
        .await
        .expect("first request should observe staged policy");
    egress
        .execute(request_without_credentials(scope, capability_id))
        .await
        .expect("second request in same invocation should observe borrowed staged policy");

    let requests = recorded_requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].policy, staged_policy);
    assert_eq!(requests[1].policy, staged_policy);
}

#[tokio::test]
async fn host_http_egress_helper_injects_staged_credentials_from_handoff_store() {
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();

    let network = RecordingNetwork::ok();
    let recorded_requests = Arc::clone(&network.requests);
    let services = test_services()
        .with_secret_store(Arc::new(InMemorySecretStore::new()))
        .try_with_host_http_egress(network)
        .expect("host HTTP egress should wire with graph secret store");
    services
        .network_policy_store
        .insert(&scope, &capability_id, staged_policy());
    services
        .secret_injection_store
        .insert(
            &scope,
            &capability_id,
            &handle,
            SecretMaterial::from("staged-secret"),
        )
        .expect("staged credential should be seeded");
    let egress = configured_egress(&services);

    egress
        .execute(request_with_staged_credential(scope, capability_id, handle))
        .await
        .expect("StagedObligation should inject from handoff store");

    let requests = recorded_requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer staged-secret".to_string()
        ))
    );
}

#[tokio::test]
async fn host_http_egress_helper_consumes_staged_credentials_after_first_egress() {
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();

    let network = RecordingNetwork::ok();
    let recorded_requests = Arc::clone(&network.requests);
    let services = test_services()
        .with_secret_store(Arc::new(InMemorySecretStore::new()))
        .try_with_host_http_egress(network)
        .expect("host HTTP egress should wire with graph secret store");
    services
        .network_policy_store
        .insert(&scope, &capability_id, staged_policy());
    services
        .secret_injection_store
        .insert(
            &scope,
            &capability_id,
            &handle,
            SecretMaterial::from("staged-secret"),
        )
        .expect("staged credential should be seeded");
    let egress = configured_egress(&services);

    egress
        .execute(request_with_staged_credential(
            scope.clone(),
            capability_id.clone(),
            handle.clone(),
        ))
        .await
        .expect("first request should inject staged credential");
    let replay = egress
        .execute(request_with_staged_credential(scope, capability_id, handle))
        .await
        .expect_err("consumed staged credential must not be reusable");
    assert!(matches!(
        replay,
        ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
    ));

    let requests = recorded_requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .headers
            .iter()
            .any(|(name, value)| name == "authorization" && value == "Bearer staged-secret")
    );
}

#[tokio::test]
async fn host_http_egress_treats_expired_staged_secret_as_missing() {
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();

    let network = RecordingNetwork::ok();
    let recorded_requests = Arc::clone(&network.requests);
    let mut services = test_services().with_secret_store(Arc::new(InMemorySecretStore::new()));
    services.secret_injection_store = Arc::new(RuntimeSecretInjectionStore::with_ttl(
        Duration::from_millis(5),
    ));
    services = services
        .try_with_host_http_egress(network)
        .expect("host HTTP egress should wire with graph secret store");
    services
        .network_policy_store
        .insert(&scope, &capability_id, staged_policy());
    services
        .secret_injection_store
        .insert(
            &scope,
            &capability_id,
            &handle,
            SecretMaterial::from("staged-secret"),
        )
        .expect("staged credential should be seeded");
    std::thread::sleep(Duration::from_millis(20));
    let egress = configured_egress(&services);

    let error = egress
        .execute(request_with_staged_credential(scope, capability_id, handle))
        .await
        .expect_err("expired staged secret should fail as missing");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Credential { ref reason }
            if reason == "required credential is unavailable"
    ));
    assert!(recorded_requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_verification_rejects_mismatched_handoff_stores() {
    let mismatched_network_policies = Arc::new(NetworkObligationPolicyStore::new());
    let mismatched_secret_injections = Arc::new(RuntimeSecretInjectionStore::new());
    let egress = Arc::new(crate::HostHttpEgressService::production(
        RecordingNetwork::ok(),
        InMemorySecretStore::new(),
        mismatched_network_policies,
        mismatched_secret_injections,
        Arc::new(crate::http_body::UnsupportedRuntimeHttpBodyStore),
    ));
    let services = test_services()
        .with_secret_store(Arc::new(InMemorySecretStore::new()))
        .with_host_http_egress_service(egress);

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([]).require_runtime_http_egress())
        .expect_err("mismatched handoff stores must not satisfy production egress verification");

    assert!(
        report.contains(
            ProductionWiringComponent::RuntimeHttpEgress,
            ProductionWiringIssueKind::UnverifiedProductionImplementation
        ),
        "mismatched host HTTP egress stores should be reported as unverified: {report:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn inherited_env_runtime_policy_selects_inherited_local_process_port() {
    let workdir = tempfile::tempdir().expect("tempdir");
    let home = std::env::var("HOME").expect("HOME set for inherited env test");
    let services = test_services().with_runtime_policy(policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::Direct,
        SecretMode::InheritedEnv,
    ));

    let output = services
        .process_port
        .run_command(CommandExecutionRequest {
            scope: sample_scope(),
            mounts: None,
            command: "printf '%s' \"$HOME\"".to_string(),
            workdir: Some(workdir.path().display().to_string()),
            timeout_secs: Some(5),
            extra_env: Default::default(),
        })
        .await
        .expect("command succeeds");

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.output, home);
}

#[cfg(unix)]
#[tokio::test]
async fn scrubbed_runtime_policy_resets_managed_local_process_port_after_inherited_policy() {
    let workdir = tempfile::tempdir().expect("tempdir");
    let services = test_services()
        .with_runtime_policy(policy_with(
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::LocalHost,
            NetworkMode::Direct,
            SecretMode::InheritedEnv,
        ))
        .with_runtime_policy(policy_with(
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::LocalHost,
            NetworkMode::Direct,
            SecretMode::ScrubbedEnv,
        ));

    let output = services
        .process_port
        .run_command(CommandExecutionRequest {
            scope: sample_scope(),
            mounts: None,
            command: "printf '%s' \"$HOME\"".to_string(),
            workdir: Some(workdir.path().display().to_string()),
            timeout_secs: Some(5),
            extra_env: Default::default(),
        })
        .await
        .expect("command succeeds");

    assert_eq!(output.exit_code, 0);
    assert_eq!(output.output, workdir.path().display().to_string());
}

#[tokio::test]
async fn host_runtime_services_with_security_audit_sink_records_leak_block() {
    use ironclaw_events::{InMemorySecurityAuditSink, SecurityBoundary, SecurityDecision};

    let security_sink = Arc::new(InMemorySecurityAuditSink::new());
    let services = test_services().with_security_audit_sink(Arc::clone(&security_sink));
    let handler = services.builtin_obligation_handler();

    let invocation_id = InvocationId::new();
    let resource_scope = ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    let context = ExecutionContext {
        run_id: None,
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        authenticated_actor_user_id: None,
        agent_id: resource_scope.agent_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::Sandbox,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        resource_scope: resource_scope.clone(),
    };
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let dispatch = CapabilityDispatchResult {
        capability_id: capability_id.clone(),
        provider: context.extension_id.clone(),
        runtime: RuntimeKind::Wasm,
        output: Value::String("hello AKIAABCDEFGHIJKLMNOP goodbye".to_string()),
        display_preview: None,
        usage: ResourceUsage::default(),
        receipt: ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: resource_scope.clone(),
            status: ReservationStatus::Released,
            estimate: ResourceEstimate::default(),
            actual: None,
        },
    };

    let result = handler
        .complete_dispatch(CapabilityObligationCompletionRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &ResourceEstimate::default(),
            obligations: &[Obligation::RedactOutput],
            dispatch: &dispatch,
        })
        .await;

    assert!(
        matches!(
            result,
            Err(CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Output
            })
        ),
        "expected output-obligation failure, got {result:?}"
    );
    let events = security_sink.snapshot();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.boundary, SecurityBoundary::LeakDetector);
    assert_eq!(event.decision, SecurityDecision::Blocked);
    assert_eq!(event.capability_id.as_ref(), Some(&capability_id));
    assert_eq!(event.scope.as_ref(), Some(&resource_scope));
}

#[tokio::test]
async fn service_guard_releases_reservation_on_planner_denial() {
    let inner = Arc::new(RecordingRuntimeAdapter::default());
    let adapter = ServiceResolvedRuntimeAdapter::new(
        Arc::clone(&inner),
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let estimate = ResourceEstimate::default().set_process_count(1);
    let reservation = governor
        .reserve(scope.clone(), estimate.clone())
        .expect("test reservation should be created");
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    assert_eq!(governor.reserved_for(&tenant_account).process_count, 1);

    let package = test_package(SCRIPT_MANIFEST, "test-script");
    let descriptor = test_descriptor(RuntimeKind::Script, vec![EffectKind::ExecuteCode]);
    let policy = policy_with(
        FilesystemBackendKind::ScopedVirtual,
        ProcessBackendKind::None,
        NetworkMode::Deny,
        SecretMode::Deny,
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
            scope,
            authenticated_actor_user_id: None,
            estimate,
            mounts: None,
            resource_reservation: Some(reservation),
            input: json!({}),
        })
        .await;

    assert!(matches!(
        result,
        Err(DispatchError::Script {
            kind: RuntimeDispatchErrorKind::UnsupportedRunner
        })
    ));
    assert_eq!(inner.call_count(), 0);
    assert_eq!(
        governor.reserved_for(&tenant_account),
        ResourceTally::default()
    );
}

#[tokio::test]
async fn service_guard_rejects_resolution_before_wasm_dispatch() {
    let inner = Arc::new(RecordingRuntimeAdapter::default());
    let adapter = ServiceResolvedRuntimeAdapter::new(
        Arc::clone(&inner),
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let estimate = ResourceEstimate::default();
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let descriptor = test_descriptor(RuntimeKind::Wasm, vec![EffectKind::Network]);
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
            scope,
            authenticated_actor_user_id: None,
            estimate,
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    assert!(matches!(
        result,
        Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::NetworkDenied,
            safe_summary: None,
        })
    ));
    assert_eq!(inner.call_count(), 0);
}

#[tokio::test]
async fn service_guard_releases_reservation_on_invocation_service_resolution_denial() {
    let inner = Arc::new(RecordingRuntimeAdapter::default());
    let adapter = ServiceResolvedRuntimeAdapter::new(
        Arc::clone(&inner),
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let estimate = ResourceEstimate::default().set_network_egress_bytes(1);
    let reservation = governor
        .reserve(scope.clone(), estimate.clone())
        .expect("test reservation should be created");
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    assert_eq!(
        governor.reserved_for(&tenant_account).network_egress_bytes,
        1
    );

    let package = test_package(WASM_MANIFEST, "test-wasm");
    let descriptor = test_descriptor(RuntimeKind::Wasm, vec![EffectKind::Network]);
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
            scope,
            authenticated_actor_user_id: None,
            estimate,
            mounts: None,
            resource_reservation: Some(reservation),
            input: json!({}),
        })
        .await;

    assert!(matches!(
        result,
        Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::NetworkDenied,
            safe_summary: None,
        })
    ));
    assert_eq!(inner.call_count(), 0);
    assert_eq!(
        governor.reserved_for(&tenant_account),
        ResourceTally::default()
    );
}

#[tokio::test]
async fn service_guard_rejects_required_secret_without_secret_store_before_dispatch() {
    let inner = Arc::new(RecordingRuntimeAdapter::default());
    let adapter = ServiceResolvedRuntimeAdapter::new(
        Arc::clone(&inner),
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let estimate = ResourceEstimate::default();
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let descriptor = test_descriptor(RuntimeKind::Wasm, vec![EffectKind::UseSecret]);
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::Deny,
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
            scope,
            authenticated_actor_user_id: None,
            estimate,
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    assert!(matches!(
        result,
        Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::SecretDenied,
            safe_summary: None,
        })
    ));
    assert_eq!(inner.call_count(), 0);
}

#[tokio::test]
async fn first_party_adapter_releases_reservation_when_invocation_service_resolution_denies() {
    let descriptor = test_descriptor(RuntimeKind::FirstParty, vec![EffectKind::Network]);
    let registry = Arc::new(
        FirstPartyCapabilityRegistry::new()
            .with_handler(descriptor.id.clone(), Arc::new(PanicFirstPartyHandler)),
    );
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    let estimate = ResourceEstimate::default().set_network_egress_bytes(1);
    let reservation = governor
        .reserve(scope.clone(), estimate.clone())
        .expect("test reservation should be created");
    assert_eq!(
        governor.reserved_for(&tenant_account).network_egress_bytes,
        1
    );
    let package = test_package(WASM_MANIFEST, "test-wasm");
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
            scope,
            authenticated_actor_user_id: None,
            estimate,
            mounts: None,
            resource_reservation: Some(reservation),
            input: json!({}),
        })
        .await;

    assert!(matches!(
        result,
        Err(DispatchError::FirstParty {
            kind: RuntimeDispatchErrorKind::NetworkDenied,
            ..
        })
    ));
    assert_eq!(
        governor.reserved_for(&tenant_account),
        ResourceTally::default()
    );
}

#[tokio::test]
async fn first_party_adapter_denies_hosted_unsupported_services_before_handler_dispatch() {
    assert_first_party_denies_before_handler(
        vec![EffectKind::ReadFilesystem],
        hosted_policy_with(
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::None,
            NetworkMode::Deny,
            SecretMode::Deny,
        ),
        None,
        RuntimeDispatchErrorKind::FilesystemDenied,
    )
    .await;
    assert_first_party_denies_before_handler(
        vec![EffectKind::ReadFilesystem],
        hosted_policy_with(
            FilesystemBackendKind::ScopedVirtual,
            ProcessBackendKind::None,
            NetworkMode::Deny,
            SecretMode::Deny,
        ),
        None,
        RuntimeDispatchErrorKind::FilesystemDenied,
    )
    .await;
    assert_first_party_denies_before_handler(
        vec![EffectKind::ExecuteCode],
        hosted_policy_with(
            FilesystemBackendKind::ScopedVirtual,
            ProcessBackendKind::LocalHost,
            NetworkMode::Deny,
            SecretMode::Deny,
        ),
        None,
        RuntimeDispatchErrorKind::UnsupportedRunner,
    )
    .await;
    assert_first_party_denies_before_handler(
        vec![EffectKind::Network],
        hosted_policy_with(
            FilesystemBackendKind::ScopedVirtual,
            ProcessBackendKind::None,
            NetworkMode::Direct,
            SecretMode::Deny,
        ),
        None,
        RuntimeDispatchErrorKind::NetworkDenied,
    )
    .await;
    assert_first_party_denies_before_handler(
        vec![EffectKind::UseSecret],
        hosted_policy_with(
            FilesystemBackendKind::ScopedVirtual,
            ProcessBackendKind::None,
            NetworkMode::Deny,
            SecretMode::InheritedEnv,
        ),
        None,
        RuntimeDispatchErrorKind::SecretDenied,
    )
    .await;
}

#[tokio::test]
async fn first_party_adapter_releases_reservation_when_planner_denies() {
    let descriptor = test_descriptor(RuntimeKind::FirstParty, vec![EffectKind::Network]);
    let registry = Arc::new(
        FirstPartyCapabilityRegistry::new()
            .with_handler(descriptor.id.clone(), Arc::new(PanicFirstPartyHandler)),
    );
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    let estimate = ResourceEstimate::default().set_network_egress_bytes(1);
    let reservation = governor
        .reserve(scope.clone(), estimate.clone())
        .expect("test reservation should be created");
    assert_eq!(
        governor.reserved_for(&tenant_account).network_egress_bytes,
        1
    );
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::Deny,
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
            scope,
            authenticated_actor_user_id: None,
            estimate,
            mounts: None,
            resource_reservation: Some(reservation),
            input: json!({}),
        })
        .await;

    assert!(matches!(
        result,
        Err(DispatchError::FirstParty {
            kind: RuntimeDispatchErrorKind::NetworkDenied,
            ..
        })
    ));
    assert_eq!(
        governor.reserved_for(&tenant_account),
        ResourceTally::default()
    );
}

fn test_services() -> HostRuntimeServices<
    DiskFilesystem,
    InMemoryResourceGovernor,
    FilesystemProcessStore<InMemoryBackend>,
    FilesystemProcessResultStore<InMemoryBackend>,
> {
    HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
}

fn configured_egress<
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
>(
    services: &HostRuntimeServices<F, G, S, R>,
) -> Arc<dyn RuntimeHttpEgress> {
    services
        .runtime_http_egress
        .lock()
        .unwrap()
        .as_ref()
        .expect("runtime HTTP egress should be configured")
        .clone()
}

fn test_package(manifest: &str, extension_id: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse(
        manifest,
        ManifestSource::HostBundled,
        &HostPortCatalog::empty(),
    )
    .expect("test manifest should parse");
    ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new(format!("/system/extensions/{extension_id}")).unwrap(),
    )
    .expect("test package should build")
}

fn test_descriptor(runtime: RuntimeKind, effects: Vec<EffectKind>) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new("test.capability").unwrap(),
        provider: ironclaw_host_api::ExtensionId::new("test").unwrap(),
        runtime,
        trust_ceiling: TrustClass::UserTrusted,
        description: "test capability".to_string(),
        parameters_schema: serde_json::Value::Null,
        effects,
        default_permission: PermissionMode::Allow,
        runtime_credentials: Vec::new(),
        network_targets: Vec::new(),
        resource_profile: None,
    }
}

fn policy_with(
    filesystem_backend: FilesystemBackendKind,
    process_backend: ProcessBackendKind,
    network_mode: NetworkMode,
    secret_mode: SecretMode,
) -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend,
        process_backend,
        network_mode,
        secret_mode,
        approval_policy: ironclaw_host_api::runtime_policy::ApprovalPolicy::AskDestructive,
        audit_mode: ironclaw_host_api::runtime_policy::AuditMode::LocalMinimal,
    }
}

fn hosted_policy_with(
    filesystem_backend: FilesystemBackendKind,
    process_backend: ProcessBackendKind,
    network_mode: NetworkMode,
    secret_mode: SecretMode,
) -> EffectiveRuntimePolicy {
    let mut policy = policy_with(
        filesystem_backend,
        process_backend,
        network_mode,
        secret_mode,
    );
    policy.deployment = DeploymentMode::HostedMultiTenant;
    policy.requested_profile = RuntimeProfile::HostedSafe;
    policy.resolved_profile = RuntimeProfile::HostedSafe;
    policy
}

async fn assert_first_party_denies_before_handler(
    effects: Vec<EffectKind>,
    policy: EffectiveRuntimePolicy,
    mounts: Option<MountView>,
    expected_kind: RuntimeDispatchErrorKind,
) {
    let descriptor = test_descriptor(RuntimeKind::FirstParty, effects);
    let registry = Arc::new(
        FirstPartyCapabilityRegistry::new()
            .with_handler(descriptor.id.clone(), Arc::new(PanicFirstPartyHandler)),
    );
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            Some(Arc::new(InMemorySecretStore::new())),
        )),
    );
    let filesystem = DiskFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let package = test_package(WASM_MANIFEST, "test-wasm");

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
            mounts,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    assert!(
        matches!(
            result,
            Err(DispatchError::FirstParty { kind, .. }) if kind == expected_kind
        ),
        "expected first-party denial {expected_kind:?}, got {result:?}"
    );
}

#[derive(Default)]
struct RecordingRuntimeAdapter {
    calls: Mutex<usize>,
}

impl RecordingRuntimeAdapter {
    fn call_count(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl RuntimeAdapter<DiskFilesystem, InMemoryResourceGovernor> for RecordingRuntimeAdapter {
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, DiskFilesystem, InMemoryResourceGovernor>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        *self.calls.lock().unwrap() += 1;
        let usage = ResourceUsage::default();
        let reservation = match request.resource_reservation {
            Some(reservation) => reservation,
            None => request
                .governor
                .reserve(request.scope, request.estimate)
                .map_err(|_| DispatchError::Wasm {
                    kind: RuntimeDispatchErrorKind::Resource,
                    safe_summary: None,
                })?,
        };
        let receipt: ResourceReceipt = request
            .governor
            .reconcile(reservation.id, usage.clone())
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
                safe_summary: None,
            })?;
        Ok(RuntimeAdapterResult {
            output: Value::Null,
            display_preview: None,
            usage,
            receipt,
            output_bytes: 0,
        })
    }
}

struct PanicFirstPartyHandler;

#[async_trait]
impl crate::FirstPartyCapabilityHandler for PanicFirstPartyHandler {
    async fn dispatch(
        &self,
        _request: crate::FirstPartyCapabilityRequest,
    ) -> Result<crate::FirstPartyCapabilityResult, crate::FirstPartyCapabilityError> {
        panic!("service-resolution denial should happen before handler dispatch")
    }
}

fn request_without_credentials(
    scope: ResourceScope,
    capability_id: CapabilityId,
) -> RuntimeHttpEgressRequest {
    RuntimeHttpEgressRequest {
        runtime: RuntimeKind::Script,
        scope,
        capability_id,
        method: NetworkMethod::Get,
        url: "https://api.example.test/v1/run".to_string(),
        headers: vec![],
        body: Vec::new(),
        network_policy: caller_policy(),
        credential_injections: vec![],
        response_body_limit: Some(4096),
        save_body_to: None,
        timeout_ms: None,
    }
}

fn request_with_staged_credential(
    scope: ResourceScope,
    capability_id: CapabilityId,
    handle: SecretHandle,
) -> RuntimeHttpEgressRequest {
    let mut request = request_without_credentials(scope, capability_id.clone());
    request.credential_injections = vec![RuntimeCredentialInjection {
        handle,
        source: RuntimeCredentialSource::StagedObligation { capability_id },
        target: RuntimeCredentialTarget::Header {
            name: "authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        },
        required: true,
    }];
    request
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn sample_capability_id() -> CapabilityId {
    CapabilityId::new("runtime.http").unwrap()
}

fn staged_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(4096),
    }
}

fn caller_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "caller.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: false,
        max_egress_bytes: Some(1),
    }
}

const SCRIPT_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "test-script"
name = "Test Script"
version = "0.1.0"
description = "Script test extension"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "sh"
args = ["-c", "cat"]

[[capabilities]]
id = "test-script.run"
description = "Run script"
effects = ["execute_code"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/test-script/run.input.v1.json"
output_schema_ref = "schemas/test-script/run.output.v1.json"
prompt_doc_ref = "prompts/test-script/run.md"
"#;

const WASM_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "test-wasm"
name = "Test Wasm"
version = "0.1.0"
description = "WASM test extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "test.wasm"

[[capabilities]]
id = "test-wasm.run"
description = "Run WASM"
effects = ["network"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/test-wasm/run.input.v1.json"
output_schema_ref = "schemas/test-wasm/run.output.v1.json"
prompt_doc_ref = "prompts/test-wasm/run.md"
"#;

#[derive(Clone)]
struct RecordingNetwork {
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

impl RecordingNetwork {
    fn ok() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl NetworkHttpEgress for RecordingNetwork {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        self.requests.lock().unwrap().push(request);
        Ok(NetworkHttpResponse {
            status: 200,
            headers: vec![],
            body: br#"{"ok":true}"#.to_vec(),
            usage: NetworkUsage {
                request_bytes: 0,
                response_bytes: 11,
                resolved_ip: None,
            },
        })
    }
}

struct DenyingObligationHandler;

#[async_trait]
impl CapabilityObligationHandler for DenyingObligationHandler {
    async fn satisfy(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        Err(CapabilityObligationError::Failed {
            kind: CapabilityObligationFailureKind::Network,
        })
    }
}

/// `stage_secret_error` maps `SecretStoreError` variants to `ProductAuthCredentialStageError`.
///
/// These are synchronous unit tests of a pure function — no store, no async.
#[test]
fn stage_secret_error_maps_auth_and_backend_variants() {
    use crate::services::stage_secret_error;
    use ironclaw_host_api::CredentialStageError::{AuthRequired, Backend};

    let scope = sample_scope();
    let cases: &[(SecretStoreError, crate::ProductAuthCredentialStageError)] = &[
        (
            SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle: SecretHandle::new("h").unwrap(),
            },
            AuthRequired,
        ),
        (SecretStoreError::SecretExpired, AuthRequired),
        (
            SecretStoreError::LeaseRevoked {
                lease_id: SecretLeaseId::default(),
            },
            AuthRequired,
        ),
        (
            SecretStoreError::LeaseExpired {
                lease_id: SecretLeaseId::default(),
            },
            AuthRequired,
        ),
        // Consumed lease: one-shot lease already used by a concurrent call.
        // stable_reason = "CredentialExpired" — user-actionable, must be AuthRequired.
        (
            SecretStoreError::LeaseConsumed {
                lease_id: SecretLeaseId::default(),
            },
            AuthRequired,
        ),
        // Unknown lease: lease gone (expired/evicted between lease_once and consume).
        // stable_reason = "MissingCredential" — user-actionable, must be AuthRequired.
        (
            SecretStoreError::UnknownLease {
                scope: Box::new(scope.clone()),
                lease_id: SecretLeaseId::default(),
            },
            AuthRequired,
        ),
        (
            SecretStoreError::BackendMisconfigured {
                reason: "vault offline".to_string(),
            },
            Backend,
        ),
        (
            SecretStoreError::StoreUnavailable {
                reason: "down".to_string(),
            },
            Backend,
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(
            stage_secret_error(error.clone()),
            *expected,
            "error: {error:?}"
        );
    }
}

// T6 — RegisteredRuntimeHealth
#[tokio::test]
async fn registered_runtime_health_empty_available_reports_all_required_as_missing() {
    use crate::services::{RegisteredRuntimeHealth, RuntimeBackendHealth};
    let health = RegisteredRuntimeHealth::new(vec![]);
    let missing = health
        .missing_runtime_backends(&[RuntimeKind::Wasm, RuntimeKind::Mcp])
        .await
        .expect("health check must succeed");
    // Both kinds are missing; order is normalized by runtime_sort_key.
    assert!(
        missing.contains(&RuntimeKind::Wasm),
        "Wasm must be missing; got {missing:?}"
    );
    assert!(
        missing.contains(&RuntimeKind::Mcp),
        "Mcp must be missing; got {missing:?}"
    );
    assert_eq!(missing.len(), 2);
}

#[tokio::test]
async fn registered_runtime_health_deduplicates_duplicate_required_kinds() {
    use crate::services::{RegisteredRuntimeHealth, RuntimeBackendHealth};
    let health = RegisteredRuntimeHealth::new(vec![RuntimeKind::Wasm]);
    let missing = health
        .missing_runtime_backends(&[RuntimeKind::Mcp, RuntimeKind::Mcp, RuntimeKind::Wasm])
        .await
        .expect("health check must succeed");
    assert_eq!(missing, vec![RuntimeKind::Mcp], "got {missing:?}");
}

#[tokio::test]
async fn registered_runtime_health_returns_empty_when_all_required_available() {
    use crate::services::{RegisteredRuntimeHealth, RuntimeBackendHealth};
    let health = RegisteredRuntimeHealth::new(vec![RuntimeKind::Wasm, RuntimeKind::Mcp]);
    let missing = health
        .missing_runtime_backends(&[RuntimeKind::Wasm])
        .await
        .expect("health check must succeed");
    assert!(
        missing.is_empty(),
        "expected no missing kinds; got {missing:?}"
    );
}
