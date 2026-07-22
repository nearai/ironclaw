//! Integration test: a production-shaped `RebornRuntime` wires the SSO / admin
//! identity resolver over its production store graph, and that resolver is
//! multi-tenant isolated.
//!
//! Regression for the bucket-2 production-parity gap (#5013): production
//! profiles have `local_runtime: None` and `production_runtime: Some(..)`, and
//! before `open_reborn_identity_resolver` grew a production fallback it
//! `?`-returned on `local_runtime` and therefore yielded `None` on *every*
//! production build — so SSO login and the admin user directory were dead on
//! production profiles. This test drives the accessor through a fully-built
//! production runtime and asserts (1) it is wired (returns `Some(Ok(..))`),
//! (2) it persists round-trip over the production
//! `/tenant-shared/reborn-identity` mount, and (3) records are partitioned by
//! the per-call tenant so one owner-scoped store cannot leak identities across
//! tenants.
//!
//! Lives in its own integration-test binary (mirroring
//! `production_runtime_automations.rs`) so the CPU-heavy production build does
//! not starve the lib unit tests' hard `RunTimeout` budgets, and is gated on
//! `libsql` because the production-runtime path under test requires the libSQL
//! substrate.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::{
    TenantId,
    runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
        NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
    },
};
use ironclaw_host_runtime::{
    CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError, SandboxCommandTransport,
    TenantSandboxProcessPort,
};
use ironclaw_reborn_composition::{
    ExternalSubjectId, ProviderKind, RebornBuildInput, RebornCompositionProfile,
    RebornRuntimeIdentity, RebornRuntimeInput, RebornRuntimeProcessBinding,
    ResolveExternalIdentity, SurfaceKind, build_reborn_runtime, builtin_first_party_trust_policy,
};

// ─── minimal sandbox transport stub (production requires a process binding) ───

#[derive(Debug)]
struct RecordingSandboxTransport;

#[async_trait]
impl SandboxCommandTransport for RecordingSandboxTransport {
    async fn run_command(
        &self,
        _request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        Ok(CommandExecutionOutput {
            output: String::new(),
            saved_output: None,
            exit_code: 0,
            sandboxed: true,
            duration: Duration::ZERO,
        })
    }
}

/// A verified-email OAuth identity for `subject` under `tenant`. Email is held
/// constant across tenants on purpose: the isolation assertion varies *only*
/// the tenant, so a same-email/same-subject pair minting distinct users proves
/// the store partitions by tenant rather than deduping globally.
fn oauth_identity(tenant: &TenantId, subject: &str) -> ResolveExternalIdentity {
    ResolveExternalIdentity {
        tenant_id: tenant.clone(),
        surface_kind: SurfaceKind::Oauth,
        provider_kind: ProviderKind::new("google").expect("provider"),
        provider_instance_id: None,
        external_subject_id: ExternalSubjectId::new(subject).expect("subject"),
        email: Some("alice@example.com".to_string()),
        email_verified: true,
        display_name: Some("Alice".to_string()),
    }
}

