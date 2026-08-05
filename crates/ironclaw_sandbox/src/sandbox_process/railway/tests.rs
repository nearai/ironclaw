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
    next_sandbox: AtomicUsize,
    expired_liveness_checks: AtomicUsize,
    failed_worker_execs: AtomicUsize,
    delay: Option<Duration>,
}

impl FakeRailwayCli {
    fn new() -> Self {
        Self {
            invocations: Mutex::new(Vec::new()),
            checkpoints: Mutex::new(BTreeSet::new()),
            next_sandbox: AtomicUsize::new(1),
            expired_liveness_checks: AtomicUsize::new(0),
            failed_worker_execs: AtomicUsize::new(0),
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

    fn fail_next_worker_exec(&self) {
        self.failed_worker_execs.store(1, Ordering::SeqCst);
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
        if invocation
            .args
            .starts_with(&["sandbox".into(), "create".into()])
            && let Some(index) = invocation.args.iter().position(|arg| arg == "--checkpoint")
            && let Some(name) = invocation.args.get(index + 1)
            && !self.checkpoints.lock().await.contains(name)
        {
            return Err(RuntimeProcessError::ExecutionFailed(
                "Railway preview failed to create sandbox: checkpoint not found".into(),
            ));
        }
        self.invocations.lock().await.push(invocation.clone());
        if invocation.args.iter().any(|arg| arg == OUTER_EXEC_WRAPPER)
            && self.failed_worker_execs.swap(0, Ordering::SeqCst) > 0
        {
            return Err(RuntimeProcessError::Timeout(timeout));
        }
        if invocation.args.iter().any(|arg| arg == "info")
            && self.expired_liveness_checks.swap(0, Ordering::SeqCst) > 0
        {
            return Err(RuntimeProcessError::ExecutionFailed(
                "fake sandbox expired".into(),
            ));
        }
        if invocation
            .args
            .starts_with(&["sandbox".into(), "create".into()])
        {
            let number = self.next_sandbox.fetch_add(1, Ordering::SeqCst);
            return Ok(RailwayCliOutput {
                stdout: format!(r#"{{"id":"sandbox-{number}"}}"#),
                stderr: String::new(),
            });
        }
        if invocation
            .args
            .starts_with(&["sandbox".into(), "checkpoint".into(), "create".into()])
        {
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
    assert_eq!(count_worker_launches(&invocations), 0);
    let model_runs = model_container_runs(&invocations);
    assert_eq!(model_runs.len(), 2);
    for run in model_runs {
        assert!(
            run.args
                .windows(3)
                .any(|args| args == ["docker", "run", "--rm"])
        );
        assert!(!run.args.iter().any(|arg| arg == "-d"));
        assert!(!run.args.iter().any(|arg| arg == "--restart"));
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
    assert!(invocations.iter().all(|call| {
        !call
            .args
            .starts_with(&["sandbox".into(), "checkpoint".into(), "list".into()])
    }));
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

#[test]
fn cli_private_home_is_owner_only_and_removed_on_drop() {
    let home = PrivateRailwayCliHome::create().expect("private Railway CLI home is created");
    let path = home.path().to_path_buf();
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

    drop(home);

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
fn only_an_explicit_missing_checkpoint_error_allows_fresh_provisioning() {
    assert!(is_missing_checkpoint_error(
        &RuntimeProcessError::ExecutionFailed("sandbox checkpoint not found".into())
    ));
    assert!(!is_missing_checkpoint_error(
        &RuntimeProcessError::ExecutionFailed("project not found".into())
    ));
    assert!(!is_missing_checkpoint_error(&RuntimeProcessError::Timeout(
        Duration::from_secs(1)
    )));
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

fn count_worker_launches(calls: &[RailwayCliInvocation]) -> usize {
    calls
        .iter()
        .filter(|call| worker_from_launch(call).is_some())
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

fn worker_from_launch(call: &RailwayCliInvocation) -> Option<&str> {
    let index = call.args.iter().position(|arg| arg == "--name")?;
    call.args.get(index + 1).map(String::as_str)
}

fn assert_pair(argv: &[String], flag: &str, value: &str) {
    assert!(
        argv.windows(2)
            .any(|pair| pair[0] == flag && pair[1] == value)
    );
}
