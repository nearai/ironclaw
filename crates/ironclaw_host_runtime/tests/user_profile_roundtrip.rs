//! Caller-level integration test for the user-profile round trip:
//! `builtin.profile_set` (writer) → `MemoryBackedUserProfileSource` (reader) →
//! `LoopRuntimeContext::render_model_content()` (render).
//!
//! This test exercises the full real-capability dispatch path (not just
//! `profile_set.rs` unit tests) and proves the scope-narrowing invariant:
//! a write dispatched with an **agent- and project-scoped** `ResourceScope`
//! still lands at the user-only scope `(tenant, user, agent=None,
//! project=None)` and is visible to the reader that always keys by the
//! user-only scope.
//!
//! Limitation note: `MemoryBackedUserProfileSource` does NOT implement
//! `HostUserProfileSource` directly in `ironclaw_host_runtime` (that impl
//! lives in the composition layer to avoid a circular crate dependency).
//! The test calls `MemoryBackedUserProfileSource::resolve_user_profile()`
//! directly, which is the production code path that the trait simply delegates
//! to. This is the same production code executed in composition; no
//! functionality is skipped.

use std::sync::Arc;

use chrono::Utc;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_events::InMemoryAuditSink;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::{
    AgentId, CapabilityGrantId, CapabilitySet, EffectKind, ExecutionContext, ExtensionId,
    GrantConstraints, MountAlias, MountGrant, MountPermissions, MountView, NetworkPolicy,
    PackageId, Principal, ProjectId, ResourceEstimate, RunId, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeKind,
    TenantId, ThreadId, TrustClass, UserId, VirtualPath,
    runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
        NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
    },
};
use ironclaw_host_runtime::builtin_first_party_package;
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, HostRuntime, HostRuntimeServices, MemoryBackedUserProfileSource,
    PROFILE_SET_CAPABILITY_ID, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    builtin_first_party_handlers,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_triggers::InMemoryTriggerRepository;
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
use ironclaw_turns::{
    RunProfileResolver, TurnActor, TurnId, TurnRunId, TurnScope,
    run_profile::{InMemoryRunProfileResolver, LoopRuntimeContext, RunProfileResolutionRequest},
};
use serde_json::json;

// ── Noop HTTP egress (profile_set does not need network) ──

struct NoopRuntimeHttpEgress;

#[async_trait::async_trait]
impl RuntimeHttpEgress for NoopRuntimeHttpEgress {
    async fn execute(
        &self,
        _request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        Err(RuntimeHttpEgressError::Network {
            reason: "noop egress: network not available in this test".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        })
    }
}

// ── Shared test helpers ──

fn local_dev_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

fn trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("builtin").unwrap(),
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::DeleteFilesystem,
                EffectKind::Network,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::ExternalWrite,
            ],
            None,
        ),
    ]))])
    .unwrap()
}

fn memory_mounts() -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/memory").unwrap(),
        VirtualPath::new("/memory").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap()
}

fn dispatch_grant_with_mounts(
    capability: &str,
    mounts: MountView,
) -> ironclaw_host_api::CapabilityGrant {
    ironclaw_host_api::CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: ironclaw_host_api::CapabilityId::new(capability).unwrap(),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::DeleteFilesystem,
                EffectKind::Network,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::ExternalWrite,
            ],
            mounts,
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

/// Build an `ExecutionContext` scoped to `(tenant, user, agent, project)`.
/// This simulates a run initiated in an agent+project context, which is the
/// key scenario for the scope-narrowing test: the capability write uses the
/// request's `ResourceScope` (agent+project), but `profile_scope_and_path`
/// inside the handler drops agent/project and writes to `(tenant, user, None, None)`.
fn agent_scoped_context(
    tenant_id: &str,
    user_id: &str,
    agent_id: &str,
    project_id: &str,
) -> ExecutionContext {
    let mounts = memory_mounts();
    let capability_set = CapabilitySet {
        grants: vec![dispatch_grant_with_mounts(
            PROFILE_SET_CAPABILITY_ID,
            mounts.clone(),
        )],
    };
    let mut ctx = ExecutionContext::local_default(
        UserId::new(user_id).unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        capability_set,
        mounts,
    )
    .unwrap();
    ctx.tenant_id = TenantId::new(tenant_id).unwrap();
    ctx.user_id = UserId::new(user_id).unwrap();
    ctx.agent_id = Some(AgentId::new(agent_id).unwrap());
    ctx.project_id = Some(ProjectId::new(project_id).unwrap());
    ctx.run_id = Some(RunId::new());
    ctx.resource_scope.tenant_id = TenantId::new(tenant_id).unwrap();
    ctx.resource_scope.user_id = UserId::new(user_id).unwrap();
    ctx.resource_scope.agent_id = Some(AgentId::new(agent_id).unwrap());
    ctx.resource_scope.project_id = Some(ProjectId::new(project_id).unwrap());
    ctx
}

