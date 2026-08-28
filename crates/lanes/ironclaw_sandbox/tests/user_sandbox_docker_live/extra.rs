use super::*;

fn labeled_resource_ids(
    resource_args: &[&str],
    tenant_label: &str,
    user_label: &str,
    scope: &TestScope,
) -> Vec<String> {
    let tenant = format!("label={tenant_label}={}", scope.tenant);
    let user = format!("label={user_label}={}", scope.user);
    let output = Command::new("docker")
        .args(resource_args)
        .args([
            "--quiet",
            "--filter",
            tenant.as_str(),
            "--filter",
            user.as_str(),
        ])
        .output()
        .expect("docker lists managed-egress resources");
    assert!(
        output.status.success(),
        "docker managed-egress resource listing failed"
    );
    String::from_utf8(output.stdout)
        .expect("docker resource ids are UTF-8")
        .lines()
        .map(str::to_string)
        .collect()
}

fn managed_proxy_ids(scope: &TestScope) -> Vec<String> {
    labeled_resource_ids(
        &["container", "list", "--all"],
        ironclaw_sandbox::sandbox_process::USER_SANDBOX_PROXY_LABEL_TENANT,
        ironclaw_sandbox::sandbox_process::USER_SANDBOX_PROXY_LABEL_USER,
        scope,
    )
}

fn managed_proxy_id(scope: &TestScope) -> String {
    let ids = managed_proxy_ids(scope);
    assert_eq!(ids.len(), 1, "expected one managed-egress proxy: {ids:?}");
    ids[0].clone()
}

fn proxy_logs(proxy_id: &str) -> String {
    let output = Command::new("docker")
        .args(["logs", proxy_id])
        .output()
        .expect("docker reads proxy logs");
    assert!(output.status.success(), "docker proxy log read failed");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn container_network_ids(container_id: &str) -> Vec<String> {
    let output = Command::new("docker")
        .args([
            "container",
            "inspect",
            "--format",
            "{{range .NetworkSettings.Networks}}{{.NetworkID}}\n{{end}}",
            container_id,
        ])
        .output()
        .expect("docker inspects container networks");
    assert!(
        output.status.success(),
        "docker container network inspection failed"
    );
    String::from_utf8(output.stdout)
        .expect("docker network ids are UTF-8")
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn managed_network_ids(scope: &TestScope) -> Vec<String> {
    labeled_resource_ids(
        &["network", "list", "--no-trunc"],
        ironclaw_sandbox::sandbox_process::USER_SANDBOX_NETWORK_LABEL_TENANT,
        ironclaw_sandbox::sandbox_process::USER_SANDBOX_NETWORK_LABEL_USER,
        scope,
    )
}

fn user_container_ids(scope: &TestScope) -> Vec<String> {
    labeled_resource_ids(
        &["container", "list", "--all"],
        ironclaw_sandbox::sandbox_process::USER_SANDBOX_LABEL_TENANT,
        ironclaw_sandbox::sandbox_process::USER_SANDBOX_LABEL_USER,
        scope,
    )
}

async fn wait_for_managed_bundle_suspension(scope: &TestScope, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if managed_proxy_ids(scope).is_empty() && managed_network_ids(scope).len() == 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "managed-egress bundle did not suspend: proxies={:?}, networks={:?}",
        managed_proxy_ids(scope),
        managed_network_ids(scope)
    );
}

async fn wait_for_managed_bundle_removal(scope: &TestScope, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if managed_proxy_ids(scope).is_empty() && managed_network_ids(scope).is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "managed-egress bundle still exists: proxies={:?}, networks={:?}",
        managed_proxy_ids(scope),
        managed_network_ids(scope)
    );
}

async fn wait_for_user_container_removal(scope: &TestScope, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if user_container_ids(scope).is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "stopped user sandbox still exists after retention: {:?}",
        user_container_ids(scope)
    );
}

