use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use base64::Engine as _;
use ironclaw_host_api::{
    ids::{AgentId, InvocationId, TenantId, UserId},
    mount::MountView,
    process::{
        SandboxWorkspaceFileError, SandboxWorkspaceFileReadRequest, SandboxWorkspaceFileTransport,
        SandboxWorkspaceFileWriteRequest,
    },
    resource::ResourceScope,
};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;

use super::*;

#[derive(Debug)]
struct FakeRailwayCli {
    invocations: Mutex<Vec<RailwayCliInvocation>>,
    checkpoints: Mutex<BTreeSet<String>>,
    live_sandboxes: Mutex<BTreeSet<String>>,
    next_sandbox: AtomicUsize,
    expired_liveness_checks: AtomicUsize,
    failed_liveness_lists: AtomicUsize,
    failed_bootstrap_execs: AtomicUsize,
    failed_worker_execs: AtomicUsize,
    malformed_worker_outputs: AtomicUsize,
    malformed_file_outputs: AtomicUsize,
    failed_checkpoint_creates: AtomicUsize,
    failed_file_execs: AtomicUsize,
    failed_destroys: AtomicUsize,
    malformed_checkpoint_lists: AtomicUsize,
    checkpoint_timeouts: Mutex<Vec<Duration>>,
    workspace_files: Mutex<HashMap<(String, String), Vec<u8>>>,
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
            failed_bootstrap_execs: AtomicUsize::new(0),
            failed_worker_execs: AtomicUsize::new(0),
            malformed_worker_outputs: AtomicUsize::new(0),
            malformed_file_outputs: AtomicUsize::new(0),
            failed_checkpoint_creates: AtomicUsize::new(0),
            failed_file_execs: AtomicUsize::new(0),
            failed_destroys: AtomicUsize::new(0),
            malformed_checkpoint_lists: AtomicUsize::new(0),
            checkpoint_timeouts: Mutex::new(Vec::new()),
            workspace_files: Mutex::new(HashMap::new()),
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

    fn malform_next_worker_output(&self) {
        self.malformed_worker_outputs.store(1, Ordering::SeqCst);
    }

    fn malform_next_file_output(&self) {
        self.malformed_file_outputs.store(1, Ordering::SeqCst);
    }

    fn fail_next_bootstrap_exec(&self) {
        self.failed_bootstrap_execs.store(1, Ordering::SeqCst);
    }

    fn fail_next_checkpoint_create(&self) {
        self.failed_checkpoint_creates.store(1, Ordering::SeqCst);
    }

