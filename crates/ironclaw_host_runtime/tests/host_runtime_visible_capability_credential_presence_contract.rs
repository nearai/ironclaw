//! Caller-level regression tests for `visible_capabilities` credential-presence
//! downgrade to `VisibleCapabilityAccess::NeedsAuth` (issue #5416, Phase 2).
//!
//! `crates/ironclaw_host_runtime/src/surface.rs`'s unit tests pin the
//! downgrade/fingerprint behavior against a fake `CapabilityCredentialPresence`.
//! These tests drive the real production composition instead — a
//! `HostRuntimeServices`-built `DefaultHostRuntime` backed by a real
//! `SecretStore` and a real `RuntimeCredentialAccountResolver` — so the wiring
//! in `services.rs`/`production.rs` (not just the trait contract) is covered.
//! Per `.claude/rules/testing.md` ("Test Through the Caller, Not Just the
//! Helper"): `ProductionCredentialPresence` is a transform with more than one
//! input (secret store presence, resolver outcome) gating a model-visible
//! signal, with `HostRuntimeServices::build_host_runtime` as the wrapper
//! between it and the surface — a unit test on the helper alone would not
//! catch a wiring regression in that wrapper.
//!
//! Helpers/manifests are duplicated from `host_runtime_credential_preflight_contract.rs`
//! (same convention documented there: Rust integration test binaries cannot
//! share helpers across files without a support module, and the duplication is
//! small and intentional).

mod support;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CapabilitySurfacePolicy, CapabilitySurfaceVersion, HostRuntime, HostRuntimeServices,
    RuntimeCredentialAccessSecret, RuntimeCredentialAccountRequest,
    RuntimeCredentialAccountResolver, SurfaceKind, VisibleCapabilityAccess,
    VisibleCapabilityRequest,
};
use ironclaw_processes::ProcessServices;
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_secrets::{
    InMemorySecretStore, SecretLease, SecretLeaseId, SecretMaterial, SecretMetadata, SecretStore,
    SecretStoreError,
};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use support::legacy_capability_fixture_to_v2;
use tempfile::TempDir;

// ─── Manifests (duplicated from host_runtime_credential_preflight_contract.rs) ──

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

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Builds a registry from a v1-style test manifest, plus the backing
/// `LocalFilesystem` with real schema/prompt files written under a tempdir —
/// `visible_capabilities` (unlike `invoke_capability`, which the sibling
/// `host_runtime_credential_preflight_contract.rs` file drives) resolves each
/// descriptor's `input_schema_ref` against the wired surface filesystem, so
/// the referenced files must actually exist on disk. The returned `TempDir`
/// must be kept alive for the duration of the test (dropping it deletes the
/// files).
fn registry_with_manifest(manifest: &str) -> (TempDir, LocalFilesystem, ExtensionRegistry) {
    let manifest_text = legacy_capability_fixture_to_v2(manifest);
    let manifest = ExtensionManifest::parse(
        &manifest_text,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
    )
    .expect("manifest must parse");

    let storage = tempfile::tempdir().unwrap();
    let extension_dir = storage.path().join(manifest.id.as_str());
    std::fs::create_dir_all(extension_dir.join("schemas/test")).unwrap();
    std::fs::create_dir_all(extension_dir.join("prompts")).unwrap();
    std::fs::write(
        extension_dir.join("schemas/test/input.v1.json"),
        r#"{"type":"object"}"#,
    )
    .unwrap();
    std::fs::write(
        extension_dir.join("schemas/test/output.v1.json"),
        r#"{"type":"object"}"#,
    )
    .unwrap();
    std::fs::write(extension_dir.join("prompts/test.md"), "Test prompt").unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();

    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    let package = ExtensionPackage::from_manifest(manifest, root).expect("package must build");
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    (storage, fs, registry)
}

