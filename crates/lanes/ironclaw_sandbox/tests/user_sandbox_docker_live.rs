//! Real-Docker proof for persistent per-user containers and workspaces.

use std::{collections::HashMap, process::Command, sync::Arc, time::Duration};

use ironclaw_host_api::{
    ids::InvocationId,
    process::{
        CredentialedSandboxCommandRequest, RuntimeProcessError, SandboxCommandCredential,
        SandboxCommandTransport,
    },
};
use ironclaw_sandbox::{RebornSandboxConfig, RebornScopedSandboxCommandTransport};

#[path = "support/docker_gate.rs"]
mod docker_gate;
#[path = "support/user_sandbox_live.rs"]
mod user_sandbox_live;
use user_sandbox_live::*;

static LIVE_DOCKER_TEST_SERIALIZER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn docker_worker_image(
    test_name: &str,
) -> Option<(String, tokio::sync::MutexGuard<'static, ()>)> {
    if !docker_gate::docker_available() {
        eprintln!("SKIP: {test_name} — no Docker daemon is reachable");
        return None;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!("SKIP: {test_name} — worker image {image:?} is not built");
        return None;
    }
    let serial = LIVE_DOCKER_TEST_SERIALIZER.lock().await;
    Some((image, serial))
}

async fn docker_worker_and_proxy_images(
    test_name: &str,
) -> Option<(String, tokio::sync::MutexGuard<'static, ()>)> {
    let ready = docker_worker_image(test_name).await?;
    let proxy_image = docker_gate::configured_sandbox_proxy_image();
    if !docker_gate::docker_image_available(&proxy_image) {
        eprintln!("SKIP: {test_name} — proxy image {proxy_image:?} is not present");
        return None;
    }
    Some(ready)
}

async fn wait_for_host_path(path: &std::path::Path, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("host path {path:?} was not created within {timeout:?}");
}

#[path = "user_sandbox_docker_live/extra.rs"]
mod extra;