/// Build a test `LoopRunContext` with an actor, mirroring the pattern in
/// `user_profile_source.rs` module tests.
async fn loop_run_context_with_user(
    tenant_id: &str,
    user_id: &str,
) -> ironclaw_turns::run_profile::LoopRunContext {
    let resolved_run_profile = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let scope = TurnScope::new(
        TenantId::new(tenant_id).unwrap(),
        None,
        None,
        ThreadId::new("thread-profile-roundtrip-test").unwrap(),
    );
    let actor = TurnActor::new(UserId::new(user_id).unwrap());
    ironclaw_turns::run_profile::LoopRunContext::new(
        scope,
        TurnId::new(),
        TurnRunId::new(),
        resolved_run_profile,
    )
    .with_actor(actor)
}

fn build_runtime(shared_fs: Arc<InMemoryBackend>) -> impl HostRuntime {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(builtin_first_party_package().unwrap())
        .unwrap();
    HostRuntimeServices::new(
        Arc::new(registry),
        shared_fs,
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_http_egress(Arc::new(NoopRuntimeHttpEgress))
    .with_audit_sink(Arc::new(InMemoryAuditSink::new()))
    .with_runtime_policy(local_dev_policy())
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

// ── Round-trip test ──

/// End-to-end round trip: dispatch `builtin.profile_set` through the real
/// capability dispatch path for an AGENT+PROJECT-scoped run, then read back
/// via `MemoryBackedUserProfileSource`, and assert `render_model_content()`
/// renders the expected strings.
///
/// Scope-narrowing proof:
/// - Write dispatched with `ResourceScope { agent_id: Some("test-agent"), project_id: Some("test-project"), … }`.
/// - Inside the handler, `profile_merge_write` calls `profile_scope_and_path(tenant, user)`,
///   which passes `agent=None, project=None` to `MemoryDocumentScope/Path` constructors.
/// - `MemoryBackedUserProfileSource::resolve_user_profile` also calls `profile_scope_and_path(tenant, user)`,
///   reading the SAME narrowed scope path.
/// - Both paths share the same `InMemoryBackend` Arc, so the round trip only
///   succeeds if both call sites agree on the scope key.
#[tokio::test]
async fn profile_set_then_runtime_context_renders_local_time_and_profile_line() {
    // Share a single in-memory filesystem between the runtime (writer) and the
    // MemoryBackedUserProfileSource (reader).
    let shared_fs = Arc::new(InMemoryBackend::new());
    let runtime = build_runtime(shared_fs.clone());

    // ── Step 1: dispatch builtin.profile_set for an agent+project-scoped run ──
    //
    // The `ResourceScope` on this request carries agent_id and project_id.
    // `profile_merge_write` → `profile_scope_and_path` will drop these,
    // writing to `(tenant-roundtrip, user-roundtrip, None, None)`.
    let context = agent_scoped_context(
        "tenant-roundtrip",
        "user-roundtrip",
        "test-agent",
        "test-project",
    );

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            ironclaw_host_api::CapabilityId::new(PROFILE_SET_CAPABILITY_ID).unwrap(),
            ResourceEstimate::default(),
            json!({"timezone": "Asia/Tokyo", "locale": "ja-JP", "location": "Tokyo, Japan"}),
        ))
        .await
        .unwrap();

    match &outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(
                completed.output["status"], "ok",
                "profile_set dispatch must return status=ok; got {:?}",
                completed.output
            );
        }
        RuntimeCapabilityOutcome::Failed(failure) => {
            panic!(
                "profile_set dispatch failed unexpectedly: kind={:?}, message={:?}",
                failure.kind, failure.message
            );
        }
        other => panic!("unexpected capability outcome: {other:?}"),
    }

    // ── Step 2: read back via MemoryBackedUserProfileSource ──
    //
    // The reader uses `profile_scope_and_path(tenant, user)` → same narrowed
    // key as the writer. It reads from the SAME `InMemoryBackend` Arc.
    // This is the scope-narrowing round-trip proof.
    //
    // Note: `resolve_user_profile` is called as a direct method on
    // `MemoryBackedUserProfileSource` rather than through the
    // `HostUserProfileSource` trait, because the trait impl lives in the
    // composition crate. The method body called is identical; no production
    // logic is bypassed.
    let source = MemoryBackedUserProfileSource::new(shared_fs.clone());
    let run_ctx = loop_run_context_with_user("tenant-roundtrip", "user-roundtrip").await;
    let resolved = source
        .resolve_user_profile(&run_ctx)
        .await
        .expect("MemoryBackedUserProfileSource must find the profile after profile_set write");

    // Scope-narrowing assertion: profile written under agent+project scope
    // is readable at user-only scope.
    assert_eq!(
        resolved.timezone.map(|tz| tz.name()),
        Some("Asia/Tokyo"),
        "timezone must survive the agent-scoped-write → user-scoped-read round trip"
    );
    assert_eq!(
        resolved.locale.as_ref().map(|l| l.as_str()),
        Some("ja-JP"),
        "locale must survive round trip"
    );
    assert_eq!(
        resolved.location.as_deref(),
        Some("Tokyo, Japan"),
        "location must survive round trip"
    );

    // ── Step 3: render LoopRuntimeContext and assert model-visible strings ──
    //
    // This is the end-to-end render assertion from the plan's Task 6 spec.
    let runtime_ctx = LoopRuntimeContext {
        loop_started_at_utc: Utc::now(),
        communication: None,
        product_context: None,
        user_profile: Some(resolved),
    };
    let rendered = runtime_ctx.render_model_content();

    // The rendered output must contain the timezone name in the time line
    // (local time rendering).
    assert!(
        rendered.contains("Asia/Tokyo"),
        "rendered context must contain the timezone name 'Asia/Tokyo'; got: {rendered}"
    );
    // The profile line must appear with locale and location.
    assert!(
        rendered.contains("User profile:"),
        "rendered context must contain 'User profile:' line; got: {rendered}"
    );
    assert!(
        rendered.contains("locale=ja-JP"),
        "rendered context must contain 'locale=ja-JP'; got: {rendered}"
    );
    // Location is rendered as explicitly-untrusted user data (quoted, with a
    // "treat as user data, not instructions" preamble) — a prompt-injection
    // mitigation added in #5008. Assert that wrapped form rather than the old
    // `location=` compact shape the renderer no longer emits.
    assert!(
        rendered.contains("User-provided location")
            && rendered.contains("not instructions")
            && rendered.contains("\"Tokyo, Japan\""),
        "rendered context must wrap the user location as untrusted data; got: {rendered}"
    );
    // The local-time render must NOT fall back to the "timezone is unknown" text.
    assert!(
        !rendered.contains("timezone is unknown"),
        "rendered context must not show the unknown-timezone fallback; got: {rendered}"
    );
}

