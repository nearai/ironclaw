use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, DispatchError, EffectKind, HostPortCatalog, InvocationId,
    NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, PermissionMode,
    ResourceEstimate, ResourceReceipt, ResourceScope, ResourceUsage, RuntimeCredentialInjection,
    RuntimeCredentialSource, RuntimeCredentialTarget, RuntimeDispatchErrorKind, RuntimeHttpEgress,
    RuntimeHttpEgressRequest, RuntimeKind, SecretHandle, TenantId, TrustClass, UserId, VirtualPath,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_processes::{InMemoryProcessResultStore, InMemoryProcessStore, ProcessServices};
use ironclaw_resources::{
    InMemoryResourceGovernor, ResourceAccount, ResourceGovernor, ResourceTally,
};
use ironclaw_secrets::{InMemorySecretStore, SecretMaterial};
use serde_json::{Value, json};

use super::{
    CapabilitySurfaceVersion, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    FirstPartyCapabilityRegistry, FirstPartyRuntimeAdapter, HostRuntimeServices,
    LocalHostProcessPort, LocalInvocationServicesResolver, NetworkMode, ProcessBackendKind,
    ProcessResultStore, ProcessStore, RootFilesystem, RuntimeAdapter, RuntimeAdapterRequest,
    RuntimeAdapterResult, RuntimeProfile, SecretMode, ServiceResolvedRuntimeAdapter,
};

#[test]
fn host_http_egress_borrows_staged_policy_for_repeated_invocation_requests() {
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
        .expect("first request should observe staged policy");
    egress
        .execute(request_without_credentials(scope, capability_id))
        .expect("second request in same invocation should observe borrowed staged policy");

    let requests = recorded_requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].policy, staged_policy);
    assert_eq!(requests[1].policy, staged_policy);
}

#[test]
fn host_http_egress_helper_injects_staged_credentials_from_handoff_store() {
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
async fn service_guard_releases_reservation_on_planner_denial() {
    let inner = Arc::new(RecordingRuntimeAdapter::default());
    let adapter = ServiceResolvedRuntimeAdapter::new(
        Arc::clone(&inner),
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(LocalFilesystem::new()),
            None,
            Arc::new(LocalHostProcessPort::new()),
            None,
        )),
    );
    let filesystem = LocalFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let estimate = ResourceEstimate {
        process_count: Some(1),
        ..ResourceEstimate::default()
    };
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
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
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
            Arc::new(LocalFilesystem::new()),
            None,
            Arc::new(LocalHostProcessPort::new()),
            None,
        )),
    );
    let filesystem = LocalFilesystem::new();
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
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            estimate,
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    assert!(matches!(
        result,
        Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::NetworkDenied
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
            Arc::new(LocalFilesystem::new()),
            None,
            Arc::new(LocalHostProcessPort::new()),
            None,
        )),
    );
    let filesystem = LocalFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let estimate = ResourceEstimate {
        network_egress_bytes: Some(1),
        ..ResourceEstimate::default()
    };
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
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            estimate,
            mounts: None,
            resource_reservation: Some(reservation),
            input: json!({}),
        })
        .await;

    assert!(matches!(
        result,
        Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::NetworkDenied
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
            Arc::new(LocalFilesystem::new()),
            None,
            Arc::new(LocalHostProcessPort::new()),
            None,
        )),
    );
    let filesystem = LocalFilesystem::new();
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
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            estimate,
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    assert!(matches!(
        result,
        Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::SecretDenied
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
            Arc::new(LocalFilesystem::new()),
            None,
            Arc::new(LocalHostProcessPort::new()),
            None,
        )),
    );
    let filesystem = LocalFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    let estimate = ResourceEstimate {
        network_egress_bytes: Some(1),
        ..ResourceEstimate::default()
    };
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
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            estimate,
            mounts: None,
            resource_reservation: Some(reservation),
            input: json!({}),
        })
        .await;

    assert!(matches!(
        result,
        Err(DispatchError::FirstParty {
            kind: RuntimeDispatchErrorKind::NetworkDenied
        })
    ));
    assert_eq!(
        governor.reserved_for(&tenant_account),
        ResourceTally::default()
    );
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
            Arc::new(LocalFilesystem::new()),
            None,
            Arc::new(LocalHostProcessPort::new()),
            None,
        )),
    );
    let filesystem = LocalFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    let estimate = ResourceEstimate {
        network_egress_bytes: Some(1),
        ..ResourceEstimate::default()
    };
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
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            estimate,
            mounts: None,
            resource_reservation: Some(reservation),
            input: json!({}),
        })
        .await;

    assert!(matches!(
        result,
        Err(DispatchError::FirstParty {
            kind: RuntimeDispatchErrorKind::NetworkDenied
        })
    ));
    assert_eq!(
        governor.reserved_for(&tenant_account),
        ResourceTally::default()
    );
}

fn test_services() -> HostRuntimeServices<
    LocalFilesystem,
    InMemoryResourceGovernor,
    InMemoryProcessStore,
    InMemoryProcessResultStore,
> {
    HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
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
impl RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> for RecordingRuntimeAdapter {
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
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
                })?,
        };
        let receipt: ResourceReceipt = request
            .governor
            .reconcile(reservation.id, usage.clone())
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
            })?;
        Ok(RuntimeAdapterResult {
            output: Value::Null,
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
        timeout_ms: None,
    }
}

fn request_with_staged_credential(
    scope: ResourceScope,
    capability_id: CapabilityId,
    handle: SecretHandle,
) -> RuntimeHttpEgressRequest {
    RuntimeHttpEgressRequest {
        credential_injections: vec![RuntimeCredentialInjection {
            handle,
            source: RuntimeCredentialSource::StagedObligation {
                capability_id: capability_id.clone(),
            },
            target: RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            required: true,
        }],
        ..request_without_credentials(scope, capability_id)
    }
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

impl NetworkHttpEgress for RecordingNetwork {
    fn execute(
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
