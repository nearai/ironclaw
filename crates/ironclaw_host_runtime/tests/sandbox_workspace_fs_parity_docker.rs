//! Real-Docker test proving the persistent sandbox container's shell writes
//! and the composition `/workspace` abstract-FS mount (Task A8) resolve the
//! identical host directory, in both directions.
//!
//! Requires a reachable Docker daemon AND a locally-built `ironclaw-worker`
//! image, same gate as `sandbox_reaper_docker.rs`/`sandbox_cross_tenant_escape.rs`.
//! Neither is available on this development machine — this test is authored
//! to run for real in CI/hosted Docker lanes and skip cleanly (a visible
//! `SKIP: ...` line, never a silent pass) everywhere else.

#[path = "support/docker_gate.rs"]
mod docker_gate;
#[path = "support/sandbox_transport.rs"]
mod sandbox_transport;

use std::collections::HashMap;

use ironclaw_host_api::{AgentId, InvocationId, ResourceScope, TenantId, UserId, VirtualPath};
use ironclaw_host_runtime::{CommandExecutionRequest, RebornSandboxUserKey, RuntimeProcessPort};

fn owner_scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).expect("tenant id"),
        user_id: UserId::new(user).expect("user id"),
        agent_id: Some(AgentId::new("reborn-cli").expect("agent id")),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

/// Shell writes to /workspace inside the persistent sandbox container; the
/// abstract-FS /workspace mount (this task) must read the identical bytes
/// back from the same host directory, and vice versa.
#[tokio::test]
async fn shell_write_and_abstract_fs_read_share_the_same_workspace_bytes() {
    if !docker_gate::docker_available() {
        eprintln!("SKIP: no reachable Docker daemon");
        return;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!("SKIP: sandbox image {image} not built locally");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().expect("canonical root");
    let scope = owner_scope("docker-parity-tenant", "docker-parity-user");
    let workspace_dir = RebornSandboxUserKey::from_scope(&scope).workspace_path(&root);
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");

    // Composition-equivalent abstract-FS mount, built the same way
    // mount_sandbox_user_workspace_root does in ironclaw_reborn_composition.
    let mut disk = ironclaw_filesystem::DiskFilesystem::new();
    disk.mount_local(
        VirtualPath::new("/workspace").expect("virtual path"),
        ironclaw_host_api::HostPath::from_path_buf(workspace_dir.clone()),
    )
    .expect("mount /workspace");

    // Persistent sandbox transport (Task A3), workspace bind == workspace_dir.
    let port = sandbox_transport::connect_for_test(&workspace_dir, &image)
        .await
        .expect("sandbox transport connects");

    port.run_command(CommandExecutionRequest {
        scope: scope.clone(),
        mounts: None,
        command: "echo hi > /workspace/f.txt".to_string(),
        workdir: None,
        timeout_secs: Some(30),
        output_limit_bytes: None,
        extra_env: HashMap::new(),
        background: false,
    })
    .await
    .expect("shell write succeeds");

    let bytes = ironclaw_filesystem::RootFilesystem::read_file(
        &disk,
        &VirtualPath::new("/workspace/f.txt").expect("virtual path"),
    )
    .await
    .expect("abstract FS read succeeds");
    assert_eq!(bytes, b"hi\n");

    ironclaw_filesystem::RootFilesystem::write_file(
        &disk,
        &VirtualPath::new("/workspace/g.txt").expect("virtual path"),
        b"from-write-file",
    )
    .await
    .expect("abstract FS write succeeds");

    let cat = port
        .run_command(CommandExecutionRequest {
            scope,
            mounts: None,
            command: "cat /workspace/g.txt".to_string(),
            workdir: None,
            timeout_secs: Some(30),
            output_limit_bytes: None,
            extra_env: HashMap::new(),
            background: false,
        })
        .await
        .expect("shell read succeeds");
    assert_eq!(cat.output.trim(), "from-write-file");
}
