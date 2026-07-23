//! Real-Docker tests for the exec-based persistent container lifecycle
//! ([`sandbox_process::exec_transport`]), driven through the public
//! [`RuntimeProcessPort::run_command`] surface rather than the crate-private
//! `ensure_container`/`exec_in_container` helpers, per this crate's own
//! convention (`sandbox_reaper_docker.rs`, `cli_session_docker.rs`,
//! `sandbox_workspace_fs_parity_docker.rs`).
//!
//! Requires a reachable Docker daemon AND a locally-built `ironclaw-worker`
//! image, same gate as those sibling files. Neither is available on this
//! development machine — these tests are authored to run for real in
//! CI/hosted Docker lanes and skip cleanly (a visible `SKIP: ...` line, never
//! a silent pass) everywhere else.

#[path = "support/docker_gate.rs"]
mod docker_gate;
#[path = "support/sandbox_transport.rs"]
mod sandbox_transport;

use std::collections::HashMap;

use bollard::{
    Docker,
    container::{InspectContainerOptions, ListContainersOptions, RemoveContainerOptions},
};
use ironclaw_host_api::{AgentId, InvocationId, ResourceScope, TenantId, UserId};
use ironclaw_host_runtime::{
    CommandExecutionRequest, RebornSandboxUserKey, RuntimeProcessError, RuntimeProcessPort,
};

// Docker label keys the production launch config attaches (see
// `sandbox_process/registry.rs`). Written as literals here, matching
// `sandbox_reaper_docker.rs`'s own convention — those helper fns are
// `pub(crate)` and unreachable from an integration test.
const LABEL_TENANT: &str = "ironclaw.tenant";
const LABEL_USER: &str = "ironclaw.user";

fn scope(tenant: &str, user: &str) -> ResourceScope {
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

fn request(scope: ResourceScope, command: &str) -> CommandExecutionRequest {
    CommandExecutionRequest {
        scope,
        mounts: None,
        command: command.to_string(),
        workdir: None,
        timeout_secs: Some(10),
        output_limit_bytes: None,
        extra_env: HashMap::new(),
        background: false,
    }
}

fn background_request(scope: ResourceScope, command: &str) -> CommandExecutionRequest {
    let mut request = request(scope, command);
    request.background = true;
    request
}

/// Finds the single container labeled for `{tenant, user}`, the same way the
/// production `ensure_container` lookup does (see `exec_transport.rs`).
async fn find_labeled_container(docker: &Docker, tenant: &str, user: &str) -> Option<String> {
    let filters = HashMap::from([(
        "label".to_string(),
        vec![
            format!("{LABEL_TENANT}={tenant}"),
            format!("{LABEL_USER}={user}"),
        ],
    )]);
    let found = docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        }))
        .await
        .expect("container lookup succeeds");
    found.into_iter().next().and_then(|summary| summary.id)
}

