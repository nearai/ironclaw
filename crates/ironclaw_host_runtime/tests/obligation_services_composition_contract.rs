use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_events::InMemoryAuditSink;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    BuiltinObligationServices, CapabilitySurfaceVersion, DefaultHostRuntime, HostRuntime,
    NetworkObligationPolicyStore, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeSecretInjectionStore,
};
use ironclaw_resources::{InMemoryResourceGovernor, ResourceGovernor};
use ironclaw_secrets::{InMemorySecretStore, SecretMaterial, SecretStore};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use secrecy::ExposeSecret;
use serde_json::json;

#[tokio::test]
async fn default_runtime_installs_configured_builtin_obligation_services() {
    let registry = Arc::new(registry_with_echo_capability());
    let audit_sink = Arc::new(InMemoryAuditSink::new());
    let secret_store = Arc::new(InMemorySecretStore::new());
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
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer {
            policy: expected_policy.clone(),
            secret_handle: secret_handle.clone(),
            reservation_id,
        });
    let dispatcher = Arc::new(ObligationAwareDispatcher::new(
        services.network_policy_store(),
        services.secret_injection_store(),
        resource_governor.clone(),
        expected_policy,
        secret_handle.clone(),
    ));

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_builtin_obligation_services(&services);

    let context = execution_context_with_dispatch_grant();
    secret_store
        .put(
            context.resource_scope.clone(),
            secret_handle,
            SecretMaterial::from("runtime-secret"),
        )
        .await
        .unwrap();

    let request = RuntimeCapabilityRequest::new(
        context,
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "hello"}),
        trust_decision_with_dispatch_authority(),
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

struct ObligationAwareDispatcher {
    network_policies: Arc<NetworkObligationPolicyStore>,
    secret_injections: Arc<RuntimeSecretInjectionStore>,
    resource_governor: Arc<InMemoryResourceGovernor>,
    expected_policy: NetworkPolicy,
    secret_handle: SecretHandle,
    dispatched: Mutex<bool>,
}

impl ObligationAwareDispatcher {
    fn new(
        network_policies: Arc<NetworkObligationPolicyStore>,
        secret_injections: Arc<RuntimeSecretInjectionStore>,
        resource_governor: Arc<InMemoryResourceGovernor>,
        expected_policy: NetworkPolicy,
        secret_handle: SecretHandle,
    ) -> Self {
        Self {
            network_policies,
            secret_injections,
            resource_governor,
            expected_policy,
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
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        let policy = self
            .network_policies
            .take(&request.scope, &request.capability_id)
            .expect("configured obligation store should stage network policy");
        assert_eq!(policy, self.expected_policy);

        let material = self
            .secret_injections
            .take(&request.scope, &request.capability_id, &self.secret_handle)
            .unwrap()
            .expect("configured obligation store should stage one-shot secret material");
        assert_eq!(material.expose_secret(), "runtime-secret");

        let reservation = request
            .resource_reservation
            .as_ref()
            .expect("configured resource governor should reserve before dispatch");
        let receipt = self.resource_governor.release(reservation.id).unwrap();
        *self
            .dispatched
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;

        Ok(CapabilityDispatchResult {
            capability_id: request.capability_id,
            provider: extension_id(),
            runtime: RuntimeKind::Wasm,
            output: json!({"ok": true}),
            usage: ResourceUsage::default(),
            receipt,
        })
    }
}

fn registry_with_echo_capability() -> ExtensionRegistry {
    let manifest = ExtensionManifest::parse(ECHO_MANIFEST).unwrap();
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
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
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap()
}

fn trust_decision_with_dispatch_authority() -> TrustDecision {
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
