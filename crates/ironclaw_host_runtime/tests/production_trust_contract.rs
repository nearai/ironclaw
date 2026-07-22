mod support;

use support::legacy_capability_fixture_to_v2;

use std::sync::Arc;

use ironclaw_authorization::{GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_host_api::dispatch_test_support::TestDispatcher;
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, DefaultHostRuntime, HostRuntime, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeFailureKind,
};
use ironclaw_trust::{
    AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy, InvalidationBus,
    TrustPolicyInput,
};
use serde_json::json;

fn local_test_runtime_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    ironclaw_runtime_policy::resolve(ironclaw_runtime_policy::ResolveRequest::new(
        ironclaw_host_api::runtime_policy::DeploymentMode::LocalSingleUser,
        ironclaw_host_api::runtime_policy::RuntimeProfile::LocalDev,
    ))
    .unwrap()
}

// The former `production_runtime_ignores_caller_supplied_privileged_trust_decision`
// test forged a caller-supplied `trust_decision` to prove the host ignored it.
// That attack surface is now structurally eliminated: the request types no longer
// carry a `trust_decision` field, so there is nothing for a caller to forge — the
// host always evaluates trust itself.

#[tokio::test]
async fn production_runtime_uses_host_policy_decision_instead_of_request_claims() {
    let registry = Arc::new(registry_with_manifest(LOCAL_INSTALLED_MANIFEST));
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let runtime = DefaultHostRuntime::new(
        Arc::clone(&registry),
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(privileged_local_manifest_policy()));

    let request = RuntimeCapabilityRequest::new(
        execution_context_with_dispatch_grant(TrustClass::Sandbox),
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "host policy decides"}),
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
        dispatcher.call_count(),
        1,
        "host-owned trust policy should supply the effective decision before authorization"
    );
}

#[tokio::test]
async fn trust_downgrade_denies_future_invocation_before_dispatch_side_effects() {
    let registry = Arc::new(registry_with_manifest(LOCAL_INSTALLED_MANIFEST));
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let policy = Arc::new(privileged_local_manifest_policy());
    let runtime = DefaultHostRuntime::new(
        Arc::clone(&registry),
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::clone(&policy));

    let trusted_input = trust_input_for_registry(&registry);
    let first = RuntimeCapabilityRequest::new(
        execution_context_with_dispatch_grant(TrustClass::Sandbox),
        capability_id(),
        ResourceEstimate::default(),
        json!({"message": "before downgrade"}),
    );
    let first_outcome = runtime.invoke_capability(first).await.unwrap();
    assert!(
        matches!(first_outcome, RuntimeCapabilityOutcome::Completed(_)),
        "first invocation should use the host policy's privileged decision"
    );
    assert_eq!(dispatcher.call_count(), 1);

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
    );
    let second_outcome = runtime.invoke_capability(second).await.unwrap();

    assert_authorization_failed(second_outcome);
    assert_eq!(
        dispatcher.call_count(),
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

fn dispatch_result() -> CapabilityDispatchResult {
    CapabilityDispatchResult {
        capability_id: capability_id(),
        provider: extension_id(),
        runtime: RuntimeKind::Wasm,
        output: json!({"ok": true}),
        display_preview: None,
        usage: ResourceUsage::default(),
        receipt: ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: ResourceScope::system(),
            status: ReservationStatus::Reconciled,
            estimate: ResourceEstimate::default(),
            actual: Some(ResourceUsage::default()),
        },
    }
}

fn registry_with_manifest(manifest: &str) -> ExtensionRegistry {
    let manifest = parse_manifest(manifest);
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
    let mut context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        trust,
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

fn local_manifest_path() -> String {
    "/system/extensions/echo/manifest.toml".to_string()
}

const LOCAL_INSTALLED_MANIFEST: &str = r#"
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
