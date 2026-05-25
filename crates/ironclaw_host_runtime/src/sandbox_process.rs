//! Reborn-native tenant sandbox command transport.
//!
//! The transport derives host workspace and container identity from the full
//! [`ResourceScope`]. It deliberately avoids the legacy project-only sandbox
//! lifecycle so hosted tenants with matching user/project strings cannot share
//! command state.

use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use bollard::{
    Docker,
    container::{
        Config, CreateContainerOptions, LogOutput, LogsOptions, RemoveContainerOptions,
        StartContainerOptions, WaitContainerOptions,
    },
    models::HostConfig,
};
use futures_util::StreamExt;
use ironclaw_host_api::ResourceScope;

use crate::{
    CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError, SandboxCommandTransport,
    TenantSandboxProcessPort,
};

mod container_identity;
mod scope_key;

pub use container_identity::{RebornSandboxContainerIdentity, RebornSandboxWorkspaceMode};
pub use scope_key::RebornSandboxScopeKey;

const DEFAULT_IMAGE: &str = "ironclaw-worker:latest";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DEFAULT_CPU_SHARES: u32 = 1024;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;
const CONTAINER_WORKSPACE_ROOT: &str = "/workspace";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContainerWorkdir(String);

impl ContainerWorkdir {
    fn workspace_root() -> Self {
        Self(CONTAINER_WORKSPACE_ROOT.to_string())
    }

    fn from_relative(relative: impl AsRef<Path>) -> Result<Self, RuntimeProcessError> {
        let relative = relative.as_ref().to_string_lossy();
        if relative.is_empty() || relative == "." {
            return Ok(Self::workspace_root());
        }
        Ok(Self(format!(
            "{CONTAINER_WORKSPACE_ROOT}/{}",
            relative.trim_start_matches('/')
        )))
    }

    fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone)]
pub struct RebornSandboxConfig {
    workspace_root: PathBuf,
    image: String,
    default_timeout: Duration,
    memory_bytes: u64,
    cpu_shares: u32,
    max_output_bytes: usize,
    disable_network: bool,
    container_identity: RebornSandboxContainerIdentity,
}

impl RebornSandboxConfig {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            image: std::env::var("IRONCLAW_REBORN_SANDBOX_IMAGE")
                .or_else(|_| std::env::var("IRONCLAW_SANDBOX_IMAGE"))
                .unwrap_or_else(|_| DEFAULT_IMAGE.to_string()),
            default_timeout: DEFAULT_TIMEOUT,
            memory_bytes: DEFAULT_MEMORY_BYTES,
            cpu_shares: DEFAULT_CPU_SHARES,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            disable_network: true,
            container_identity: RebornSandboxContainerIdentity::image_default(),
        }
    }

    pub fn with_image(mut self, image: impl Into<String>) -> Self {
        self.image = image.into();
        self
    }

    pub fn with_default_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    pub fn with_network_enabled(mut self) -> Self {
        self.disable_network = false;
        self
    }

    pub fn with_container_identity(mut self, identity: RebornSandboxContainerIdentity) -> Self {
        self.container_identity = identity;
        self
    }

    pub fn with_container_user(
        mut self,
        user: impl Into<String>,
        workspace_mode: RebornSandboxWorkspaceMode,
    ) -> Self {
        self.container_identity =
            RebornSandboxContainerIdentity::configured_user(user, workspace_mode);
        self
    }
}

#[derive(Clone)]
pub struct RebornScopedSandboxCommandTransport {
    docker: Docker,
    config: RebornSandboxConfig,
}

impl std::fmt::Debug for RebornScopedSandboxCommandTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornScopedSandboxCommandTransport")
            .field("workspace_root", &self.config.workspace_root)
            .field("image", &self.config.image)
            .field("disable_network", &self.config.disable_network)
            .field("container_identity", &self.config.container_identity)
            .finish_non_exhaustive()
    }
}

