use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use ironclaw_host_api::{
    ids::{AgentId, InvocationId, TenantId, UserId},
    mount::MountView,
    resource::ResourceScope,
};

use super::*;

#[derive(Debug)]
struct FakeRailwayCli {
    invocations: Mutex<Vec<RailwayCliInvocation>>,
    checkpoints: Mutex<BTreeSet<String>>,
    live_sandboxes: Mutex<BTreeSet<String>>,
    next_sandbox: AtomicUsize,
    expired_liveness_checks: AtomicUsize,
    failed_liveness_lists: AtomicUsize,
    failed_worker_execs: AtomicUsize,
    failed_checkpoint_creates: AtomicUsize,
    failed_destroys: AtomicUsize,
    malformed_checkpoint_lists: AtomicUsize,
    checkpoint_timeouts: Mutex<Vec<Duration>>,
    delay: Option<Duration>,
}

impl FakeRailwayCli {
    fn new() -> Self {
        Self {
            invocations: Mutex::new(Vec::new()),
            checkpoints: Mutex::new(BTreeSet::new()),
            live_sandboxes: Mutex::new(BTreeSet::new()),
            next_sandbox: AtomicUsize::new(1),
            expired_liveness_checks: AtomicUsize::new(0),
            failed_liveness_lists: AtomicUsize::new(0),
            failed_worker_execs: AtomicUsize::new(0),
            failed_checkpoint_creates: AtomicUsize::new(0),
            failed_destroys: AtomicUsize::new(0),
            malformed_checkpoint_lists: AtomicUsize::new(0),
            checkpoint_timeouts: Mutex::new(Vec::new()),
            delay: None,
        }
    }

    fn delayed(delay: Duration) -> Self {
        Self {
            delay: Some(delay),
            ..Self::new()
        }
    }

    async fn invocations(&self) -> Vec<RailwayCliInvocation> {
        self.invocations.lock().await.clone()
    }

    fn expire_next_liveness_check(&self) {
        self.expired_liveness_checks.store(1, Ordering::SeqCst);
    }

    fn fail_next_liveness_list(&self) {
        self.failed_liveness_lists.store(1, Ordering::SeqCst);
    }

    fn fail_next_worker_exec(&self) {
        self.failed_worker_execs.store(1, Ordering::SeqCst);
    }

    fn fail_next_checkpoint_create(&self) {
        self.failed_checkpoint_creates.store(1, Ordering::SeqCst);
    }

    fn fail_next_destroy(&self) {
        self.failed_destroys.store(1, Ordering::SeqCst);
    }

    fn malform_next_checkpoint_list(&self) {
        self.malformed_checkpoint_lists.store(1, Ordering::SeqCst);
    }

    async fn checkpoint_timeouts(&self) -> Vec<Duration> {
        self.checkpoint_timeouts.lock().await.clone()
    }
}

