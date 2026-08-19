//! Real-Docker proof for persistent per-user containers and workspaces.

use std::{
    collections::{HashMap, HashSet},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use ironclaw_host_api::{
    ids::{AgentId, InvocationId, ProjectId, TenantId, ThreadId, UserId},
    process::{CommandExecutionRequest, RuntimeProcessError, SandboxCommandTransport},
    resource::ResourceScope,
};
use ironclaw_sandbox::{RebornSandboxConfig, RebornScopedSandboxCommandTransport};

#[path = "support/docker_gate.rs"]
mod docker_gate;

fn scope(tenant: &str, user: &str, project: &str, thread: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).expect("tenant id"),
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

const CONTAINER_PREFIX: &str = "ironclaw-reborn-sandbox-user-";
const LABEL_TENANT: &str = "ironclaw.tenant";
const LABEL_USER: &str = "ironclaw.user";
const LABEL_IMAGE: &str = "ironclaw.image";
const LABEL_SECURITY_POSTURE: &str = "ironclaw.security_posture";

#[derive(Clone, Debug)]
struct TestScope {
    tenant: String,
    user: String,
    project: String,
    thread: String,
}

impl TestScope {
    fn unique(label: &str) -> Self {
        let suffix = InvocationId::new();
        Self {
            tenant: format!("{label}-tenant-{suffix}"),
            user: format!("{label}-user-{suffix}"),
            project: format!("{label}-project-{suffix}"),
            thread: format!("{label}-thread-{suffix}"),
        }
    }

    fn resource_scope(&self) -> ResourceScope {
        scope(&self.tenant, &self.user, &self.project, &self.thread)
    }
}

#[derive(Clone, Debug)]
struct ContainerSnapshot {
    id: String,
    name: String,
    hostname: String,
    image: String,
    running: bool,
    labels: HashMap<String, String>,
}

#[derive(Default)]
struct DockerCleanup {
    scopes: Vec<TestScope>,
    container_ids: HashSet<String>,
    image_tags: Vec<String>,
}

impl DockerCleanup {
    fn with_scopes(scopes: impl IntoIterator<Item = TestScope>) -> Self {
        Self {
            scopes: scopes.into_iter().collect(),
            container_ids: HashSet::new(),
            image_tags: Vec::new(),
        }
    }

    fn capture(&mut self, scope: &TestScope) -> ContainerSnapshot {
        let matches = containers_for_user(scope);
        self.container_ids
            .extend(matches.iter().map(|container| container.id.clone()));
        assert_eq!(
            matches.len(),
            1,
            "expected exactly one container for tenant {:?} and user {:?}, found {matches:?}",
            scope.tenant,
            scope.user,
        );
        matches.into_iter().next().expect("length checked")
    }

    fn track_image(&mut self, image: String) {
        self.image_tags.push(image);
    }
}

impl Drop for DockerCleanup {
    fn drop(&mut self) {
        for scope in &self.scopes {
            let tenant = format!("{LABEL_TENANT}={}", scope.tenant);
            let user = format!("{LABEL_USER}={}", scope.user);
            let tenant_filter = format!("label={tenant}");
            let user_filter = format!("label={user}");
            if let Ok(output) = Command::new("docker")
                .args([
                    "container",
                    "list",
                    "--all",
                    "--quiet",
                    "--filter",
                    tenant_filter.as_str(),
                    "--filter",
                    user_filter.as_str(),
                ])
                .output()
                && output.status.success()
            {
                self.container_ids.extend(
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .map(str::to_string),
                );
            }
        }
        for id in &self.container_ids {
            let _ = Command::new("docker")
                .args(["container", "rm", "--force", id])
                .output();
        }
        for image in &self.image_tags {
            let _ = Command::new("docker")
                .args(["image", "rm", "--force", image])
                .output();
        }
    }
}

fn docker_worker_image(test_name: &str) -> Option<String> {
    if !docker_gate::docker_available() {
        eprintln!("SKIP: {test_name} — no Docker daemon is reachable");
        return None;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!("SKIP: {test_name} — worker image {image:?} is not built");
        return None;
    }
    Some(image)
}

fn docker_command(args: &[&str]) -> std::process::Output {
    let output = Command::new("docker")
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("docker {args:?} could not start: {error}"));
    assert!(
        output.status.success(),
        "docker {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn containers_for_user(scope: &TestScope) -> Vec<ContainerSnapshot> {
    let tenant_filter = format!("label={LABEL_TENANT}={}", scope.tenant);
    let user_filter = format!("label={LABEL_USER}={}", scope.user);
    let output = Command::new("docker")
        .args(["container", "list", "--all", "--filter"])
        .arg(&tenant_filter)
        .arg("--filter")
        .arg(&user_filter)
        .args(["--format", "{{.ID}}"])
        .output()
        .expect("docker container list starts");
    assert!(
        output.status.success(),
        "docker container list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|id| inspect_container(id.trim()))
        .collect()
}

fn inspect_container(id: &str) -> ContainerSnapshot {
    let output = docker_command(&["container", "inspect", id]);
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("docker inspect returns JSON");
    let container = value
        .as_array()
        .and_then(|containers| containers.first())
        .expect("docker inspect returns one container");
    let labels = container["Config"]["Labels"]
        .as_object()
        .expect("sandbox container has labels")
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                value
                    .as_str()
                    .unwrap_or_else(|| panic!("label {key:?} is not a string"))
                    .to_string(),
            )
        })
        .collect();
    ContainerSnapshot {
        id: container["Id"]
            .as_str()
            .expect("container has an id")
            .to_string(),
        name: container["Name"]
            .as_str()
            .expect("container has a name")
            .trim_start_matches('/')
            .to_string(),
        hostname: container["Config"]["Hostname"]
            .as_str()
            .expect("container has a hostname")
            .to_string(),
        image: container["Config"]["Image"]
            .as_str()
            .expect("container has an image")
            .to_string(),
        running: container["State"]["Running"]
            .as_bool()
            .expect("container running state is boolean"),
        labels,
    }
}

