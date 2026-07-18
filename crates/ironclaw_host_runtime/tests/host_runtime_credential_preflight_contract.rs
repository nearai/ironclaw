//! Regression tests for credential pre-flight ordering and security properties.
//!
//! Covers:
//! - `ProductAuthAccount`-source credentials must NOT trip the secret-store
//!   pre-flight (Fix A regression: false-positive AuthRequired for connected
//!   product-auth accounts).
//! - A `RuntimeCapabilityRequest` whose `context.resource_scope` does not match
//!   the top-level context fields must be rejected before any secret-store probe
//!   (Fix B regression: forged-scope presence probe).
//!
//! These tests are intentionally kept in a dedicated file so that the coverage
//! surface for pre-flight security properties is easy to locate and extend
//! without touching the larger host_runtime_services_contract.rs.
//!
//! Helpers are duplicated from host_runtime_services_contract.rs because Rust
//! integration test binaries cannot share helpers across files without a
//! support module or re-export. The duplication is intentional and small.

mod support;

use std::sync::Arc;

use chrono::Utc;
use ironclaw_authorization::{GrantAuthorizer, in_memory_backed_capability_lease_store};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_filesystem::DiskFilesystem;
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, HostRuntime, HostRuntimeError, HostRuntimeServices,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
};
use ironclaw_processes::ProcessServices;
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_secrets::{InMemorySecretStore, SecretMaterial, SecretStore};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use serde_json::json;
use support::legacy_capability_fixture_to_v2;

// ─── Manifests ──────────────────────────────────────────────────────────────

/// A script capability that declares a required credential with
/// `source = { type = "product_auth_account", ... }`. The secret store
/// does NOT contain any material for the slot handle — but since the source is
/// `ProductAuthAccount` the pre-flight must NOT probe the store and must NOT
/// return AuthRequired.
const SCRIPT_WITH_PRODUCT_AUTH_MANIFEST: &str = r#"
id = "script"
name = "Script With Product Auth"
version = "0.1.0"
description = "Script extension that requires a product-auth account credential"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "echo"
args = []

[[capabilities]]
id = "script.echo"
description = "Echo through Script"
effects = ["dispatch_capability", "use_secret"]
default_permission = "allow"
parameters_schema = { type = "object" }

[[capabilities.runtime_credentials]]
handle = "google_oauth_token"
source = { type = "product_auth_account", provider = "google", setup = { kind = "oauth", scopes = ["https://www.googleapis.com/auth/gmail.readonly"] } }
audience = { scheme = "https", host_pattern = "gmail.googleapis.com" }
target = { type = "header", name = "Authorization" }
required = true
"#;

/// Same shape as above but with `source = { type = "secret_handle" }` — used
/// as a baseline to confirm the regular pre-flight still works alongside the
/// product-auth no-op path.
const SCRIPT_WITH_SECRET_HANDLE_MANIFEST: &str = r#"
id = "script"
name = "Script With Secret Handle"
version = "0.1.0"
description = "Script extension that requires a secret-handle credential"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "echo"
args = []

[[capabilities]]
id = "script.echo"
description = "Echo through Script"
effects = ["dispatch_capability", "use_secret"]
default_permission = "allow"
parameters_schema = { type = "object" }

[[capabilities.runtime_credentials]]
handle = "script_api_token"
source = { type = "secret_handle" }
audience = { scheme = "https", host_pattern = "api.example.com" }
target = { type = "header", name = "x-api-key" }
required = true
"#;

// ─── Helpers ────────────────────────────────────────────────────────────────

fn registry_with_manifest(manifest: &str) -> ExtensionRegistry {
    let manifest = legacy_capability_fixture_to_v2(manifest);
    let manifest = ExtensionManifest::parse(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
    )
    .expect("manifest must parse");
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    let package = ExtensionPackage::from_manifest(manifest, root).expect("package must build");
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn script_capability_id() -> CapabilityId {
    CapabilityId::new("script.echo").unwrap()
}

fn execution_context_without_grants() -> ExecutionContext {
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Script,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap()
}

fn local_manifest_trust_policy(
    extension_id: &str,
    allowed_effects: Vec<EffectKind>,
) -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new(extension_id).unwrap(),
            format!("/system/extensions/{extension_id}/manifest.toml"),
            None,
            HostTrustAssignment::user_trusted(),
            allowed_effects,
            None,
        ),
    ]))])
    .unwrap()
}

fn trust_decision_with_dispatch_authority() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

// ─── Test A-regression: ProductAuthAccount must NOT trip pre-flight ──────────

