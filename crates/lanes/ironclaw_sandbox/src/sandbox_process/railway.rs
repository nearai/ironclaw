//! Railway-backed preview provider for persistent user sandboxes.
//!
//! Railway is the trusted outer runtime. The model never receives Railway CLI
//! access: this transport invokes a scrubbed local CLI and the outer VM owns a
//! fixed inner Python image against a per-user directory in Railway's
//! checkpointed workspace. Inner containers are intentionally ephemeral:
//! Railway does not preserve their mount namespaces between outer exec calls.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Mutex,
};

use ironclaw_host_api::process::{
    CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError, SandboxCommandTransport,
    SandboxWorkspaceFileError, SandboxWorkspaceFileReadOutput, SandboxWorkspaceFileReadRequest,
    SandboxWorkspaceFileTransport, SandboxWorkspaceFileWriteOutput,
    SandboxWorkspaceFileWriteRequest,
};

use crate::sandbox_process::RebornSandboxUserKey;

use super::worker_spec::DOCKER_WORKER_USER as WORKER_USER;

const DEFAULT_IDLE_TIMEOUT_MINUTES: u16 = 5;
// Pin the exact multi-platform manifest exercised by the Railway preview
// canary. Operators may override this explicitly, but the default must not
// drift between review and deployment.
const DEFAULT_WORKER_IMAGE: &str =
    "python:3.12-slim@sha256:57cd7c3a7a273101a6485ba99423ee568157882804b1124b4dd04266317710de";
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_COMMAND_TIMEOUT: Duration = Duration::from_secs(600);
const DEFAULT_OUTPUT_LIMIT: usize = 16 * 1024;
const REMOTE_CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
const REMOTE_CHECKPOINT_TIMEOUT: Duration = Duration::from_secs(120);
// A checkpoint list is provider metadata rather than model output. Keep it
// bounded, but large enough for the maximum tracked-user registry. Truncation
// produces invalid JSON and therefore fails closed instead of treating an
// existing durable checkpoint as absent.
const PROVIDER_LIST_OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const MAX_WORKSPACE_FILE_BYTES: usize = 10 * 1024 * 1024;
const MAX_WORKSPACE_FILE_PATH_BYTES: usize = 4_096;
const MAX_WORKSPACE_FILE_COMPONENT_BYTES: usize = 255;
const WORKSPACE_FILE_JSON_OVERHEAD: usize = 512;
const MAX_TRACKED_USERS: usize = 4_096;
const WORKSPACE_ROOT: &str = "/workspace";
const RAILWAY_WORKSPACES_ROOT: &str = "/workspace/ironclaw-users";
const EXIT_SENTINEL: &str = "__IRONCLAW_RAILWAY_PRIVATE_EXIT_CODE__=";
const WORKSPACE_BOOTSTRAP_MARKER: &str = "ironclaw-railway-workspace-bootstrap";
const WORKSPACE_WORKER_UID: u32 = 1000;
const WORKSPACE_WORKER_GID: u32 = 1000;
const CHECKPOINT_FAILURE_WARNING: &str = "[IronClaw warning: command completed, but the Railway workspace checkpoint failed; recent state may not survive sandbox restart.]";

// The complete Docker command arrives as distinct argv values. In particular,
// the model command is the final argv value and is never interpolated into
// this shell program.
const OUTER_EXEC_WRAPPER: &str = "set +e\n\"$@\"\nstatus=$?\nprintf '\\n__IRONCLAW_RAILWAY_PRIVATE_EXIT_CODE__=%s\\n' \"$status\"\nexit 0";

// Host-authored helpers. The untrusted workspace path is passed as one
// positional argv value and file bytes travel only over stdin.
const WORKSPACE_FILE_WRITE_HELPER: &str = r#"import hashlib, json, os, stat, sys, uuid
def result(value):
 print(json.dumps(value,separators=(",",":"))); raise SystemExit(0)
def fail(code): result({"status":"error","code":code})
root=os.path.realpath(sys.argv[1]); model_path=sys.argv[2]; overwrite=sys.argv[3]=="1"; max_bytes=int(sys.argv[4]); expected_len=int(sys.argv[5]); expected_digest=sys.argv[6]; worker_uid=int(sys.argv[7]); worker_gid=int(sys.argv[8]); prefix="/workspace/"
if not model_path.startswith(prefix): fail("invalid_path")
parts=model_path[len(prefix):].split("/")
if not parts or any(part in ("",".","..") for part in parts): fail("invalid_path")
if expected_len>max_bytes: fail("too_large")
data=sys.stdin.buffer.read(max_bytes+1)
digest=hashlib.sha256(data).hexdigest()
if len(data)!=expected_len or digest!=expected_digest: fail("input_mismatch")
try: root_stat=os.lstat(root)
except OSError: fail("invalid_path")
if not stat.S_ISDIR(root_stat.st_mode) or os.path.islink(root): fail("invalid_path")
flags=os.O_RDONLY|getattr(os,"O_DIRECTORY",0)|getattr(os,"O_NOFOLLOW",0)
try: parent_fd=os.open(root,flags)
except OSError: fail("invalid_path")
if not stat.S_ISDIR(os.fstat(parent_fd).st_mode): fail("invalid_path")
for part in parts[:-1]:
 try: item=os.stat(part,dir_fd=parent_fd,follow_symlinks=False)
 except FileNotFoundError:
  try: os.mkdir(part,0o700,dir_fd=parent_fd)
  except FileExistsError: pass
  item=os.stat(part,dir_fd=parent_fd,follow_symlinks=False)
 if stat.S_ISLNK(item.st_mode) or not stat.S_ISDIR(item.st_mode): fail("invalid_path")
 try: next_fd=os.open(part,flags,dir_fd=parent_fd)
 except OSError: fail("invalid_path")
 if not stat.S_ISDIR(os.fstat(next_fd).st_mode): fail("invalid_path")
 os.fchown(next_fd,worker_uid,worker_gid); os.fchmod(next_fd,0o700)
 os.close(parent_fd); parent_fd=next_fd
leaf=parts[-1]
def existing_bytes():
 try: item=os.stat(leaf,dir_fd=parent_fd,follow_symlinks=False)
 except FileNotFoundError: return None
 if stat.S_ISLNK(item.st_mode) or not stat.S_ISREG(item.st_mode): fail("not_regular_file")
 try: fd=os.open(leaf,os.O_RDONLY|getattr(os,"O_NOFOLLOW",0),dir_fd=parent_fd)
 except OSError: fail("not_regular_file")
 opened=os.fstat(fd)
 if not stat.S_ISREG(opened.st_mode): fail("not_regular_file")
 value=b""
 while len(value)<=max_bytes:
  chunk=os.read(fd,min(65536,max_bytes+1-len(value)))
  if not chunk: break
  value+=chunk
 os.close(fd); return (value,opened.st_dev,opened.st_ino)
def repair_existing(existing):
 value,expected_dev,expected_ino=existing
 try: fd=os.open(leaf,os.O_RDONLY|getattr(os,"O_NOFOLLOW",0),dir_fd=parent_fd)
 except OSError: fail("not_regular_file")
 opened=os.fstat(fd)
 if not stat.S_ISREG(opened.st_mode) or opened.st_dev!=expected_dev or opened.st_ino!=expected_ino: fail("verification_failed")
 os.fchown(fd,worker_uid,worker_gid); os.fchmod(fd,0o600); os.fsync(fd)
 repaired=os.fstat(fd); os.close(fd)
 if repaired.st_uid!=worker_uid or repaired.st_gid!=worker_gid or stat.S_IMODE(repaired.st_mode)!=0o600: fail("verification_failed")
 return value
present=existing_bytes()
if present is not None and not overwrite:
 value=present[0]
 if len(value)==len(data) and hashlib.sha256(value).hexdigest()==digest:
  repair_existing(present); result({"status":"ok","bytes_written":len(data),"sha256":digest,"already_present":True})
 fail("conflict")
