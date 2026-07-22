//! Real-Docker cross-tenant escape test for the Reborn sandbox process
//! transport.
//!
//! Mandated by `.claude/rules/safety-and-sandbox.md` ("Process and shell
//! execution: real OS isolation, per tenant") for any change touching
//! process ports / profile->backend mapping: "user B runs a shell command
//! and the test asserts it cannot read user A's files." This proves the
//! **host bind-mount boundary** specifically — not the container-workdir
//! string validation `sandbox_process.rs`'s existing unit tests
//! (`relative_workdir_rejects_escape`, `container_workdir_rejects_host_absolute_paths`)
//! already cover. Those tests prove `resolve_container_workdir` rejects a
//! malformed *string*; this test proves that even a well-formed in-container
//! path can never resolve to another tenant's data, because each tenant's
//! container only ever has ITS OWN scope-keyed host directory bind-mounted
//! at `/workspace` (`container_launch_config` in `sandbox_process.rs`).
//!
//! Requires a reachable Docker daemon AND a locally-built `ironclaw-worker`
//! image. Neither is available on a typical dev machine (this worktree has
//! no Docker) — the test is authored to run for real in CI/hosted lanes that
//! have both, and skips cleanly (a visible `SKIP: ...` line, never a silent
//! pass) everywhere else.

#[path = "support/docker_gate.rs"]
mod docker_gate;

use std::collections::HashMap;
use std::time::Duration;

use ironclaw_host_api::{AgentId, InvocationId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_host_runtime::{
    CommandExecutionRequest, RebornSandboxConfig, RebornSandboxUserKey,
    RebornScopedSandboxCommandTransport, SandboxCommandTransport,
};

fn tenant_scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: Some(AgentId::new("agent").unwrap()),
        project_id: Some(ProjectId::new("project").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

#[tokio::test]
async fn sandbox_containers_cannot_read_across_tenant_host_bind_mounts() {
    if !docker_gate::docker_available() {
        eprintln!(
            "SKIP: no docker daemon reachable — sandbox_containers_cannot_read_across_tenant_host_bind_mounts requires a real Docker daemon (CI/hosted Docker lane only)"
        );
        return;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!(
            "SKIP: sandbox worker image {image:?} is not built locally — sandbox_containers_cannot_read_across_tenant_host_bind_mounts requires a locally-built ironclaw-worker image (CI/hosted Docker lane only)"
        );
        return;
    }

    let workspace_root = tempfile::tempdir().expect("tempdir for sandbox workspace root");
    let config = RebornSandboxConfig::new(workspace_root.path());

    // One shared transport instance, exactly as composition wires it in
    // production (a single `Arc<TenantSandboxProcessPort>` binding serves
    // every tenant) — the test must prove per-request scope isolation holds
    // from a single shared connection, not merely across separately
    // connected transports.
    let transport = RebornScopedSandboxCommandTransport::connect(config)
        .await
        .expect("real docker connect should succeed when the daemon is reachable");

    let scope_a = tenant_scope("tenant-a", "user-a");
    let scope_b = tenant_scope("tenant-b", "user-b");

    // Host-side leaf directory `ironclaw-worker`'s container for tenant A
    // gets bind-mounted at (`<root>/scopes/<digest-a>`). Only its basename is
    // used inside the escape attempt below — the test never needs A's
    // container to disclose it, it derives it independently host-side,
    // exactly like an attacker who already knows (or guesses) the scope
    // digest scheme would.
    let key_a = RebornSandboxUserKey::from_scope(&scope_a);
    let a_dir_name = key_a
        .workspace_path(workspace_root.path())
        .file_name()
        .expect("scope workspace path has a leaf directory name")
        .to_string_lossy()
        .to_string();

    // --- User A writes a marker file under its own /workspace. ---
    let marker_secret = "IRONCLAW-TENANT-A-SECRET-MARKER";
    let write_output = transport
        .run_command(CommandExecutionRequest {
            scope: scope_a.clone(),
            mounts: None,
            command: format!("printf '{marker_secret}' > /workspace/marker.txt"),
            workdir: None,
            timeout_secs: Some(30),
            extra_env: HashMap::new(),
            output_limit_bytes: None,
            background: false,
        })
        .await
        .expect("tenant A marker write should succeed");
    assert_eq!(
        write_output.exit_code, 0,
        "tenant A marker write failed: {}",
        write_output.output
    );

    // --- User B attempts to read A's marker two ways. ---
    // 1. Workspace-relative path: if request-scope isolation were broken and
    //    every tenant shared one workspace root, this alone would already
    //    leak A's file.
    // 2. `/workspace/../<A-scope-digest>/marker.txt`: a well-formed,
    //    in-container relative-looking path that only resolves to A's host
    //    directory if B's container has A's *parent* directory bind-mounted
    //    (not just B's own scope-keyed leaf dir). Docker bind mounts do not
    //    expose the host parent of a bind-mounted directory inside the
    //    container, so `/workspace/..` resolves to the container's own root
    //    filesystem, never the host `scopes/` directory — this is the real
    //    host-level containment boundary, independent of any string
    //    validation `resolve_container_workdir` performs before launch.
    let read_output = transport
        .run_command(CommandExecutionRequest {
            scope: scope_b.clone(),
            mounts: None,
            command: format!(
                "{{ echo --relative--; cat marker.txt; echo --escape--; cat /workspace/../{a_dir_name}/marker.txt; }} 2>&1; true"
            ),
            workdir: None,
            timeout_secs: Some(30),
            extra_env: HashMap::new(),
            output_limit_bytes: None,
            background: false,
        })
        .await
        .expect("tenant B read attempt should complete (denial is expected inside the output, not a transport error)");

    assert_eq!(
        read_output.exit_code, 0,
        "the wrapper command always exits 0 regardless of the inner `cat` failures; a nonzero \
         exit means the harness itself broke, not that isolation held: {}",
        read_output.output
    );
    assert!(
        !read_output.output.contains(marker_secret),
        "tenant B must never be able to read tenant A's marker content via either path; got: {}",
        read_output.output
    );
    assert!(
        !read_output
            .output
            .contains(&workspace_root.path().display().to_string()),
        "tenant B's output must never disclose the host sandbox workspace path; got: {}",
        read_output.output
    );

    // Independent host-side confirmation: A's marker really did land under
    // A's own scope-keyed host directory (proves the write above was a real
    // assertion, not a false negative from a broken write path).
    let host_marker_path = key_a
        .workspace_path(workspace_root.path())
        .join("marker.txt");
    let host_marker_contents = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::fs::read_to_string(&host_marker_path),
    )
    .await
    .expect("host marker read should not time out")
    .expect("tenant A's marker file should exist under A's own host-scoped directory");
    assert_eq!(host_marker_contents, marker_secret);

    // And B's own scope directory must never have received A's file either
    // (rules out a bug where the write silently fanned out to every scope
    // dir under the shared root).
    let key_b = RebornSandboxUserKey::from_scope(&scope_b);
    let b_marker_path = key_b
        .workspace_path(workspace_root.path())
        .join("marker.txt");
    assert!(
        !b_marker_path.exists(),
        "tenant B's host-scoped directory must not contain tenant A's marker file"
    );
}