#[tokio::test]
async fn user_container_reuses_state_across_threads_and_isolates_other_users_and_tenants() {
    let Some((_image, _serial)) = docker_worker_and_proxy_images("user container reuse test").await
    else {
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
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
            .with_managed_egress_proxy()
            .expect("managed egress policy is valid"),
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
                 assert os.environ['IRONCLAW_REBORN_NETWORK_MODE'] == 'brokered'\n\
                 assert os.environ['HTTPS_PROXY'].startswith('http://')\n\
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
    let Some((_image, _serial)) = docker_worker_image("user container adoption test").await else {
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
async fn workspace_owner_lock_rejects_a_second_live_transport() {
    let Some((_image, _serial)) = docker_worker_image("workspace owner lock test").await else {
        return;
    };
    let temp = docker_visible_tempdir();
    let config = RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"));
    let first = RebornScopedSandboxCommandTransport::connect(config.clone())
        .await
        .expect("first transport acquires workspace ownership");

    let error = RebornScopedSandboxCommandTransport::connect(config.clone())
        .await
        .expect_err("second live transport must not share workspace ownership");
    assert!(
        error.to_string().contains("already owned"),
        "ownership error must be sanitized and actionable: {error}"
    );

    drop(first);
    RebornScopedSandboxCommandTransport::connect(config)
        .await
        .expect("workspace ownership releases with the final transport");
}

#[tokio::test]
#[ignore = "subprocess entrypoint for workspace ownership proof"]
async fn workspace_owner_lock_child_process() {
    let Ok(role) = std::env::var("IRONCLAW_SANDBOX_OWNER_LOCK_CHILD") else {
        return;
    };
    let workspace_root = std::env::var("IRONCLAW_SANDBOX_OWNER_LOCK_ROOT")
        .expect("owner-lock child receives workspace root");
    let ready_path = std::env::var("IRONCLAW_SANDBOX_OWNER_LOCK_READY")
        .expect("owner-lock child receives ready path");
    let release_path = std::env::var("IRONCLAW_SANDBOX_OWNER_LOCK_RELEASE")
        .expect("owner-lock child receives release path");
    let config = RebornSandboxConfig::new(workspace_root);

    match role.as_str() {
        "owner" => {
            let transport = RebornScopedSandboxCommandTransport::connect(config)
                .await
                .expect("owner child acquires workspace ownership");
            std::fs::write(&ready_path, b"ready").expect("owner child signals readiness");
            wait_for_host_path(std::path::Path::new(&release_path), Duration::from_secs(30)).await;
            drop(transport);
        }
        "contender" => {
            let error = RebornScopedSandboxCommandTransport::connect(config)
                .await
                .expect_err("independent process must not share workspace ownership");
            assert!(
                error.to_string().contains("already owned"),
                "cross-process ownership error must be sanitized and actionable: {error}"
            );
        }
        unexpected => panic!("unexpected owner-lock child role {unexpected:?}"),
    }
}

#[tokio::test]
async fn workspace_owner_lock_rejects_an_independent_process() {
    let Some((_image, _serial)) =
        docker_worker_image("cross-process workspace owner lock test").await
    else {
        return;
    };
    let temp = docker_visible_tempdir();
    let workspace_root = temp.path().join("sandbox-workspaces");
    let ready_path = temp.path().join("owner-ready");
    let release_path = temp.path().join("owner-release");
    let test_binary = std::env::current_exe().expect("current integration-test binary is known");
    let child_args = [
        "--exact",
        "workspace_owner_lock_child_process",
        "--ignored",
        "--nocapture",
    ];

    let mut owner = Command::new(&test_binary)
        .args(child_args)
        .env("IRONCLAW_SANDBOX_OWNER_LOCK_CHILD", "owner")
        .env("IRONCLAW_SANDBOX_OWNER_LOCK_ROOT", &workspace_root)
        .env("IRONCLAW_SANDBOX_OWNER_LOCK_READY", &ready_path)
        .env("IRONCLAW_SANDBOX_OWNER_LOCK_RELEASE", &release_path)
        .spawn()
        .expect("owner child process starts");
    wait_for_host_path(&ready_path, Duration::from_secs(10)).await;

    let contender = Command::new(&test_binary)
        .args(child_args)
        .env("IRONCLAW_SANDBOX_OWNER_LOCK_CHILD", "contender")
        .env("IRONCLAW_SANDBOX_OWNER_LOCK_ROOT", &workspace_root)
        .env("IRONCLAW_SANDBOX_OWNER_LOCK_READY", &ready_path)
        .env("IRONCLAW_SANDBOX_OWNER_LOCK_RELEASE", &release_path)
        .output()
        .expect("contender child process runs");
    std::fs::write(&release_path, b"release").expect("parent releases owner child");
    let owner_status = owner.wait().expect("owner child process exits");

    assert!(
        contender.status.success(),
        "contender child did not observe the ownership rejection: {}",
        String::from_utf8_lossy(&contender.stderr)
    );
    assert!(owner_status.success(), "owner child exited unsuccessfully");
}

#[tokio::test]
async fn same_user_shell_commands_are_intentionally_serialized_across_threads() {
    let Some((_image, _serial)) = docker_worker_image("same-user command serialization test").await
    else {
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
async fn different_users_execute_in_parallel() {
    let Some((_image, _serial)) = docker_worker_image("cross-user parallelism test").await else {
        return;
    };
    let first_user = TestScope::unique("parallel-a");
    let second_user = TestScope::unique("parallel-b");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([first_user.clone(), second_user.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(RebornSandboxConfig::new(
        temp.path().join("sandbox-workspaces"),
    ))
    .await
    .expect("Docker transport connects");

    let first_transport = transport.clone();
    let first_scope = first_user.resource_scope();
    let first = tokio::spawn(async move {
        first_transport
            .run_command(request(
                first_scope,
                "touch /tmp/parallel-a-started; \
                 while [ ! -e /tmp/parallel-a-release ]; do sleep 0.02; done; \
                 echo USER_A_COMPLETED",
            ))
            .await
    });
    let first_container = wait_for_user_container(&first_user, Duration::from_secs(10)).await;
    cleanup.container_ids.insert(first_container.id.clone());
    wait_for_container_file(
        &first_container.id,
        "/tmp/parallel-a-started",
        Duration::from_secs(10),
    )
    .await;

    let second = tokio::time::timeout(
        Duration::from_secs(5),
        transport.run_command(request(
            second_user.resource_scope(),
            "echo USER_B_COMPLETED",
        )),
    )
    .await
    .expect("a different user's command is not blocked by user A")
    .expect("user B command runs");
    assert_eq!(second.exit_code, 0, "user B output: {}", second.output);
    assert!(second.output.contains("USER_B_COMPLETED"));
    let second_container = cleanup.capture(&second_user);
    assert_ne!(first_container.id, second_container.id);

    docker_command(&[
        "container",
        "exec",
        &first_container.id,
        "touch",
        "/tmp/parallel-a-release",
    ]);
    let first = first
        .await
        .expect("user A task joins")
        .expect("user A command completes");
    assert_eq!(first.exit_code, 0, "user A output: {}", first.output);
    assert!(first.output.contains("USER_A_COMPLETED"));
}

#[tokio::test]
async fn aborting_caller_does_not_release_same_user_serialization() {
    let Some((_image, _serial)) = docker_worker_image("user container cancellation test").await
    else {
        return;
    };
    let primary = TestScope::unique("cancel");
    let mut other_thread = primary.clone();
    other_thread.thread = format!("other-thread-{}", InvocationId::new());
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([primary.clone(), other_thread.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(RebornSandboxConfig::new(
        temp.path().join("sandbox-workspaces"),
    ))
    .await
    .expect("Docker transport connects");

    let first_transport = transport.clone();
    let first_scope = primary.resource_scope();
    let token = format!("cancelled-exec-{}", InvocationId::new());
    let mut first_request = request(
        first_scope,
        format!(
            "printf '%s' '{token}' > /workspace/cancel.token; \
             touch /tmp/cancel-started; \
             python -c 'import time; time.sleep(3)' '{token}'; \
             touch /tmp/cancel-finished"
        ),
    );
    first_request.timeout_secs = Some(10);
    let first = tokio::spawn(async move { first_transport.run_command(first_request).await });

    let container = wait_for_user_container(&primary, Duration::from_secs(10)).await;
    cleanup.container_ids.insert(container.id.clone());
    wait_for_container_file(
        &container.id,
        "/tmp/cancel-started",
        Duration::from_secs(10),
    )
    .await;
    first.abort();
    let _ = first.await;

    let second = transport
        .run_command(request(
            other_thread.resource_scope(),
            "token=$(cat /workspace/cancel.token); \
             if grep -al \"$token\" /proc/[0-9]*/cmdline 2>/dev/null; then \
             echo CANCELLED_EXEC_STILL_RUNNING; exit 1; fi; \
             echo CANCELLED_CALLER_DID_NOT_OVERLAP",
        ))
        .await
        .expect("queued same-user command runs after detached execution settles");
    assert_eq!(
        second.exit_code, 0,
        "aborted caller released the lifecycle gate too early: {}",
        second.output
    );
    assert!(second.output.contains("CANCELLED_CALLER_DID_NOT_OVERLAP"));
    let final_container = cleanup.capture(&other_thread);
    cleanup.container_ids.insert(final_container.id);
}

#[tokio::test]
async fn stopped_user_container_restarts_and_image_mismatch_recycles_it() {
    let Some((_image, _serial)) = docker_worker_image("user container recycle test").await else {
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
    assert_eq!(
        replacement_container.labels.get(LABEL_IMAGE),
        Some(&replacement_container.image),
        "container config and compatibility label use one immutable image id"
    );
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
async fn foreground_exit_reaps_descendant_that_detaches_into_a_new_session() {
    let Some((_image, _serial)) = docker_worker_image("detached descendant cleanup test").await
    else {
        return;
    };
    let user = TestScope::unique("detached");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([user.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(RebornSandboxConfig::new(
        temp.path().join("sandbox-workspaces"),
    ))
    .await
    .expect("Docker transport connects");
    let token = format!("detached-descendant-{}", InvocationId::new());

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        transport.run_command(request(
            user.resource_scope(),
            format!(
                "setsid python -c 'import time; time.sleep(300)' '{token}' \
                 >/dev/null 2>&1 </dev/null & \
                 printf '%s' \"$!\" > /workspace/detached.pid; \
                 printf '%s' '{token}' > /workspace/detached.token; \
                 echo FOREGROUND_RETURNED"
            ),
        )),
    )
    .await
    .expect("foreground command and detached cleanup are bounded")
    .expect("foreground command succeeds");
    assert_eq!(result.exit_code, 0, "foreground output: {}", result.output);
    assert!(result.output.contains("FOREGROUND_RETURNED"));
    let container = cleanup.capture(&user);

    let inspection = transport
        .run_command(request(
            user.resource_scope(),
            "pid=$(cat /workspace/detached.pid); \
             token=$(cat /workspace/detached.token); \
             if [ -d \"/proc/$pid\" ]; then echo DETACHED_PID_ALIVE; exit 1; fi; \
             if grep -al \"$token\" /proc/[0-9]*/cmdline 2>/dev/null; then \
             echo DETACHED_TOKEN_STILL_PRESENT; exit 1; fi; \
             echo DETACHED_DESCENDANT_GONE",
        ))
        .await
        .expect("post-exit descendant inspection runs");
    assert_eq!(
        inspection.exit_code, 0,
        "detached descendant survived: {}",
        inspection.output
    );
    assert!(inspection.output.contains("DETACHED_DESCENDANT_GONE"));
    assert_eq!(cleanup.capture(&user).id, container.id);
}

#[tokio::test]
async fn mutable_image_tag_retarget_recycles_existing_user_container() {
    let Some((base_image, _serial)) = docker_worker_image("mutable sandbox image test").await
    else {
        return;
    };
    let user = TestScope::unique("mutable-image");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([user.clone()]);
    let mutable_tag = format!("ironclaw-sandbox-mutable-{}:test", InvocationId::new());
    let replacement_tag = format!(
        "ironclaw-sandbox-mutable-replacement-{}:test",
        InvocationId::new()
    );
    cleanup.track_image(mutable_tag.clone());
    cleanup.track_image(replacement_tag.clone());
    docker_command(&["image", "tag", &base_image, &mutable_tag]);

    let config = RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
        .with_image(mutable_tag.clone());
    let transport = RebornScopedSandboxCommandTransport::connect(config)
        .await
        .expect("Docker transport connects");
    transport
        .run_command(request(user.resource_scope(), "echo MUTABLE_INITIAL"))
        .await
        .expect("initial mutable-tag command runs");
    let initial = cleanup.capture(&user);

    docker_command(&["container", "commit", &initial.id, &replacement_tag]);
    docker_command(&["image", "tag", &replacement_tag, &mutable_tag]);

    let result = transport
        .run_command(request(user.resource_scope(), "echo MUTABLE_RETARGETED"))
        .await
        .expect("same configured tag resolves its new immutable image");
    let replacement = cleanup.capture(&user);
    assert_eq!(result.exit_code, 0, "retargeted command: {}", result.output);
    assert!(result.output.contains("MUTABLE_RETARGETED"));
    assert_ne!(
        replacement.id, initial.id,
        "same-tag retarget must recycle the old resolved image"
    );
    assert_ne!(
        replacement.labels.get(LABEL_IMAGE),
        initial.labels.get(LABEL_IMAGE),
        "image label stores immutable Docker image identity"
    );
}

#[tokio::test]
async fn idle_sweeper_reclaims_an_incompatible_running_container() {
    let Some((_image, _serial)) = docker_worker_image("incompatible idle container test").await
    else {
        return;
    };
    let user = TestScope::unique("incompatible-idle");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([user.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
            .with_idle_timeout(Duration::from_secs(1)),
    )
    .await
    .expect("Docker transport connects");
    transport
        .run_command(request(user.resource_scope(), "echo READY_TO_RECYCLE"))
        .await
        .expect("user container starts");
    let container = cleanup.capture(&user);

    docker_command(&["container", "pause", &container.id]);
    wait_for_container_absent(&container.id, Duration::from_secs(10)).await;
    cleanup.container_ids.remove(&container.id);
}

#[tokio::test]
async fn timeout_kills_descendants_while_nonzero_exit_remains_output() {
    let Some((_image, _serial)) = docker_worker_image("user container timeout test").await else {
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

    let ordinary_124 = transport
        .run_command(request(
            thread.resource_scope(),
            "echo ORDINARY_124_OUTPUT; exit 124",
        ))
        .await
        .expect("ordinary exit 124 remains a command result");
    assert_eq!(ordinary_124.exit_code, 124);
    assert!(ordinary_124.output.contains("ORDINARY_124_OUTPUT"));

    let signaled = transport
        .run_command(request(thread.resource_scope(), "kill -TERM $$"))
        .await
        .expect("signal termination remains an ordinary command result");
    assert_eq!(signaled.exit_code, 143);
}

#[tokio::test]
async fn idle_stop_respects_one_active_serialized_command_and_restarts_the_same_container() {
    let Some((_image, _serial)) =
        docker_worker_image("serialized user container idle-stop test").await
    else {
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
        .args([
            "sh",
            "-c",
            "if [ -e /tmp/idle-queued-entered ]; then printf present; else printf absent; fi",
        ])
        .output()
        .expect("docker container exec starts");
    assert!(
        queued_marker.status.success(),
        "queued-marker inspection itself failed: {}",
        String::from_utf8_lossy(&queued_marker.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&queued_marker.stdout).trim(),
        "absent",
        "a queued same-user shell command entered while the active command held the gate"
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