temporary=".ironclaw-upload-"+uuid.uuid4().hex; temporary_created=False
try:
 fd=os.open(temporary,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600,dir_fd=parent_fd); temporary_created=True; offset=0
 while offset<len(data): offset+=os.write(fd,data[offset:])
 os.fchown(fd,worker_uid,worker_gid); os.fchmod(fd,0o600)
 os.fsync(fd)
 temporary_stat=os.fstat(fd)
 if not stat.S_ISREG(temporary_stat.st_mode) or temporary_stat.st_uid!=worker_uid or temporary_stat.st_gid!=worker_gid: fail("verification_failed")
 os.close(fd)
 if overwrite:
  os.replace(temporary,leaf,src_dir_fd=parent_fd,dst_dir_fd=parent_fd); temporary_created=False
 else:
  try:
   os.link(temporary,leaf,src_dir_fd=parent_fd,dst_dir_fd=parent_fd,follow_symlinks=False); os.unlink(temporary,dir_fd=parent_fd); temporary_created=False
  except FileExistsError:
   present=existing_bytes()
   if present is None or len(present[0])!=len(data) or hashlib.sha256(present[0]).hexdigest()!=digest: fail("conflict")
   repair_existing(present)
 verified=existing_bytes()
 if verified is None or len(verified[0])!=len(data) or hashlib.sha256(verified[0]).hexdigest()!=digest: fail("verification_failed")
 result({"status":"ok","bytes_written":len(data),"sha256":digest,"already_present":present is not None and not overwrite})
finally:
 if temporary_created:
  try: os.unlink(temporary,dir_fd=parent_fd)
  except OSError: pass
 os.close(parent_fd)
"#;

const WORKSPACE_FILE_READ_HELPER: &str = r#"import base64, hashlib, json, os, stat, sys
def result(value):
 print(json.dumps(value,separators=(",",":"))); raise SystemExit(0)
def fail(code): result({"status":"error","code":code})
root=os.path.realpath(sys.argv[1]); model_path=sys.argv[2]; max_bytes=int(sys.argv[3]); prefix="/workspace/"
if not model_path.startswith(prefix): fail("invalid_path")
parts=model_path[len(prefix):].split("/")
if not parts or any(part in ("",".","..") for part in parts): fail("invalid_path")
try: root_stat=os.lstat(root)
except OSError: fail("invalid_path")
if not stat.S_ISDIR(root_stat.st_mode) or os.path.islink(root): fail("invalid_path")
flags=os.O_RDONLY|getattr(os,"O_DIRECTORY",0)|getattr(os,"O_NOFOLLOW",0)
try: parent_fd=os.open(root,flags)
except OSError: fail("invalid_path")
if not stat.S_ISDIR(os.fstat(parent_fd).st_mode): fail("invalid_path")
for part in parts[:-1]:
 try: item=os.stat(part,dir_fd=parent_fd,follow_symlinks=False)
 except FileNotFoundError: fail("not_found")
 if stat.S_ISLNK(item.st_mode): fail("invalid_path")
 if not stat.S_ISDIR(item.st_mode): fail("not_regular_file")
 try: next_fd=os.open(part,flags,dir_fd=parent_fd)
 except OSError: fail("invalid_path")
 if not stat.S_ISDIR(os.fstat(next_fd).st_mode): fail("invalid_path")
 os.close(parent_fd); parent_fd=next_fd
leaf=parts[-1]
try: item=os.stat(leaf,dir_fd=parent_fd,follow_symlinks=False)
except FileNotFoundError: fail("not_found")
if stat.S_ISLNK(item.st_mode) or not stat.S_ISREG(item.st_mode): fail("not_regular_file")
try: fd=os.open(leaf,os.O_RDONLY|getattr(os,"O_NOFOLLOW",0),dir_fd=parent_fd)
except OSError: fail("not_regular_file")
opened=os.fstat(fd)
if not stat.S_ISREG(opened.st_mode): fail("not_regular_file")
if opened.st_size>max_bytes: fail("too_large")
data=b""
while len(data)<=max_bytes:
 chunk=os.read(fd,min(65536,max_bytes+1-len(data)))
 if not chunk: break
 data+=chunk
after=os.fstat(fd); os.close(fd); os.close(parent_fd)
if len(data)>max_bytes: fail("too_large")
if after.st_size!=len(data) or after.st_ino!=opened.st_ino or after.st_dev!=opened.st_dev: fail("verification_failed")
result({"status":"ok","bytes":base64.b64encode(data).decode("ascii"),"sha256":hashlib.sha256(data).hexdigest()})
"#;

/// Explicit, host-only configuration for the preview transport.
///
/// Railway auth is not stored here. Each CLI child copies only a selected
/// `RAILWAY_TOKEN` or `RAILWAY_API_TOKEN` plus minimal CLI runtime variables.
#[derive(Debug, Clone, PartialEq, Eq)]
/// Configuration for the experimental Railway sandbox transport.
///
/// # Deployment constraint
///
/// PR1 requires exactly one IronClaw application replica. Per-user command
/// serialization is process-local; Railway checkpoints are durable, but they
/// are not a distributed lease. Running multiple IronClaw replicas can let two
/// transports restore and replace the same per-user checkpoint concurrently.
pub struct RailwayPreviewSandboxConfig {
    project_id: String,
    environment_id: String,
    cli_path: PathBuf,
    idle_timeout_minutes: u16,
    worker_image: String,
    disable_network: bool,
}

impl RailwayPreviewSandboxConfig {
    pub fn new(
        project_id: impl Into<String>,
        environment_id: impl Into<String>,
    ) -> Result<Self, RuntimeProcessError> {
        Ok(Self {
            project_id: required_config_value("project identifier", project_id.into())?,
            environment_id: required_config_value("environment identifier", environment_id.into())?,
            cli_path: PathBuf::from("railway"),
            idle_timeout_minutes: DEFAULT_IDLE_TIMEOUT_MINUTES,
            worker_image: DEFAULT_WORKER_IMAGE.to_string(),
            disable_network: true,
        })
    }

    pub fn with_cli_path(mut self, cli_path: impl Into<PathBuf>) -> Self {
        self.cli_path = cli_path.into();
        self
    }

    pub fn with_idle_timeout_minutes(mut self, minutes: u16) -> Result<Self, RuntimeProcessError> {
        if minutes == 0 {
            return Err(RuntimeProcessError::ExecutionFailed(
                "Railway preview configuration requires a non-zero idle timeout".to_string(),
            ));
        }
        self.idle_timeout_minutes = minutes;
        Ok(self)
    }

    pub fn with_worker_image(
        mut self,
        image: impl Into<String>,
    ) -> Result<Self, RuntimeProcessError> {
        self.worker_image = required_config_value("worker image", image.into())?;
        Ok(self)
    }

    /// Allow the inner worker to use the outer Railway sandbox's NAT egress.
    ///
    /// This is deliberately explicit: direct egress is enabled by the
    /// sandbox deployment factory, while ad-hoc transport construction keeps
    /// the fail-closed `--network none` default.
    pub fn with_network_enabled(mut self) -> Self {
        self.disable_network = false;
        self
    }

    fn container_network_mode(&self) -> Option<String> {
        self.disable_network.then(|| "none".to_string())
    }

    fn network_mode_env(&self) -> String {
        format!(
            "{}={}",
            super::broker::REBORN_NETWORK_MODE_ENV,
            if self.disable_network {
                "disabled"
            } else {
                "direct"
            }
        )
    }
}

/// Railway preview transport for a Docker daemon hosted inside a Railway
/// sandbox VM. Live IDs are process-local; deterministic per-user checkpoints
/// restore the outer workspace when a VM expires or IronClaw restarts. Each
/// command receives a fresh inner container so no mounted inner container has
/// to survive across Railway exec calls. Network access follows the explicit
/// configuration posture; construction defaults to `--network none`.
#[derive(Clone)]
pub struct RailwayPreviewSandboxTransport {
    config: RailwayPreviewSandboxConfig,
    cli: Arc<dyn RailwayCli>,
    users: Arc<Mutex<HashMap<RebornSandboxUserKey, TrackedUserState>>>,
    max_tracked_users: usize,
}

impl std::fmt::Debug for RailwayPreviewSandboxTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RailwayPreviewSandboxTransport")
            .field("project_id", &self.config.project_id)
            .field("environment_id", &self.config.environment_id)
            .field("cli_path", &self.config.cli_path)
            .field("idle_timeout_minutes", &self.config.idle_timeout_minutes)
            .field("worker_image", &self.config.worker_image)
            .finish_non_exhaustive()
    }
}

impl RailwayPreviewSandboxTransport {
    pub fn new(config: RailwayPreviewSandboxConfig) -> Self {
        Self {
            config,
            cli: Arc::new(SystemRailwayCli),
            users: Arc::new(Mutex::new(HashMap::new())),
            max_tracked_users: MAX_TRACKED_USERS,
        }
    }