    fn fail_next_file_exec(&self) {
        self.failed_file_execs.store(1, Ordering::SeqCst);
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
            .iter()
            .any(|arg| arg == WORKSPACE_BOOTSTRAP_MARKER)
            && self.failed_bootstrap_execs.swap(0, Ordering::SeqCst) > 0
        {
            return Err(RuntimeProcessError::ExecutionFailed(
                "fake workspace bootstrap failed".into(),
            ));
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
            if self.malformed_worker_outputs.swap(0, Ordering::SeqCst) > 0 {
                return Ok(RailwayCliOutput {
                    stdout: "truncated worker response".to_string(),
                    stderr: String::new(),
                });
            }
            return Ok(RailwayCliOutput {
                stdout: format!("command output\n{EXIT_SENTINEL}0\n"),
                stderr: String::new(),
            });
        }
        if matches!(
            invocation.operation,
            RailwayCliOperation::ReadWorkspaceFile | RailwayCliOperation::WriteWorkspaceFile
        ) && self.failed_file_execs.swap(0, Ordering::SeqCst) > 0
        {
            return Err(RuntimeProcessError::Timeout(timeout));
        }
        if invocation.operation == RailwayCliOperation::WriteWorkspaceFile {
            let root = invocation.args.get(invocation.args.len().saturating_sub(8));
            let path = invocation.args.get(invocation.args.len().saturating_sub(7));
            let overwrite = invocation.args.get(invocation.args.len().saturating_sub(6));
            let (Some(root), Some(path), Some(overwrite), Some(bytes)) =
                (root, path, overwrite, invocation.stdin.as_ref())
            else {
                return Err(RuntimeProcessError::ExecutionFailed(
                    "fake workspace write invocation was malformed".into(),
                ));
            };
            let key = (root.clone(), path.clone());
            let mut files = self.workspace_files.lock().await;
            let already_present = match files.get(&key) {
                Some(existing) if existing == bytes => true,
                Some(_) if overwrite == "0" => {
                    return Ok(RailwayCliOutput {
                        stdout: serde_json::json!({"status":"error","code":"conflict"}).to_string(),
                        stderr: String::new(),
                    });
                }
                Some(_) | None => false,
            };
            if !already_present {
                files.insert(key, bytes.clone());
            }
            if self.malformed_file_outputs.swap(0, Ordering::SeqCst) > 0 {
                return Ok(RailwayCliOutput {
                    stdout: "truncated workspace write response".into(),
                    stderr: String::new(),
                });
            }
            return Ok(RailwayCliOutput {
                stdout: serde_json::json!({
                    "status": "ok",
                    "bytes_written": bytes.len(),
                    "sha256": hex::encode(Sha256::digest(bytes)),
                    "already_present": already_present,
                })
                .to_string(),
                stderr: String::new(),
            });
        }
        if invocation.operation == RailwayCliOperation::ReadWorkspaceFile {
            let root = invocation.args.get(invocation.args.len().saturating_sub(3));
            let path = invocation.args.get(invocation.args.len().saturating_sub(2));
            let max_bytes = invocation
                .args
                .last()
                .and_then(|value| value.parse::<usize>().ok());
            let (Some(root), Some(path), Some(max_bytes)) = (root, path, max_bytes) else {
                return Err(RuntimeProcessError::ExecutionFailed(
                    "fake workspace read invocation was malformed".into(),
                ));
            };
            let files = self.workspace_files.lock().await;
            let Some(bytes) = files.get(&(root.clone(), path.clone())) else {
                return Ok(RailwayCliOutput {
                    stdout: serde_json::json!({"status":"error","code":"not_found"}).to_string(),
                    stderr: String::new(),
                });
            };
            if bytes.len() > max_bytes {
                return Ok(RailwayCliOutput {
                    stdout: serde_json::json!({"status":"error","code":"too_large"}).to_string(),
                    stderr: String::new(),
                });
            }
            return Ok(RailwayCliOutput {
                stdout: serde_json::json!({
                    "status": "ok",
                    "bytes": base64::engine::general_purpose::STANDARD.encode(bytes),
                    "sha256": hex::encode(Sha256::digest(bytes)),
                })
                .to_string(),
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

fn read_request(
    tenant: &str,
    user: &str,
    path: &str,
    max_bytes: usize,
) -> SandboxWorkspaceFileReadRequest {
    SandboxWorkspaceFileReadRequest {
        scope: request(tenant, user, "true").scope,
        path: path.into(),
        max_bytes,
    }
}

fn write_request(
    tenant: &str,
    user: &str,
    path: &str,
    bytes: &[u8],
    overwrite: bool,
) -> SandboxWorkspaceFileWriteRequest {
    SandboxWorkspaceFileWriteRequest {
        scope: request(tenant, user, "true").scope,
        path: path.into(),
        bytes: bytes.to_vec(),
        overwrite,
    }
}

#[test]
fn workspace_file_requests_serialize_the_typed_resource_scope() {
    let scope = request("tenant", "user", "true").scope;
    let read = SandboxWorkspaceFileReadRequest {
        scope: scope.clone(),
        path: "/workspace/data.bin".into(),
        max_bytes: 42,
    };
    let write = SandboxWorkspaceFileWriteRequest {
        scope,
        path: "/workspace/data.bin".into(),
        bytes: vec![0, 255, 1],
        overwrite: false,
    };

    let read_json = serde_json::to_value(&read).unwrap();
    let write_json = serde_json::to_value(&write).unwrap();
    assert_eq!(read_json["scope"]["tenant_id"], "tenant");
    assert_eq!(read_json["scope"]["user_id"], "user");
    assert_eq!(write_json["scope"]["tenant_id"], "tenant");
    assert_eq!(write_json["bytes"], serde_json::json!([0, 255, 1]));
    assert_eq!(
        serde_json::from_value::<SandboxWorkspaceFileReadRequest>(read_json).unwrap(),
        read
    );
    assert_eq!(
        serde_json::from_value::<SandboxWorkspaceFileWriteRequest>(write_json).unwrap(),
        write
    );
}

#[tokio::test]
async fn workspace_file_paths_are_rejected_before_remote_provisioning() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());

    for path in [
        "/workspace",
        "/workspace/../secret",
        "/workspace-link/file",
        "relative/file",
        "/workspace/file\0tail",
    ] {
        assert_eq!(
            transport
                .write_file(write_request("tenant", "user", path, b"data", false))
                .await,
            Err(SandboxWorkspaceFileError::InvalidPath)
        );
        assert_eq!(
            transport
                .read_file(read_request("tenant", "user", path, 32))
                .await,
            Err(SandboxWorkspaceFileError::InvalidPath)
        );
    }
    assert!(cli.invocations().await.is_empty());
}

#[tokio::test]
async fn oversized_workspace_file_paths_are_rejected_before_remote_provisioning() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    let oversized_component = format!("/workspace/{}", "a".repeat(256));
    let oversized_path = format!("/workspace/{}/file", "a/".repeat(2_043));

    for path in [oversized_component, oversized_path] {
        assert_eq!(
            transport
                .write_file(write_request("tenant", "user", &path, b"data", false))
                .await,
            Err(SandboxWorkspaceFileError::InvalidPath)
        );
        assert_eq!(
            transport
                .read_file(read_request("tenant", "user", &path, 32))
                .await,
            Err(SandboxWorkspaceFileError::InvalidPath)
        );
    }
    assert!(cli.invocations().await.is_empty());
}

#[tokio::test]
async fn workspace_upload_uses_fixed_argv_and_binary_stdin_then_checkpoints() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    let bytes = [0, 255, b'\n', 1, 2, 3];

    let output = transport
        .write_file(write_request(
            "tenant",
            "user",
            "/workspace/nested/data.bin",
            &bytes,
            false,
        ))
        .await
        .unwrap();

    assert_eq!(output.bytes_written, bytes.len());
    assert_eq!(output.sha256, hex::encode(Sha256::digest(bytes)));
    assert!(!output.already_present);
    let invocations = cli.invocations().await;
    let upload = invocations
        .iter()
        .find(|call| call.operation == RailwayCliOperation::WriteWorkspaceFile)
        .expect("workspace upload invocation");
    assert_eq!(upload.stdin.as_deref(), Some(bytes.as_slice()));
    let remote_command = &upload.args[upload
        .args
        .iter()
        .position(|arg| arg == "--")
        .expect("Railway exec command separator")
        + 1..];
    assert_eq!(&remote_command[..3], ["docker", "run", "--rm"]);
    assert!(remote_command.iter().any(|arg| arg == "-i"));
    assert_pair(remote_command, "--network", "none");
    assert!(remote_command.iter().any(|arg| arg == "--read-only"));
    assert!(!remote_command.iter().any(|arg| arg == "--user"));
    assert_pair(remote_command, "--cap-drop", "ALL");
    for capability in ["CHOWN", "DAC_OVERRIDE", "FOWNER"] {
        assert!(
            remote_command
                .windows(2)
                .any(|pair| pair == ["--cap-add", capability])
        );
    }
    assert!(remote_command.iter().any(|arg| {
        arg.starts_with("type=bind,src=/workspace/ironclaw-users/")
            && arg.ends_with(",dst=/workspace")
    }));
    assert!(
        remote_command
            .windows(2)
            .any(|pair| { pair[0] == transport.config.worker_image && pair[1] == "python3" })
    );
    assert!(
        upload
            .args
            .iter()
            .any(|arg| arg == "/workspace/nested/data.bin")
    );
    assert!(!upload.args.iter().any(|arg| arg.as_bytes() == bytes));
    assert!(
        upload
            .args
            .iter()
            .any(|arg| arg == WORKSPACE_FILE_WRITE_HELPER)
    );
    let protocol = &upload.args[upload.args.len() - 8..];
    assert_eq!(protocol[4], bytes.len().to_string());
    assert_eq!(protocol[5], hex::encode(Sha256::digest(bytes)));
    assert_eq!(protocol[6], "1000");
    assert_eq!(protocol[7], "1000");
    assert_eq!(count_checkpoint_creates(&invocations), 1);
}

#[tokio::test]
async fn workspace_upload_no_clobber_accepts_identical_retry_and_rejects_difference() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli);
    let request = write_request("tenant", "user", "/workspace/data.bin", b"original", false);

    assert!(
        !transport
            .write_file(request.clone())
            .await
            .unwrap()
            .already_present
    );
    assert!(transport.write_file(request).await.unwrap().already_present);
    assert_eq!(
        transport
            .write_file(write_request(
                "tenant",
                "user",
                "/workspace/data.bin",
                b"different",
                false,
            ))
            .await,
        Err(SandboxWorkspaceFileError::Conflict)
    );
}