#[async_trait]
impl RailwayCli for FakeRailwayCli {
    async fn execute(
        &self,
        invocation: RailwayCliInvocation,
        timeout: Duration,
    ) -> Result<RailwayCliOutput, RuntimeProcessError> {
        if let Some(delay) = self.delay
            && tokio::time::timeout(timeout, tokio::time::sleep(delay))
                .await
                .is_err()
        {
            return Err(RuntimeProcessError::Timeout(timeout));
        }
        self.invocations.lock().await.push(invocation.clone());
        if invocation
            .args
            .starts_with(&["sandbox".into(), "list".into()])
        {
            if self.failed_liveness_lists.swap(0, Ordering::SeqCst) > 0 {
                return Err(RuntimeProcessError::ExecutionFailed(
                    "fake Railway authentication failed".into(),
                ));
            }
            if self.expired_liveness_checks.swap(0, Ordering::SeqCst) > 0 {
                self.live_sandboxes.lock().await.clear();
            }
            let sandboxes = self.live_sandboxes.lock().await;
            return Ok(RailwayCliOutput {
                stdout: serde_json::Value::Array(
                    sandboxes
                        .iter()
                        .map(|id| serde_json::json!({ "id": id }))
                        .collect(),
                )
                .to_string(),
                stderr: String::new(),
            });
        }
        if invocation
            .args
            .starts_with(&["sandbox".into(), "checkpoint".into(), "list".into()])
        {
            if self.malformed_checkpoint_lists.swap(0, Ordering::SeqCst) > 0 {
                return Ok(RailwayCliOutput {
                    stdout: "truncated checkpoint response".to_string(),
                    stderr: String::new(),
                });
            }
            let checkpoints = self.checkpoints.lock().await;
            return Ok(RailwayCliOutput {
                stdout: serde_json::Value::Array(
                    checkpoints
                        .iter()
                        .map(|name| serde_json::json!({ "key": name }))
                        .collect(),
                )
                .to_string(),
                stderr: String::new(),
            });
        }
        if invocation.args.iter().any(|arg| arg == OUTER_EXEC_WRAPPER)
            && self.failed_worker_execs.swap(0, Ordering::SeqCst) > 0
        {
            return Err(RuntimeProcessError::Timeout(timeout));
        }
        if invocation
            .args
            .starts_with(&["sandbox".into(), "create".into()])
        {
            let number = self.next_sandbox.fetch_add(1, Ordering::SeqCst);
            let sandbox_id = format!("sandbox-{number}");
            self.live_sandboxes.lock().await.insert(sandbox_id.clone());
            return Ok(RailwayCliOutput {
                stdout: serde_json::json!({ "id": sandbox_id }).to_string(),
                stderr: String::new(),
            });
        }
        if invocation
            .args
            .starts_with(&["sandbox".into(), "checkpoint".into(), "create".into()])
        {
            self.checkpoint_timeouts.lock().await.push(timeout);
            if self.failed_checkpoint_creates.swap(0, Ordering::SeqCst) > 0 {
                return Err(RuntimeProcessError::ExecutionFailed(
                    "fake checkpoint create failed".into(),
                ));
            }
            let Some(name) = invocation.args.get(3) else {
                return Err(RuntimeProcessError::ExecutionFailed(
                    "fake checkpoint create has no name".into(),
                ));
            };
            self.checkpoints.lock().await.insert(name.clone());
            return Ok(RailwayCliOutput {
                stdout: serde_json::json!({ "key": name }).to_string(),
                stderr: String::new(),
            });
        }
        if invocation.args.iter().any(|arg| arg == OUTER_EXEC_WRAPPER) {
            return Ok(RailwayCliOutput {
                stdout: format!("command output\n{EXIT_SENTINEL}0\n"),
                stderr: String::new(),
            });
        }
        if invocation
            .args
            .starts_with(&["sandbox".into(), "destroy".into()])
            && let Some(index) = invocation.args.iter().position(|arg| arg == "--id")
            && let Some(id) = invocation.args.get(index + 1)
        {
            if self.failed_destroys.swap(0, Ordering::SeqCst) > 0 {
                return Err(RuntimeProcessError::ExecutionFailed(
                    "fake sandbox destroy failed".into(),
                ));
            }
            self.live_sandboxes.lock().await.remove(id);
        }
        Ok(RailwayCliOutput {
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

fn config() -> RailwayPreviewSandboxConfig {
    RailwayPreviewSandboxConfig::new("project-id", "environment-id").unwrap()
}

fn request(tenant: &str, user: &str, command: &str) -> CommandExecutionRequest {
    CommandExecutionRequest {
        scope: ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: Some(AgentId::new("agent").unwrap()),
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        mounts: None,
        command: command.into(),
        workdir: None,
        timeout_secs: Some(10),
        extra_env: HashMap::new(),
    }
}

fn user_key(tenant: &str, user: &str) -> RebornSandboxUserKey {
    RebornSandboxUserKey::from_scope(&request(tenant, user, "true").scope)
}

#[tokio::test]
async fn provisions_once_per_user_and_runs_ephemeral_workers_per_command() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    let (first, second) = tokio::join!(
        transport.run_command(request("tenant", "user", "python -c 'print(1)'")),
        transport.run_command(request("tenant", "user", "python -c 'print(2)'")),
    );
    first.unwrap();
    second.unwrap();
    let invocations = cli.invocations().await;
    assert_eq!(count_creates(&invocations), 1);
    let model_runs = model_container_runs(&invocations);
    assert_eq!(model_runs.len(), 2);
    for run in model_runs {
        assert!(
            run.args
                .windows(3)
                .any(|args| args == ["docker", "run", "--rm"])
        );
        assert!(!run.args.iter().any(|arg| arg == "-d"));
        assert!(!run.args.iter().any(|arg| arg == "--detach"));
        assert!(!run.args.iter().any(|arg| arg == "--restart"));
        assert!(!run.args.iter().any(|arg| arg == "--name"));
        assert!(run.args.iter().any(|arg| {
            arg.starts_with("type=bind,src=/workspace/ironclaw-users/")
                && arg.ends_with(",dst=/workspace")
        }));
    }
    assert!(
        invocations
            .iter()
            .all(|call| !call.args.iter().any(|argument| argument == "--variable"))
    );
    assert_eq!(count_checkpoint_lists(&invocations), 1);
    for invocation in invocations.iter().filter(|invocation| {
        invocation
            .args
            .starts_with(&["sandbox".into(), "exec".into()])
    }) {
        assert_pair(&invocation.args, "--project", "project-id");
        assert_pair(&invocation.args, "--environment", "environment-id");
        assert!(
            !invocation
                .args
                .windows(2)
                .any(|pair| { pair[0] == "sh" && pair[1] == "-lc" })
        );
    }
}

#[tokio::test]
async fn isolates_workers_for_distinct_tenant_users() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    transport
        .run_command(request("tenant-a", "same", "true"))
        .await
        .unwrap();
    transport
        .run_command(request("tenant-b", "same", "true"))
        .await
        .unwrap();
    let invocations = cli.invocations().await;
    assert_eq!(count_creates(&invocations), 2);
    let workspaces: Vec<_> = model_container_runs(&invocations)
        .into_iter()
        .filter_map(host_workspace_from_run)
        .collect();
    assert_eq!(workspaces.len(), 2);
    assert_ne!(workspaces[0], workspaces[1]);
}

#[tokio::test]
async fn user_state_registry_evicts_the_least_recently_used_idle_entry() {
    let transport = RailwayPreviewSandboxTransport::with_cli_and_capacity(
        config(),
        Arc::new(FakeRailwayCli::new()),
        2,
    );
    let key_a = user_key("tenant", "a");
    let key_b = user_key("tenant", "b");
    let key_c = user_key("tenant", "c");

    drop(transport.state_for(key_a.clone()).await.unwrap());
    tokio::time::sleep(Duration::from_millis(1)).await;
    drop(transport.state_for(key_b.clone()).await.unwrap());
    tokio::time::sleep(Duration::from_millis(1)).await;
    drop(transport.state_for(key_a.clone()).await.unwrap());
    tokio::time::sleep(Duration::from_millis(1)).await;
    drop(transport.state_for(key_c.clone()).await.unwrap());

    let users = transport.users.lock().await;
    assert_eq!(users.len(), 2);
    assert!(users.contains_key(&key_a));
    assert!(!users.contains_key(&key_b));
    assert!(users.contains_key(&key_c));
}

#[tokio::test]
async fn user_state_registry_fails_closed_when_all_entries_are_active() {
    let transport = RailwayPreviewSandboxTransport::with_cli_and_capacity(
        config(),
        Arc::new(FakeRailwayCli::new()),
        2,
    );
    let active_a = transport.state_for(user_key("tenant", "a")).await.unwrap();
    let active_b = transport.state_for(user_key("tenant", "b")).await.unwrap();

    let error = transport
        .state_for(user_key("tenant", "c"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, RuntimeProcessError::ExecutionFailed(message) if message.contains("capacity is exhausted"))
    );
    drop((active_a, active_b));
}

#[tokio::test]
async fn restores_the_deterministic_checkpoint_after_host_restart() {
    let cli = Arc::new(FakeRailwayCli::new());
    let first_host = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    first_host
        .run_command(request("tenant", "user", "echo persisted > state.txt"))
        .await
        .unwrap();
    drop(first_host);

    let restarted_host = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    restarted_host
        .run_command(request("tenant", "user", "cat state.txt"))
        .await
        .unwrap();

    let invocations = cli.invocations().await;
    assert_eq!(count_creates(&invocations), 2);
    assert!(invocations.iter().any(|call| {
        call.args
            .windows(2)
            .any(|pair| pair[0] == "--checkpoint" && pair[1].ends_with("-checkpoint"))
    }));
}

#[tokio::test]
async fn malformed_checkpoint_listing_fails_closed_before_fresh_creation() {
    let cli = Arc::new(FakeRailwayCli::new());
    cli.malform_next_checkpoint_list();
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());

    assert!(matches!(
        transport.run_command(request("tenant", "user", "true")).await,
        Err(RuntimeProcessError::ExecutionFailed(message))
            if message.contains("checkpoint listing returned an invalid response")
    ));
    assert_eq!(count_creates(&cli.invocations().await), 0);
}

#[tokio::test]
async fn expired_sandbox_id_reprovisions_from_the_last_checkpoint() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    transport
        .run_command(request("tenant", "user", "echo persisted > state.txt"))
        .await
        .unwrap();
    cli.expire_next_liveness_check();