    async fn state_for(
        &self,
        key: RebornSandboxUserKey,
    ) -> Result<Arc<Mutex<UserSandboxState>>, RuntimeProcessError> {
        let mut users = self.users.lock().await;
        if let Some(tracked) = users.get_mut(&key) {
            tracked.last_touched = Instant::now();
            return Ok(tracked.state.clone());
        }
        if users.len() >= self.max_tracked_users {
            let idle_lru = users
                .iter()
                .filter(|(_, tracked)| Arc::strong_count(&tracked.state) == 1)
                // Losing a pending ID could allow replacement provisioning
                // while cleanup of the previous sandbox remains unconfirmed.
                .filter(|(_, tracked)| {
                    tracked.state.try_lock().is_ok_and(|state| {
                        !matches!(state.lifecycle, UserSandboxLifecycle::CleanupPending(_))
                    })
                })
                .min_by_key(|(_, tracked)| tracked.last_touched)
                .map(|(key, _)| key.clone())
                .ok_or_else(|| {
                    RuntimeProcessError::ExecutionFailed(
                        "Railway preview user-state capacity is exhausted".to_string(),
                    )
                })?;
            users.remove(&idle_lru);
        }
        let state = Arc::new(Mutex::new(UserSandboxState::default()));
        users.insert(
            key,
            TrackedUserState {
                state: state.clone(),
                last_touched: Instant::now(),
            },
        );
        Ok(state)
    }

    async fn provision(
        &self,
        key: &RebornSandboxUserKey,
        state: &mut UserSandboxState,
        deadline: Instant,
        output_limit: usize,
    ) -> Result<String, RuntimeProcessError> {
        let checkpoint_name = checkpoint_name(key);
        let checkpoints = self
            .execute_cli(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::ListCheckpoints,
                    checkpoint_list_argv(&self.config),
                    PROVIDER_LIST_OUTPUT_LIMIT,
                ),
                deadline,
            )
            .await?;
        let restore_checkpoint = checkpoint_exists(&checkpoints.stdout, &checkpoint_name)?;
        let created = self
            .execute_cli(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::CreateSandbox,
                    sandbox_create_argv(
                        &self.config,
                        restore_checkpoint.then_some(checkpoint_name.as_str()),
                    ),
                    output_limit,
                ),
                deadline,
            )
            .await?;
        let sandbox_id = parse_sandbox_id(&created.stdout)?;
        state.lifecycle = UserSandboxLifecycle::CleanupPending(sandbox_id.clone());
        let bootstrap = async {
            let timeout = deadline_remaining(deadline)?;
            self.execute_cli(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::ExecuteSandboxCommand,
                    sandbox_exec_argv(
                        &self.config,
                        &sandbox_id,
                        timeout,
                        workspace_bootstrap_argv(&railway_workspace_path(key)),
                    ),
                    output_limit,
                ),
                deadline,
            )
            .await
        }
        .await;
        if let Err(bootstrap_error) = bootstrap {
            self.destroy_after_failed_execution(&sandbox_id, output_limit, &bootstrap_error)
                .await?;
            state.lifecycle = UserSandboxLifecycle::Absent;
            return Err(bootstrap_error);
        }
        state.lifecycle = UserSandboxLifecycle::Live(sandbox_id.clone());
        Ok(sandbox_id)
    }

    async fn sandbox_is_live(
        &self,
        sandbox_id: &str,
        deadline: Instant,
    ) -> Result<bool, RuntimeProcessError> {
        let sandboxes = self
            .execute_cli(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::ListSandboxes,
                    sandbox_list_argv(&self.config),
                    PROVIDER_LIST_OUTPUT_LIMIT,
                ),
                deadline,
            )
            .await?;
        sandbox_exists(&sandboxes.stdout, sandbox_id)
    }

    async fn checkpoint(
        &self,
        key: &RebornSandboxUserKey,
        sandbox_id: &str,
        output_limit: usize,
    ) -> Result<(), RuntimeProcessError> {
        // Railway checkpoint names are unique within the environment and the
        // CLI's documented create semantics replace an existing checkpoint of
        // the same name synchronously. One deterministic name per user bounds
        // retention to the latest workspace snapshot; the live canary writes,
        // restores, and replaces that same name across transport restart.
        let result = self
            .cli
            .execute(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::CreateCheckpoint,
                    checkpoint_create_argv(&self.config, sandbox_id, &checkpoint_name(key)),
                    output_limit,
                ),
                REMOTE_CHECKPOINT_TIMEOUT,
            )
            .await;
        result.map(|_| ()).map_err(|error| {
            tracing::error!(
                ?error,
                sandbox_id,
                "Railway command completed but durable checkpoint creation failed"
            );
            RuntimeProcessError::ExecutionFailed(
                "Railway preview command completed but its workspace checkpoint failed".to_string(),
            )
        })
    }

    async fn destroy_after_failed_execution(
        &self,
        sandbox_id: &str,
        output_limit: usize,
        execution_error: &RuntimeProcessError,
    ) -> Result<(), RuntimeProcessError> {
        if let Err(cleanup_error) = self.destroy_sandbox(sandbox_id, output_limit).await {
            tracing::error!(
                ?execution_error,
                ?cleanup_error,
                "Railway sandbox execution failed and remote cleanup was not confirmed"
            );
            return Err(RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox execution failed and remote cleanup could not be confirmed"
                    .to_string(),
            ));
        }
        Ok(())
    }

    async fn destroy_sandbox(
        &self,
        sandbox_id: &str,
        output_limit: usize,
    ) -> Result<(), RuntimeProcessError> {
        self.cli
            .execute(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::DestroySandbox,
                    sandbox_destroy_argv(&self.config, sandbox_id),
                    output_limit,
                ),
                REMOTE_CLEANUP_TIMEOUT,
            )
            .await
            .map(|_| ())
    }

    async fn shutdown_owned_sandboxes(&self) -> Result<(), RuntimeProcessError> {
        let users = {
            let users = self.users.lock().await;
            users
                .iter()
                .map(|(key, tracked)| (key.clone(), tracked.state.clone()))
                .collect::<Vec<_>>()
        };
        let mut first_error = None;
        for (key, state) in users {
            let mut state = state.lock().await;
            let sandbox_id = match state.lifecycle.clone() {
                UserSandboxLifecycle::Absent => continue,
                UserSandboxLifecycle::Live(sandbox_id) => {
                    if !state.checkpoint_current {
                        if let Err(error) = self
                            .checkpoint(&key, &sandbox_id, DEFAULT_OUTPUT_LIMIT)
                            .await
                        {
                            tracing::error!(
                                ?error,
                                sandbox_id,
                                "Railway sandbox shutdown left a live sandbox because its final checkpoint failed"
                            );
                            if first_error.is_none() {
                                first_error = Some(error);
                            }
                            continue;
                        }
                        state.checkpoint_current = true;
                    }
                    sandbox_id
                }
                UserSandboxLifecycle::CleanupPending(sandbox_id) => sandbox_id,
            };
            state.lifecycle = UserSandboxLifecycle::CleanupPending(sandbox_id.clone());
            match self
                .destroy_sandbox(&sandbox_id, DEFAULT_OUTPUT_LIMIT)
                .await
            {
                Ok(()) => state.lifecycle = UserSandboxLifecycle::Absent,
                Err(error) => {
                    tracing::error!(
                        ?error,
                        sandbox_id,
                        "Railway sandbox shutdown could not confirm remote cleanup"
                    );
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn execute_cli(
        &self,
        invocation: RailwayCliInvocation,
        deadline: Instant,
    ) -> Result<RailwayCliOutput, RuntimeProcessError> {
        let operation = invocation.operation;
        let result = self
            .cli
            .execute(invocation, deadline_remaining(deadline)?)
            .await;
        if let Err(error) = &result {
            // SystemRailwayCli constructs this error only after redacting CLI
            // stderr and configured credentials. Keep the operation alongside
            // that sanitized cause so provider, preflight, and remote-command
            // failures remain distinguishable in deployment logs.
            tracing::error!(
                operation = operation.label(),
                ?error,
                "Railway preview CLI operation failed"
            );
        }
        result
    }

    async fn live_sandbox_for_file_operation(
        &self,
        key: &RebornSandboxUserKey,
        state: &mut UserSandboxState,
        deadline: Instant,
        output_limit: usize,
    ) -> Result<String, RuntimeProcessError> {
        if let UserSandboxLifecycle::CleanupPending(sandbox_id) = &state.lifecycle {
            let sandbox_id = sandbox_id.clone();
            let prior_error = RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox cleanup was previously unconfirmed".to_string(),
            );
            self.destroy_after_failed_execution(&sandbox_id, output_limit, &prior_error)
                .await?;
            state.lifecycle = UserSandboxLifecycle::Absent;
        }
        if let UserSandboxLifecycle::Live(sandbox_id) = &state.lifecycle {
            let sandbox_id = sandbox_id.clone();
            if !self.sandbox_is_live(&sandbox_id, deadline).await? {
                state.lifecycle = UserSandboxLifecycle::Absent;
            }
        }
        match &state.lifecycle {
            UserSandboxLifecycle::Live(id) => Ok(id.clone()),
            UserSandboxLifecycle::Absent => {
                self.provision(key, state, deadline, output_limit).await
            }
            UserSandboxLifecycle::CleanupPending(_) => Err(RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox cleanup remains pending".to_string(),
            )),
        }
    }

    async fn cleanup_after_file_transport_failure(
        &self,
        state: &mut UserSandboxState,
        sandbox_id: &str,
        output_limit: usize,
        execution_error: &RuntimeProcessError,
    ) {
        state.lifecycle = UserSandboxLifecycle::CleanupPending(sandbox_id.to_string());
        if self
            .destroy_after_failed_execution(sandbox_id, output_limit, execution_error)
            .await
            .is_ok()
        {
            state.lifecycle = UserSandboxLifecycle::Absent;
        }
    }
}

#[async_trait]
impl SandboxCommandTransport for RailwayPreviewSandboxTransport {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        reject_request_environment(&request)?;
        reject_nul("command", &request.command)?;
        let workdir = validated_workdir(request.workdir.as_deref())?;
        let timeout = request_timeout(request.timeout_secs);
        let output_limit = DEFAULT_OUTPUT_LIMIT;
        let started = Instant::now();
        let deadline = started + timeout;
        let key = RebornSandboxUserKey::from_scope(&request.scope);
        let state = self.state_for(key.clone()).await?;

        // Per-user lifecycle gate: first provision, worker bootstrap, and all
        // use of that worker serialize without blocking other tenant/users.
        let mut state = state.lock().await;
        if let UserSandboxLifecycle::CleanupPending(sandbox_id) = &state.lifecycle {
            let sandbox_id = sandbox_id.clone();
            let prior_error = RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox cleanup was previously unconfirmed".to_string(),
            );
            self.destroy_after_failed_execution(&sandbox_id, output_limit, &prior_error)
                .await?;
            state.lifecycle = UserSandboxLifecycle::Absent;
        }
        if let UserSandboxLifecycle::Live(sandbox_id) = &state.lifecycle {
            let sandbox_id = sandbox_id.clone();
            if !self.sandbox_is_live(&sandbox_id, deadline).await? {
                state.lifecycle = UserSandboxLifecycle::Absent;
            }
        }
        let sandbox_id = match &state.lifecycle {
            UserSandboxLifecycle::Live(id) => id.clone(),
            UserSandboxLifecycle::Absent => {
                self.provision(&key, &mut state, deadline, output_limit)
                    .await?
            }
            UserSandboxLifecycle::CleanupPending(_) => {
                return Err(RuntimeProcessError::ExecutionFailed(
                    "Railway preview sandbox cleanup remains pending".to_string(),
                ));
            }
        };
        // The remote command may mutate `/workspace` even when its transport
        // response is lost or malformed. Mark the prior checkpoint stale
        // before dispatch so graceful shutdown cannot destroy those writes
        // without first retrying a checkpoint.
        state.checkpoint_current = false;
        let output = self
            .execute_cli(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::ExecuteSandboxCommand,
                    sandbox_exec_argv(
                        &self.config,
                        &sandbox_id,
                        deadline_remaining(deadline)?,
                        ephemeral_worker_argv(
                            &self.config,
                            &railway_workspace_path(&key),
                            &workdir,
                            &request.command,
                        ),
                    ),
                    output_limit,
                ),
                deadline,
            )
            .await;
        let output = match output {
            Ok(output) => output,
            Err(execution_error) => {
                // Killing a local Railway CLI process does not prove that its
                // remote command stopped. Destroy the whole outer sandbox on
                // any worker-exec transport failure, then force reprovisioning
                // from the last durable checkpoint on the next request.
                state.lifecycle = UserSandboxLifecycle::CleanupPending(sandbox_id.clone());
                self.destroy_after_failed_execution(&sandbox_id, output_limit, &execution_error)
                    .await?;
                state.lifecycle = UserSandboxLifecycle::Absent;
                return Err(execution_error);
            }
        };
        let (stdout, exit_code) = parse_exit_sentinel(output.stdout)?;
        let checkpoint_failed = self
            .checkpoint(&key, &sandbox_id, output_limit)
            .await
            .is_err();
        state.checkpoint_current = !checkpoint_failed;
        let mut raw_output = combine_output(stdout, output.stderr);
        if checkpoint_failed {
            if !raw_output.is_empty() && !raw_output.ends_with('\n') {
                raw_output.push('\n');
            }
            raw_output.push_str(CHECKPOINT_FAILURE_WARNING);
            raw_output.push('\n');
        }
        let output = redact_command_output(raw_output);
        Ok(CommandExecutionOutput {
            output,
            saved_output: None,
            exit_code: i64::from(exit_code),
            sandboxed: true,
            duration: started.elapsed(),
        })
    }

    async fn shutdown(&self) -> Result<(), RuntimeProcessError> {
        self.shutdown_owned_sandboxes().await
    }
}