async fn best_effort_remove(docker: &Docker, container_id: &str) {
    let _ = docker
        .remove_container(
            container_id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;
}

macro_rules! skip_unless_docker_ready {
    ($test_name:literal) => {{
        if !docker_gate::docker_available() {
            eprintln!(
                "SKIP: no docker daemon reachable — {} requires a real Docker daemon (CI/hosted Docker lane only)",
                $test_name
            );
            return;
        }
        let image = docker_gate::configured_sandbox_image();
        if !docker_gate::docker_image_available(&image) {
            eprintln!(
                "SKIP: sandbox worker image {image:?} is not built locally — {} requires a locally-built ironclaw-worker image (CI/hosted Docker lane only)",
                $test_name
            );
            return;
        }
        image
    }};
}

#[tokio::test]
async fn exec_reuses_container_across_commands_file_persists_env_does_not() {
    let image = skip_unless_docker_ready!(
        "exec_reuses_container_across_commands_file_persists_env_does_not"
    );

    let docker = Docker::connect_with_local_defaults().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let user_scope = scope("exec-reuse-tenant", "exec-reuse-user");
    let workspace = RebornSandboxUserKey::from_scope(&user_scope).workspace_path(temp.path());
    std::fs::create_dir_all(&workspace).unwrap();
    let port = sandbox_transport::connect_for_test(&workspace, &image)
        .await
        .expect("sandbox transport connects");

    port.run_command(request(
        user_scope.clone(),
        "echo persisted > /workspace/marker.txt",
    ))
    .await
    .expect("write command succeeds");

    let read = port
        .run_command(request(user_scope.clone(), "cat /workspace/marker.txt"))
        .await
        .expect("read command succeeds against the SAME container");
    assert!(
        read.output.contains("persisted"),
        "file written in one command must be visible to the next: {read:?}"
    );

    let mut with_env = request(user_scope.clone(), "echo $PROBE_VAR");
    with_env.extra_env = HashMap::from([("PROBE_VAR".to_string(), "set".to_string())]);
    let with_env_output = port
        .run_command(with_env)
        .await
        .expect("env-setting command succeeds");
    assert!(with_env_output.output.contains("set"));

    let without_env = port
        .run_command(request(user_scope.clone(), "echo [$PROBE_VAR]"))
        .await
        .expect("later command succeeds");
    assert!(
        without_env.output.contains("[]"),
        "env set in one command must NOT bleed into the next (stateless exec): {without_env:?}"
    );

    if let Some(container_id) =
        find_labeled_container(&docker, "exec-reuse-tenant", "exec-reuse-user").await
    {
        best_effort_remove(&docker, &container_id).await;
    }
}

#[tokio::test]
async fn stopped_container_restarts_transparently_on_next_exec() {
    let image = skip_unless_docker_ready!("stopped_container_restarts_transparently_on_next_exec");

    let docker = Docker::connect_with_local_defaults().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let user_scope = scope("restart-tenant", "restart-user");
    let workspace = RebornSandboxUserKey::from_scope(&user_scope).workspace_path(temp.path());
    std::fs::create_dir_all(&workspace).unwrap();
    let port = sandbox_transport::connect_for_test(&workspace, &image)
        .await
        .expect("sandbox transport connects");

    port.run_command(request(user_scope.clone(), "true"))
        .await
        .expect("first command creates the container");
    let container_id = find_labeled_container(&docker, "restart-tenant", "restart-user")
        .await
        .expect("container exists after first command");
    docker
        .stop_container(&container_id, None)
        .await
        .expect("stop out of band");

    let output = port
        .run_command(request(user_scope, "echo alive"))
        .await
        .expect("command against a transparently restarted container succeeds");
    assert!(output.output.contains("alive"));

    let reused_id = find_labeled_container(&docker, "restart-tenant", "restart-user")
        .await
        .expect("container still exists");
    assert_eq!(
        reused_id, container_id,
        "restart must reuse the same container, not recreate one"
    );

    best_effort_remove(&docker, &container_id).await;
}

#[tokio::test]
async fn timeout_kills_process_group_but_container_survives() {
    let image = skip_unless_docker_ready!("timeout_kills_process_group_but_container_survives");

    let docker = Docker::connect_with_local_defaults().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let user_scope = scope("timeout-tenant", "timeout-user");
    let workspace = RebornSandboxUserKey::from_scope(&user_scope).workspace_path(temp.path());
    std::fs::create_dir_all(&workspace).unwrap();
    let port = sandbox_transport::connect_for_test(&workspace, &image)
        .await
        .expect("sandbox transport connects");

    let mut timeout_request = request(user_scope.clone(), "sleep 100");
    timeout_request.timeout_secs = Some(1);
    let timed_out = port.run_command(timeout_request).await;
    assert!(
        matches!(timed_out, Err(RuntimeProcessError::Timeout(_))),
        "long-running command must time out: {timed_out:?}"
    );

    let still_alive = port
        .run_command(request(user_scope, "echo alive"))
        .await
        .expect("the container itself must survive a timeout kill of the exec'd process group");
    assert!(still_alive.output.contains("alive"));

    if let Some(container_id) =
        find_labeled_container(&docker, "timeout-tenant", "timeout-user").await
    {
        best_effort_remove(&docker, &container_id).await;
    }
}

#[tokio::test]
async fn cross_user_containers_and_workspaces_are_isolated() {
    let image = skip_unless_docker_ready!("cross_user_containers_and_workspaces_are_isolated");

    let docker = Docker::connect_with_local_defaults().unwrap();
    let temp = tempfile::tempdir().unwrap();

    let scope_a = scope("isolation-tenant", "isolation-user-a");
    let scope_b = scope("isolation-tenant", "isolation-user-b");
    let workspace_a = RebornSandboxUserKey::from_scope(&scope_a).workspace_path(temp.path());
    let workspace_b = RebornSandboxUserKey::from_scope(&scope_b).workspace_path(temp.path());
    std::fs::create_dir_all(&workspace_a).unwrap();
    std::fs::create_dir_all(&workspace_b).unwrap();

    let port_a = sandbox_transport::connect_for_test(&workspace_a, &image)
        .await
        .expect("sandbox transport A connects");
    let port_b = sandbox_transport::connect_for_test(&workspace_b, &image)
        .await
        .expect("sandbox transport B connects");

    port_a
        .run_command(request(
            scope_a,
            "echo user-a-secret > /workspace/user-a-only.txt",
        ))
        .await
        .unwrap();

    let leak_check = port_b
        .run_command(request(
            scope_b,
            "cat /workspace/user-a-only.txt 2>&1 || echo NOT_FOUND",
        ))
        .await
        .unwrap();
    assert!(
        leak_check.output.contains("NOT_FOUND"),
        "user B's container must not see user A's workspace file: {leak_check:?}"
    );

    let container_a = find_labeled_container(&docker, "isolation-tenant", "isolation-user-a")
        .await
        .expect("container A exists");
    let container_b = find_labeled_container(&docker, "isolation-tenant", "isolation-user-b")
        .await
        .expect("container B exists");
    assert_ne!(
        container_a, container_b,
        "distinct users must get distinct containers"
    );

    // The design's hard invariant: user B's workspace host path must not
    // appear ANYWHERE in user A's container mount table, and vice versa — a
    // bind-mount-source leak would be a full sandbox escape.
    let inspected_a = docker
        .inspect_container(&container_a, None::<InspectContainerOptions>)
        .await
        .unwrap();
    let binds_a = inspected_a.host_config.unwrap().binds.unwrap_or_default();
    let workspace_b_str = workspace_b.to_string_lossy().to_string();
    assert!(
        binds_a.iter().all(|bind| !bind.contains(&workspace_b_str)),
        "user B's workspace path must not appear in user A's mount table: {binds_a:?}"
    );

    let inspected_b = docker
        .inspect_container(&container_b, None::<InspectContainerOptions>)
        .await
        .unwrap();
    let binds_b = inspected_b.host_config.unwrap().binds.unwrap_or_default();
    let workspace_a_str = workspace_a.to_string_lossy().to_string();
    assert!(
        binds_b.iter().all(|bind| !bind.contains(&workspace_a_str)),
        "user A's workspace path must not appear in user B's mount table: {binds_b:?}"
    );

    best_effort_remove(&docker, &container_a).await;
    best_effort_remove(&docker, &container_b).await;
}

#[tokio::test]
async fn fat_image_provides_git_node_python_rust_gh_tmux_under_non_root_home() {
    let image = skip_unless_docker_ready!(
        "fat_image_provides_git_node_python_rust_gh_tmux_under_non_root_home"
    );

    let docker = Docker::connect_with_local_defaults().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let user_scope = scope("fat-image-tenant", "fat-image-user");
    let workspace = RebornSandboxUserKey::from_scope(&user_scope).workspace_path(temp.path());
    std::fs::create_dir_all(&workspace).unwrap();
    let port = sandbox_transport::connect_for_test(&workspace, &image)
        .await
        .expect("sandbox transport connects");

    let mut probe_request = request(
        user_scope,
        "command -v git node python3 cargo gh tmux npm && whoami && echo $HOME",
    );
    probe_request.timeout_secs = Some(20);
    let probe = port
        .run_command(probe_request)
        .await
        .expect("probe command succeeds");

    for binary in [
        "/git", "/node", "/python3", "/cargo", "/gh", "/tmux", "/npm",
    ] {
        assert!(
            probe.output.contains(binary),
            "expected {binary} on PATH: {probe:?}"
        );
    }
    assert!(
        probe.output.contains("sandbox"),
        "must run as the non-root sandbox user: {probe:?}"
    );
    assert!(
        !probe.output.contains("\nroot\n"),
        "must not run as root: {probe:?}"
    );
    assert!(
        probe.output.contains("/workspace/.home"),
        "HOME must be workspace-relative: {probe:?}"
    );

    if let Some(container_id) =
        find_labeled_container(&docker, "fat-image-tenant", "fat-image-user").await
    {
        best_effort_remove(&docker, &container_id).await;
    }
}

/// End-to-end round trip for `background: true` against a real container:
/// starts a detached command, then proves the pid-agreement invariant that
/// only the script-shape unit test (`background_launch_script_puts_the_
/// dollar_dollar_log_redirect_inside_the_inner_shell` in `exec_transport.rs`)
/// otherwise pins — that the log file the launched job actually writes to
/// lives at the exact `log_path` reported back to the caller. If the `$$`
/// used to build the log filename ever disagreed with the `$!` pid reported
/// to Rust again, this test would fail by finding no file (or the wrong
/// content) at the reported path, where the unit test alone cannot catch it
/// (it only asserts the script string's shape, never runs it).
#[tokio::test]
async fn background_command_writes_its_output_to_the_reported_log_path() {
    let image =
        skip_unless_docker_ready!("background_command_writes_its_output_to_the_reported_log_path");

    let docker = Docker::connect_with_local_defaults().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let user_scope = scope("background-tenant", "background-user");
    let workspace = RebornSandboxUserKey::from_scope(&user_scope).workspace_path(temp.path());
    std::fs::create_dir_all(&workspace).unwrap();
    let port = sandbox_transport::connect_for_test(&workspace, &image)
        .await
        .expect("sandbox transport connects");

    let marker = "background-e2e-marker-9f3c1a";
    let launch = port
        .run_command(background_request(
            user_scope.clone(),
            &format!("echo {marker}"),
        ))
        .await
        .expect("background launch succeeds");
    assert!(
        launch.output.starts_with("Started in background: pid "),
        "expected the background-launch acknowledgement, got: {launch:?}"
    );
    let log_path = launch
        .output
        .split_once(", log ")
        .map(|(_, log_path)| log_path.trim().to_string())
        .expect("background launch output names a log path");
    assert!(
        log_path.starts_with("/workspace/.ironclaw/bg-") && log_path.ends_with(".log"),
        "unexpected log path shape: {log_path}"
    );

    // The background job races the `cat` below; poll briefly rather than
    // assuming it has already flushed its output by the time we check.
    let mut log_contents = String::new();
    for _ in 0..20 {
        let read = port
            .run_command(request(
                user_scope.clone(),
                &format!("cat {log_path} 2>&1 || echo NOT_YET"),
            ))
            .await
            .expect("log read command succeeds");
        if read.output.contains(marker) {
            log_contents = read.output;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert!(
        log_contents.contains(marker),
        "expected the background job's output at its reported log_path {log_path}, got: {log_contents:?}"
    );

    if let Some(container_id) =
        find_labeled_container(&docker, "background-tenant", "background-user").await
    {
        best_effort_remove(&docker, &container_id).await;
    }
}