fn assert_stable_identity(container: &ContainerSnapshot, scope: &TestScope) {
    let suffix = container
        .name
        .strip_prefix(CONTAINER_PREFIX)
        .unwrap_or_else(|| panic!("unexpected sandbox container name: {}", container.name));
    assert_eq!(suffix.len(), 24, "stable name uses a 96-bit hex digest");
    assert!(
        suffix.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "stable name digest is hexadecimal: {}",
        container.name
    );
    assert_eq!(container.labels.get(LABEL_TENANT), Some(&scope.tenant));
    assert_eq!(container.labels.get(LABEL_USER), Some(&scope.user));
    assert!(
        container
            .labels
            .get(LABEL_IMAGE)
            .is_some_and(|value| !value.is_empty()),
        "container records image identity"
    );
    assert!(
        container
            .labels
            .get(LABEL_SECURITY_POSTURE)
            .is_some_and(|value| !value.is_empty()),
        "container records the security-posture stamp"
    );
}

async fn wait_for_running_state(id: &str, running: bool, timeout: Duration) -> ContainerSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = inspect_container(id);
        if snapshot.running == running {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "container {id} did not reach running={running} within {timeout:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_user_container(scope: &TestScope, timeout: Duration) -> ContainerSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let mut matches = containers_for_user(scope);
        assert!(
            matches.len() <= 1,
            "expected at most one container for tenant {:?} and user {:?}, found {matches:?}",
            scope.tenant,
            scope.user,
        );
        if let Some(container) = matches.pop() {
            return container;
        }
        assert!(
            Instant::now() < deadline,
            "user container did not appear within {timeout:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_container_file(id: &str, path: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let output = Command::new("docker")
            .args(["container", "exec", id, "test", "-e", path])
            .output()
            .expect("docker container exec starts");
        if output.status.success() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "container {id} did not create {path:?} within {timeout:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn docker_visible_tempdir() -> tempfile::TempDir {
    // Colima only bind-mounts configured host roots into its Docker VM.
    tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .expect("Docker-visible sandbox workspace tempdir")
}

#[tokio::test]
async fn user_container_reuses_state_across_threads_and_isolates_other_users_and_tenants() {
    let Some(_image) = docker_worker_image("user container reuse test") else {
        return;
    };
    let primary = TestScope::unique("reuse");
    let mut other_thread = primary.clone();
    other_thread.project = format!("other-project-{}", InvocationId::new());
    other_thread.thread = format!("other-thread-{}", InvocationId::new());
    let mut other_user = primary.clone();
    other_user.user = format!("other-user-{}", InvocationId::new());
    let mut other_tenant = primary.clone();
    other_tenant.tenant = format!("other-tenant-{}", InvocationId::new());
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([
        primary.clone(),
        other_thread.clone(),
        other_user.clone(),
        other_tenant.clone(),
    ]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces")).with_network_enabled(),
    )
    .await
    .expect("Docker transport connects");
    let workspace_marker = format!("workspace-{}", InvocationId::new());
    let ephemeral_marker = format!("ephemeral-{}", InvocationId::new());

    let first = transport
        .run_command(request(
            primary.resource_scope(),
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
                 Path('/workspace/state.txt').write_text('{workspace_marker}')\n\
                 Path('/tmp/user-state.txt').write_text('{ephemeral_marker}')\n\
                 print('LOCAL_DOCKER_SANDBOX_OK')\n\
                 PY"
            ),
        ))
        .await
        .expect("first thread command runs");
    let first_container = cleanup.capture(&primary);
    assert_eq!(first.exit_code, 0, "first command failed: {}", first.output);
    assert!(first.output.contains("LOCAL_DOCKER_SANDBOX_OK"));
    assert!(first.sandboxed);
    assert!(first_container.running);
    assert_stable_identity(&first_container, &primary);

    let cross_thread = transport
        .run_command(request(
            other_thread.resource_scope(),
            "cat /workspace/state.txt /tmp/user-state.txt",
        ))
        .await
        .expect("second thread reads the user container state");
    let other_thread_container = cleanup.capture(&other_thread);
    assert_eq!(
        cross_thread.exit_code, 0,
        "cross-thread read failed: {}",
        cross_thread.output
    );
    assert!(cross_thread.output.contains(&workspace_marker));
    assert!(cross_thread.output.contains(&ephemeral_marker));
    assert_eq!(
        other_thread_container.id, first_container.id,
        "threads for one user must reuse the exact container"
    );
    assert_eq!(other_thread_container.name, first_container.name);
    assert_eq!(other_thread_container.hostname, first_container.hostname);
    assert_stable_identity(&other_thread_container, &other_thread);

    let isolated = [
        (
            &other_user,
            format!("other-user-workspace-{}", InvocationId::new()),
        ),
        (
            &other_tenant,
            format!("other-tenant-workspace-{}", InvocationId::new()),
        ),
    ];
    let mut isolated_containers = Vec::new();
    for (isolated_scope, isolated_marker) in &isolated {
        let output = transport
            .run_command(request(
                isolated_scope.resource_scope(),
                format!(
                    "test ! -e /workspace/state.txt && \
                     test ! -e /tmp/user-state.txt && \
                     printf '%s' '{isolated_marker}' > /workspace/isolation.txt && \
                     echo ISOLATED"
                ),
            ))
            .await
            .expect("isolated scope command runs");
        let isolated_container = cleanup.capture(isolated_scope);
        assert_eq!(
            output.exit_code, 0,
            "isolation check failed: {}",
            output.output
        );
        assert!(output.output.contains("ISOLATED"));
        assert_ne!(isolated_container.id, first_container.id);
        assert_ne!(isolated_container.name, first_container.name);
        assert_ne!(isolated_container.hostname, first_container.hostname);
        assert_stable_identity(&isolated_container, isolated_scope);
        isolated_containers.push(isolated_container);
    }
    assert_ne!(isolated_containers[0].id, isolated_containers[1].id);
    assert_ne!(isolated_containers[0].name, isolated_containers[1].name);
    assert_ne!(
        isolated_containers[0].hostname,
        isolated_containers[1].hostname
    );
    for (index, (isolated_scope, isolated_marker)) in isolated.iter().enumerate() {
        let output = transport
            .run_command(request(
                isolated_scope.resource_scope(),
                "cat /workspace/isolation.txt",
            ))
            .await
            .expect("isolated workspace marker remains private");
        assert_eq!(
            output.exit_code, 0,
            "isolated workspace read failed: {}",
            output.output
        );
        assert_eq!(output.output.trim(), isolated_marker.as_str());
        assert_eq!(
            cleanup.capture(isolated_scope).id,
            isolated_containers[index].id
        );
    }
}