#[async_trait]
impl SandboxWorkspaceFileTransport for RailwayPreviewSandboxTransport {
    async fn read_file(
        &self,
        request: SandboxWorkspaceFileReadRequest,
    ) -> Result<SandboxWorkspaceFileReadOutput, SandboxWorkspaceFileError> {
        validate_workspace_file_path(&request.path)?;
        if request.max_bytes == 0 || request.max_bytes > MAX_WORKSPACE_FILE_BYTES {
            return Err(SandboxWorkspaceFileError::InvalidLimit);
        }
        let output_limit = workspace_read_output_limit(request.max_bytes);
        let deadline = Instant::now() + MAX_COMMAND_TIMEOUT;
        let key = RebornSandboxUserKey::from_scope(&request.scope);
        let state = self
            .state_for(key.clone())
            .await
            .map_err(|_| SandboxWorkspaceFileError::TransportFailed)?;
        let mut state = state.lock().await;
        let sandbox_id = self
            .live_sandbox_for_file_operation(&key, &mut state, deadline, output_limit)
            .await
            .map_err(|_| SandboxWorkspaceFileError::TransportFailed)?;
        let invocation = RailwayCliInvocation::new(
            self.config.cli_path.clone(),
            RailwayCliOperation::ReadWorkspaceFile,
            sandbox_exec_argv(
                &self.config,
                &sandbox_id,
                deadline_remaining(deadline)
                    .map_err(|_| SandboxWorkspaceFileError::TransportFailed)?,
                workspace_file_read_argv(
                    &self.config,
                    &railway_workspace_path(&key),
                    &request.path,
                    request.max_bytes,
                ),
            ),
            output_limit,
        );
        let output = match self.execute_cli(invocation, deadline).await {
            Ok(output) => output,
            Err(error) => {
                self.cleanup_after_file_transport_failure(
                    &mut state,
                    &sandbox_id,
                    output_limit,
                    &error,
                )
                .await;
                return Err(SandboxWorkspaceFileError::TransportFailed);
            }
        };
        match parse_workspace_file_read_output(&output.stdout, request.max_bytes) {
            Err(error @ SandboxWorkspaceFileError::InvalidResponse) => {
                log_workspace_file_response_rejection(
                    RailwayCliOperation::ReadWorkspaceFile,
                    &output,
                    &error,
                );
                Err(error)
            }
            result => result,
        }
    }