/// A required `ProductAuthAccount`-source credential must not trip the
/// secret-store pre-flight. The pre-flight probes the secret store for
/// `SecretHandle`-source credentials only; `ProductAuthAccount` credentials
/// are staged by the credential-account resolver at dispatch time.
///
/// Before Fix A, `capability_credential_requirements` pushed ALL required
/// handles (including `ProductAuthAccount` slot handles) into `required_secrets`,
/// so the pre-flight queried the store and returned `AuthRequired` for product-
/// auth accounts whose connected token lives outside the secret store.
///
/// After Fix A, only `SecretHandle`-source handles enter `required_secrets`,
/// so the pre-flight does NOT fire for `ProductAuthAccount` capabilities and
/// the flow proceeds to the approval gate.
#[tokio::test]
async fn product_auth_account_credential_does_not_trip_preflight() {
    let fs = ironclaw_run_state::in_memory_backed_run_state_filesystem();
    let run_state = Arc::new(ironclaw_run_state::FilesystemRunStateStore::new(
        std::sync::Arc::clone(&fs),
    ));
    let approval_requests = Arc::new(ironclaw_run_state::FilesystemApprovalRequestStore::new(fs));
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let secret_store = Arc::new(InMemorySecretStore::new());
    // Deliberately do NOT seed any secret under "google_oauth_token".
    // The secret store is empty. If the pre-flight incorrectly probes the
    // product-auth slot, it will return AuthRequired — which is the bug this
    // test catches.

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_WITH_PRODUCT_AUTH_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_secret_store(Arc::clone(&secret_store));
    let runtime = services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "product auth account"});

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    // The pre-flight must NOT have returned AuthRequired for a ProductAuthAccount
    // credential — the store is empty but that must not matter for this source type.
    // (The authorization layer may still deny due to missing grants, which is a
    // distinct code path from the pre-flight. The key assertion is: AuthRequired
    // was NOT returned by the credential pre-flight.)
    assert!(
        !matches!(outcome, RuntimeCapabilityOutcome::AuthRequired(_)),
        "ProductAuthAccount credential must not trip secret-store pre-flight (no store probe expected); got {outcome:?}"
    );
}

/// Baseline: a `SecretHandle`-source required credential with no secret in the
/// store MUST still trip the pre-flight and return `AuthRequired`. This confirms
/// the ProductAuthAccount exemption does not accidentally disable the pre-flight
/// for SecretHandle credentials.
#[tokio::test]
async fn secret_handle_credential_absent_still_trips_preflight() {
    let fs = ironclaw_run_state::in_memory_backed_run_state_filesystem();
    let run_state = Arc::new(ironclaw_run_state::FilesystemRunStateStore::new(
        std::sync::Arc::clone(&fs),
    ));
    let approval_requests = Arc::new(ironclaw_run_state::FilesystemApprovalRequestStore::new(fs));
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let secret_store = Arc::new(InMemorySecretStore::new());
    // No secret seeded — the SecretHandle pre-flight must fire.

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_WITH_SECRET_HANDLE_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_secret_store(Arc::clone(&secret_store));
    let runtime = services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "needs secret handle"});

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    // SecretHandle absent → pre-flight must return AuthRequired.
    match outcome {
        RuntimeCapabilityOutcome::AuthRequired(auth_gate) => {
            assert_eq!(
                auth_gate.capability_id,
                script_capability_id(),
                "auth gate must reference the invoked capability"
            );
        }
        other => panic!("expected AuthRequired for absent SecretHandle credential; got {other:?}"),
    }
}