fn script_capability_id() -> CapabilityId {
    CapabilityId::new("script.echo").unwrap()
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

/// Execution context carrying a grant for `script.echo` with the given
/// effects, so `GrantAuthorizer` (the real authorizer, not a fake) returns
/// `Allow` and the credential-presence downgrade is the only thing gating
/// `NeedsAuth` vs `Available`.
///
/// `secrets` mirrors `GrantConstraints.secrets` — the caller's AUTHORITY to
/// use a `SecretHandle`-source credential slot. This is orthogonal to whether
/// the secret's material is actually stored (`SecretStore` presence): a grant
/// can authorize the slot while the store is empty (never completed a
/// paste-token flow) — exactly the "authorized but credential missing"
/// scenario Phase 2 covers. `ironclaw_authorization::obligations_for_grant`
/// denies with `PolicyDenied` (not `NeedsAuth`) if a required `SecretHandle`
/// credential's handle is absent from `secrets`, so callers exercising a
/// `SecretHandle`-source capability must include it here.
fn execution_context_with_script_grant(
    effects: Vec<EffectKind>,
    secrets: Vec<SecretHandle>,
) -> ExecutionContext {
    let grants = CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: script_capability_id(),
            grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: effects,
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets,
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }],
    };
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Script,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap()
}

fn visible_request(context: ExecutionContext) -> VisibleCapabilityRequest {
    let mut provider_trust = std::collections::BTreeMap::new();
    provider_trust.insert(
        ExtensionId::new("script").unwrap(),
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::Default,
            evaluated_at: Utc::now(),
        },
    );
    VisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap())
        .with_policy(CapabilitySurfacePolicy::allow_all())
        .with_provider_trust(provider_trust)
}

#[derive(Debug)]
struct FixedRuntimeCredentialAccountResolver {
    result: Result<SecretHandle, CredentialStageError>,
}