    async fn write_file(
        &self,
        request: SandboxWorkspaceFileWriteRequest,
    ) -> Result<SandboxWorkspaceFileWriteOutput, SandboxWorkspaceFileError> {
        validate_workspace_file_path(&request.path)?;
        if request.bytes.len() > MAX_WORKSPACE_FILE_BYTES {
            return Err(SandboxWorkspaceFileError::TooLarge {
                max_bytes: MAX_WORKSPACE_FILE_BYTES,
            });
        }
        let output_limit = DEFAULT_OUTPUT_LIMIT;
        let deadline = Instant::now() + MAX_COMMAND_TIMEOUT;
        let key = RebornSandboxUserKey::from_scope(&request.scope);
        let state = self
            .state_for(key.clone())
            .await
            .map_err(|_| SandboxWorkspaceFileError::TransportFailed)?;
        let mut state = state.lock().await;
        let sandbox_id = self
            .live_sandbox_for_file_operation(&key, &mut state, deadline, output_limit)
            .await
            .map_err(|_| SandboxWorkspaceFileError::TransportFailed)?;
        state.checkpoint_current = false;
        let invocation = RailwayCliInvocation::new(
            self.config.cli_path.clone(),
            RailwayCliOperation::WriteWorkspaceFile,
            sandbox_exec_argv(
                &self.config,
                &sandbox_id,
                deadline_remaining(deadline)
                    .map_err(|_| SandboxWorkspaceFileError::TransportFailed)?,
                workspace_file_write_argv(
                    &self.config,
                    &railway_workspace_path(&key),
                    &request.path,
                    request.overwrite,
                    request.bytes.len(),
                    &sha256_hex(&request.bytes),
                ),
            ),
            output_limit,
        )
        .with_stdin(request.bytes.clone());
        let output = match self.execute_cli(invocation, deadline).await {
            Ok(output) => output,
            Err(error) => {
                self.cleanup_after_file_transport_failure(
                    &mut state,
                    &sandbox_id,
                    output_limit,
                    &error,
                )
                .await;
                return Err(SandboxWorkspaceFileError::TransportFailed);
            }
        };
        let output = match parse_workspace_file_write_output(&output.stdout, &request.bytes) {
            Ok(output) => output,
            Err(SandboxWorkspaceFileError::InvalidResponse) => {
                let error = RuntimeProcessError::ExecutionFailed(
                    "Railway workspace write returned an ambiguous verification response"
                        .to_string(),
                );
                log_workspace_file_response_rejection(
                    RailwayCliOperation::WriteWorkspaceFile,
                    &output,
                    &SandboxWorkspaceFileError::InvalidResponse,
                );
                self.cleanup_after_file_transport_failure(
                    &mut state,
                    &sandbox_id,
                    output_limit,
                    &error,
                )
                .await;
                return Err(SandboxWorkspaceFileError::InvalidResponse);
            }
            Err(error) => return Err(error),
        };
        if self
            .checkpoint(&key, &sandbox_id, output_limit)
            .await
            .is_err()
        {
            return Err(SandboxWorkspaceFileError::CheckpointFailed);
        }
        state.checkpoint_current = true;
        Ok(output)
    }
}

fn log_workspace_file_response_rejection(
    operation: RailwayCliOperation,
    output: &RailwayCliOutput,
    error: &SandboxWorkspaceFileError,
) {
    // Do not log response content: the file bridge can carry arbitrary user
    // data. Byte counts plus the typed parse failure are enough to distinguish
    // a malformed helper response from a provider or remote-command failure.
    tracing::error!(
        operation = operation.label(),
        stdout_bytes = output.stdout.len(),
        stderr_bytes = output.stderr.len(),
        ?error,
        "Railway workspace file response was rejected"
    );
}

#[cfg(test)]
mod constructor_test_support {
    use super::*;

    impl RailwayPreviewSandboxTransport {
        pub(super) fn with_cli(
            config: RailwayPreviewSandboxConfig,
            cli: Arc<dyn RailwayCli>,
        ) -> Self {
            Self::with_cli_and_capacity(config, cli, MAX_TRACKED_USERS)
        }

        pub(super) fn with_cli_and_capacity(
            config: RailwayPreviewSandboxConfig,
            cli: Arc<dyn RailwayCli>,
            max_tracked_users: usize,
        ) -> Self {
            Self {
                config,
                cli,
                users: Arc::new(Mutex::new(HashMap::new())),
                max_tracked_users,
            }
        }
    }
}

#[derive(Debug)]
struct TrackedUserState {
    state: Arc<Mutex<UserSandboxState>>,
    last_touched: Instant,
}

#[derive(Debug, Default)]
struct UserSandboxState {
    lifecycle: UserSandboxLifecycle,
    checkpoint_current: bool,
}

#[derive(Debug, Clone, Default)]
enum UserSandboxLifecycle {
    #[default]
    Absent,
    Live(String),
    CleanupPending(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RailwayCliInvocation {
    program: PathBuf,
    operation: RailwayCliOperation,
    args: Vec<String>,
    stdin: Option<Vec<u8>>,
    output_limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RailwayCliOperation {
    ListCheckpoints,
    ListSandboxes,
    CreateSandbox,
    ExecuteSandboxCommand,
    ReadWorkspaceFile,
    WriteWorkspaceFile,
    CreateCheckpoint,
    DestroySandbox,
}

impl RailwayCliOperation {
    fn label(self) -> &'static str {
        match self {
            Self::ListCheckpoints => "list checkpoints",
            Self::ListSandboxes => "list sandboxes",
            Self::CreateSandbox => "create sandbox",
            Self::ExecuteSandboxCommand => "execute sandbox command",
            Self::ReadWorkspaceFile => "read sandbox workspace file",
            Self::WriteWorkspaceFile => "write sandbox workspace file",
            Self::CreateCheckpoint => "create checkpoint",
            Self::DestroySandbox => "destroy sandbox",
        }
    }
}

impl RailwayCliInvocation {
    fn new(
        program: PathBuf,
        operation: RailwayCliOperation,
        args: Vec<String>,
        output_limit: usize,
    ) -> Self {
        // Leave enough headroom for the fixed remote exit sentinel even when
        // the model command emits exactly its requested output cap.
        Self {
            program,
            operation,
            args,
            stdin: None,
            output_limit: output_limit.saturating_add(EXIT_SENTINEL.len() + 32),
        }
    }

    fn with_stdin(mut self, stdin: Vec<u8>) -> Self {
        self.stdin = Some(stdin);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RailwayCliOutput {
    stdout: String,
    stderr: String,
}

#[async_trait]
trait RailwayCli: Send + Sync {
    async fn execute(
        &self,
        invocation: RailwayCliInvocation,
        timeout: Duration,
    ) -> Result<RailwayCliOutput, RuntimeProcessError>;
}

#[derive(Debug)]
struct SystemRailwayCli;

#[derive(Debug)]
struct PrivateRailwayCliHome {
    path: Option<PathBuf>,
}

impl PrivateRailwayCliHome {
    async fn create() -> Result<Self, RuntimeProcessError> {
        tokio::task::spawn_blocking(Self::create_blocking)
            .await
            .map_err(|error| {
                tracing::error!(?error, "Railway CLI private state setup task failed");
                RuntimeProcessError::ExecutionFailed(
                    "Railway preview could not create private CLI state".to_string(),
                )
            })?
    }

    fn create_blocking() -> Result<Self, RuntimeProcessError> {
        let path =
            std::env::temp_dir().join(format!("ironclaw-railway-cli-{}", uuid::Uuid::new_v4()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};

            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700).create(&path).map_err(|error| {
                tracing::error!(
                    ?error,
                    "Railway CLI private state directory creation failed"
                );
                RuntimeProcessError::ExecutionFailed(
                    "Railway preview could not create private CLI state".to_string(),
                )
            })?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    tracing::error!(?error, "Railway CLI private state permission update failed");
                    let _ = std::fs::remove_dir_all(&path);
                    RuntimeProcessError::ExecutionFailed(
                        "Railway preview could not secure private CLI state".to_string(),
                    )
                },
            )?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir(&path).map_err(|error| {
            tracing::error!(
                ?error,
                "Railway CLI private state directory creation failed"
            );
            RuntimeProcessError::ExecutionFailed(
                "Railway preview could not create private CLI state".to_string(),
            )
        })?;
        Ok(Self { path: Some(path) })
    }

    fn path(&self) -> Result<&Path, RuntimeProcessError> {
        self.path.as_deref().ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "Railway preview private CLI state is unavailable".to_string(),
            )
        })
    }

    async fn cleanup(mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        match tokio::task::spawn_blocking(move || std::fs::remove_dir_all(path)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::debug!(
                ?error,
                "best-effort Railway CLI private state cleanup failed"
            ),
            Err(error) => tracing::debug!(
                ?error,
                "best-effort Railway CLI private state cleanup task failed"
            ),
        }
    }
}

impl Drop for PrivateRailwayCliHome {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if let Err(error) = std::fs::remove_dir_all(path) {
            tracing::debug!(
                ?error,
                "best-effort Railway CLI private state cleanup failed"
            );
        }
    }
}