#[tokio::test]
async fn checkpoint_failure_after_upload_is_recoverable_by_identical_retry() {
    let cli = Arc::new(FakeRailwayCli::new());
    cli.fail_next_checkpoint_create();
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    let request = write_request(
        "tenant",
        "user",
        "/workspace/data.bin",
        b"durable after retry",
        false,
    );

    assert_eq!(
        transport.write_file(request.clone()).await,
        Err(SandboxWorkspaceFileError::CheckpointFailed)
    );
    let retry = transport.write_file(request).await.unwrap();
    assert!(retry.already_present);
    assert_eq!(count_checkpoint_creates(&cli.invocations().await), 2);
}

#[tokio::test]
async fn workspace_download_decodes_binary_bytes_and_enforces_requested_bound() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    let bytes = [0, 255, 7, 8, 9];
    transport
        .write_file(write_request(
            "tenant",
            "user",
            "/workspace/data.bin",
            &bytes,
            false,
        ))
        .await
        .unwrap();

    let output = transport
        .read_file(read_request(
            "tenant",
            "user",
            "/workspace/data.bin",
            bytes.len(),
        ))
        .await
        .unwrap();
    assert_eq!(output.bytes, bytes);
    assert_eq!(output.sha256, hex::encode(Sha256::digest(bytes)));
    let invocations = cli.invocations().await;
    let download = invocations
        .iter()
        .find(|call| call.operation == RailwayCliOperation::ReadWorkspaceFile)
        .expect("workspace download invocation");
    let remote_command = &download.args[download
        .args
        .iter()
        .position(|arg| arg == "--")
        .expect("Railway exec command separator")
        + 1..];
    assert_eq!(&remote_command[..3], ["docker", "run", "--rm"]);
    assert!(!remote_command.iter().any(|arg| arg == "-i"));
    assert!(remote_command.iter().any(|arg| {
        arg.starts_with("type=bind,src=/workspace/ironclaw-users/")
            && arg.ends_with(",dst=/workspace,readonly")
    }));
    assert!(
        remote_command
            .windows(2)
            .any(|pair| { pair[0] == transport.config.worker_image && pair[1] == "python3" })
    );
    assert_eq!(
        transport
            .read_file(read_request(
                "tenant",
                "user",
                "/workspace/data.bin",
                bytes.len() - 1,
            ))
            .await,
        Err(SandboxWorkspaceFileError::TooLarge {
            max_bytes: bytes.len() - 1,
        })
    );
}