impl RebornScopedSandboxCommandTransport {
    pub async fn connect(config: RebornSandboxConfig) -> Result<Self, RuntimeProcessError> {
        let docker = connect_docker().await?;
        Ok(Self::new(docker, config))
    }

    pub fn new(docker: Docker, config: RebornSandboxConfig) -> Self {
        Self { docker, config }
    }

    pub fn into_process_port(self) -> TenantSandboxProcessPort {
        TenantSandboxProcessPort::new(Arc::new(self))
    }

    fn prepare_workspace(&self, scope: &ResourceScope) -> Result<PathBuf, RuntimeProcessError> {
        let key = RebornSandboxScopeKey::from_scope(scope);
        let workspace = key.workspace_path(&self.config.workspace_root);
        std::fs::create_dir_all(&workspace).map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox workspace could not be initialized: {error}"
            ))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &workspace,
                std::fs::Permissions::from_mode(self.config.container_identity.workspace_mode()),
            )
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox workspace permissions could not be set: {error}"
                ))
            })?;
        }
        workspace.canonicalize().map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox workspace could not be resolved: {error}"
            ))
        })
    }

    fn resolve_container_workdir(
        workspace: &Path,
        workdir: Option<&str>,
    ) -> Result<ContainerWorkdir, RuntimeProcessError> {
        let Some(workdir) = workdir.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(ContainerWorkdir::workspace_root());
        };
        reject_nul("sandbox working directory", workdir)?;
        if workdir == CONTAINER_WORKSPACE_ROOT {
            return Ok(ContainerWorkdir::workspace_root());
        }
        if let Some(relative) = workdir.strip_prefix("/workspace/") {
            validate_relative_workdir(Path::new(relative))?;
            return ContainerWorkdir::from_relative(relative);
        }

        let requested = Path::new(workdir);
        if requested.is_absolute() {
            Err(RuntimeProcessError::ExecutionFailed(
                "sandbox working directory must be workspace-relative or under /workspace"
                    .to_string(),
            ))
        } else {
            validate_relative_workdir(requested)?;
            let canonical_workspace = workspace.canonicalize().map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox workspace could not be resolved: {error}"
                ))
            })?;
            let candidate = canonical_workspace.join(requested);
            if !candidate.starts_with(&canonical_workspace) {
                return Err(RuntimeProcessError::ExecutionFailed(
                    "sandbox working directory must stay inside the scoped workspace".to_string(),
                ));
            }
            ContainerWorkdir::from_relative(requested)
        }
    }

    async fn execute_in_container(
        &self,
        request: CommandExecutionRequest,
        workspace: &Path,
        workdir: ContainerWorkdir,
        timeout: Duration,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        let scope_key = RebornSandboxScopeKey::from_scope(&request.scope);
        let container_name = format!(
            "{}-{}",
            scope_key.container_name_prefix(),
            uuid::Uuid::new_v4()
        );
        let env = validate_env(request.extra_env)?;
        let container_user = self.config.container_identity.container_user()?;
        let binds = vec![format!(
            "{}:{CONTAINER_WORKSPACE_ROOT}:rw",
            workspace.display()
        )];
        let host_config = HostConfig {
            binds: Some(binds),
            memory: Some(self.config.memory_bytes as i64),
            cpu_shares: Some(self.config.cpu_shares as i64),
            auto_remove: Some(false),
            network_mode: self.config.disable_network.then(|| "none".to_string()),
            cap_drop: Some(vec!["ALL".to_string()]),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            readonly_rootfs: Some(true),
            tmpfs: Some(
                [("/tmp".to_string(), "size=512M".to_string())]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        };
        let container_config = Config {
            image: Some(self.config.image.clone()),
            cmd: Some(vec!["sh".to_string(), "-c".to_string(), request.command]),
            working_dir: Some(workdir.into_string()),
            env: Some(env),
            host_config: Some(host_config),
            user: container_user,
            attach_stdout: Some(false),
            attach_stderr: Some(false),
            ..Default::default()
        };

        let created = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: container_name.clone(),
                    platform: None,
                }),
                container_config,
            )
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox container create failed: {error}"
                ))
            })?;
        let container_id = created.id;
        let started_at = Instant::now();

        let result = async {
            self.docker
                .start_container(&container_id, None::<StartContainerOptions<String>>)
                .await
                .map_err(|error| {
                    RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox container start failed: {error}"
                    ))
                })?;
            let exit_code = wait_for_container(&self.docker, &container_id).await?;
            let output =
                collect_logs(&self.docker, &container_id, self.config.max_output_bytes).await?;
            Ok(CommandExecutionOutput {
                output,
                exit_code,
                sandboxed: true,
                duration: started_at.elapsed(),
            })
        };

        let result = match tokio::time::timeout(timeout, result).await {
            Ok(result) => result,
            Err(_) => Err(RuntimeProcessError::Timeout(timeout)),
        };
        let _ = self
            .docker
            .remove_container(
                &container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;
        result
    }
}