#[async_trait]
impl RailwayCli for SystemRailwayCli {
    async fn execute(
        &self,
        invocation: RailwayCliInvocation,
        timeout: Duration,
    ) -> Result<RailwayCliOutput, RuntimeProcessError> {
        let environment = railway_cli_environment()?;
        let cli_home = PrivateRailwayCliHome::create().await?;
        let operation = invocation.operation;
        let mut command = Command::new(&invocation.program);
        command
            .args(&invocation.args)
            .env_clear()
            .envs(&environment)
            .env("HOME", cli_home.path()?)
            .stdin(if invocation.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                tracing::error!(?error, "trusted Railway CLI process could not be started");
                cli_home.cleanup().await;
                return Err(RuntimeProcessError::ExecutionFailed(
                    "Railway preview could not start the trusted Railway CLI".to_string(),
                ));
            }
        };
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let result = tokio::time::timeout(timeout, async {
            let stdin = async {
                match (stdin, invocation.stdin) {
                    (Some(mut stream), Some(bytes)) => {
                        stream.write_all(&bytes).await.map_err(|error| {
                            tracing::error!(?error, "trusted Railway CLI stdin write failed");
                            RuntimeProcessError::ExecutionFailed(
                                "Railway preview could not write trusted CLI input".to_string(),
                            )
                        })?;
                        stream.shutdown().await.map_err(|error| {
                            tracing::error!(?error, "trusted Railway CLI stdin close failed");
                            RuntimeProcessError::ExecutionFailed(
                                "Railway preview could not close trusted CLI input".to_string(),
                            )
                        })
                    }
                    (None, None) => Ok(()),
                    _ => Err(RuntimeProcessError::ExecutionFailed(
                        "Railway preview CLI input pipe was unavailable".to_string(),
                    )),
                }
            };
            let stdout = async {
                match stdout {
                    Some(stream) => read_stream_bounded(stream, invocation.output_limit).await,
                    None => Ok(String::new()),
                }
            };
            let stderr = async {
                match stderr {
                    Some(stream) => read_stream_bounded(stream, invocation.output_limit).await,
                    None => Ok(String::new()),
                }
            };
            let (stdin, stdout, stderr, status) = tokio::join!(stdin, stdout, stderr, child.wait());
            let status = status.map_err(|error| {
                tracing::error!(?error, "trusted Railway CLI process wait failed");
                RuntimeProcessError::ExecutionFailed(
                    "Railway preview CLI command did not complete".to_string(),
                )
            })?;
            stdin?;
            let stdout = stdout?;
            let stderr = stderr?;
            if !status.success() {
                return Err(railway_cli_status_error(
                    operation,
                    status.code(),
                    &stderr,
                    &environment,
                    timeout,
                ));
            }
            Ok(RailwayCliOutput { stdout, stderr })
        })
        .await;
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                if let Err(error) = child.kill().await {
                    tracing::debug!(?error, "best-effort Railway CLI termination failed");
                }
                if let Err(error) = child.wait().await {
                    tracing::debug!(?error, "best-effort Railway CLI reap failed");
                }
                Err(RuntimeProcessError::Timeout(timeout))
            }
        };
        cli_home.cleanup().await;
        result
    }
}

fn railway_cli_status_error(
    operation: RailwayCliOperation,
    exit_code: Option<i32>,
    stderr: &str,
    environment: &BTreeMap<String, String>,
    timeout: Duration,
) -> RuntimeProcessError {
    if matches!(
        operation,
        RailwayCliOperation::ExecuteSandboxCommand
            | RailwayCliOperation::ReadWorkspaceFile
            | RailwayCliOperation::WriteWorkspaceFile
    ) && exit_code == Some(124)
    {
        return RuntimeProcessError::Timeout(timeout);
    }
    RuntimeProcessError::ExecutionFailed(railway_cli_failure_message(
        operation,
        exit_code,
        stderr,
        environment,
    ))
}

fn railway_cli_failure_message(
    operation: RailwayCliOperation,
    exit_code: Option<i32>,
    stderr: &str,
    environment: &BTreeMap<String, String>,
) -> String {
    let mut detail = stderr.to_string();
    for name in ["RAILWAY_TOKEN", "RAILWAY_API_TOKEN"] {
        if let Some(token) = environment.get(name).filter(|token| !token.is_empty()) {
            detail = detail.replace(token, "[REDACTED]");
        }
    }
    let (detail, _) = ironclaw_safety::LeakDetector::new().redact_all_secrets(&detail);
    let exit = exit_code
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| "terminated by signal".to_string());
    let detail = detail.trim();
    if detail.is_empty() {
        format!("Railway preview failed to {} ({exit})", operation.label())
    } else {
        format!(
            "Railway preview failed to {} ({exit}): {detail}",
            operation.label()
        )
    }
}

fn required_config_value(label: &str, value: String) -> Result<String, RuntimeProcessError> {
    if value.trim().is_empty() || value.as_bytes().contains(&0) {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "Railway preview configuration requires a valid {label}"
        )));
    }
    Ok(value)
}

fn checkpoint_list_argv(config: &RailwayPreviewSandboxConfig) -> Vec<String> {
    vec![
        "sandbox".into(),
        "checkpoint".into(),
        "list".into(),
        "--json".into(),
        "--project".into(),
        config.project_id.clone(),
        "--environment".into(),
        config.environment_id.clone(),
    ]
}

fn sandbox_list_argv(config: &RailwayPreviewSandboxConfig) -> Vec<String> {
    vec![
        "sandbox".into(),
        "list".into(),
        "--json".into(),
        "--project".into(),
        config.project_id.clone(),
        "--environment".into(),
        config.environment_id.clone(),
    ]
}

fn sandbox_create_argv(
    config: &RailwayPreviewSandboxConfig,
    checkpoint: Option<&str>,
) -> Vec<String> {
    let mut argv = vec![
        "sandbox".into(),
        "create".into(),
        "--json".into(),
        "--project".into(),
        config.project_id.clone(),
        "--environment".into(),
        config.environment_id.clone(),
        "--idle-timeout-minutes".into(),
        config.idle_timeout_minutes.to_string(),
    ];
    if let Some(checkpoint) = checkpoint {
        argv.push("--checkpoint".into());
        argv.push(checkpoint.into());
    }
    argv
}

fn checkpoint_create_argv(
    config: &RailwayPreviewSandboxConfig,
    sandbox_id: &str,
    checkpoint_name: &str,
) -> Vec<String> {
    vec![
        "sandbox".into(),
        "checkpoint".into(),
        "create".into(),
        checkpoint_name.into(),
        "--id".into(),
        sandbox_id.into(),
        "--json".into(),
        "--project".into(),
        config.project_id.clone(),
        "--environment".into(),
        config.environment_id.clone(),
    ]
}

fn sandbox_destroy_argv(config: &RailwayPreviewSandboxConfig, sandbox_id: &str) -> Vec<String> {
    vec![
        "sandbox".into(),
        "destroy".into(),
        "--id".into(),
        sandbox_id.into(),
        "--project".into(),
        config.project_id.clone(),
        "--environment".into(),
        config.environment_id.clone(),
    ]
}

fn sandbox_exec_argv(
    config: &RailwayPreviewSandboxConfig,
    sandbox_id: &str,
    timeout: Duration,
    command: Vec<String>,
) -> Vec<String> {
    let mut args = vec![
        "sandbox".into(),
        "exec".into(),
        "--project".into(),
        config.project_id.clone(),
        "--environment".into(),
        config.environment_id.clone(),
        "--id".into(),
        sandbox_id.into(),
        "--timeout".into(),
        timeout.as_secs().max(1).to_string(),
        "--".into(),
    ];
    args.extend(command);
    args
}

fn workspace_bootstrap_argv(workspace: &str) -> Vec<String> {
    vec![
        "sh".into(),
        "-c".into(),
        format!("mkdir -p \"$1\" && chown {WORKER_USER} \"$1\""),
        WORKSPACE_BOOTSTRAP_MARKER.into(),
        workspace.into(),
    ]
}

fn workspace_file_write_argv(
    config: &RailwayPreviewSandboxConfig,
    railway_workspace: &str,
    path: &str,
    overwrite: bool,
    expected_len: usize,
    expected_sha256: &str,
) -> Vec<String> {
    workspace_file_helper_argv(
        config,
        railway_workspace,
        true,
        WORKSPACE_FILE_WRITE_HELPER,
        vec![
            WORKSPACE_ROOT.into(),
            path.into(),
            if overwrite { "1" } else { "0" }.into(),
            MAX_WORKSPACE_FILE_BYTES.to_string(),
            expected_len.to_string(),
            expected_sha256.into(),
            WORKSPACE_WORKER_UID.to_string(),
            WORKSPACE_WORKER_GID.to_string(),
        ],
    )
}

fn workspace_file_read_argv(
    config: &RailwayPreviewSandboxConfig,
    railway_workspace: &str,
    path: &str,
    max_bytes: usize,
) -> Vec<String> {
    workspace_file_helper_argv(
        config,
        railway_workspace,
        false,
        WORKSPACE_FILE_READ_HELPER,
        vec![WORKSPACE_ROOT.into(), path.into(), max_bytes.to_string()],
    )
}

