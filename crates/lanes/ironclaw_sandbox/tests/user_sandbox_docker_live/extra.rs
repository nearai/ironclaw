use super::*;

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
#[ignore = "requires public DNS and Internet access; run as a live egress canary"]
async fn sandbox_profile_allows_public_https_egress() {
    let Some((_image, _serial)) = docker_worker_image("sandbox egress canary").await else {
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