    transport
        .run_command(request("tenant", "user", "cat state.txt"))
        .await
        .unwrap();

    let invocations = cli.invocations().await;
    assert_eq!(count_creates(&invocations), 2);
    assert!(invocations.iter().any(|call| {
        call.args
            .windows(2)
            .any(|pair| pair[0] == "--checkpoint" && pair[1].ends_with("-checkpoint"))
    }));
}

#[tokio::test]
async fn liveness_provider_failure_is_propagated_without_reprovisioning() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    transport
        .run_command(request("tenant", "user", "true"))
        .await
        .unwrap();
    cli.fail_next_liveness_list();

    assert!(matches!(
        transport.run_command(request("tenant", "user", "true")).await,
        Err(RuntimeProcessError::ExecutionFailed(message))
            if message.contains("authentication failed")
    ));
    assert_eq!(count_creates(&cli.invocations().await), 1);
}

#[tokio::test]
async fn preserves_metacharacters_as_a_single_model_command_argv_value() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    let command = "printf '%s' '$HOME; $(id); \"quoted\"'";
    transport
        .run_command(request("tenant", "user", command))
        .await
        .unwrap();
    let invocation = cli
        .invocations()
        .await
        .into_iter()
        .find(|call| call.args.iter().any(|arg| arg == OUTER_EXEC_WRAPPER))
        .unwrap();
    assert_eq!(invocation.args.last(), Some(&command.to_string()));
    assert!(invocation.args.iter().any(|arg| arg == OUTER_EXEC_WRAPPER));
}