fn workspace_file_helper_argv(
    config: &RailwayPreviewSandboxConfig,
    railway_workspace: &str,
    attach_stdin: bool,
    script: &str,
    protocol: Vec<String>,
) -> Vec<String> {
    let mut command = vec!["docker".into(), "run".into(), "--rm".into()];
    if attach_stdin {
        command.push("-i".into());
    }
    command.extend(
        super::worker_spec::DockerWorkerSecuritySpec::new(Some("none".to_string()))
            .trusted_root_helper_docker_run_args(),
    );
    command.extend([
        "--mount".into(),
        format!(
            "type=bind,src={railway_workspace},dst={WORKSPACE_ROOT}{}",
            if attach_stdin { "" } else { ",readonly" }
        ),
        "--memory".into(),
        "128m".into(),
        config.worker_image.clone(),
        "python3".into(),
        "-c".into(),
        script.into(),
    ]);
    command.extend(protocol);
    command
}

fn ephemeral_worker_argv(
    config: &RailwayPreviewSandboxConfig,
    railway_workspace: &str,
    workdir: &str,
    model_command: &str,
) -> Vec<String> {
    let mut command = vec!["docker".into(), "run".into(), "--rm".into()];
    command.extend(
        super::worker_spec::DockerWorkerSecuritySpec::new(config.container_network_mode())
            .docker_run_args(),
    );
    command.extend([
        "--env".into(),
        config.network_mode_env(),
        "--mount".into(),
        format!("type=bind,src={railway_workspace},dst={WORKSPACE_ROOT}"),
        "--workdir".into(),
        workdir.into(),
        "--memory".into(),
        "512m".into(),
        config.worker_image.clone(),
        "sh".into(),
        "-c".into(),
        model_command.into(),
    ]);
    let mut args = vec![
        "sh".into(),
        "-c".into(),
        OUTER_EXEC_WRAPPER.into(),
        "ironclaw-railway-wrapper".into(),
    ];
    args.extend(command);
    args
}

fn railway_workspace_path(key: &RebornSandboxUserKey) -> String {
    format!("{RAILWAY_WORKSPACES_ROOT}/{}", key.container_name())
}

fn checkpoint_name(key: &RebornSandboxUserKey) -> String {
    format!("{}-checkpoint", key.container_name())
}

fn request_timeout(requested: Option<u64>) -> Duration {
    requested
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_COMMAND_TIMEOUT)
        .min(MAX_COMMAND_TIMEOUT)
}

fn deadline_remaining(deadline: Instant) -> Result<Duration, RuntimeProcessError> {
    deadline
        .checked_duration_since(Instant::now())
        .ok_or(RuntimeProcessError::Timeout(Duration::ZERO))
}

fn reject_request_environment(
    request: &CommandExecutionRequest,
) -> Result<(), RuntimeProcessError> {
    // Mount views are host-side authorization metadata. This remote-only
    // preview deliberately materializes none of them; ignoring the metadata
    // grants the worker less access, while accepting the `Some(view)` shape
    // used by the production shell capability.
    if !request.extra_env.is_empty() {
        return Err(RuntimeProcessError::ExecutionFailed(
            "Railway preview sandbox does not support request environment variables".into(),
        ));
    }
    Ok(())
}

fn validated_workdir(workdir: Option<&str>) -> Result<String, RuntimeProcessError> {
    let Some(workdir) = workdir else {
        return Ok(WORKSPACE_ROOT.into());
    };
    reject_nul("working directory", workdir)?;
    if workdir == WORKSPACE_ROOT {
        return Ok(WORKSPACE_ROOT.into());
    }
    let Some(relative) = workdir.strip_prefix("/workspace/") else {
        return Err(RuntimeProcessError::ExecutionFailed(
            "Railway preview sandbox working directory must be /workspace or a descendant".into(),
        ));
    };
    for component in Path::new(relative).components() {
        if !matches!(component, Component::Normal(_) | Component::CurDir) {
            return Err(RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox working directory must stay under /workspace".into(),
            ));
        }
    }
    Ok(workdir.into())
}

fn validate_workspace_file_path(path: &str) -> Result<(), SandboxWorkspaceFileError> {
    if path.len() > MAX_WORKSPACE_FILE_PATH_BYTES || path.as_bytes().contains(&0) {
        return Err(SandboxWorkspaceFileError::InvalidPath);
    }
    let Some(relative) = path.strip_prefix("/workspace/") else {
        return Err(SandboxWorkspaceFileError::InvalidPath);
    };
    if relative.is_empty()
        || relative.split('/').any(|component| {
            component.is_empty()
                || component.len() > MAX_WORKSPACE_FILE_COMPONENT_BYTES
                || matches!(component, "." | "..")
        })
        || Path::new(relative)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(SandboxWorkspaceFileError::InvalidPath);
    }
    Ok(())
}

fn workspace_read_output_limit(max_bytes: usize) -> usize {
    max_bytes
        .saturating_add(2)
        .checked_div(3)
        .unwrap_or(usize::MAX)
        .saturating_mul(4)
        .saturating_add(WORKSPACE_FILE_JSON_OVERHEAD)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn workspace_file_error_from_code(code: &str, max_bytes: usize) -> SandboxWorkspaceFileError {
    match code {
        "invalid_path" => SandboxWorkspaceFileError::InvalidPath,
        "too_large" => SandboxWorkspaceFileError::TooLarge { max_bytes },
        "not_found" => SandboxWorkspaceFileError::NotFound,
        "not_regular_file" => SandboxWorkspaceFileError::NotRegularFile,
        "conflict" => SandboxWorkspaceFileError::Conflict,
        "verification_failed" => SandboxWorkspaceFileError::InvalidResponse,
        _ => SandboxWorkspaceFileError::InvalidResponse,
    }
}

fn parse_workspace_file_write_output(
    stdout: &str,
    expected_bytes: &[u8],
) -> Result<SandboxWorkspaceFileWriteOutput, SandboxWorkspaceFileError> {
    let value: serde_json::Value =
        serde_json::from_str(stdout).map_err(|_| SandboxWorkspaceFileError::InvalidResponse)?;
    if value.get("status").and_then(serde_json::Value::as_str) == Some("error") {
        let code = value
            .get("code")
            .and_then(serde_json::Value::as_str)
            .ok_or(SandboxWorkspaceFileError::InvalidResponse)?;
        return Err(workspace_file_error_from_code(
            code,
            MAX_WORKSPACE_FILE_BYTES,
        ));
    }
    if value.get("status").and_then(serde_json::Value::as_str) != Some("ok") {
        return Err(SandboxWorkspaceFileError::InvalidResponse);
    }
    let bytes_written = value
        .get("bytes_written")
        .and_then(serde_json::Value::as_u64)
        .and_then(|size| usize::try_from(size).ok())
        .ok_or(SandboxWorkspaceFileError::InvalidResponse)?;
    let sha256 = value
        .get("sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or(SandboxWorkspaceFileError::InvalidResponse)?;
    let already_present = value
        .get("already_present")
        .and_then(serde_json::Value::as_bool)
        .ok_or(SandboxWorkspaceFileError::InvalidResponse)?;
    let expected_sha256 = sha256_hex(expected_bytes);
    if bytes_written != expected_bytes.len() || sha256 != expected_sha256 {
        return Err(SandboxWorkspaceFileError::InvalidResponse);
    }
    Ok(SandboxWorkspaceFileWriteOutput {
        bytes_written,
        sha256: expected_sha256,
        already_present,
    })
}

fn parse_workspace_file_read_output(
    stdout: &str,
    max_bytes: usize,
) -> Result<SandboxWorkspaceFileReadOutput, SandboxWorkspaceFileError> {
    let value: serde_json::Value =
        serde_json::from_str(stdout).map_err(|_| SandboxWorkspaceFileError::InvalidResponse)?;
    if value.get("status").and_then(serde_json::Value::as_str) == Some("error") {
        let code = value
            .get("code")
            .and_then(serde_json::Value::as_str)
            .ok_or(SandboxWorkspaceFileError::InvalidResponse)?;
        return Err(workspace_file_error_from_code(code, max_bytes));
    }
    if value.get("status").and_then(serde_json::Value::as_str) != Some("ok") {
        return Err(SandboxWorkspaceFileError::InvalidResponse);
    }
    let encoded = value
        .get("bytes")
        .and_then(serde_json::Value::as_str)
        .ok_or(SandboxWorkspaceFileError::InvalidResponse)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| SandboxWorkspaceFileError::InvalidResponse)?;
    if bytes.len() > max_bytes {
        return Err(SandboxWorkspaceFileError::TooLarge { max_bytes });
    }
    let sha256 = value
        .get("sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or(SandboxWorkspaceFileError::InvalidResponse)?;
    let expected_sha256 = sha256_hex(&bytes);
    if sha256 != expected_sha256 {
        return Err(SandboxWorkspaceFileError::InvalidResponse);
    }
    Ok(SandboxWorkspaceFileReadOutput {
        bytes,
        sha256: expected_sha256,
    })
}

fn reject_nul(label: &str, value: &str) -> Result<(), RuntimeProcessError> {
    if value.as_bytes().contains(&0) {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "Railway preview sandbox {label} contains null bytes"
        )));
    }
    Ok(())
}