#[async_trait]
impl SandboxCommandTransport for RebornScopedSandboxCommandTransport {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        reject_nul("sandbox command", &request.command)?;
        if request
            .mounts
            .as_ref()
            .is_some_and(|mounts| !mounts.mounts.is_empty())
        {
            return Err(RuntimeProcessError::ExecutionFailed(
                "scoped mounts are not supported by the Reborn Docker command sandbox yet"
                    .to_string(),
            ));
        }

        let workspace = self.prepare_workspace(&request.scope)?;
        let workdir = Self::resolve_container_workdir(&workspace, request.workdir.as_deref())?;
        let timeout = request
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(self.config.default_timeout);
        self.execute_in_container(request, &workspace, workdir, timeout)
            .await
    }
}

async fn connect_docker() -> Result<Docker, RuntimeProcessError> {
    if let Ok(docker) = Docker::connect_with_local_defaults()
        && docker.ping().await.is_ok()
    {
        return Ok(docker);
    }
    #[cfg(unix)]
    {
        for socket in unix_socket_candidates() {
            if socket.exists() {
                let socket = socket.to_string_lossy();
                if let Ok(docker) =
                    Docker::connect_with_socket(&socket, 120, bollard::API_DEFAULT_VERSION)
                    && docker.ping().await.is_ok()
                {
                    return Ok(docker);
                }
            }
        }
    }
    Err(RuntimeProcessError::ExecutionFailed(
        "could not connect to Docker daemon for Reborn sandbox".to_string(),
    ))
}

#[cfg(unix)]
fn unix_socket_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        candidates.push(home.join(".docker/run/docker.sock"));
        candidates.push(home.join(".colima/default/docker.sock"));
        candidates.push(home.join(".rd/docker.sock"));
    }
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        candidates.push(runtime_dir.join("docker.sock"));
    }
    candidates
}

async fn wait_for_container(
    docker: &Docker,
    container_id: &str,
) -> Result<i64, RuntimeProcessError> {
    let mut stream = docker.wait_container(
        container_id,
        Some(WaitContainerOptions {
            condition: "not-running",
        }),
    );
    match stream.next().await {
        Some(Ok(result)) => Ok(result.status_code),
        Some(Err(error)) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox container wait failed: {error}"
        ))),
        None => Err(RuntimeProcessError::ExecutionFailed(
            "sandbox container wait stream ended unexpectedly".to_string(),
        )),
    }
}

