//! Real-Docker round-trip for `builtin.cli_session`. Requires a reachable
//! Docker daemon AND a locally-built `ironclaw-worker` image with tmux
//! installed (Phase A's fat image) — same gate as `sandbox_reaper_docker.rs`
//! / `sandbox_cross_tenant_escape.rs`. Skips cleanly (a visible `SKIP: ...`
//! line, never a silent pass) everywhere else; runs for real in CI/hosted
//! Docker lanes.

#[path = "support/docker_gate.rs"]
mod docker_gate;

use std::sync::Arc;

use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::DiskFilesystem;
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CLI_SESSION_CAPABILITY_ID, CapabilitySurfaceVersion, HostRuntime, HostRuntimeServices,
    RebornSandboxConfig, RebornScopedSandboxCommandTransport, RuntimeCapabilityOutcome,
    builtin_first_party_handlers, builtin_first_party_package,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_triggers::InMemoryTriggerRepository;
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
use serde_json::{Value, json};

fn test_scope(user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("cli-session-docker-tenant").unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: Some(AgentId::new("agent").unwrap()),
        project_id: Some(ProjectId::new("project").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

/// Same `origin_gate_matrix`-style effect allowlist `first_party_builtin_tools.rs`
/// grants its "caller" extension — broad enough to cover every builtin's
/// declared effects, including `builtin.cli_session`'s
/// `{SpawnProcess, ExecuteCode, ReadFilesystem, WriteFilesystem, Network}`.
fn builtin_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
        EffectKind::DeleteFilesystem,
        EffectKind::Network,
        EffectKind::SpawnProcess,
        EffectKind::ExecuteCode,
        EffectKind::ExternalWrite,
    ]
}

/// Trust policy admitting the `builtin` first-party package as first-party —
/// mirrors `first_party_builtin_tools.rs`'s local `trust_policy()` helper.
/// Without this, `HostRuntimeServices` defaults to `HostTrustPolicy::fail_closed()`
/// and every capability call is denied before it ever reaches `cli_session::dispatch`.
fn trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("builtin").unwrap(),
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            builtin_effects(),
            None,
        ),
    ]))])
    .unwrap()
}

fn registry() -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(builtin_first_party_package().unwrap())
        .unwrap();
    registry
}

/// Grants the caller `capability` under `scope`'s tenant/user/agent/project,
/// with network fully open — this test proves per-scope container isolation,
/// not network policy, so the grant itself stays permissive.
fn execution_context_for_scope(scope: &ResourceScope, capability: &str) -> ExecutionContext {
    let grant = CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: CapabilityId::new(capability).unwrap(),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: builtin_effects(),
            mounts: MountView::default(),
            network: NetworkPolicy {
                allowed_targets: vec![NetworkTargetPattern {
                    scheme: None,
                    host_pattern: "*".to_string(),
                    port: None,
                }],
                deny_private_ip_ranges: false,
                max_egress_bytes: None,
            },
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    };
    ExecutionContext {
        invocation_id: scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        authenticated_actor_user_id: None,
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: scope.mission_id.clone(),
        thread_id: scope.thread_id.clone(),
        run_id: None,
        origin: None,
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::FirstParty,
        trust: TrustClass::FirstParty,
        grants: CapabilitySet {
            grants: vec![grant],
        },
        mounts: MountView::default(),
        resource_scope: scope.clone(),
    }
}