#[tokio::test]
async fn workspace_download_stdout_ceiling_covers_ten_mib_base64_expansion() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    transport
        .write_file(write_request(
            "tenant",
            "user",
            "/workspace/data.bin",
            b"x",
            false,
        ))
        .await
        .unwrap();

    transport
        .read_file(read_request(
            "tenant",
            "user",
            "/workspace/data.bin",
            10 * 1024 * 1024,
        ))
        .await
        .unwrap();

    let invocation = cli
        .invocations()
        .await
        .into_iter()
        .find(|call| call.operation == RailwayCliOperation::ReadWorkspaceFile)
        .unwrap();
    assert!(invocation.output_limit >= 13_981_528);
}

#[tokio::test]
#[tracing_test::traced_test]
async fn workspace_file_transport_failure_logs_operation_and_sanitized_cause() {
    let cli = Arc::new(FakeRailwayCli::new());
    cli.fail_next_file_exec();
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli);

    assert_eq!(
        transport
            .write_file(write_request(
                "tenant",
                "user",
                "/workspace/data.bin",
                b"content",
                false,
            ))
            .await,
        Err(SandboxWorkspaceFileError::TransportFailed)
    );
    assert!(logs_contain("write sandbox workspace file"));
    assert!(logs_contain("Timeout"));
}

