use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_authorization::{GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, DefaultHostRuntime, HostRuntime, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeFailureKind,
};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, InvalidationBus, TrustDecision, TrustPolicy, TrustPolicyInput,
    TrustProvenance,
};
use serde_json::json;

#[tokio::test]
async fn production_runtime_ignores_caller_supplied_privileged_trust_decision() {
    let registry = Arc::new(registry_with_manifest(FIRST_PARTY_REQUESTED_MANIFEST));
    let dispatcher = Arc::new(CountingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::new(
        Arc::clone(&registry),
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    );

    let forged_decision = privileged_local_manifest_policy()
        .evaluate(&trust_input_for_registry(&registry))
        .unwrap();
    let request = RuntimeCapabilityRequest::new(
        execution_context_with_dispatch_grant(TrustClass::FirstParty),
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "must not dispatch"}),
        forged_decision,
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    assert_authorization_failed(outcome);
    assert_eq!(
        dispatcher.count(),
        0,
        "stale or caller-supplied privileged TrustDecision must not authorize dispatch"
    );
}

#[tokio::test]
async fn production_runtime_uses_host_policy_decision_instead_of_request_claims() {
    let registry = Arc::new(registry_with_manifest(FIRST_PARTY_REQUESTED_MANIFEST));
    let dispatcher = Arc::new(CountingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::new(
        Arc::clone(&registry),
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(privileged_local_manifest_policy()));

    let request = RuntimeCapabilityRequest::new(
        execution_context_with_dispatch_grant(TrustClass::Sandbox),
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "host policy decides"}),
        sandbox_caller_decision(),
    );

    let outcome = runtime.invoke_capability(request).await.unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id());
            assert_eq!(completed.output, json!({"ok": true}));
        }
        other => panic!("expected Completed outcome, got {other:?}"),
    }
    assert_eq!(
        dispatcher.count(),
        1,
        "host-owned trust policy should supply the effective decision before authorization"
    );
}

#[tokio::test]
async fn trust_downgrade_denies_future_invocation_before_dispatch_side_effects() {
    let registry = Arc::new(registry_with_manifest(FIRST_PARTY_REQUESTED_MANIFEST));
    let dispatcher = Arc::new(CountingDispatcher::default());
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let policy = Arc::new(privileged_local_manifest_policy());
    let runtime = DefaultHostRuntime::new(
        Arc::clone(&registry),
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::clone(&policy));

    let trusted_input = trust_input_for_registry(&registry);
    let stale_privileged_decision = policy.evaluate(&trusted_input).unwrap();
    let first = RuntimeCapabilityRequest::new(
        execution_context_with_dispatch_grant(TrustClass::Sandbox),
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "before downgrade"}),
        sandbox_caller_decision(),
    );
    let first_outcome = runtime.invoke_capability(first).await.unwrap();
    assert!(
        matches!(first_outcome, RuntimeCapabilityOutcome::Completed(_)),
        "first invocation should use the host policy's privileged decision"
    );
    assert_eq!(dispatcher.count(), 1);

    policy
        .mutate_with(
            &InvalidationBus::new(),
            trusted_input.identity.clone(),
            trusted_input.requested_authority.clone(),
            trusted_input.requested_trust,
            |sources| {
                sources.admin_remove(
                    &trusted_input.identity.package_id,
                    &trusted_input.identity.source,
                )?;
                Ok(())
            },
        )
        .unwrap();

    let second = RuntimeCapabilityRequest::new(
        execution_context_with_dispatch_grant(TrustClass::FirstParty),
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "after downgrade"}),
        stale_privileged_decision,
    );
    let second_outcome = runtime.invoke_capability(second).await.unwrap();

    assert_authorization_failed(second_outcome);
    assert_eq!(
        dispatcher.count(),
        1,
        "downgraded trust must fail closed before any second dispatch side effect"
    );
}

fn assert_authorization_failed(outcome: RuntimeCapabilityOutcome) {
    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id());
            assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
        }
        other => panic!("expected Failed(Authorization), got {other:?}"),
    }
}

#[derive(Default)]
struct CountingDispatcher {
    count: Mutex<usize>,
}

impl CountingDispatcher {
    fn count(&self) -> usize {
        *self
            .count
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl CapabilityDispatcher for CountingDispatcher {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        *self
            .count
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) += 1;
        Ok(CapabilityDispatchResult {
            capability_id: request.capability_id,
            provider: extension_id(),
            runtime: RuntimeKind::Wasm,
            output: json!({"ok": true}),
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: request.scope,
                status: ReservationStatus::Reconciled,
                estimate: request.estimate,
                actual: Some(ResourceUsage::default()),
            },
        })
    }
}

fn registry_with_manifest(manifest: &str) -> ExtensionRegistry {
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn trust_input_for_registry(registry: &ExtensionRegistry) -> TrustPolicyInput {
    registry
        .get_extension(&extension_id())
        .unwrap()
        .trust_policy_input(
            PackageSource::LocalManifest {
                path: local_manifest_path(),
            },
            None,
            None,
        )
        .unwrap()
}

fn privileged_local_manifest_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("echo").unwrap(),
            local_manifest_path(),
            None,
            HostTrustAssignment::first_party(),
            vec![EffectKind::DispatchCapability],
            None,
        ),
    ]))])
    .unwrap()
}

fn execution_context_with_dispatch_grant(trust: TrustClass) -> ExecutionContext {
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
        trust,
        grants,
        MountView::default(),
    )
    .unwrap()
}

fn sandbox_caller_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::sandbox(),
        authority_ceiling: AuthorityCeiling::empty(),
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

fn local_manifest_path() -> String {
    "/system/extensions/echo/manifest.toml".to_string()
}

const FIRST_PARTY_REQUESTED_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "first_party_requested"

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
