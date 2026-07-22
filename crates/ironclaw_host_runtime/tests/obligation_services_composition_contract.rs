mod support;

use support::legacy_capability_fixture_to_v2;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_events::InMemoryAuditSink;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    BuiltinObligationServices, CapabilitySurfaceVersion, DefaultHostRuntime, HostRuntime,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_resources::{InMemoryResourceGovernor, ResourceGovernor};
use ironclaw_secrets::{FilesystemSecretStore, SecretMaterial, SecretStore};
use ironclaw_trust::TrustDecision;
use serde_json::json;

fn local_test_runtime_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    ironclaw_runtime_policy::resolve(ironclaw_runtime_policy::ResolveRequest::new(
        ironclaw_host_api::runtime_policy::DeploymentMode::LocalSingleUser,
        ironclaw_host_api::runtime_policy::RuntimeProfile::LocalDev,
    ))
    .unwrap()
}

#[tokio::test]
async fn default_runtime_installs_configured_builtin_obligation_services() {
    let registry = Arc::new(registry_with_echo_capability());
    let audit_sink = Arc::new(InMemoryAuditSink::new());
    let secret_store = Arc::new(FilesystemSecretStore::ephemeral());
    let resource_governor = Arc::new(InMemoryResourceGovernor::new());
    let services = BuiltinObligationServices::new(
        audit_sink.clone(),
        secret_store.clone(),
        resource_governor.clone(),
    );

    let secret_handle = SecretHandle::new("api_token").unwrap();
    let expected_policy = NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.com".to_string(),
            port: Some(443),
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(2048),
    };
    let reservation_id = ResourceReservationId::new();
    let network = RecordingNetwork::default();
    let egress: Arc<dyn RuntimeHttpEgress> = Arc::new(services.host_http_egress(network.clone()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer {
            policy: expected_policy.clone(),
            secret_handle: secret_handle.clone(),
            reservation_id,
        });
    let dispatcher = Arc::new(ObligationAwareDispatcher::new(
        egress,
        resource_governor.clone(),
        secret_handle.clone(),
    ));

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_builtin_obligation_services(&services);

    let context = execution_context_with_dispatch_grant();
    secret_store
        .put(
            context.resource_scope.clone(),
            secret_handle,
            SecretMaterial::from("runtime-secret"),
            None,
        )
        .await
        .unwrap();

    let request = RuntimeCapabilityRequest::new(
        context,
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "hello"}),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id());
            assert_eq!(completed.output, json!({"ok": true}));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    assert!(dispatcher.dispatched_with_staged_handoffs());
    let network_requests = network.requests();
    assert_eq!(network_requests.len(), 1);
    assert_eq!(network_requests[0].policy, expected_policy);
    assert_eq!(
        network_requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer runtime-secret".to_string(),
        ))
    );
    assert_eq!(audit_sink.records().len(), 1);
    assert_eq!(audit_sink.records()[0].stage, AuditStage::Before);
}

struct ObligatingAuthorizer {
    policy: NetworkPolicy,
    secret_handle: SecretHandle,
    reservation_id: ResourceReservationId,
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ObligatingAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(vec![
                Obligation::AuditBefore,
                Obligation::ReserveResources {
                    reservation_id: self.reservation_id,
                },
                Obligation::ApplyNetworkPolicy {
                    policy: self.policy.clone(),
                },
                Obligation::InjectSecretOnce {
                    handle: self.secret_handle.clone(),
                },
            ])
            .unwrap(),
        }
    }
}