#[tokio::test]
async fn rejects_request_environment_before_provisioning() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    let mut extra_env = request("tenant", "user", "true");
    extra_env.extra_env.insert("TOKEN".into(), "value".into());
    assert_rejected(&transport, extra_env).await;
    assert!(cli.invocations().await.is_empty());
}

#[tokio::test]
async fn rejects_invalid_workdirs_before_provisioning() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());

    for workdir in ["/etc", "/workspace/../../etc", "/workspace/bad\0path"] {
        let mut invalid = request("tenant", "user", "pwd");
        invalid.workdir = Some(workdir.to_string());
        assert_rejected(&transport, invalid).await;
    }
    assert!(cli.invocations().await.is_empty());
}

#[tokio::test]
async fn accepts_production_shell_mount_metadata_without_materializing_it() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    let mut request = request("tenant", "user", "python --version");
    request.mounts = Some(MountView { mounts: Vec::new() });

    transport.run_command(request).await.unwrap();

    assert_eq!(count_creates(&cli.invocations().await), 1);
}

#[tokio::test]
async fn ephemeral_worker_is_networkless_and_hardened() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    transport
        .run_command(request("tenant", "user", "true"))
        .await
        .unwrap();
    let invocations = cli.invocations().await;
    let argv = &model_container_runs(&invocations)[0].args;
    assert_pair(argv, "--network", "none");
    assert!(argv.contains(&"--read-only".to_string()));
    assert_pair(argv, "--user", WORKER_USER);
    assert_pair(argv, "--cap-drop", "ALL");
    assert_pair(argv, "--security-opt", "no-new-privileges:true");
    assert_pair(argv, "--pids-limit", "1024");
    assert_pair(argv, "--memory", "512m");
    assert_pair(argv, "--cpus", "1.0");
    assert_pair(argv, "--tmpfs", "/tmp:rw,noexec,nosuid,size=64m");
    assert_pair(argv, "--log-driver", "json-file");
    assert!(
        argv.windows(2)
            .any(|pair| pair == ["--log-opt", "max-size=1m"])
    );
    assert!(
        argv.windows(2)
            .any(|pair| pair == ["--log-opt", "max-file=1"])
    );
    assert!(argv.contains(&"--rm".to_string()));
    assert!(!argv.contains(&"-d".to_string()));
    assert!(!argv.iter().any(|arg| arg.contains("docker.sock")));
}