#[tokio::test]
#[tracing_test::traced_test]
async fn malformed_workspace_file_response_logs_bounded_metadata() {
    let cli = Arc::new(FakeRailwayCli::new());
    cli.malform_next_file_output();
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli);

    assert_eq!(
        transport
            .write_file(write_request(
                "tenant",
                "user",
                "/workspace/data.bin",
                b"content",
                false,
            ))
            .await,
        Err(SandboxWorkspaceFileError::InvalidResponse)
    );
    assert!(logs_contain("Railway workspace file response was rejected"));
    assert!(logs_contain("write sandbox workspace file"));
    assert!(logs_contain("stdout_bytes=34"));
}

#[tokio::test]
async fn workspace_file_transport_failure_destroys_before_reprovisioning() {
    let cli = Arc::new(FakeRailwayCli::new());
    cli.fail_next_file_exec();
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    let request = write_request("tenant", "user", "/workspace/data.bin", b"content", false);

    assert_eq!(
        transport.write_file(request.clone()).await,
        Err(SandboxWorkspaceFileError::TransportFailed)
    );
    assert_eq!(count_destroys(&cli.invocations().await), 1);
    transport.write_file(request).await.unwrap();
    assert_eq!(count_creates(&cli.invocations().await), 2);
}

#[tokio::test]
async fn fixed_python_helpers_reject_symlink_and_directory_targets() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
    std::fs::create_dir(root.path().join("directory")).unwrap();

    let escaped = run_write_helper(root.path(), "/workspace/escape/file", b"secret").await;
    assert_eq!(escaped["code"], "invalid_path");
    assert!(!outside.path().join("file").exists());

    let directory = run_write_helper(root.path(), "/workspace/directory", b"data").await;
    assert_eq!(directory["code"], "not_regular_file");

    let read_directory = run_read_helper(root.path(), "/workspace/directory", 32).await;
    assert_eq!(read_directory["code"], "not_regular_file");
}