#[tokio::test]
async fn concurrent_first_calls_from_threads_converge_then_new_transport_adopts_user_container() {
    let Some(_image) = docker_worker_image("user container adoption test") else {
        return;
    };
    let primary = TestScope::unique("adopt");
    let mut other_thread = primary.clone();
    other_thread.thread = format!("other-thread-{}", InvocationId::new());
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([primary.clone(), other_thread.clone()]);
    let config = RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"));
    let transport = RebornScopedSandboxCommandTransport::connect(config.clone())
        .await
        .expect("first Docker transport connects");

    let (left, right) = tokio::join!(
        transport.run_command(request(primary.resource_scope(), "echo CONCURRENT_LEFT")),
        transport.run_command(request(
            other_thread.resource_scope(),
            "echo CONCURRENT_RIGHT"
        )),
    );
    assert_eq!(left.expect("left thread command runs").exit_code, 0);
    assert_eq!(right.expect("right thread command runs").exit_code, 0);
    let concurrent_container = cleanup.capture(&primary);
    assert_eq!(
        cleanup.capture(&other_thread).id,
        concurrent_container.id,
        "concurrent first calls must serialize onto and converge on one user container"
    );

    let marker = format!("adopted-{}", InvocationId::new());
    let write = transport
        .run_command(request(
            primary.resource_scope(),
            format!("printf '%s' '{marker}' > /tmp/adoption-marker"),
        ))
        .await
        .expect("ephemeral adoption marker writes");
    assert_eq!(write.exit_code, 0, "marker write failed: {}", write.output);
    assert_eq!(cleanup.capture(&primary).id, concurrent_container.id);
    drop(transport);

    let restarted_transport = RebornScopedSandboxCommandTransport::connect(config)
        .await
        .expect("replacement Docker transport connects");
    let adopted = restarted_transport
        .run_command(request(
            other_thread.resource_scope(),
            "cat /tmp/adoption-marker",
        ))
        .await
        .expect("replacement transport adopts the user container");
    let adopted_container = cleanup.capture(&other_thread);
    assert_eq!(
        adopted.exit_code, 0,
        "adopted command failed: {}",
        adopted.output
    );
    assert!(adopted.output.contains(&marker));
    assert_eq!(adopted_container.id, concurrent_container.id);
    assert_eq!(adopted_container.name, concurrent_container.name);
    assert_eq!(adopted_container.hostname, concurrent_container.hostname);
    assert!(adopted_container.running);
}