#[tokio::test]
async fn times_out_when_the_cli_exceeds_the_request_deadline() {
    let transport = RailwayPreviewSandboxTransport::with_cli(
        config(),
        Arc::new(FakeRailwayCli::delayed(Duration::from_secs(2))),
    );
    let mut command = request("tenant", "user", "true");
    command.timeout_secs = Some(1);
    assert!(matches!(
        transport.run_command(command).await,
        Err(RuntimeProcessError::Timeout(duration))
            if duration > Duration::ZERO && duration <= Duration::from_secs(1)
    ));
}

#[tokio::test]
async fn checkpoint_gets_an_independent_budget_after_worker_uses_command_budget() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    let mut command = request("tenant", "user", "true");
    command.timeout_secs = Some(1);

    transport.run_command(command).await.unwrap();

    assert_eq!(
        cli.checkpoint_timeouts().await,
        vec![REMOTE_CHECKPOINT_TIMEOUT]
    );
}

#[tokio::test]
async fn failed_remote_worker_is_destroyed_before_return_and_reprovisioned() {
    let cli = Arc::new(FakeRailwayCli::new());
    cli.fail_next_worker_exec();
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());

    assert!(matches!(
        transport
            .run_command(request("tenant", "user", "sleep 600"))
            .await,
        Err(RuntimeProcessError::Timeout(_))
    ));
    let after_failure = cli.invocations().await;
    assert!(after_failure.iter().any(|call| {
        call.args.starts_with(&["sandbox".into(), "destroy".into()])
            && call
                .args
                .windows(2)
                .any(|pair| pair == ["--id", "sandbox-1"])
    }));

    transport
        .run_command(request("tenant", "user", "true"))
        .await
        .unwrap();
    assert_eq!(count_creates(&cli.invocations().await), 2);
}

#[tokio::test]
async fn successful_command_returns_output_with_warning_when_checkpoint_fails() {
    let cli = Arc::new(FakeRailwayCli::new());
    cli.fail_next_checkpoint_create();
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli);

    let output = transport
        .run_command(request("tenant", "user", "echo completed"))
        .await
        .expect("command result remains available");

    assert_eq!(output.exit_code, 0);
    assert!(output.output.contains("command output"));
    assert!(output.output.contains(CHECKPOINT_FAILURE_WARNING));
}