#[derive(Clone, Default)]
struct RecordingNetwork {
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

impl RecordingNetwork {
    fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl NetworkHttpEgress for RecordingNetwork {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        Ok(NetworkHttpResponse {
            status: 200,
            headers: Vec::new(),
            body: br#"{"ok":true}"#.to_vec(),
            usage: NetworkUsage {
                request_bytes: 5,
                response_bytes: 11,
                resolved_ip: None,
            },
        })
    }
}

// KEPT (not `TestDispatcher`): this double does real work inside `dispatch_json`
// that a `TestDispatcher` responder cannot express — it awaits `egress.execute`
// with the staged credential injection, releases the resource reservation via the
// governor, and records whether the staged handoffs were present. A `responding`
// closure is synchronous and cannot `.await`; `ok`/`scripted` only return canned
// values.
struct ObligationAwareDispatcher {
    egress: Arc<dyn RuntimeHttpEgress>,
    resource_governor: Arc<InMemoryResourceGovernor>,
    secret_handle: SecretHandle,
    dispatched: Mutex<bool>,
}

impl ObligationAwareDispatcher {
    fn new(
        egress: Arc<dyn RuntimeHttpEgress>,
        resource_governor: Arc<InMemoryResourceGovernor>,
        secret_handle: SecretHandle,
    ) -> Self {
        Self {
            egress,
            resource_governor,
            secret_handle,
            dispatched: Mutex::new(false),
        }
    }

    fn dispatched_with_staged_handoffs(&self) -> bool {
        *self
            .dispatched
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[async_trait]
impl CapabilityDispatcher for ObligationAwareDispatcher {
    async fn dispatch_json(
        &self,
        request: Authorized,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        let (invocation, _lane, _mounts, resource_reservation) = request
            .into_parts(chrono::Utc::now())
            .map_err(|authorized| {
                let capability = authorized.invocation().capability.clone();
                let _ = authorized.abort();
                DispatchError::AuthorizationExpired { capability }
            })?;
        let egress_response = self
            .egress
            .execute(RuntimeHttpEgressRequest {
                runtime: RuntimeKind::Wasm,
                scope: invocation.scope.clone(),
                capability_id: invocation.capability.clone(),
                method: NetworkMethod::Post,
                url: "https://api.example.com/v1/run".to_string(),
                headers: Vec::new(),
                body: b"hello".to_vec(),
                network_policy: NetworkPolicy::default(),
                credential_injections: vec![RuntimeCredentialInjection {
                    handle: self.secret_handle.clone(),
                    source: RuntimeCredentialSource::StagedObligation {
                        capability_id: invocation.capability.clone(),
                    },
                    target: RuntimeCredentialTarget::Header {
                        name: "authorization".to_string(),
                        prefix: Some("Bearer ".to_string()),
                    },
                    required: true,
                }],
                response_body_limit: None,
                save_body_to: None,
                timeout_ms: None,
            })
            .await
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::NetworkDenied,
                model_visible_cause: None,
            })?;
        assert_eq!(egress_response.status, 200);

        let reservation = resource_reservation
            .as_ref()
            .expect("configured resource governor should reserve before dispatch");
        let receipt = self.resource_governor.release(reservation.id).unwrap();
        *self
            .dispatched
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;

        Ok(CapabilityDispatchResult {
            capability_id: invocation.capability,
            provider: extension_id(),
            runtime: RuntimeKind::Wasm,
            output: json!({"ok": true}),
            display_preview: None,
            usage: ResourceUsage::default(),
            receipt,
        })
    }
}

fn registry_with_echo_capability() -> ExtensionRegistry {
    let manifest = parse_manifest(ECHO_MANIFEST);
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn parse_manifest(manifest: &str) -> ExtensionManifest {
    let manifest = legacy_capability_fixture_to_v2(manifest);
    ExtensionManifest::parse(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap()
}

fn execution_context_with_dispatch_grant() -> ExecutionContext {
    let mut grants = CapabilitySet::default();
    grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
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
    });
    let mut context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap();
    context.run_id = Some(RunId::new());
    context
}

fn capability_id() -> CapabilityId {
    CapabilityId::new("echo.say").unwrap()
}

fn extension_id() -> ExtensionId {
    ExtensionId::new("echo").unwrap()
}

const ECHO_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes input"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = {}
"#;

fn capability_provider_contracts() -> ironclaw_extensions::HostApiContractRegistry {
    let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(
            ironclaw_extensions::CapabilityProviderHostApiContract::new()
                .expect("capability provider contract"),
        ))
        .expect("register capability provider contract");
    contracts
}
