//! Real-Docker proof for per-user workspace persistence and isolation.

use std::collections::HashMap;

use ironclaw_host_api::{
    ids::{AgentId, InvocationId, ProjectId, TenantId, ThreadId, UserId},
    process::{CommandExecutionRequest, SandboxCommandTransport},
    resource::ResourceScope,
};
use ironclaw_sandbox::{RebornSandboxConfig, RebornScopedSandboxCommandTransport};

#[path = "support/docker_gate.rs"]
mod docker_gate;

fn scope(user: &str, project: &str, thread: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("docker-live-tenant").expect("tenant id"),
        user_id: UserId::new(user).expect("user id"),
        agent_id: Some(AgentId::new("docker-live-agent").expect("agent id")),
        project_id: Some(ProjectId::new(project).expect("project id")),
        mission_id: None,
        thread_id: Some(ThreadId::new(thread).expect("thread id")),
        invocation_id: InvocationId::new(),
    }
}

fn request(scope: ResourceScope, command: impl Into<String>) -> CommandExecutionRequest {
    CommandExecutionRequest {
        scope,
        mounts: None,
        command: command.into(),
        workdir: Some("/workspace".to_string()),
        timeout_secs: Some(60),
        extra_env: HashMap::new(),
    }
}

#[tokio::test]
async fn user_workspace_persists_across_turns_and_isolates_other_users() {
    if !docker_gate::docker_available() {
        eprintln!("SKIP: user sandbox Docker test — no Docker daemon is reachable");
        return;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!("SKIP: user sandbox Docker test — worker image {image:?} is not built");
        return;
    }

    // Colima only bind-mounts configured host roots into its Docker VM. The
    // default macOS tempfile root is `/var/folders`, which the daemon cannot
    // see; a worktree-local tempdir exercises the real bind contract instead.
    let temp =
        tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("Docker-visible workspace tempdir");
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces")).with_network_enabled(),
    )
    .await
    .expect("Docker transport connects");
    let unique = InvocationId::new().to_string();
    let marker = format!("docker-user-a-{unique}");

    let first = transport
        .run_command(request(
            scope("user-a", "project-a", "thread-a"),
            format!(
                "python - <<'PY'\n\
                 import os\n\
                 from pathlib import Path\n\
                 assert os.getuid() != 0\n\
                 assert Path('/.dockerenv').is_file()\n\
                 assert not Path('/var/run/docker.sock').exists()\n\
                 forbidden_env = {{\n\
                     'OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'NEARAI_API_KEY',\n\
                     'RAILWAY_TOKEN', 'RAILWAY_API_TOKEN', 'AWS_ACCESS_KEY_ID',\n\
                     'AWS_SECRET_ACCESS_KEY', 'GITHUB_TOKEN', 'GH_TOKEN',\n\
                 }}\n\
                 assert forbidden_env.isdisjoint(os.environ)\n\
                 root = next(line.split() for line in Path('/proc/mounts').read_text().splitlines() if line.split()[1] == '/')\n\
                 assert 'ro' in root[3].split(',')\n\
                 routes = [line.split() for line in Path('/proc/net/route').read_text().splitlines()[1:]]\n\
                 assert any(route[1] == '00000000' for route in routes)\n\
                 assert os.environ['IRONCLAW_REBORN_NETWORK_MODE'] == 'direct'\n\
                 Path('state.txt').write_text('{marker}')\n\
                 print(f'LOCAL_DOCKER_SANDBOX_OK uid={{os.getuid()}}')\n\
                 PY"
            ),
        ))
        .await
        .expect("first command runs");
    assert_eq!(first.exit_code, 0, "first command failed: {}", first.output);
    assert!(first.output.contains("LOCAL_DOCKER_SANDBOX_OK"));
    assert!(first.sandboxed);

    let nonzero = transport
        .run_command(request(
            scope("user-a", "project-a", "thread-a"),
            "echo EXPECTED_NONZERO_STDERR >&2; exit 7",
        ))
        .await
        .expect("non-zero command exit remains a sandbox result");
    assert_eq!(nonzero.exit_code, 7);
    assert!(nonzero.output.contains("EXPECTED_NONZERO_STDERR"));
    assert!(nonzero.sandboxed);

    let same_user = transport
        .run_command(request(
            scope("user-a", "project-b", "thread-b"),
            "python -c 'from pathlib import Path; print(Path(\"state.txt\").read_text())'",
        ))
        .await
        .expect("same user reads persistent workspace");
    assert_eq!(same_user.exit_code, 0, "read failed: {}", same_user.output);
    assert!(same_user.output.contains(&marker));

    let other_user = transport
        .run_command(request(
            scope("user-b", "project-a", "thread-a"),
            "python -c 'from pathlib import Path; print(Path(\"state.txt\").exists())'",
        ))
        .await
        .expect("other user receives isolated workspace");
    assert_eq!(
        other_user.exit_code, 0,
        "read failed: {}",
        other_user.output
    );
    assert!(other_user.output.contains("False"));
    assert!(!other_user.output.contains(&marker));
}

#[tokio::test]
#[ignore = "requires public DNS and Internet access; run as a live egress canary"]
async fn sandbox_profile_allows_public_https_egress() {
    if !docker_gate::docker_available() {
        eprintln!("SKIP: sandbox egress canary — no Docker daemon is reachable");
        return;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!("SKIP: sandbox egress canary — worker image {image:?} is not built");
        return;
    }

    let temp = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .expect("Docker-visible egress canary workspace");
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces")).with_network_enabled(),
    )
    .await
    .expect("Docker transport connects");

    let result = transport
        .run_command(request(
            scope("egress-user", "egress-project", "egress-thread"),
            "python -c \"import os, urllib.request; assert os.environ['IRONCLAW_REBORN_NETWORK_MODE'] == 'direct'; response = urllib.request.urlopen('https://example.com', timeout=15); assert response.status == 200; response.close(); print('SANDBOX_PUBLIC_HTTPS_OK')\"",
        ))
        .await
        .expect("public HTTPS request runs");

    assert_eq!(
        result.exit_code, 0,
        "egress canary failed: {}",
        result.output
    );
    assert!(result.output.contains("SANDBOX_PUBLIC_HTTPS_OK"));
    assert!(result.sandboxed);
}