async fn collect_logs(
    docker: &Docker,
    container_id: &str,
    limit: usize,
) -> Result<String, RuntimeProcessError> {
    let mut stream = docker.logs(
        container_id,
        Some(LogsOptions::<String> {
            stdout: true,
            stderr: true,
            follow: false,
            ..Default::default()
        }),
    );
    let mut stdout = String::new();
    let mut stderr = String::new();
    let half_limit = limit / 2;
    while let Some(result) = stream.next().await {
        match result {
            Ok(LogOutput::StdOut { message }) => {
                append_with_limit(&mut stdout, &String::from_utf8_lossy(&message), half_limit);
            }
            Ok(LogOutput::StdErr { message }) => {
                append_with_limit(&mut stderr, &String::from_utf8_lossy(&message), half_limit);
            }
            Ok(_) => {}
            Err(error) => {
                return Err(RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox log collection failed: {error}"
                )));
            }
        }
    }
    if stderr.is_empty() {
        Ok(stdout)
    } else if stdout.is_empty() {
        Ok(stderr)
    } else {
        Ok(format!("{stdout}\n\n--- stderr ---\n{stderr}"))
    }
}

fn append_with_limit(buffer: &mut String, text: &str, limit: usize) {
    if buffer.len() >= limit {
        return;
    }
    let remaining = limit - buffer.len();
    if text.len() <= remaining {
        buffer.push_str(text);
        return;
    }
    let end = floor_char_boundary(text, remaining);
    buffer.push_str(&text[..end]);
}

fn floor_char_boundary(value: &str, index: usize) -> usize {
    if index >= value.len() {
        return value.len();
    }
    let mut index = index;
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn reject_nul(label: &str, value: &str) -> Result<(), RuntimeProcessError> {
    if value.as_bytes().contains(&0) {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "{label} contains null bytes"
        )));
    }
    Ok(())
}

fn validate_env(env: HashMap<String, String>) -> Result<Vec<String>, RuntimeProcessError> {
    env.into_iter()
        .map(|(key, value)| {
            reject_nul("environment variable name", &key)?;
            reject_nul("environment variable value", &value)?;
            if key.contains('=') || key.is_empty() {
                return Err(RuntimeProcessError::ExecutionFailed(
                    "environment variable names must be non-empty and cannot contain '='"
                        .to_string(),
                ));
            }
            Ok(format!("{key}={value}"))
        })
        .collect()
}

fn validate_relative_workdir(path: &Path) -> Result<(), RuntimeProcessError> {
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => {
                return Err(RuntimeProcessError::ExecutionFailed(
                    "sandbox working directory must stay inside the scoped workspace".to_string(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_workdir_rejects_escape() {
        let workspace = Path::new("/tmp/reborn-sandbox/tenant/user");
        let error =
            RebornScopedSandboxCommandTransport::resolve_container_workdir(workspace, Some("../x"))
                .unwrap_err();

        assert!(format!("{error}").contains("scoped workspace"));
    }

    #[test]
    fn container_workdir_rejects_host_absolute_paths() {
        let workspace = Path::new("/tmp/reborn-sandbox/tenant/user");
        let error = RebornScopedSandboxCommandTransport::resolve_container_workdir(
            workspace,
            Some("/tmp/reborn-sandbox/tenant/user/app"),
        )
        .unwrap_err();

        assert!(format!("{error}").contains("workspace-relative"));
    }

    #[test]
    fn container_workdir_accepts_typed_container_paths() {
        let workspace = Path::new("/tmp/reborn-sandbox/tenant/user");
        let workdir = RebornScopedSandboxCommandTransport::resolve_container_workdir(
            workspace,
            Some("/workspace/app"),
        )
        .unwrap();

        assert_eq!(workdir.into_string(), "/workspace/app");
    }

    #[test]
    fn configured_workspace_modes_are_explicit_shapes() {
        let private = RebornSandboxConfig::new("/tmp/reborn-sandbox")
            .with_container_user("1000:1000", RebornSandboxWorkspaceMode::Private);
        let group_shared = RebornSandboxConfig::new("/tmp/reborn-sandbox")
            .with_container_user("1000:1000", RebornSandboxWorkspaceMode::GroupShared);

        assert_eq!(private.container_identity.workspace_mode(), 0o700);
        assert_eq!(group_shared.container_identity.workspace_mode(), 0o770);
    }
}