#[async_trait]
impl RuntimeCredentialAccountResolver for FixedRuntimeCredentialAccountResolver {
    async fn resolve_access_secret(
        &self,
        request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError> {
        self.result
            .clone()
            .map(|handle| RuntimeCredentialAccessSecret {
                scope: request.scope.clone(),
                handle,
            })
    }

    async fn account_configured(
        &self,
        _request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<bool, CredentialStageError> {
        match &self.result {
            Ok(_) => Ok(true),
            Err(CredentialStageError::AuthRequired) => Ok(false),
            Err(CredentialStageError::Backend) => Err(CredentialStageError::Backend),
        }
    }
}

/// Regression guard for issue #5416 Phase 2 Fix A: the capability-surface
/// presence check must call the side-effect-free `account_configured`, never
/// the refresh-performing `resolve_access_secret`. `resolve_access_secret`
/// panics on any call here so a regression to the old wiring fails loudly
/// (a real "silently refreshes tokens on every render" bug would otherwise be
/// invisible from the surface's `NeedsAuth`/`Available` output alone).
#[derive(Debug, Default)]
struct PanicsOnResolveAccountResolver {
    resolve_access_secret_calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl RuntimeCredentialAccountResolver for PanicsOnResolveAccountResolver {
    async fn resolve_access_secret(
        &self,
        _request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError> {
        self.resolve_access_secret_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        panic!(
            "resolve_access_secret must never be called from the capability-surface \
             presence path — it performs a token refresh (a side effect); the presence \
             path must use account_configured instead"
        );
    }

    async fn account_configured(
        &self,
        _request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<bool, CredentialStageError> {
        Ok(false)
    }
}

/// `SecretStore` wrapper whose `metadata` call fails while `fail.load()` is
/// true and otherwise delegates to a real `InMemorySecretStore`. Used to prove
/// the fail-open + never-cache contract from the caller (not just the
/// `ProductionCredentialPresence` unit level): a `metadata` backend error must
/// leave the capability `Available` AND must not populate the presence cache,
/// so a subsequent render against a healthy store gets the fresh (not stale)
/// answer.
#[derive(Debug, Default)]
struct ToggleableFailureSecretStore {
    inner: InMemorySecretStore,
    fail: std::sync::atomic::AtomicBool,
}

impl ToggleableFailureSecretStore {
    fn set_failing(&self, failing: bool) {
        self.fail
            .store(failing, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait]
impl SecretStore for ToggleableFailureSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
        expires_at: Option<Timestamp>,
    ) -> Result<SecretMetadata, SecretStoreError> {
        self.inner.put(scope, handle, material, expires_at).await
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(SecretStoreError::StoreUnavailable {
                reason: "simulated backend outage".to_string(),
            });
        }
        self.inner.metadata(scope, handle).await
    }

    async fn metadata_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
        self.inner.metadata_for_scope(scope).await
    }

    async fn delete(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        self.inner.delete(scope, handle).await
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        self.inner.lease_once(scope, handle).await
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        self.inner.consume(scope, lease_id).await
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        self.inner.revoke(scope, lease_id).await
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        self.inner.leases_for_scope(scope).await
    }
}

fn only_capability(
    surface: &ironclaw_host_runtime::VisibleCapabilitySurface,
) -> VisibleCapabilityAccess {
    assert_eq!(
        surface.capabilities.len(),
        1,
        "expected exactly one visible capability, got {:?}",
        surface.capabilities
    );
    surface.capabilities[0].access
}

// ─── Tests ──────────────────────────────────────────────────────────────────

/// A capability requiring a `SecretHandle` credential with no matching secret
/// in the store must surface as `NeedsAuth`, not `Available` — driven through
/// `HostRuntimeServices::build_host_runtime` (the real production wiring),
/// not just the `CapabilityCredentialPresence` trait in isolation.
#[tokio::test]
async fn generic_secret_capability_needs_auth_when_secret_absent() {
    let secret_store = Arc::new(InMemorySecretStore::new());
    // Deliberately do NOT seed "script_api_token" — the store is empty.

    let (_storage, fs, registry) = registry_with_manifest(SCRIPT_WITH_SECRET_HANDLE_MANIFEST);
    let services = HostRuntimeServices::new(
        Arc::new(registry),
        Arc::new(fs),
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

    let context = execution_context_with_script_grant(
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
        vec![SecretHandle::new("script_api_token").unwrap()],
    );
    let surface = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    assert_eq!(
        only_capability(&surface),
        VisibleCapabilityAccess::NeedsAuth
    );
}

/// A capability requiring a product-auth account credential whose resolver
/// reports `AuthRequired` (account missing/unconfigured/expired/revoked) must
/// surface as `NeedsAuth`.
#[tokio::test]
async fn product_auth_capability_needs_auth_when_resolver_reports_auth_required() {
    let resolver = Arc::new(FixedRuntimeCredentialAccountResolver {
        result: Err(CredentialStageError::AuthRequired),
    });

    let (_storage, fs, registry) = registry_with_manifest(SCRIPT_WITH_PRODUCT_AUTH_MANIFEST);
    let services = HostRuntimeServices::new(
        Arc::new(registry),
        Arc::new(fs),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    )))
    .with_runtime_credential_account_resolver(resolver);
    let runtime = services.host_runtime_for_local_testing();

    let context = execution_context_with_script_grant(
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
        Vec::new(),
    );
    let surface = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    assert_eq!(
        only_capability(&surface),
        VisibleCapabilityAccess::NeedsAuth
    );
}

/// A resolver `Backend` error (internal staging failure, not attributable to
/// the user's credentials) must fail open — the capability stays `Available`
/// rather than falsely prompting sign-in for a transient backend blip.
#[tokio::test]
async fn product_auth_capability_stays_available_when_resolver_backend_error() {
    let resolver = Arc::new(FixedRuntimeCredentialAccountResolver {
        result: Err(CredentialStageError::Backend),
    });

    let (_storage, fs, registry) = registry_with_manifest(SCRIPT_WITH_PRODUCT_AUTH_MANIFEST);
    let services = HostRuntimeServices::new(
        Arc::new(registry),
        Arc::new(fs),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    )))
    .with_runtime_credential_account_resolver(resolver);
    let runtime = services.host_runtime_for_local_testing();