#[tokio::test]
async fn shortened_upload_stdin_never_publishes_and_full_retry_recovers() {
    let root = tempfile::tempdir().unwrap();
    let expected = b"complete payload";
    let shortened = &expected[..expected.len() - 3];

    let rejected = run_write_helper_with_expected(
        root.path(),
        "/workspace/nested/data.bin",
        shortened,
        expected,
    )
    .await;
    assert_eq!(rejected["code"], "input_mismatch");
    assert!(!root.path().join("nested/data.bin").exists());

    let recovered = run_write_helper(root.path(), "/workspace/nested/data.bin", expected).await;
    assert_eq!(recovered["status"], "ok");
    assert_eq!(
        std::fs::read(root.path().join("nested/data.bin")).unwrap(),
        expected
    );
}

#[tokio::test]
async fn upload_helper_sets_nested_directories_and_file_to_requested_worker_owner() {
    use std::os::unix::fs::MetadataExt as _;

    let root = tempfile::tempdir().unwrap();
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    let output =
        run_write_helper_for_owner(root.path(), "/workspace/a/b/data.bin", b"owned", uid, gid)
            .await;
    assert_eq!(output["status"], "ok");
    for relative in ["a", "a/b", "a/b/data.bin"] {
        let metadata = std::fs::metadata(root.path().join(relative)).unwrap();
        assert_eq!(metadata.uid(), uid);
        assert_eq!(metadata.gid(), gid);
    }
}

#[tokio::test]
async fn identical_no_clobber_retry_repairs_legacy_file_owner_and_mode() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.bin");
    std::fs::write(&path, b"identical").unwrap();
    // The production helper runs as root and can open a legacy mode-000 file.
    // This unprivileged helper regression uses read-only owner mode to reach
    // the same identical-file repair branch without requiring test elevation.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };

    let output =
        run_write_helper_for_owner(root.path(), "/workspace/data.bin", b"identical", uid, gid)
            .await;

    assert_eq!(output["status"], "ok");
    assert_eq!(output["already_present"], true);
    let metadata = std::fs::metadata(path).unwrap();
    assert_eq!(metadata.uid(), uid);
    assert_eq!(metadata.gid(), gid);
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
}

#[tokio::test]
async fn ambiguous_upload_response_destroys_uncheckpointed_sandbox() {
    let cli = Arc::new(FakeRailwayCli::new());
    cli.malform_next_file_output();
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());

    assert_eq!(
        transport
            .write_file(write_request(
                "tenant",
                "user",
                "/workspace/data.bin",
                b"content",
                false,
            ))
            .await,
        Err(SandboxWorkspaceFileError::InvalidResponse)
    );
    let invocations = cli.invocations().await;
    assert_eq!(count_checkpoint_creates(&invocations), 0);
    assert_eq!(count_destroys(&invocations), 1);
}

async fn run_write_helper(root: &Path, path: &str, bytes: &[u8]) -> serde_json::Value {
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    run_write_helper_for_owner(root, path, bytes, uid, gid).await
}

async fn run_write_helper_for_owner(
    root: &Path,
    path: &str,
    bytes: &[u8],
    uid: u32,
    gid: u32,
) -> serde_json::Value {
    run_write_helper_with_protocol(root, path, bytes, bytes.len(), &sha256_hex(bytes), uid, gid)
        .await
}

async fn run_write_helper_with_expected(
    root: &Path,
    path: &str,
    stdin: &[u8],
    expected: &[u8],
) -> serde_json::Value {
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    run_write_helper_with_protocol(
        root,
        path,
        stdin,
        expected.len(),
        &sha256_hex(expected),
        uid,
        gid,
    )
    .await
}

async fn run_write_helper_with_protocol(
    root: &Path,
    path: &str,
    stdin: &[u8],
    expected_len: usize,
    expected_sha256: &str,
    uid: u32,
    gid: u32,
) -> serde_json::Value {
    let root = root.to_string_lossy().into_owned();
    let expected_len = expected_len.to_string();
    let uid = uid.to_string();
    let gid = gid.to_string();
    run_python_helper(
        WORKSPACE_FILE_WRITE_HELPER,
        &[
            &root,
            path,
            "0",
            "10485760",
            &expected_len,
            expected_sha256,
            &uid,
            &gid,
        ],
        stdin,
    )
    .await
}