#[tokio::test]
async fn same_user_shell_commands_are_intentionally_serialized_across_threads() {
    let Some(_image) = docker_worker_image("same-user command serialization test") else {
        return;
    };
    let primary = TestScope::unique("serialize");
    let mut other_thread = primary.clone();
    other_thread.thread = format!("other-thread-{}", InvocationId::new());
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([primary.clone(), other_thread.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(RebornSandboxConfig::new(
        temp.path().join("sandbox-workspaces"),
    ))
    .await
    .expect("Docker transport connects");

    let primed = transport
        .run_command(request(
            primary.resource_scope(),
            "echo SERIALIZATION_PRIMED",
        ))
        .await
        .expect("user container is created before the concurrency proof");
    assert_eq!(
        primed.exit_code, 0,
        "container priming failed: {}",
        primed.output
    );
    let user_container = cleanup.capture(&primary);
    docker_command(&[
        "container",
        "exec",
        &user_container.id,
        "rm",
        "-f",
        "/tmp/serialization-first-active",
        "/tmp/serialization-second-dispatched",
        "/tmp/serialization-second-entered",
        "/tmp/serialization-first-finished",
    ]);

    let first_transport = transport.clone();
    let first_scope = primary.resource_scope();
    let first = tokio::spawn(async move {
        first_transport
            .run_command(request(
                first_scope,
                "touch /tmp/serialization-first-active; \
                 i=0; \
                 while [ ! -e /tmp/serialization-second-dispatched ] && [ \"$i\" -lt 100 ]; do \
                 i=$((i+1)); sleep 0.1; done; \
                 if [ ! -e /tmp/serialization-second-dispatched ]; then \
                 echo SECOND_COMMAND_WAS_NOT_DISPATCHED; exit 90; fi; \
                 i=0; \
                 while [ ! -e /tmp/serialization-second-entered ] && [ \"$i\" -lt 20 ]; do \
                 i=$((i+1)); sleep 0.1; done; \
                 if [ -e /tmp/serialization-second-entered ]; then \
                 echo SAME_USER_COMMANDS_OVERLAPPED; exit 91; fi; \
                 touch /tmp/serialization-first-finished; \
                 echo FIRST_SERIALIZED_COMMAND_COMPLETED",
            ))
            .await
    });
    wait_for_container_file(
        &user_container.id,
        "/tmp/serialization-first-active",
        Duration::from_secs(10),
    )
    .await;

    let launch_second = Arc::new(tokio::sync::Barrier::new(2));
    let second_transport = transport.clone();
    let second_scope = other_thread.resource_scope();
    let second_launch = launch_second.clone();
    let second = tokio::spawn(async move {
        second_launch.wait().await;
        second_transport
            .run_command(request(
                second_scope,
                "touch /tmp/serialization-second-entered; \
                 if [ ! -e /tmp/serialization-first-finished ]; then \
                 echo SECOND_COMMAND_ENTERED_BEFORE_FIRST_FINISHED; exit 92; fi; \
                 echo SECOND_SERIALIZED_COMMAND_COMPLETED",
            ))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), launch_second.wait())
        .await
        .expect("second command reaches its bounded launch barrier");
    docker_command(&[
        "container",
        "exec",
        &user_container.id,
        "touch",
        "/tmp/serialization-second-dispatched",
    ]);

    let first = tokio::time::timeout(Duration::from_secs(10), first)
        .await
        .expect("first serialized command completes before its bounded barrier expires")
        .expect("first serialized command task joins")
        .expect("first serialized command runs");
    assert_eq!(
        first.exit_code, 0,
        "same-user overlap interrupted the first command: {}",
        first.output
    );
    assert!(first.output.contains("FIRST_SERIALIZED_COMMAND_COMPLETED"));

    let second = tokio::time::timeout(Duration::from_secs(10), second)
        .await
        .expect("queued same-user command runs after the first command")
        .expect("second serialized command task joins")
        .expect("second serialized command runs");
    assert_eq!(
        second.exit_code, 0,
        "same-user command did not wait for the lifecycle gate: {}",
        second.output
    );
    assert!(
        second
            .output
            .contains("SECOND_SERIALIZED_COMMAND_COMPLETED")
    );
    assert_eq!(cleanup.capture(&other_thread).id, user_container.id);
}

#[tokio::test]
async fn stopped_user_container_restarts_and_image_mismatch_recycles_it() {
    let Some(_image) = docker_worker_image("user container recycle test") else {
        return;
    };
    let primary = TestScope::unique("recycle");
    let mut other_thread = primary.clone();
    other_thread.thread = format!("other-thread-{}", InvocationId::new());
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([primary.clone(), other_thread.clone()]);
    let config = RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"));
    let transport = RebornScopedSandboxCommandTransport::connect(config.clone())
        .await
        .expect("Docker transport connects");

    let initial = transport
        .run_command(request(primary.resource_scope(), "echo INITIAL"))
        .await
        .expect("initial command runs");
    assert_eq!(
        initial.exit_code, 0,
        "initial command failed: {}",
        initial.output
    );
    let initial_container = cleanup.capture(&primary);
    docker_command(&["container", "stop", "--time", "1", &initial_container.id]);
    assert!(!inspect_container(&initial_container.id).running);

    let restarted = transport
        .run_command(request(other_thread.resource_scope(), "echo RESTARTED"))
        .await
        .expect("stopped compatible user container restarts from another thread");
    let restarted_container = cleanup.capture(&other_thread);
    assert_eq!(
        restarted.exit_code, 0,
        "restart command failed: {}",
        restarted.output
    );
    assert!(restarted.output.contains("RESTARTED"));
    assert_eq!(restarted_container.id, initial_container.id);
    assert_eq!(restarted_container.name, initial_container.name);
    assert_eq!(restarted_container.hostname, initial_container.hostname);
    assert!(restarted_container.running);

    let mismatch_image = format!("ironclaw-sandbox-mismatch-{}:test", InvocationId::new());
    cleanup.track_image(mismatch_image.clone());
    docker_command(&[
        "container",
        "commit",
        &restarted_container.id,
        &mismatch_image,
    ]);
    drop(transport);

    let replacement_transport =
        RebornScopedSandboxCommandTransport::connect(config.with_image(mismatch_image.clone()))
            .await
            .expect("replacement-image Docker transport connects");
    let replacement = replacement_transport
        .run_command(request(primary.resource_scope(), "echo RECYCLED"))
        .await
        .expect("user container image mismatch is safely recycled");
    let replacement_container = cleanup.capture(&primary);
    assert_eq!(
        replacement.exit_code, 0,
        "replacement command failed: {}",
        replacement.output
    );
    assert!(replacement.output.contains("RECYCLED"));
    assert_ne!(replacement_container.id, initial_container.id);
    assert_eq!(replacement_container.image, mismatch_image);
    assert_ne!(
        replacement_container.labels.get(LABEL_IMAGE),
        initial_container.labels.get(LABEL_IMAGE),
        "image identity label must change with the configured image"
    );
    assert_eq!(
        replacement_container.labels.get(LABEL_SECURITY_POSTURE),
        initial_container.labels.get(LABEL_SECURITY_POSTURE),
        "changing only the image must preserve the security posture"
    );
}

#[tokio::test]
async fn timeout_kills_descendants_while_nonzero_exit_remains_output() {
    let Some(_image) = docker_worker_image("user container timeout test") else {
        return;
    };
    let thread = TestScope::unique("timeout");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([thread.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(RebornSandboxConfig::new(
        temp.path().join("sandbox-workspaces"),
    ))
    .await
    .expect("Docker transport connects");
    let token = format!("timeout-descendant-{}", InvocationId::new());
    let mut timeout_request = request(
        thread.resource_scope(),
        format!(
            "printf '%s' '{token}' > /workspace/descendant.token; \
             python -c 'import time; time.sleep(60)' '{token}' & \
             child=$!; printf '%s' \"$child\" > /workspace/descendant.pid; wait \"$child\""
        ),
    );
    timeout_request.timeout_secs = Some(1);

    let error = transport
        .run_command(timeout_request)
        .await
        .expect_err("long-running command times out");
    let timed_out_container = cleanup.capture(&thread);
    assert_eq!(error, RuntimeProcessError::Timeout(Duration::from_secs(1)));
    assert!(timed_out_container.running);

    // Indentation-independent on purpose: Rust's `\`-line-continuation strips
    // leading whitespace, so an indented Python heredoc here would reach the
    // container dedented and fail to parse. The token is read from the file
    // rather than interpolated so this checker's own cmdline cannot match it.
    let descendant_check = transport
        .run_command(request(
            thread.resource_scope(),
            "pid=$(cat /workspace/descendant.pid); \
             token=$(cat /workspace/descendant.token); \
             i=0; \
             while [ \"$i\" -lt 100 ] && [ -d \"/proc/$pid\" ]; do \
             i=$((i+1)); sleep 0.02; done; \
             if [ -d \"/proc/$pid\" ]; then echo \"STILL_ALIVE_$pid\"; exit 1; fi; \
             if grep -al \"$token\" /proc/[0-9]*/cmdline 2>/dev/null; then \
             echo TOKEN_STILL_PRESENT; exit 1; fi; \
             echo DESCENDANT_GONE",
        ))
        .await
        .expect("post-timeout inspection runs");
    let after_timeout_container = cleanup.capture(&thread);
    assert_eq!(
        descendant_check.exit_code, 0,
        "descendant inspection failed: {}",
        descendant_check.output
    );
    assert!(descendant_check.output.contains("DESCENDANT_GONE"));
    assert_eq!(after_timeout_container.id, timed_out_container.id);

    let nonzero = transport
        .run_command(request(
            thread.resource_scope(),
            "echo EXPECTED_NONZERO_STDERR >&2; exit 23",
        ))
        .await
        .expect("non-zero exit remains an ordinary command result");
    assert_eq!(nonzero.exit_code, 23);
    assert!(nonzero.output.contains("EXPECTED_NONZERO_STDERR"));
    assert!(nonzero.sandboxed);
}

#[tokio::test]
async fn idle_stop_respects_one_active_serialized_command_and_restarts_the_same_container() {
    let Some(_image) = docker_worker_image("serialized user container idle-stop test") else {
        return;
    };
    let primary = TestScope::unique("idle");
    let mut other_thread = primary.clone();
    other_thread.thread = format!("other-thread-{}", InvocationId::new());
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([primary.clone(), other_thread.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
            .with_idle_timeout(Duration::from_secs(1)),
    )
    .await
    .expect("Docker transport connects");

    let active_transport = transport.clone();
    let active_scope = primary.resource_scope();
    let active = tokio::spawn(async move {
        active_transport
            .run_command(request(
                active_scope,
                "touch /tmp/idle-active; \
                 i=0; \
                 while [ ! -e /tmp/idle-release ] && [ \"$i\" -lt 100 ]; do \
                 i=$((i+1)); sleep 0.1; done; \
                 if [ ! -e /tmp/idle-release ]; then \
                 echo IDLE_RELEASE_BARRIER_EXPIRED; exit 93; fi; \
                 touch /tmp/idle-active-finished; \
                 echo ACTIVE_SERIALIZED_COMMAND_COMPLETED",
            ))
            .await
    });

    let running_container = wait_for_user_container(&primary, Duration::from_secs(10)).await;
    cleanup.container_ids.insert(running_container.id.clone());
    wait_for_container_file(
        &running_container.id,
        "/tmp/idle-active",
        Duration::from_secs(10),
    )
    .await;

    let launch_queued = Arc::new(tokio::sync::Barrier::new(2));
    let queued_transport = transport.clone();
    let queued_scope = other_thread.resource_scope();
    let queued_launch = launch_queued.clone();
    let queued = tokio::spawn(async move {
        queued_launch.wait().await;
        queued_transport
            .run_command(request(
                queued_scope,
                "touch /tmp/idle-queued-entered; \
                 if [ ! -e /tmp/idle-active-finished ]; then \
                 echo QUEUED_COMMAND_OVERLAPPED_ACTIVE_COMMAND; exit 94; fi; \
                 echo QUEUED_SERIALIZED_COMMAND_COMPLETED",
            ))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), launch_queued.wait())
        .await
        .expect("queued command reaches its bounded launch barrier");

    tokio::time::sleep(Duration::from_millis(1200)).await;
    let queued_marker = Command::new("docker")
        .args(["container", "exec"])
        .arg(&running_container.id)
        .args(["test", "!", "-e", "/tmp/idle-queued-entered"])
        .output()
        .expect("docker container exec starts");
    assert!(
        queued_marker.status.success(),
        "a queued same-user shell command must not enter while the active command holds the gate"
    );
    assert!(
        !queued.is_finished(),
        "a queued same-user shell command must wait behind the intentional serialization gate"
    );
    assert!(
        inspect_container(&running_container.id).running,
        "idle stop must not interrupt one active serialized user command"
    );

    docker_command(&[
        "container",
        "exec",
        &running_container.id,
        "touch",
        "/tmp/idle-release",
    ]);
    let active = tokio::time::timeout(Duration::from_secs(10), active)
        .await
        .expect("active command completes after its release barrier")
        .expect("active command task joins")
        .expect("active command runs");
    assert_eq!(
        active.exit_code, 0,
        "idle stop interrupted the active command: {}",
        active.output
    );
    assert!(
        active
            .output
            .contains("ACTIVE_SERIALIZED_COMMAND_COMPLETED")
    );

    let queued = tokio::time::timeout(Duration::from_secs(10), queued)
        .await
        .expect("queued command runs after the active command releases the gate")
        .expect("queued command task joins")
        .expect("queued command runs");
    assert_eq!(
        queued.exit_code, 0,
        "queued command did not wait for the active command: {}",
        queued.output
    );
    assert!(
        queued
            .output
            .contains("QUEUED_SERIALIZED_COMMAND_COMPLETED")
    );

    let stopped =
        wait_for_running_state(&running_container.id, false, Duration::from_secs(10)).await;
    assert_eq!(stopped.id, running_container.id);

    let restarted = transport
        .run_command(request(
            other_thread.resource_scope(),
            "echo IDLE_RESTARTED",
        ))
        .await
        .expect("either thread can restart the idle-stopped user container");
    let restarted_container = cleanup.capture(&primary);
    assert_eq!(
        restarted.exit_code, 0,
        "idle restart failed: {}",
        restarted.output
    );
    assert!(restarted.output.contains("IDLE_RESTARTED"));
    assert_eq!(restarted_container.id, running_container.id);
    assert_eq!(restarted_container.name, running_container.name);
    assert_eq!(restarted_container.hostname, running_container.hostname);
    assert!(restarted_container.running);
}

#[tokio::test]
async fn thread_id_none_uses_and_reuses_the_user_container() {
    let Some(_image) = docker_worker_image("optional thread identity test") else {
        return;
    };
    let user = TestScope::unique("optional-thread");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([user.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(RebornSandboxConfig::new(
        temp.path().join("sandbox-workspaces"),
    ))
    .await
    .expect("Docker transport connects");
    let mut no_thread = user.resource_scope();
    no_thread.thread_id = None;
    let workspace_marker = format!("workspace-{}", InvocationId::new());
    let ephemeral_marker = format!("ephemeral-{}", InvocationId::new());

    let without_thread = transport
        .run_command(request(
            no_thread,
            format!(
                "printf '%s' '{workspace_marker}' > /workspace/optional-thread; \
                 printf '%s' '{ephemeral_marker}' > /tmp/optional-thread; \
                 echo NO_THREAD_OK"
            ),
        ))
        .await
        .expect("scope without a thread id runs");
    assert_eq!(
        without_thread.exit_code, 0,
        "threadless command failed: {}",
        without_thread.output
    );
    assert!(without_thread.output.contains("NO_THREAD_OK"));
    let without_thread_container = cleanup.capture(&user);

    let with_thread = transport
        .run_command(request(
            user.resource_scope(),
            "cat /workspace/optional-thread /tmp/optional-thread",
        ))
        .await
        .expect("threaded scope reuses the user container");
    let with_thread_container = cleanup.capture(&user);
    assert_eq!(
        with_thread.exit_code, 0,
        "threaded command failed: {}",
        with_thread.output
    );
    assert!(with_thread.output.contains(&workspace_marker));
    assert!(with_thread.output.contains(&ephemeral_marker));
    assert_eq!(with_thread_container.id, without_thread_container.id);
    assert_eq!(with_thread_container.name, without_thread_container.name);
    assert_eq!(
        with_thread_container.hostname,
        without_thread_container.hostname
    );
    assert_stable_identity(&with_thread_container, &user);
}

#[tokio::test]
#[ignore = "requires public DNS and Internet access; run as a live egress canary"]
async fn sandbox_profile_allows_public_https_egress() {
    let Some(_image) = docker_worker_image("sandbox egress canary") else {
        return;
    };

    let egress_scope = TestScope::unique("egress");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([egress_scope.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces")).with_network_enabled(),
    )
    .await
    .expect("Docker transport connects");

    let result = transport
        .run_command(request(
            egress_scope.resource_scope(),
            "python -c \"import os, urllib.request; assert os.environ['IRONCLAW_REBORN_NETWORK_MODE'] == 'direct'; response = urllib.request.urlopen('https://example.com', timeout=15); assert response.status == 200; response.close(); print('SANDBOX_PUBLIC_HTTPS_OK')\"",
        ))
        .await
        .expect("public HTTPS request runs");
    let container = cleanup.capture(&egress_scope);
    assert_stable_identity(&container, &egress_scope);

    assert_eq!(
        result.exit_code, 0,
        "egress canary failed: {}",
        result.output
    );
    assert!(result.output.contains("SANDBOX_PUBLIC_HTTPS_OK"));
    assert!(result.sandboxed);
}