    let context = execution_context_with_script_grant(
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
        Vec::new(),
    );
    let surface = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    assert_eq!(
        only_capability(&surface),
        VisibleCapabilityAccess::Available
    );
}

/// Issue #5416 Phase 2 Fix A (BLOCKER): the capability-surface presence check
/// must be side-effect-free. `visible_capabilities` runs on every LLM step —
/// if it called `resolve_access_secret` (which performs an OAuth token
/// refresh) instead of the presence-only `account_configured`, every render
/// would burn a network round-trip. `PanicsOnResolveAccountResolver` panics
/// if the old (wrong) method is ever invoked, so this test fails loudly
/// instead of silently passing while re-introducing the side effect.
#[tokio::test]
async fn product_auth_capability_presence_check_never_calls_resolve_access_secret() {
    let resolver = Arc::new(PanicsOnResolveAccountResolver::default());

    let (_storage, fs, registry) = registry_with_manifest(SCRIPT_WITH_PRODUCT_AUTH_MANIFEST);
    let services = HostRuntimeServices::new(
        Arc::new(registry),
        Arc::new(fs),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy(
        "script",
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    )))
    .with_runtime_credential_account_resolver(Arc::clone(&resolver));
    let runtime = services.host_runtime_for_local_testing();

    let context = execution_context_with_script_grant(
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
        Vec::new(),
    );
    let surface = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();

    assert_eq!(
        only_capability(&surface),
        VisibleCapabilityAccess::NeedsAuth,
        "account_configured returned Ok(false) so the capability must downgrade"
    );
    assert_eq!(
        resolver
            .resolve_access_secret_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "capability-surface presence check must never invoke resolve_access_secret"
    );
}

/// PR #5528 review regression (fail-open coverage gap): a `SecretStore::metadata`
/// backend error must fail open (capability stays `Available`, matching
/// `generic_secret_capability_needs_auth_when_secret_absent`'s Ok(None) case
/// staying missing) AND must not populate the credential-presence cache.
/// Proven from the caller (`HostRuntimeServices::build_host_runtime`'s real
/// wiring, not just the `ProductionCredentialPresence` unit) by rendering
/// twice against the SAME runtime instance (so the cache persists across
/// renders): first while the store fails (must stay `Available`), then after
/// the store recovers (must get the FRESH answer — `NeedsAuth`, since the
/// secret was never actually stored). If the first render had wrongly cached
/// anything, the second render would still report the render-1 outcome
/// instead of re-probing live.
#[tokio::test]
async fn generic_secret_capability_stays_available_and_uncached_when_metadata_errors() {
    let secret_store = Arc::new(ToggleableFailureSecretStore::default());
    secret_store.set_failing(true);

    let (_storage, fs, registry) = registry_with_manifest(SCRIPT_WITH_SECRET_HANDLE_MANIFEST);
    let services = HostRuntimeServices::new(
        Arc::new(registry),
        Arc::new(fs),
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

    let context = execution_context_with_script_grant(
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
        vec![SecretHandle::new("script_api_token").unwrap()],
    );

    let surface_while_failing = runtime
        .visible_capabilities(visible_request(context.clone()))
        .await
        .unwrap();
    assert_eq!(
        only_capability(&surface_while_failing),
        VisibleCapabilityAccess::Available,
        "a metadata backend error must fail open, not report NeedsAuth"
    );

    secret_store.set_failing(false);
    let surface_after_recovery = runtime
        .visible_capabilities(visible_request(context))
        .await
        .unwrap();
    assert_eq!(
        only_capability(&surface_after_recovery),
        VisibleCapabilityAccess::NeedsAuth,
        "the indeterminate render must not have cached anything — once the store \
         recovers, the capability must get the fresh (never-stored) answer, not a \
         stale value carried over from the failing render"
    );
}