/// Regression guard for the production identity-resolver wiring (#5013) plus
/// its cross-tenant isolation. Before the production fallback,
/// `open_reborn_identity_resolver` returned `None` here.
#[tokio::test]
async fn production_runtime_wires_identity_resolver_and_isolates_tenants() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("reborn.db"))
            .build()
            .await
            .expect("libsql db"),
    );

    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            "prod-identity-owner",
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
        )
        .with_production_trust_policy(Arc::new(
            builtin_first_party_trust_policy().expect("trust policy"),
        ))
        .with_runtime_policy(EffectiveRuntimePolicy {
            deployment: DeploymentMode::HostedMultiTenant,
            requested_profile: RuntimeProfile::SecureDefault,
            resolved_profile: RuntimeProfile::SecureDefault,
            filesystem_backend: FilesystemBackendKind::ScopedVirtual,
            process_backend: ProcessBackendKind::TenantSandbox,
            network_mode: NetworkMode::Deny,
            secret_mode: SecretMode::BrokeredHandles,
            approval_policy: ApprovalPolicy::AskAlways,
            audit_mode: AuditMode::Standard,
        })
        .with_runtime_process_binding(RebornRuntimeProcessBinding::tenant_sandbox(Arc::new(
            TenantSandboxProcessPort::new(Arc::new(RecordingSandboxTransport)),
        ))),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "prod-identity-runtime-tenant".to_string(),
        agent_id: "prod-identity-runtime-agent".to_string(),
        source_binding_id: "prod-identity-source".to_string(),
        reply_target_binding_id: "prod-identity-reply".to_string(),
    });

    let runtime = build_reborn_runtime(input)
        .await
        .expect("production runtime builds");

    let tenant_a = TenantId::new("prod-identity-tenant-a").expect("tenant a");
    let tenant_b = TenantId::new("prod-identity-tenant-b").expect("tenant b");

    // (1) THE WIRING. On a production build `local_runtime` is `None`; the
    // accessor must fall back to the production store graph and hand back a
    // resolver instead of `None`. This `expect` on the outer `Option` is the
    // core regression — it panicked before the production fallback existed.
    let resolver = runtime
        .open_reborn_identity_resolver(&tenant_a)
        .await
        .expect("production runtime must wire the identity resolver (#5013)")
        .expect("resolver opens over the production store graph");

    // (2) PERSISTENCE ROUND-TRIP over the production
    // `/tenant-shared/reborn-identity` mount: first contact mints a user, and
    // re-resolving the same external identity returns the same canonical id
    // (proves the mount is reachable and writes are durable, not a no-op).
    let user_a = resolver
        .resolve_or_create(oauth_identity(&tenant_a, "google-sub-1"))
        .await
        .expect("first contact mints a user over the production substrate");
    let user_a_again = resolver
        .resolve_or_create(oauth_identity(&tenant_a, "google-sub-1"))
        .await
        .expect("re-resolution succeeds");
    assert_eq!(
        user_a, user_a_again,
        "same external identity resolves to the same canonical user id"
    );

    // (3) MULTI-TENANT ISOLATION. The runtime runs one fixed owner scope, so a
    // single owner-scoped store serves every tenant; isolation must come purely
    // from the per-call tenant path partitioning. The SAME external subject
    // (and email) under a DIFFERENT tenant must mint a DIFFERENT user — tenant B
    // cannot observe tenant A's identity record. A failure here is a
    // cross-tenant identity leak, not a flake.
    let user_b = resolver
        .resolve_or_create(oauth_identity(&tenant_b, "google-sub-1"))
        .await
        .expect("tenant b mints its own user");
    assert_ne!(
        user_a, user_b,
        "an identical external subject in a different tenant must not resolve to \
         tenant A's user (cross-tenant identity isolation)"
    );

    // (4) ADMIN USER DIRECTORY WIRING (#5013). `RebornRuntime::reborn_user_directory`
    // got the identical `None`→production-fallback fix as the resolver above, so it
    // must be wired (return `Some`) on a production build and enumerate exactly the
    // users SSO login persists — under the same per-call tenant partitioning. Tenant
    // A's page contains `user_a` and never `user_b` (which was minted under tenant B);
    // a leak here is a cross-tenant directory read, not a flake.
    let directory = runtime
        .reborn_user_directory_for_tests()
        .expect("production runtime must wire the admin user directory (#5013)");
    let tenant_a_users = directory
        .list_users(&tenant_a, None, None, 10)
        .await
        .expect("list_users succeeds");
    assert!(
        tenant_a_users.iter().any(|u| u.user_id == user_a),
        "tenant A's directory page must contain the user SSO login minted for tenant A"
    );
    assert!(
        tenant_a_users.iter().all(|u| u.user_id != user_b),
        "tenant A's directory page must not expose tenant B's user (cross-tenant isolation)"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}