fn parse_sandbox_id(stdout: &str) -> Result<String, RuntimeProcessError> {
    let value: serde_json::Value = serde_json::from_str(stdout).map_err(|error| {
        tracing::error!(
            ?error,
            "Railway preview sandbox creation returned invalid JSON"
        );
        RuntimeProcessError::ExecutionFailed(
            "Railway preview sandbox creation returned an invalid response".into(),
        )
    })?;
    let id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            value
                .get("sandbox")
                .and_then(|sandbox| sandbox.get("id"))
                .and_then(serde_json::Value::as_str)
        })
        .filter(|id| !id.trim().is_empty() && !id.as_bytes().contains(&0))
        .ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox creation returned no sandbox identifier".into(),
            )
        })?;
    Ok(id.into())
}

fn checkpoint_exists(stdout: &str, checkpoint_name: &str) -> Result<bool, RuntimeProcessError> {
    let value: serde_json::Value = serde_json::from_str(stdout).map_err(|error| {
        tracing::error!(
            ?error,
            "Railway preview checkpoint listing returned invalid JSON"
        );
        RuntimeProcessError::ExecutionFailed(
            "Railway preview checkpoint listing returned an invalid response".into(),
        )
    })?;
    let checkpoints = value
        .as_array()
        .or_else(|| {
            value
                .get("checkpoints")
                .and_then(serde_json::Value::as_array)
        })
        .ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "Railway preview checkpoint listing returned an invalid response".into(),
            )
        })?;
    Ok(checkpoints.iter().any(|checkpoint| {
        checkpoint
            .get("key")
            .or_else(|| checkpoint.get("name"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|name| name == checkpoint_name)
    }))
}

fn sandbox_exists(stdout: &str, sandbox_id: &str) -> Result<bool, RuntimeProcessError> {
    let value: serde_json::Value = serde_json::from_str(stdout).map_err(|error| {
        tracing::error!(
            ?error,
            "Railway preview sandbox listing returned invalid JSON"
        );
        RuntimeProcessError::ExecutionFailed(
            "Railway preview sandbox listing returned an invalid response".into(),
        )
    })?;
    let sandboxes = value
        .as_array()
        .or_else(|| value.get("sandboxes").and_then(serde_json::Value::as_array))
        .ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox listing returned an invalid response".into(),
            )
        })?;
    Ok(sandboxes.iter().any(|sandbox| {
        sandbox
            .get("id")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                sandbox
                    .get("sandbox")
                    .and_then(|sandbox| sandbox.get("id"))
                    .and_then(serde_json::Value::as_str)
            })
            .is_some_and(|id| id == sandbox_id)
    }))
}

fn parse_exit_sentinel(stdout: String) -> Result<(String, i32), RuntimeProcessError> {
    let Some(marker_start) = stdout.rfind(EXIT_SENTINEL) else {
        return Err(RuntimeProcessError::ExecutionFailed(
            "Railway preview command completed without a verified exit status".into(),
        ));
    };
    let status_start = marker_start + EXIT_SENTINEL.len();
    let status_end = stdout[status_start..]
        .find('\n')
        .map(|offset| status_start + offset)
        .unwrap_or(stdout.len());
    let code = stdout[status_start..status_end]
        .parse::<i32>()
        .map_err(|error| {
            tracing::error!(
                ?error,
                "Railway preview command returned invalid exit status"
            );
            RuntimeProcessError::ExecutionFailed(
                "Railway preview command returned an invalid exit status".into(),
            )
        })?;
    let line_start = stdout[..marker_start] // safety: `str::rfind` returns a UTF-8 boundary.
        .rfind('\n')
        .map(|offset| offset + 1)
        .unwrap_or(marker_start);
    let line_end = if status_end < stdout.len() {
        status_end + 1
    } else {
        status_end
    };
    let mut output = stdout;
    output.replace_range(line_start..line_end, "");
    Ok((output, code))
}

fn combine_output(stdout: String, stderr: String) -> String {
    if stdout.is_empty() || stderr.is_empty() {
        format!("{stdout}{stderr}")
    } else {
        format!("{stdout}\n\n--- stderr ---\n{stderr}")
    }
}

fn redact_command_output(output: String) -> String {
    match ironclaw_safety::LeakDetector::new().scan_and_clean(&output) {
        Ok(redacted) => redacted,
        Err(error) => {
            tracing::error!(
                ?error,
                "Railway preview command output failed secret scanning"
            );
            "Railway preview command output was blocked because it may contain secret material."
                .to_string()
        }
    }
}

/// This cannot use `process_output::read_stream_capped`: sentinel parsing
/// needs raw stdout before model-output sanitization. It remains bounded and
/// drains beyond the cap so verbose CLI output cannot block the child.
async fn read_stream_bounded<R>(mut stream: R, limit: usize) -> Result<String, RuntimeProcessError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut captured = Vec::with_capacity(limit.min(16 * 1024));
    let head_limit = limit / 2;
    let tail_limit = limit.saturating_sub(head_limit);
    let mut tail = Vec::with_capacity(tail_limit.min(16 * 1024));
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = stream.read(&mut buffer).await.map_err(|error| {
            tracing::error!(?error, "trusted Railway CLI output read failed");
            RuntimeProcessError::ExecutionFailed(
                "Railway preview could not read trusted CLI output".into(),
            )
        })?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read]; // safety: `read` is at most the byte-buffer length.
        if !truncated && captured.len().saturating_add(read) <= limit {
            captured.extend_from_slice(chunk);
            continue;
        }
        if !truncated {
            truncated = true;
            if captured.len() > head_limit {
                tail.extend_from_slice(&captured[head_limit..]);
                captured.truncate(head_limit);
            }
        }
        tail.extend_from_slice(chunk);
        if tail.len() > tail_limit {
            tail.drain(..tail.len() - tail_limit);
        }
    }
    if !truncated {
        return Ok(String::from_utf8_lossy(&captured).into_owned());
    }
    Ok(format!(
        "{}\n\n... [trusted CLI output truncated] ...\n\n{}",
        String::from_utf8_lossy(&captured),
        String::from_utf8_lossy(&tail)
    ))
}

fn railway_cli_environment() -> Result<BTreeMap<String, String>, RuntimeProcessError> {
    railway_cli_environment_from(std::env::vars())
}

fn railway_cli_environment_from<I>(
    variables: I,
) -> Result<BTreeMap<String, String>, RuntimeProcessError>
where
    I: IntoIterator<Item = (String, String)>,
{
    let variables: BTreeMap<_, _> = variables.into_iter().collect();
    let mut allowed = BTreeMap::new();
    for name in [
        "PATH",
        "TMPDIR",
        "TMP",
        "TEMP",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "CURL_CA_BUNDLE",
        "REQUESTS_CA_BUNDLE",
        "NODE_EXTRA_CA_CERTS",
    ] {
        if let Some(value) = variables.get(name).filter(|value| !value.is_empty()) {
            allowed.insert(name.into(), value.clone());
        }
    }
    let project_token = variables
        .get("RAILWAY_TOKEN")
        .filter(|value| !value.is_empty());
    let api_token = variables
        .get("RAILWAY_API_TOKEN")
        .filter(|value| !value.is_empty());
    let auth = match (project_token, api_token) {
        (Some(value), None) => ("RAILWAY_TOKEN", value),
        (None, Some(value)) => ("RAILWAY_API_TOKEN", value),
        (None, None) => {
            return Err(RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox requires Railway CLI authentication".into(),
            ));
        }
        (Some(_), Some(_)) => {
            return Err(RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox requires exactly one Railway token variable".into(),
            ));
        }
    };
    allowed.insert(auth.0.into(), auth.1.clone());
    Ok(allowed)
}

#[cfg(test)]
mod tests;