async fn run_read_helper(root: &Path, path: &str, max_bytes: usize) -> serde_json::Value {
    let root = root.to_string_lossy().into_owned();
    let max_bytes = max_bytes.to_string();
    run_python_helper(WORKSPACE_FILE_READ_HELPER, &[&root, path, &max_bytes], &[]).await
}

async fn run_python_helper(script: &str, args: &[&str], stdin: &[u8]) -> serde_json::Value {
    let mut child = tokio::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(stdin).await.unwrap();
    input.shutdown().await.unwrap();
    drop(input);
    let output = child.wait_with_output().await.unwrap();
    assert!(
        output.status.success(),
        "python helper failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
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
    let create = invocations
        .iter()
        .find(|call| call.args.starts_with(&["sandbox".into(), "create".into()]))
        .expect("sandbox create invocation");
    assert_pair(&create.args, "--idle-timeout-minutes", "5");
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
async fn shutdown_checkpoints_and_destroys_every_process_owned_sandbox() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    transport
        .run_command(request("tenant", "user-a", "true"))
        .await
        .unwrap();
    transport
        .run_command(request("tenant", "user-b", "true"))
        .await
        .unwrap();

    transport.shutdown().await.unwrap();

    let invocations = cli.invocations().await;
    assert_eq!(count_destroys(&invocations), 2);
    assert_eq!(count_checkpoint_creates(&invocations), 2);
    assert!(cli.live_sandboxes.lock().await.is_empty());
    let states = transport
        .users
        .lock()
        .await
        .values()
        .map(|tracked| tracked.state.clone())
        .collect::<Vec<_>>();
    for state in states {
        assert!(matches!(
            state.lock().await.lifecycle,
            UserSandboxLifecycle::Absent
        ));
    }
}

#[tokio::test]
async fn shutdown_preserves_live_sandbox_when_final_checkpoint_fails() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    transport
        .run_command(request("tenant", "user", "true"))
        .await
        .unwrap();
    let state = transport
        .state_for(user_key("tenant", "user"))
        .await
        .unwrap();
    state.lock().await.checkpoint_current = false;
    cli.fail_next_checkpoint_create();

    assert!(transport.shutdown().await.is_err());

    assert_eq!(count_destroys(&cli.invocations().await), 0);
    assert_eq!(cli.live_sandboxes.lock().await.len(), 1);
}