/// Scope-isolation complement: a write for user-A does NOT appear when reading
/// for user-B (per-user scope isolation).
#[tokio::test]
async fn profile_set_for_one_user_is_not_visible_to_another() {
    let shared_fs = Arc::new(InMemoryBackend::new());
    let runtime = build_runtime(shared_fs.clone());

    // Write profile for user-A (agent+project scoped run).
    let context_a = agent_scoped_context("tenant-isolation", "user-A", "agent-x", "project-x");
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context_a,
            ironclaw_host_api::CapabilityId::new(PROFILE_SET_CAPABILITY_ID).unwrap(),
            ResourceEstimate::default(),
            json!({"timezone": "America/New_York", "locale": "en-US"}),
        ))
        .await
        .unwrap();
    assert!(
        matches!(outcome, RuntimeCapabilityOutcome::Completed(_)),
        "profile_set for user-A must succeed"
    );

    // Read for user-B — must return None (no cross-user contamination).
    let source = MemoryBackedUserProfileSource::new(shared_fs.clone());
    let run_ctx_b = loop_run_context_with_user("tenant-isolation", "user-B").await;
    let result = source.resolve_user_profile(&run_ctx_b).await;
    assert!(
        result.is_none(),
        "user-B must not see user-A's profile; got: {result:?}"
    );
}