#[tokio::test]
async fn worker_and_cleanup_failure_reports_unconfirmed_remote_cleanup() {
    let cli = Arc::new(FakeRailwayCli::new());
    cli.fail_next_worker_exec();
    cli.fail_next_destroy();
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli);

    assert!(matches!(
        transport
            .run_command(request("tenant", "user", "sleep 600"))
            .await,
        Err(RuntimeProcessError::ExecutionFailed(message))
            if message.contains("remote cleanup could not be confirmed")
    ));
}

#[tokio::test]
async fn bounded_stdout_keeps_the_private_exit_sentinel_in_the_tail() {
    let mut stdout = vec![b'x'; 8 * 1024];
    stdout.extend_from_slice(format!("\n{EXIT_SENTINEL}17\n").as_bytes());

    let bounded = read_stream_bounded(stdout.as_slice(), 1024)
        .await
        .expect("bounded read succeeds");
    let (model_output, exit_code) =
        parse_exit_sentinel(bounded).expect("tail sentinel remains parseable");

    assert_eq!(exit_code, 17);
    assert!(model_output.contains("trusted CLI output truncated"));
}

#[test]
fn trailing_wrapper_exit_sentinel_wins_over_worker_output() {
    let fake_worker_sentinel = format!("{EXIT_SENTINEL}0");
    let stdout = format!("worker output\n{fake_worker_sentinel}\n{EXIT_SENTINEL}7\n");

    let (model_output, exit_code) =
        parse_exit_sentinel(stdout).expect("trailing wrapper sentinel is authoritative");

    assert_eq!(exit_code, 7);
    assert!(model_output.contains(&fake_worker_sentinel));
}

#[test]
fn cli_child_environment_is_an_explicit_allowlist() {
    let environment = railway_cli_environment_from([
        ("RAILWAY_TOKEN".into(), "railway-secret".into()),
        ("PATH".into(), "/bin".into()),
        ("HOME".into(), "/safe-home".into()),
        ("AWS_SECRET_ACCESS_KEY".into(), "must-not-forward".into()),
        ("UNRELATED".into(), "must-not-forward".into()),
    ])
    .unwrap();
    assert_eq!(
        environment.get("RAILWAY_TOKEN"),
        Some(&"railway-secret".to_string())
    );
    assert_eq!(environment.get("PATH"), Some(&"/bin".to_string()));
    assert!(!environment.contains_key("HOME"));
    assert!(!environment.contains_key("AWS_SECRET_ACCESS_KEY"));
    assert!(!environment.contains_key("UNRELATED"));
}

#[test]
fn cli_child_environment_rejects_ambiguous_auth() {
    let error = railway_cli_environment_from([
        ("RAILWAY_TOKEN".into(), "project-secret".into()),
        ("RAILWAY_API_TOKEN".into(), "account-secret".into()),
    ])
    .expect_err("two Railway tokens must fail closed");

    assert!(error.to_string().contains("exactly one Railway token"));
    assert!(!error.to_string().contains("project-secret"));
    assert!(!error.to_string().contains("account-secret"));
}

#[tokio::test]
async fn cli_private_home_is_owner_only_and_removed_without_blocking_the_runtime() {
    let home = PrivateRailwayCliHome::create()
        .await
        .expect("private Railway CLI home is created");
    let path = home.path().unwrap().to_path_buf();
    assert!(path.is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let mode = std::fs::metadata(&path)
            .expect("private Railway CLI home metadata is readable")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }

    home.cleanup().await;

    assert!(!path.exists());
}

#[test]
fn cli_failure_diagnostic_names_operation_and_redacts_auth_token() {
    let environment = BTreeMap::from([(
        "RAILWAY_API_TOKEN".to_string(),
        "railway-secret-token".to_string(),
    )]);

    let message = railway_cli_failure_message(
        RailwayCliOperation::CreateSandbox,
        Some(17),
        "request denied for railway-secret-token",
        &environment,
    );

    assert!(message.contains("create sandbox"));
    assert!(message.contains("exit code 17"));
    assert!(message.contains("request denied for [REDACTED]"));
    assert!(!message.contains("railway-secret-token"));
}