/// Builds a `HostRuntime` bound to a *real* per-scope sandbox container: one
/// fresh workspace root, one `RebornSandboxConfig`, one
/// `RebornScopedSandboxCommandTransport::connect` (Phase A's persistent-mode
/// transport), turned into a `TenantSandboxProcessPort` via
/// `.into_process_port()` — the same connect/`into_process_port` pair
/// `sandbox_cross_tenant_escape.rs` already uses for its real-Docker tests.
/// Everything else mirrors `first_party_builtin_tools.rs`'s own
/// `runtime_with_process_port_and_policy` helper (registry/governor/
/// authorizer/process-services scaffolding), since that pattern is already
/// this crate's minimal recipe for a directly constructible `HostRuntime` in
/// an integration-test binary — `production_runtime_project_service.rs`'s
/// `RecordingSandboxTransport` wiring (in `ironclaw_reborn_composition`,
/// building a full `RebornRuntime` over libSQL) is the composition-layer
/// analogue of this same idea, not something this crate can depend on.
///
/// The returned tempdir is leaked deliberately: it must outlive every
/// `invoke` call against the returned runtime, and this fn's caller (a
/// `#[tokio::test]` body) never gets a natural drop point for it without
/// restructuring every test into a `{ ... }` block. Process exit reclaims it;
/// this matches the tolerance every other real-Docker test in this crate
/// already has for its own scratch directories.
async fn runtime_for_scope(_scope: &ResourceScope) -> impl HostRuntime {
    let workspace_root = tempfile::tempdir().expect("tempdir for sandbox workspace root");
    let config = RebornSandboxConfig::new(workspace_root.path());
    std::mem::forget(workspace_root);

    let transport = RebornScopedSandboxCommandTransport::connect(config)
        .await
        .expect("real docker connect should succeed when the daemon is reachable");
    let process_port = Arc::new(transport.into_process_port());

    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_runtime_process_port(process_port)
    .with_runtime_policy(EffectiveRuntimePolicy {
        deployment: DeploymentMode::HostedMultiTenant,
        requested_profile: RuntimeProfile::SecureDefault,
        resolved_profile: RuntimeProfile::SecureDefault,
        filesystem_backend: FilesystemBackendKind::ScopedVirtual,
        process_backend: ProcessBackendKind::TenantSandbox,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::BrokeredHandles,
        approval_policy: ApprovalPolicy::AskAlways,
        audit_mode: AuditMode::Standard,
    })
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

/// Drives `capability` through the real production caller path
/// (`HostRuntime::invoke_capability`, i.e. authorization → resource
/// accounting → `cli_session::dispatch`), returning the completed output and
/// panicking with the failure detail on anything else — every call site
/// below expects success, so a `Failed`/other outcome is a test bug or a
/// real regression either way.
async fn invoke(
    runtime: &(impl HostRuntime + ?Sized),
    scope: &ResourceScope,
    input: Value,
) -> Value {
    let outcome = runtime
        .invoke_capability((
            execution_context_for_scope(scope, CLI_SESSION_CAPABILITY_ID),
            CapabilityId::new(CLI_SESSION_CAPABILITY_ID).unwrap(),
            ResourceEstimate::default(),
            input,
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected a completed cli_session outcome, got: {other:?}"),
    }
}

#[tokio::test]
async fn start_read_send_kill_round_trip_through_a_persistent_container() {
    if !docker_gate::docker_available() {
        eprintln!(
            "SKIP: no docker daemon reachable — start_read_send_kill_round_trip_through_a_persistent_container requires a real Docker daemon (CI/hosted Docker lane only)"
        );
        return;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!(
            "SKIP: sandbox worker image {image:?} is not built locally — start_read_send_kill_round_trip_through_a_persistent_container requires a locally-built ironclaw-worker image with tmux (CI/hosted Docker lane only)"
        );
        return;
    }

    let scope = test_scope("cli-session-user");
    let runtime = runtime_for_scope(&scope).await;

    let start = invoke(
        &runtime,
        &scope,
        json!({"action": "start", "session": "smoke", "command": "cat"}),
    )
    .await;
    assert_eq!(start["success"], json!(true));
    assert!(
        start["active_sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "ic-smoke")
    );

    let send = invoke(
        &runtime,
        &scope,
        json!({"action": "send", "session": "smoke", "text": "hello from cli_session"}),
    )
    .await;
    assert_eq!(send["success"], json!(true));

    let read = invoke(
        &runtime,
        &scope,
        json!({"action": "read", "session": "smoke"}),
    )
    .await;
    assert!(
        read["output"]
            .as_str()
            .unwrap()
            .contains("hello from cli_session"),
        "pane capture must show the sent text: {read}"
    );

    let kill = invoke(
        &runtime,
        &scope,
        json!({"action": "kill", "session": "smoke"}),
    )
    .await;
    assert_eq!(kill["success"], json!(true));

    let read_after_kill = invoke(
        &runtime,
        &scope,
        json!({"action": "read", "session": "smoke"}),
    )
    .await;
    assert_eq!(
        read_after_kill["success"],
        json!(false),
        "reading a killed session must fail: {read_after_kill}"
    );
}

#[tokio::test]
async fn background_dev_server_survives_between_separate_start_and_read_execs() {
    if !docker_gate::docker_available() {
        eprintln!(
            "SKIP: no docker daemon reachable — background_dev_server_survives_between_separate_start_and_read_execs requires a real Docker daemon (CI/hosted Docker lane only)"
        );
        return;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!(
            "SKIP: sandbox worker image {image:?} is not built locally — background_dev_server_survives_between_separate_start_and_read_execs requires a locally-built ironclaw-worker image with tmux (CI/hosted Docker lane only)"
        );
        return;
    }

    let scope = test_scope("cli-session-user-2");
    let runtime = runtime_for_scope(&scope).await;

    invoke(
        &runtime,
        &scope,
        // Long-running loop simulates a dev server: still alive by the time
        // the SEPARATE `read` exec below runs, proving the session outlives
        // a single stateless exec — the entire point of Phase A/B.
        json!({"action": "start", "session": "server", "command": "sh -c 'while true; do echo tick; sleep 1; done'"}),
    )
    .await;

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let read = invoke(
        &runtime,
        &scope,
        json!({"action": "read", "session": "server"}),
    )
    .await;
    assert!(
        read["output"].as_str().unwrap().contains("tick"),
        "background loop must still be producing output on a later, separate exec: {read}"
    );

    invoke(
        &runtime,
        &scope,
        json!({"action": "kill", "session": "server"}),
    )
    .await;
}

#[tokio::test]
async fn a_second_user_cannot_read_the_first_users_session() {
    if !docker_gate::docker_available() {
        eprintln!(
            "SKIP: no docker daemon reachable — a_second_user_cannot_read_the_first_users_session requires a real Docker daemon (CI/hosted Docker lane only)"
        );
        return;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!(
            "SKIP: sandbox worker image {image:?} is not built locally — a_second_user_cannot_read_the_first_users_session requires a locally-built ironclaw-worker image with tmux (CI/hosted Docker lane only)"
        );
        return;
    }

    let owner_scope = test_scope("cli-session-owner");
    let other_scope = test_scope("cli-session-other");
    let owner_runtime = runtime_for_scope(&owner_scope).await;
    let other_runtime = runtime_for_scope(&other_scope).await;

    invoke(
        &owner_runtime,
        &owner_scope,
        json!({"action": "start", "session": "private", "command": "cat"}),
    )
    .await;

    // Different user => different container (Phase A's `{tenant_id, user_id}`
    // key) => tmux in that container has never heard of "ic-private".
    let read = invoke(
        &other_runtime,
        &other_scope,
        json!({"action": "read", "session": "private"}),
    )
    .await;
    assert_eq!(
        read["success"],
        json!(false),
        "a session started in one user's container must not be visible from another user's container: {read}"
    );
}