/// Tenant-shared credentials satisfy the pre-flight (#5459): a required
/// `SecretHandle` credential the caller never provisioned must NOT return
/// `AuthRequired` when an admin seeded it at the tenant-shared managed scope
/// (the `IRONCLAW_REBORN_DEV_SECRET__<handle>` env-provisioning path in
/// `serve`). This drives the full `invoke_capability` caller so the
/// caller-scope→tenant-shared fallback is exercised through
/// `credential_preflight_check`, not just the `secret_owner_scope` helper.
#[tokio::test]
async fn tenant_shared_secret_satisfies_credential_preflight() {
    let fs = ironclaw_run_state::in_memory_backed_run_state_filesystem();
    let run_state = Arc::new(ironclaw_run_state::FilesystemRunStateStore::new(
        std::sync::Arc::clone(&fs),
    ));
    let approval_requests = Arc::new(ironclaw_run_state::FilesystemApprovalRequestStore::new(fs));
    let capability_leases = Arc::new(in_memory_backed_capability_lease_store());
    let secret_store = Arc::new(InMemorySecretStore::new());

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_WITH_SECRET_HANDLE_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    )))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_secret_store(Arc::clone(&secret_store));
    let runtime = services.host_runtime_for_local_testing();
    let context = execution_context_without_grants();

    // Admin set the key ONLY at the tenant-shared scope; the caller's own
    // scope has nothing. The baseline test above proves this exact setup
    // WITHOUT the shared secret returns AuthRequired.
    secret_store
        .put(
            context.resource_scope.tenant_shared_managed_scope(),
            SecretHandle::new("script_api_token").unwrap(),
            SecretMaterial::from("shared-admin-key"),
            None,
        )
        .await
        .unwrap();

    let estimate = ResourceEstimate::default();
    let input = json!({"message": "tenant-shared key present"});

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert!(
        !matches!(outcome, RuntimeCapabilityOutcome::AuthRequired(_)),
        "tenant-shared admin key must satisfy the credential pre-flight for every caller in the tenant; got {outcome:?}"
    );
}

// ─── Test B-regression: forged scope rejected before secret-store probe ──────

/// An `invoke_capability` call with a mismatched `resource_scope` (i.e. the
/// `invocation_id` in `resource_scope` differs from `context.invocation_id`)
/// must return `Err(HostRuntimeError::InvalidRequest)` BEFORE any secret-store
/// probe. This closes a forged-scope presence-probe window.
///
/// We construct the mismatch by building two contexts (each gets a fresh
/// `InvocationId`) and swapping the `resource_scope` from one onto the other.
#[tokio::test]
async fn invoke_capability_forged_scope_fails_before_preflight() {
    let secret_store = Arc::new(InMemorySecretStore::new());

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_WITH_SECRET_HANDLE_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    )))
    .with_secret_store(Arc::clone(&secret_store));
    let runtime = services.host_runtime_for_local_testing();

    // Build two separate contexts — each gets a fresh InvocationId.
    let context_a = execution_context_without_grants();
    let context_b = execution_context_without_grants();

    // Forge: take the resource_scope from context_b and put it on context_a's
    // fields. The invocation_id in the scope will not match context_a's
    // invocation_id.
    let forged_context = ExecutionContext {
        run_id: None,
        resource_scope: context_b.resource_scope.clone(), // mismatched scope
        ..context_a
    };

    // Sanity: validate() must reject this combination.
    assert!(
        forged_context.validate().is_err(),
        "forged context must fail validate() — if this panics the test setup is wrong"
    );

    let estimate = ResourceEstimate::default();
    let input = json!({"message": "forged scope"});

    let result = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            forged_context,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await;

    // Must be Err — the context validation must fire before the secret-store probe.
    match result {
        Err(HostRuntimeError::InvalidRequest { .. }) => {
            // Expected: context validation fired.
        }
        Ok(outcome) => {
            panic!("expected Err(InvalidRequest) for forged-scope invocation; got Ok({outcome:?})")
        }
        Err(other) => {
            panic!("expected Err(InvalidRequest) for forged-scope invocation; got Err({other:?})")
        }
    }
}

/// Same forged-scope test through the `spawn_capability` path.
#[tokio::test]
async fn spawn_capability_forged_scope_fails_before_preflight() {
    let secret_store = Arc::new(InMemorySecretStore::new());

    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_WITH_SECRET_HANDLE_MANIFEST)),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    )))
    .with_secret_store(Arc::clone(&secret_store));
    let runtime = services.host_runtime_for_local_testing();

    let context_a = execution_context_without_grants();
    let context_b = execution_context_without_grants();

    let forged_context = ExecutionContext {
        run_id: None,
        resource_scope: context_b.resource_scope.clone(),
        ..context_a
    };

    assert!(
        forged_context.validate().is_err(),
        "forged context must fail validate() — if this panics the test setup is wrong"
    );

    let estimate = ResourceEstimate::default();
    let input = json!({"message": "forged scope on spawn"});

    let result = runtime
        .spawn_capability(RuntimeCapabilityRequest::new(
            forged_context,
            script_capability_id(),
            estimate,
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await;

    match result {
        Err(HostRuntimeError::InvalidRequest { .. }) => {
            // Expected.
        }
        Ok(outcome) => {
            panic!("expected Err(InvalidRequest) for forged-scope spawn; got Ok({outcome:?})")
        }
        Err(other) => {
            panic!("expected Err(InvalidRequest) for forged-scope spawn; got Err({other:?})")
        }
    }
}