#[test]
fn checkpoint_listing_is_structured_and_fail_closed() {
    assert!(checkpoint_exists(r#"[{"key":"wanted"}]"#, "wanted").unwrap());
    assert!(checkpoint_exists(r#"{"checkpoints":[{"name":"wanted"}]}"#, "wanted").unwrap());
    assert!(!checkpoint_exists(r#"[{"key":"other"}]"#, "wanted").unwrap());
    assert!(checkpoint_exists("checkpoint not found", "wanted").is_err());
    assert!(checkpoint_exists(r#"{"checkpoints":"truncated"}"#, "wanted").is_err());
}

#[test]
fn sandbox_listing_is_structured_and_fail_closed() {
    assert!(sandbox_exists(r#"[{"id":"sandbox-1"}]"#, "sandbox-1").unwrap());
    assert!(
        sandbox_exists(
            r#"{"sandboxes":[{"sandbox":{"id":"sandbox-1"}}]}"#,
            "sandbox-1"
        )
        .unwrap()
    );
    assert!(!sandbox_exists(r#"[{"id":"sandbox-2"}]"#, "sandbox-1").unwrap());
    assert!(sandbox_exists("truncated sandbox response", "sandbox-1").is_err());
}

#[test]
fn sandbox_creation_response_requires_a_valid_identifier() {
    assert_eq!(
        parse_sandbox_id(r#"{"id":"sandbox-1"}"#).unwrap(),
        "sandbox-1"
    );
    for response in [
        "not-json",
        r#"{}"#,
        r#"{"id":""}"#,
        "{\"id\":\"bad\\u0000id\"}",
    ] {
        assert!(parse_sandbox_id(response).is_err(), "response: {response}");
    }
}

#[test]
fn railway_exec_exit_124_is_a_timeout_but_other_operations_are_provider_failures() {
    let environment = BTreeMap::new();
    assert!(matches!(
        railway_cli_status_error(
            RailwayCliOperation::ExecuteSandboxCommand,
            Some(124),
            "timed out",
            &environment,
            Duration::from_secs(7),
        ),
        RuntimeProcessError::Timeout(duration) if duration == Duration::from_secs(7)
    ));
    assert!(matches!(
        railway_cli_status_error(
            RailwayCliOperation::CreateSandbox,
            Some(124),
            "provider failed",
            &environment,
            Duration::from_secs(7),
        ),
        RuntimeProcessError::ExecutionFailed(message) if message.contains("create sandbox")
    ));
}

async fn assert_rejected(
    transport: &RailwayPreviewSandboxTransport,
    request: CommandExecutionRequest,
) {
    assert!(matches!(
        transport.run_command(request).await,
        Err(RuntimeProcessError::ExecutionFailed(_))
    ));
}

fn count_creates(calls: &[RailwayCliInvocation]) -> usize {
    calls
        .iter()
        .filter(|call| call.args.starts_with(&["sandbox".into(), "create".into()]))
        .count()
}

fn count_checkpoint_lists(calls: &[RailwayCliInvocation]) -> usize {
    calls
        .iter()
        .filter(|call| {
            call.args
                .starts_with(&["sandbox".into(), "checkpoint".into(), "list".into()])
        })
        .count()
}

fn model_container_runs(calls: &[RailwayCliInvocation]) -> Vec<&RailwayCliInvocation> {
    calls
        .iter()
        .filter(|call| call.args.iter().any(|arg| arg == OUTER_EXEC_WRAPPER))
        .collect()
}

fn host_workspace_from_run(call: &RailwayCliInvocation) -> Option<&str> {
    call.args
        .iter()
        .find(|arg| arg.starts_with("type=bind,src=/workspace/ironclaw-users/"))
        .and_then(|mount| mount.strip_prefix("type=bind,src="))
        .and_then(|mount| mount.strip_suffix(",dst=/workspace"))
}

fn assert_pair(argv: &[String], flag: &str, value: &str) {
    assert!(
        argv.windows(2)
            .any(|pair| pair[0] == flag && pair[1] == value)
    );
}