#[tokio::test]
async fn malformed_worker_response_forces_shutdown_to_checkpoint_before_destroy() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    transport
        .run_command(request("tenant", "user", "first command"))
        .await
        .unwrap();
    cli.malform_next_worker_output();

    assert!(
        transport
            .run_command(request("tenant", "user", "mutating command"))
            .await
            .is_err()
    );

    transport.shutdown().await.unwrap();
    let invocations = cli.invocations().await;
    assert_eq!(count_checkpoint_creates(&invocations), 2);
    assert_eq!(count_destroys(&invocations), 1);
    assert!(cli.live_sandboxes.lock().await.is_empty());
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
async fn user_state_registry_never_evicts_unconfirmed_cleanup() {
    let transport = RailwayPreviewSandboxTransport::with_cli_and_capacity(
        config(),
        Arc::new(FakeRailwayCli::new()),
        1,
    );
    let pending_key = user_key("tenant", "pending");
    let pending = transport.state_for(pending_key.clone()).await.unwrap();
    pending.lock().await.lifecycle = UserSandboxLifecycle::CleanupPending("sandbox-1".to_string());
    drop(pending);

    let error = transport
        .state_for(user_key("tenant", "replacement"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, RuntimeProcessError::ExecutionFailed(message) if message.contains("capacity is exhausted"))
    );
    assert!(transport.users.lock().await.contains_key(&pending_key));
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
async fn ephemeral_worker_defaults_to_networkless_and_hardened() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());
    transport
        .run_command(request("tenant", "user", "true"))
        .await
        .unwrap();
    let invocations = cli.invocations().await;
    let argv = &model_container_runs(&invocations)[0].args;
    assert_pair(argv, "--network", "none");
    assert_pair(argv, "--env", "IRONCLAW_REBORN_NETWORK_MODE=disabled");
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
async fn explicit_direct_network_omits_network_none_without_weakening_worker_hardening() {
    let cli = Arc::new(FakeRailwayCli::new());
    let transport =
        RailwayPreviewSandboxTransport::with_cli(config().with_network_enabled(), cli.clone());
    transport
        .run_command(request("tenant", "user", "true"))
        .await
        .unwrap();
    let invocations = cli.invocations().await;
    let argv = &model_container_runs(&invocations)[0].args;

    assert!(
        !argv.iter().any(|argument| argument == "--network"),
        "direct mode must leave Docker's configured network enabled"
    );
    assert_pair(argv, "--env", "IRONCLAW_REBORN_NETWORK_MODE=direct");
    assert!(argv.contains(&"--read-only".to_string()));
    assert_pair(argv, "--user", WORKER_USER);
    assert_pair(argv, "--cap-drop", "ALL");
    assert_pair(argv, "--security-opt", "no-new-privileges:true");
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
async fn failed_workspace_bootstrap_destroys_the_new_untracked_sandbox() {
    let cli = Arc::new(FakeRailwayCli::new());
    cli.fail_next_bootstrap_exec();
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());

    assert!(matches!(
        transport
            .run_command(request("tenant", "user", "true"))
            .await,
        Err(RuntimeProcessError::ExecutionFailed(message))
            if message.contains("workspace bootstrap failed")
    ));
    let invocations = cli.invocations().await;
    assert!(invocations.iter().any(|call| {
        call.args.starts_with(&["sandbox".into(), "destroy".into()])
            && call
                .args
                .windows(2)
                .any(|pair| pair == ["--id", "sandbox-1"])
    }));
}

#[tokio::test]
async fn unconfirmed_bootstrap_cleanup_is_retried_before_replacement() {
    let cli = Arc::new(FakeRailwayCli::new());
    cli.fail_next_bootstrap_exec();
    cli.fail_next_destroy();
    let transport = RailwayPreviewSandboxTransport::with_cli(config(), cli.clone());

    assert!(matches!(
        transport
            .run_command(request("tenant", "user", "true"))
            .await,
        Err(RuntimeProcessError::ExecutionFailed(message))
            if message.contains("remote cleanup could not be confirmed")
    ));
    transport
        .run_command(request("tenant", "user", "true"))
        .await
        .expect("cleanup retry succeeds before replacement provisioning");

    let invocations = cli.invocations().await;
    let retry_destroy = invocations
        .iter()
        .rposition(|call| {
            call.args.starts_with(&["sandbox".into(), "destroy".into()])
                && call
                    .args
                    .windows(2)
                    .any(|pair| pair == ["--id", "sandbox-1"])
        })
        .expect("pending sandbox cleanup is retried");
    let replacement_create = invocations
        .iter()
        .rposition(|call| call.args.starts_with(&["sandbox".into(), "create".into()]))
        .expect("replacement sandbox is created");
    assert!(retry_destroy < replacement_create);
    assert_eq!(count_creates(&invocations), 2);
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

fn count_checkpoint_creates(calls: &[RailwayCliInvocation]) -> usize {
    calls
        .iter()
        .filter(|call| {
            call.args
                .starts_with(&["sandbox".into(), "checkpoint".into(), "create".into()])
        })
        .count()
}

fn count_destroys(calls: &[RailwayCliInvocation]) -> usize {
    calls
        .iter()
        .filter(|call| call.args.starts_with(&["sandbox".into(), "destroy".into()]))
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