#[tokio::test]
async fn thread_id_none_uses_and_reuses_the_user_container() {
    let Some((_image, _serial)) = docker_worker_image("optional thread identity test").await else {
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
async fn restarted_transport_reconciles_and_idles_an_unrequested_container() {
    let Some((_image, _serial)) = docker_worker_image("restart reconciliation test").await else {
        return;
    };
    let user = TestScope::unique("reconcile");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([user.clone()]);
    let config = RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
        .with_idle_timeout(Duration::from_secs(1));
    let original = RebornScopedSandboxCommandTransport::connect(config.clone())
        .await
        .expect("original transport connects");
    original
        .run_command(request(user.resource_scope(), "echo RECONCILE_INITIAL"))
        .await
        .expect("initial command runs");
    let container = cleanup.capture(&user);
    original.shutdown().await;
    drop(original);

    let replacement = RebornScopedSandboxCommandTransport::connect(config)
        .await
        .expect("replacement transport connects");
    let stopped = wait_for_running_state(&container.id, false, Duration::from_secs(10)).await;
    assert_eq!(stopped.id, container.id);

    let resumed = replacement
        .run_command(request(user.resource_scope(), "echo RECONCILED_RESTART"))
        .await
        .expect("reconciled container restarts");
    assert_eq!(resumed.exit_code, 0, "resumed output: {}", resumed.output);
    assert!(resumed.output.contains("RECONCILED_RESTART"));
    assert_eq!(cleanup.capture(&user).id, container.id);
}

#[tokio::test]
async fn restarted_transport_adopts_running_managed_egress_bundle() {
    let Some((_image, _serial)) =
        docker_worker_and_proxy_images("managed egress restart adoption").await
    else {
        return;
    };
    let user = TestScope::unique("egress-restart");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([user.clone()]);
    let config = RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
        .with_managed_egress_proxy()
        .expect("managed egress policy is valid");
    let transport = RebornScopedSandboxCommandTransport::connect(config.clone())
        .await
        .expect("first Docker transport connects");

    let first = transport
        .run_command(request(user.resource_scope(), "echo FIRST_EGRESS_BUNDLE"))
        .await
        .expect("first managed-egress command runs");
    assert_eq!(first.exit_code, 0, "first command failed: {}", first.output);
    let first_container = cleanup.capture(&user);
    let first_proxy = managed_proxy_id(&user);
    cleanup.container_ids.insert(first_proxy.clone());

    transport.shutdown().await;
    drop(transport);
    let replacement = RebornScopedSandboxCommandTransport::connect(config)
        .await
        .expect("replacement Docker transport connects");
    let second = replacement
        .run_command(request(
            user.resource_scope(),
            "test \"$IRONCLAW_REBORN_NETWORK_MODE\" = brokered && echo ADOPTED_EGRESS_BUNDLE",
        ))
        .await
        .expect("replacement managed-egress command runs");
    assert_eq!(
        second.exit_code, 0,
        "replacement command failed: {}",
        second.output
    );
    assert!(second.output.contains("ADOPTED_EGRESS_BUNDLE"));
    let second_container = cleanup.capture(&user);
    let second_proxy = managed_proxy_id(&user);
    cleanup.container_ids.insert(second_proxy.clone());

    assert_eq!(second_container.id, first_container.id);
    assert_eq!(second_proxy, first_proxy);
}

#[tokio::test]
async fn setup_failure_rolls_back_a_new_managed_egress_bundle() {
    let Some((_image, _serial)) =
        docker_worker_and_proxy_images("managed egress setup rollback").await
    else {
        return;
    };
    let user = TestScope::unique("egress-rollback");
    let temp = docker_visible_tempdir();
    let _cleanup = DockerCleanup::with_scopes([user.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
            .with_managed_egress_proxy()
            .expect("managed egress policy is valid"),
    )
    .await
    .expect("Docker transport connects");
    let mut invalid = request(user.resource_scope(), "echo MUST_NOT_RUN");
    // Fail after the managed-egress bundle is provisioned, at command-environment
    // validation, so the rollback path has resources to remove.
    invalid
        .extra_env
        .insert("INVALID=NAME".to_string(), "rejected".to_string());

    transport
        .run_command(invalid)
        .await
        .expect_err("caller environment injection must fail");

    assert!(user_container_ids(&user).is_empty());
    assert!(managed_proxy_ids(&user).is_empty());
    assert!(managed_network_ids(&user).is_empty());
}

#[tokio::test]
async fn proxy_config_failure_rolls_back_partially_provisioned_networks() {
    let Some((_image, _serial)) =
        docker_worker_and_proxy_images("managed egress partial provisioning rollback").await
    else {
        return;
    };
    let user = TestScope::unique("egress-partial-rollback");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([user.clone()]);
    let config = RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
        .with_managed_egress_proxy()
        .expect("managed egress policy is valid");
    let transport = RebornScopedSandboxCommandTransport::connect(config)
        .await
        .expect("Docker transport connects");
    transport
        .run_command(request(user.resource_scope(), "echo CREATE_INITIAL_BUNDLE"))
        .await
        .expect("initial managed-egress command runs");
    let worker = cleanup.capture(&user);
    let proxy = managed_proxy_id(&user);
    cleanup.container_ids.insert(proxy.clone());
    let networks = managed_network_ids(&user);
    assert_eq!(networks.len(), 1);

    let removed_proxy = Command::new("docker")
        .args(["rm", "--force", proxy.as_str()])
        .output()
        .expect("docker removes initial proxy");
    assert!(removed_proxy.status.success());
    for network in &networks {
        let _ = Command::new("docker")
            .args([
                "network",
                "disconnect",
                "--force",
                network.as_str(),
                worker.id.as_str(),
            ])
            .output();
        let removed = Command::new("docker")
            .args(["network", "rm", network.as_str()])
            .output()
            .expect("docker removes initial managed network");
        assert!(
            removed.status.success(),
            "network removal failed: {removed:?}"
        );
    }

    let material_root = temp
        .path()
        .join("sandbox-workspaces")
        .join(".managed-egress");
    let proxy_material = std::fs::read_dir(&material_root)
        .expect("managed-egress material root exists")
        .next()
        .expect("proxy material directory exists")
        .expect("proxy material entry is readable")
        .path();
    let proxy_config = proxy_material.join("proxy.yaml");
    std::fs::remove_file(&proxy_config).expect("initial proxy config is removable");
    std::fs::create_dir(&proxy_config).expect("proxy config path is poisoned");

    transport
        .run_command(request(user.resource_scope(), "echo MUST_NOT_RUN"))
        .await
        .expect_err("proxy config directory must fail provisioning");

    assert!(managed_proxy_ids(&user).is_empty());
    assert!(managed_network_ids(&user).is_empty());
}

#[tokio::test]
async fn restart_removes_managed_egress_bundle_when_worker_is_missing() {
    let Some((_image, _serial)) =
        docker_worker_and_proxy_images("managed egress orphan reconciliation").await
    else {
        return;
    };
    let user = TestScope::unique("egress-orphan");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([user.clone()]);
    let config = RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
        .with_managed_egress_proxy()
        .expect("managed egress policy is valid");
    let transport = RebornScopedSandboxCommandTransport::connect(config.clone())
        .await
        .expect("first Docker transport connects");
    let result = transport
        .run_command(request(user.resource_scope(), "echo CREATE_EGRESS_BUNDLE"))
        .await
        .expect("managed-egress command runs");
    assert_eq!(result.exit_code, 0, "command failed: {}", result.output);
    let worker = cleanup.capture(&user);
    let proxy = managed_proxy_id(&user);
    cleanup.container_ids.insert(proxy);
    assert_eq!(managed_network_ids(&user).len(), 1);

    transport.shutdown().await;
    drop(transport);
    let removed = Command::new("docker")
        .args(["rm", "--force", worker.id.as_str()])
        .output()
        .expect("docker removes orphaned worker");
    assert!(
        removed.status.success(),
        "worker removal failed: {removed:?}"
    );
    assert_eq!(managed_proxy_ids(&user).len(), 1);
    assert_eq!(managed_network_ids(&user).len(), 1);

    let replacement = RebornScopedSandboxCommandTransport::connect(config)
        .await
        .expect("replacement Docker transport connects");

    assert!(managed_proxy_ids(&user).is_empty());
    assert!(managed_network_ids(&user).is_empty());
    replacement.shutdown().await;
}

#[tokio::test]
async fn sweeper_removes_managed_egress_bundle_after_worker_disappears() {
    let Some((_image, _serial)) =
        docker_worker_and_proxy_images("managed egress missing-worker cleanup").await
    else {
        return;
    };
    let user = TestScope::unique("egress-missing-worker");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([user.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
            .with_idle_timeout(Duration::from_millis(200))
            .with_managed_egress_proxy()
            .expect("managed egress policy is valid"),
    )
    .await
    .expect("Docker transport connects");
    transport
        .run_command(request(user.resource_scope(), "echo BUNDLE_BEFORE_LOSS"))
        .await
        .expect("managed-egress command runs");
    let worker = cleanup.capture(&user);
    let proxy = managed_proxy_id(&user);
    cleanup.container_ids.insert(proxy);

    let removed = tokio::process::Command::new("docker")
        .args(["rm", "--force", worker.id.as_str()])
        .output()
        .await
        .expect("docker removes the worker out from under the sweeper");
    assert!(
        removed.status.success(),
        "worker removal failed: {removed:?}"
    );

    wait_for_managed_bundle_removal(&user, Duration::from_secs(10)).await;
    transport.shutdown().await;
}

#[tokio::test]
async fn stopped_managed_proxy_is_recreated_without_losing_bind_material() {
    let Some((_image, _serial)) =
        docker_worker_and_proxy_images("managed egress stopped proxy recovery").await
    else {
        return;
    };
    let user = TestScope::unique("egress-recover");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([user.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
            .with_managed_egress_proxy()
            .expect("managed egress policy is valid"),
    )
    .await
    .expect("Docker transport connects");

    transport
        .run_command(request(user.resource_scope(), "echo FIRST_PROXY"))
        .await
        .expect("first command runs");
    let first_proxy = managed_proxy_id(&user);
    cleanup.container_ids.insert(first_proxy.clone());
    docker_command(&["container", "stop", &first_proxy]);

    let recovered = transport
        .run_command(request(user.resource_scope(), "echo RECOVERED_PROXY"))
        .await
        .expect("command recovers a stopped managed proxy");
    assert_eq!(
        recovered.exit_code, 0,
        "recovery output: {}",
        recovered.output
    );
    assert!(recovered.output.contains("RECOVERED_PROXY"));
    let recovered_proxy = managed_proxy_id(&user);
    cleanup.container_ids.insert(recovered_proxy.clone());
    assert_ne!(recovered_proxy, first_proxy);
}

#[tokio::test]
async fn idle_stop_suspends_egress_and_preserves_the_private_network() {
    let Some((_image, _serial)) =
        docker_worker_and_proxy_images("managed egress idle cleanup").await
    else {
        return;
    };
    let user = TestScope::unique("egress-idle");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([user.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
            .with_idle_timeout(Duration::from_secs(1))
            .with_managed_egress_proxy()
            .expect("managed egress policy is valid"),
    )
    .await
    .expect("Docker transport connects");

    transport
        .run_command(request(user.resource_scope(), "echo FIRST_BUNDLE"))
        .await
        .expect("first command runs");
    let first_container = cleanup.capture(&user);
    let first_proxy = managed_proxy_id(&user);
    cleanup.container_ids.insert(first_proxy.clone());
    let initial_networks = managed_network_ids(&user);
    assert_eq!(initial_networks.len(), 1);

    wait_for_running_state(&first_container.id, false, Duration::from_secs(10)).await;
    wait_for_managed_bundle_suspension(&user, Duration::from_secs(10)).await;
    let retained_private_network = managed_network_ids(&user);
    assert_eq!(retained_private_network.len(), 1);
    assert!(initial_networks.contains(&retained_private_network[0]));
    let audit_dir = temp
        .path()
        .join("sandbox-workspaces")
        .join(".managed-egress")
        .join("audit");
    let preserved_audit = std::fs::read_dir(&audit_dir)
        .expect("suspension preserves a proxy audit root")
        .filter_map(Result::ok)
        .filter(|entry| entry.metadata().is_ok_and(|metadata| metadata.is_dir()))
        .any(|user_audit| {
            std::fs::read_dir(user_audit.path()).is_ok_and(|entries| {
                entries.filter_map(Result::ok).any(|entry| {
                    entry
                        .metadata()
                        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
                })
            })
        });
    assert!(
        preserved_audit,
        "proxy audit records must survive idle suspension"
    );

    let resumed = transport
        .run_command(request(user.resource_scope(), "echo RECREATED_BUNDLE"))
        .await
        .expect("wake command runs");
    assert_eq!(
        resumed.exit_code, 0,
        "wake command failed: {}",
        resumed.output
    );
    let resumed_container = cleanup.capture(&user);
    let resumed_proxy = managed_proxy_id(&user);
    cleanup.container_ids.insert(resumed_proxy.clone());
    assert_eq!(resumed_container.id, first_container.id);
    assert_ne!(resumed_proxy, first_proxy);
    let resumed_networks = managed_network_ids(&user);
    assert_eq!(resumed_networks.len(), 1);
    assert!(resumed_networks.contains(&retained_private_network[0]));

    wait_for_running_state(&resumed_container.id, false, Duration::from_secs(10)).await;
    wait_for_managed_bundle_suspension(&user, Duration::from_secs(10)).await;
    assert_eq!(managed_network_ids(&user), retained_private_network);
}

#[tokio::test]
async fn stopped_user_container_is_removed_after_retention() {
    let Some((_image, _serial)) = docker_worker_and_proxy_images("sandbox retention").await else {
        return;
    };
    let user = TestScope::unique("retention");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([user.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
            .with_idle_timeout(Duration::from_millis(200))
            .with_retention_timeout(Duration::from_millis(200))
            .with_managed_egress_proxy()
            .expect("managed egress policy is valid"),
    )
    .await
    .expect("Docker transport connects");

    transport
        .run_command(request(user.resource_scope(), "echo RETAINED_THEN_REAPED"))
        .await
        .expect("command runs");
    let container = cleanup.capture(&user);
    wait_for_running_state(&container.id, false, Duration::from_secs(10)).await;
    wait_for_managed_bundle_removal(&user, Duration::from_secs(10)).await;
    wait_for_user_container_removal(&user, Duration::from_secs(10)).await;
}

#[tokio::test]
async fn restart_preserves_stopped_container_retention_age() {
    let Some((_image, _serial)) = docker_worker_and_proxy_images("sandbox restart retention").await
    else {
        return;
    };
    let user = TestScope::unique("restart-retention");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([user.clone()]);
    let config = RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
        .with_idle_timeout(Duration::from_secs(60))
        .with_retention_timeout(Duration::from_millis(200));
    let transport = RebornScopedSandboxCommandTransport::connect(config.clone())
        .await
        .expect("first Docker transport connects");
    transport
        .run_command(request(user.resource_scope(), "echo STOP_BEFORE_RESTART"))
        .await
        .expect("command runs");
    let container = cleanup.capture(&user);
    transport.shutdown().await;
    drop(transport);
    let stopped = Command::new("docker")
        .args(["stop", "--time", "1", container.id.as_str()])
        .output()
        .expect("docker stops retained worker");
    assert!(stopped.status.success(), "worker stop failed: {stopped:?}");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let replacement = RebornScopedSandboxCommandTransport::connect(config)
        .await
        .expect("replacement Docker transport connects");

    wait_for_user_container_removal(&user, Duration::from_secs(10)).await;
    replacement.shutdown().await;
}
#[tokio::test]
async fn different_users_receive_distinct_private_networks_and_proxies() {
    let Some((_image, _serial)) =
        docker_worker_and_proxy_images("managed egress cross-user isolation").await
    else {
        return;
    };
    let first = TestScope::unique("egress-a");
    let second = TestScope::unique("egress-b");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([first.clone(), second.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
            .with_managed_egress_proxy()
            .expect("managed egress policy is valid"),
    )
    .await
    .expect("Docker transport connects");

    for scope in [&first, &second] {
        transport
            .run_command(request(scope.resource_scope(), "echo ISOLATED"))
            .await
            .expect("managed-egress command runs");
    }

    let first_worker = cleanup.capture(&first);
    let second_worker = cleanup.capture(&second);
    let first_proxy = managed_proxy_id(&first);
    let second_proxy = managed_proxy_id(&second);
    cleanup.container_ids.insert(first_proxy.clone());
    cleanup.container_ids.insert(second_proxy.clone());
    let first_network = managed_network_ids(&first);
    let second_network = managed_network_ids(&second);

    assert_eq!(first_network.len(), 1);
    assert_eq!(second_network.len(), 1);
    assert_ne!(first_network, second_network);
    let mut private_gateways = Vec::new();
    for network in [&first_network[0], &second_network[0]] {
        let inspected = Command::new("docker")
            .args([
                "network",
                "inspect",
                "--format",
                "{{json .Options}}|{{(index .IPAM.Config 0).Gateway}}",
                network.as_str(),
            ])
            .output()
            .expect("docker inspects private network gateway mode");
        assert!(inspected.status.success());
        let inspected =
            String::from_utf8(inspected.stdout).expect("private network inspection is UTF-8");
        let (options, gateway) = inspected
            .trim()
            .split_once('|')
            .expect("private network inspection has options and gateway");
        assert!(options.contains("\"com.docker.network.bridge.gateway_mode_ipv4\":\"isolated\""));
        private_gateways.push(gateway.to_string());
    }
    let host_listener =
        std::net::TcpListener::bind(("0.0.0.0", 0)).expect("host gateway probe binds");
    let host_port = host_listener
        .local_addr()
        .expect("host gateway probe has an address")
        .port();
    for (scope, gateway) in [
        (&first, &private_gateways[0]),
        (&second, &private_gateways[1]),
    ] {
        let probe = transport
            .run_command(request(
                scope.resource_scope(),
                format!(
                    "python -c \"import socket\ntry:\n socket.create_connection(('{gateway}', {host_port}), 1).close()\nexcept OSError:\n print('GATEWAY_ISOLATION_BLOCKED')\nelse:\n raise AssertionError('host gateway reachable')\""
                ),
            ))
            .await
            .expect("worker gateway probe runs");
        assert_eq!(
            probe.exit_code, 0,
            "gateway isolation probe itself failed: {}",
            probe.output
        );
        assert!(
            probe.output.contains("GATEWAY_ISOLATION_BLOCKED"),
            "isolated private network must reject the host wildcard listener"
        );
    }
    assert_ne!(first_proxy, second_proxy);

    let first_worker_networks = container_network_ids(&first_worker.id);
    let second_worker_networks = container_network_ids(&second_worker.id);
    assert_eq!(first_worker_networks.len(), 1);
    assert_eq!(second_worker_networks.len(), 1);
    assert!(first_network.contains(&first_worker_networks[0]));
    assert!(second_network.contains(&second_worker_networks[0]));

    let first_proxy_networks = container_network_ids(&first_proxy);
    let second_proxy_networks = container_network_ids(&second_proxy);
    assert_eq!(first_proxy_networks.len(), 2);
    assert_eq!(second_proxy_networks.len(), 2);
    assert!(first_proxy_networks.contains(&first_network[0]));
    assert!(second_proxy_networks.contains(&second_network[0]));
    let shared_upstream = first_proxy_networks
        .iter()
        .filter(|network| second_proxy_networks.contains(network))
        .collect::<Vec<_>>();
    assert_eq!(shared_upstream.len(), 1);
    assert!(!first_worker_networks.contains(shared_upstream[0]));
    assert!(!second_worker_networks.contains(shared_upstream[0]));
    let cross_user_proxy = Command::new("docker")
        .args([
            "exec",
            first_proxy.as_str(),
            "sh",
            "-c",
            "command -v nc >/dev/null || exit 91; if nc -z \"$1\" 3128; then exit 92; else echo PROXY_ISOLATION_BLOCKED; fi",
            "isolation-probe",
            second_proxy.as_str(),
        ])
        .output()
        .expect("first proxy probes second proxy over shared upstream");
    assert!(
        cross_user_proxy.status.success(),
        "proxy isolation probe itself failed: {cross_user_proxy:?}"
    );
    assert!(
        String::from_utf8_lossy(&cross_user_proxy.stdout).contains("PROXY_ISOLATION_BLOCKED"),
        "proxy listener must bind only to its private worker network"
    );
}

#[tokio::test]
#[ignore = "requires Docker, public Internet access, and IRONCLAW_TEST_GITHUB_TOKEN"]
async fn compound_github_cli_script_uses_proxy_placeholder_without_exposing_real_token() {
    let Some((_image, _serial)) =
        docker_worker_and_proxy_images("sandbox GitHub credential canary").await
    else {
        return;
    };
    let Ok(token) = std::env::var("IRONCLAW_TEST_GITHUB_TOKEN") else {
        eprintln!("SKIP: GitHub credential canary — IRONCLAW_TEST_GITHUB_TOKEN is unset");
        return;
    };
    assert!(!token.is_empty(), "GitHub canary token must not be empty");

    let scope = TestScope::unique("github-credential");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([scope.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
            .with_managed_egress_proxy()
            .expect("managed egress policy is valid"),
    )
    .await
    .expect("Docker transport connects");
    let placeholder = format!(
        "{}{}",
        ironclaw_secrets::CREDENTIAL_PLACEHOLDER_PREFIX,
        InvocationId::new()
    );
    let command = CredentialedSandboxCommandRequest {
        capability_id: ironclaw_host_api::ids::CapabilityId::new("builtin.shell")
            .expect("valid shell capability id"),
        scope: scope.resource_scope(),
        mounts: None,
        command: "set -e; gh pr list --repo nearai/ironclaw --limit 1 --json number \
                  >/tmp/first.json; gh pr list --repo nearai/ironclaw --limit 1 --json number"
            .to_string(),
        workdir: None,
        timeout_secs: Some(60),
        extra_env: HashMap::from([("GH_TOKEN".to_string(), placeholder.clone())]),
        credential_bindings: Vec::new(),
    };
    let result = transport
        .run_credentialed_command(
            command,
            vec![SandboxCommandCredential::new(
                ironclaw_host_api::ids::SecretHandle::new("github_runtime_token")
                    .expect("valid credential handle"),
                "GH_TOKEN".to_string(),
                placeholder,
                "api.github.com".to_string(),
                "Authorization".to_string(),
                Some("token ".to_string()),
                token.clone(),
            )],
        )
        .await
        .expect("compound credentialed gh script reaches GitHub through the proxy");

    assert!(
        !result.output.contains(&token),
        "gh command output exposed the real token"
    );
    let redacted_output = result.output.replace(&token, "[REDACTED]");
    assert_eq!(result.exit_code, 0, "gh command failed: {redacted_output}");
    assert!(serde_json::from_str::<serde_json::Value>(&result.output).is_ok());
    // This call also regresses the proxy's private-material boundary: the
    // cap-dropped proxy retains only DAC_READ_SEARCH so it can read host-owned
    // 0600 credentials through its per-user read-only bind mount.
    let container = cleanup.capture(&scope);
    let inspect = docker_command(&[
        "container",
        "inspect",
        "--format",
        "{{json .Config.Env}}",
        &container.id,
    ]);
    assert!(inspect.status.success());
    assert!(!String::from_utf8_lossy(&inspect.stdout).contains(&token));
    let proxy_mount_probe = docker_command(&[
        "container",
        "exec",
        &container.id,
        "test",
        "!",
        "-e",
        "/run/ironclaw-proxy",
    ]);
    assert!(
        proxy_mount_probe.status.success(),
        "proxy credential material must not be mounted in the command container"
    );
}

#[tokio::test]
#[ignore = "requires public DNS and Internet access; run as a live egress canary"]
async fn sandbox_profile_allows_allowlisted_https_through_proxy() {
    let Some((_image, _serial)) = docker_worker_and_proxy_images("sandbox egress canary").await
    else {
        return;
    };

    let egress_scope = TestScope::unique("egress");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([egress_scope.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
            .with_managed_egress_proxy()
            .expect("managed egress policy is valid"),
    )
    .await
    .expect("Docker transport connects");

    let first_request = request(
        egress_scope.resource_scope(),
        "set -eu; \
         test \"$IRONCLAW_REBORN_NETWORK_MODE\" = brokered; \
         case \"$HTTPS_PROXY\" in http://ironclaw-reborn-sandbox-proxy-*:3128/) ;; *) exit 41 ;; esac; \
         curl --fail --silent --show-error https://pypi.org >/dev/null; \
         git ls-remote https://github.com/nearai/ironclaw.git HEAD >/dev/null; \
         curl --fail --silent --show-error --location --range 0-0 https://github.com/BurntSushi/ripgrep/releases/download/14.1.1/ripgrep_14.1.1-1_amd64.deb >/dev/null; \
         node -e \"require('https').get('https://pypi.org', response => { response.resume(); if (response.statusCode !== 200) process.exitCode = 1; })\"; \
         python -c \"import urllib.request; response = urllib.request.urlopen('https://pypi.org', timeout=15); assert response.status == 200; response.close()\"; \
         echo SANDBOX_PROXY_TLS_MATRIX_OK",
    );
    let invocation_id = first_request.scope.invocation_id.to_string();
    let result = transport
        .run_command(first_request)
        .await
        .expect("public HTTPS request runs");
    let second_request = request(
        egress_scope.resource_scope(),
        "curl --fail --silent --show-error https://pypi.org >/dev/null",
    );
    let second_invocation_id = second_request.scope.invocation_id.to_string();
    let second_result = transport
        .run_command(second_request)
        .await
        .expect("second public HTTPS request runs");
    let container = cleanup.capture(&egress_scope);
    assert_stable_identity(&container, &egress_scope);
    let proxy = managed_proxy_id(&egress_scope);
    cleanup.container_ids.insert(proxy.clone());
    let proxy_logs = proxy_logs(&proxy);

    assert_eq!(
        result.exit_code, 0,
        "egress canary failed: {}\nproxy logs:\n{}",
        result.output, proxy_logs,
    );
    assert!(result.output.contains("SANDBOX_PROXY_TLS_MATRIX_OK"));
    assert!(result.sandboxed);
    assert_eq!(
        second_result.exit_code, 0,
        "second egress canary failed: {}\nproxy logs:\n{}",
        second_result.output, proxy_logs,
    );
    assert!(
        proxy_logs.contains(&invocation_id),
        "proxy audit did not retain first invocation correlation"
    );
    assert!(
        proxy_logs.contains(&second_invocation_id),
        "proxy attribution cache reused the prior invocation correlation"
    );
}

#[tokio::test]
async fn sandbox_profile_denies_unlisted_https_through_proxy() {
    let Some((_image, _serial)) = docker_worker_and_proxy_images("sandbox egress denial").await
    else {
        return;
    };

    let egress_scope = TestScope::unique("egress-denied");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([egress_scope.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
            .with_managed_egress_proxy()
            .expect("managed egress policy is valid"),
    )
    .await
    .expect("Docker transport connects");
    let result = transport
        .run_command(request(
            egress_scope.resource_scope(),
            "python -c \"import urllib.error, urllib.request\ntry:\n urllib.request.urlopen('https://example.com', timeout=15)\nexcept urllib.error.URLError as error:\n assert '403 Forbidden' in str(error.reason), error\n print('SANDBOX_PROXY_DENIED')\nelse:\n raise AssertionError('unlisted egress unexpectedly succeeded')\"",
        ))
        .await
        .expect("denied HTTPS request returns to the command");
    let container = cleanup.capture(&egress_scope);
    assert_stable_identity(&container, &egress_scope);

    assert_eq!(
        result.exit_code, 0,
        "egress denial failed: {}",
        result.output
    );
    assert!(result.output.contains("SANDBOX_PROXY_DENIED"));
    assert!(result.sandboxed);
}

#[tokio::test]
async fn sandbox_profile_blocks_direct_routes_and_proxy_management() {
    let Some((_image, _serial)) = docker_worker_and_proxy_images("sandbox route denial").await
    else {
        return;
    };

    let egress_scope = TestScope::unique("route-denied");
    let temp = docker_visible_tempdir();
    let mut cleanup = DockerCleanup::with_scopes([egress_scope.clone()]);
    let transport = RebornScopedSandboxCommandTransport::connect(
        RebornSandboxConfig::new(temp.path().join("sandbox-workspaces"))
            .with_managed_egress_proxy()
            .expect("managed egress policy is valid"),
    )
    .await
    .expect("Docker transport connects");

    let result = transport
        .run_command(request(
            egress_scope.resource_scope(),
            "python -c \"import os, socket, urllib.parse\nproxy = urllib.parse.urlparse(os.environ['HTTPS_PROXY']).hostname\ndef blocked(host, port):\n try:\n  connection = socket.create_connection((host, port), timeout=1)\n except OSError:\n  return\n connection.close()\n raise AssertionError(f'unexpected route to {host}:{port}')\nblocked('1.1.1.1', 443)\nblocked('169.254.169.254', 80)\nblocked(proxy, 8080)\nprint('SANDBOX_DIRECT_ROUTES_DENIED')\"",
        ))
        .await
        .expect("route-denial checks run");
    let container = cleanup.capture(&egress_scope);
    assert_stable_identity(&container, &egress_scope);
    let proxy = managed_proxy_id(&egress_scope);
    cleanup.container_ids.insert(proxy);

    assert_eq!(
        result.exit_code, 0,
        "direct-route denial failed: {}",
        result.output
    );
    assert!(result.output.contains("SANDBOX_DIRECT_ROUTES_DENIED"));
    assert!(result.sandboxed);
}
