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
use futures::StreamExt;
use ironclaw_host_api::ResourceScope;
use ironclaw_host_runtime::{
    CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError, SandboxCommandTransport,
    TenantSandboxProcessPort, VerifiedTenantSandboxProcessPort,
};
use sha2::{Digest, Sha256};

const DEFAULT_IMAGE: &str = "ironclaw-worker:latest";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DEFAULT_CPU_SHARES: u32 = 1024;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;
const CONTAINER_WORKSPACE_ROOT: &str = "/workspace";

#[derive(Debug, Clone)]
pub struct RebornSandboxConfig {
    workspace_root: PathBuf,
    image: String,
    default_timeout: Duration,
    memory_bytes: u64,
    cpu_shares: u32,
    max_output_bytes: usize,
    disable_network: bool,
    container_user: Option<String>,
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
            container_user: None,
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

    pub fn with_container_user(mut self, user: impl Into<String>) -> Self {
        self.container_user = Some(user.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSandboxScopeKey {
    raw: String,
    digest: String,
}

impl RebornSandboxScopeKey {
    pub fn from_scope(scope: &ResourceScope) -> Self {
        let mut raw_parts = vec![
            ("tenant", scope.tenant_id.as_str().to_string()),
            ("user", scope.user_id.as_str().to_string()),
        ];
        raw_parts.push((
            "agent",
            scope
                .agent_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_else(|| "_none".to_string()),
        ));
        if let Some(project_id) = &scope.project_id {
            raw_parts.push(("project", project_id.as_str().to_string()));
        } else if let Some(thread_id) = &scope.thread_id {
            raw_parts.push(("thread", thread_id.as_str().to_string()));
        } else {
            raw_parts.push(("invocation", scope.invocation_id.to_string()));
        }

        let raw = encode_scope_parts(&raw_parts);
        let digest = scope_digest(&raw);
        Self { raw, digest }
    }

    pub fn workspace_path(&self, root: &Path) -> PathBuf {
        root.join("scopes").join(&self.digest)
    }

    pub fn container_name_prefix(&self) -> String {
        format!("ironclaw-reborn-sandbox-{}", &self.digest[..24])
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
            .field("container_user", &self.config.container_user)
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

    pub fn into_verified_process_port(self) -> VerifiedTenantSandboxProcessPort {
        VerifiedTenantSandboxProcessPort::new(Arc::new(self.into_process_port()))
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
            std::fs::set_permissions(&workspace, std::fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox workspace permissions could not be set: {error}"
                    ))
                },
            )?;
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
    ) -> Result<String, RuntimeProcessError> {
        let Some(workdir) = workdir.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(CONTAINER_WORKSPACE_ROOT.to_string());
        };
        reject_nul("sandbox working directory", workdir)?;
        if workdir == CONTAINER_WORKSPACE_ROOT {
            return Ok(CONTAINER_WORKSPACE_ROOT.to_string());
        }
        if let Some(relative) = workdir.strip_prefix("/workspace/") {
            validate_relative_workdir(Path::new(relative))?;
            return Ok(format!("{CONTAINER_WORKSPACE_ROOT}/{relative}"));
        }

        let requested = Path::new(workdir);
        let relative = if requested.is_absolute() {
            let requested = requested.canonicalize().map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox working directory is unavailable: {error}"
                ))
            })?;
            requested
                .strip_prefix(workspace)
                .map_err(|_| {
                    RuntimeProcessError::ExecutionFailed(
                        "sandbox working directory must stay inside the scoped workspace"
                            .to_string(),
                    )
                })?
                .to_path_buf()
        } else {
            validate_relative_workdir(requested)?;
            requested.to_path_buf()
        };
        let relative = relative.to_string_lossy();
        if relative.is_empty() {
            Ok(CONTAINER_WORKSPACE_ROOT.to_string())
        } else {
            Ok(format!(
                "{CONTAINER_WORKSPACE_ROOT}/{}",
                relative.trim_start_matches('/')
            ))
        }
    }

    async fn execute_in_container(
        &self,
        request: CommandExecutionRequest,
        workspace: &Path,
        workdir: String,
        timeout: Duration,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        let scope_key = RebornSandboxScopeKey::from_scope(&request.scope);
        let container_name = format!(
            "{}-{}",
            scope_key.container_name_prefix(),
            uuid::Uuid::new_v4()
        );
        let env = validate_env(request.extra_env)?;
        let container_user = self
            .config
            .container_user
            .as_deref()
            .map(validate_container_user)
            .transpose()?;
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
            working_dir: Some(workdir),
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

fn validate_container_user(user: &str) -> Result<String, RuntimeProcessError> {
    reject_nul("sandbox container user", user)?;
    if user.trim().is_empty() {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox container user must not be empty".to_string(),
        ));
    }
    Ok(user.to_string())
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

fn encode_scope_parts(parts: &[(&str, String)]) -> String {
    let mut encoded = String::new();
    for (kind, value) in parts {
        encoded.push_str(&kind.len().to_string());
        encoded.push(':');
        encoded.push_str(kind);
        encoded.push('=');
        encoded.push_str(&value.len().to_string());
        encoded.push(':');
        encoded.push_str(value);
        encoded.push(';');
    }
    encoded
}

fn scope_digest(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, InvocationId, ProjectId, TenantId, ThreadId, UserId};

    fn scope(
        tenant: &str,
        user: &str,
        project: Option<&str>,
        thread: Option<&str>,
    ) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: Some(AgentId::new("agent").unwrap()),
            project_id: project.map(|value| ProjectId::new(value).unwrap()),
            mission_id: None,
            thread_id: thread.map(|value| ThreadId::new(value).unwrap()),
            invocation_id: InvocationId::new(),
        }
    }

    #[test]
    fn scope_key_isolates_tenants_with_same_user_and_project() {
        let root = Path::new("/tmp/reborn-sandbox");
        let left = RebornSandboxScopeKey::from_scope(&scope(
            "tenant-a",
            "same-user",
            Some("same-project"),
            None,
        ));
        let right = RebornSandboxScopeKey::from_scope(&scope(
            "tenant-b",
            "same-user",
            Some("same-project"),
            None,
        ));

        assert_ne!(left.workspace_path(root), right.workspace_path(root));
        assert_ne!(left.container_name_prefix(), right.container_name_prefix());
    }

    #[test]
    fn scope_key_uses_thread_when_project_is_absent() {
        let root = Path::new("/tmp/reborn-sandbox");
        let left = RebornSandboxScopeKey::from_scope(&scope("tenant", "user", None, Some("a")));
        let right = RebornSandboxScopeKey::from_scope(&scope("tenant", "user", None, Some("b")));

        assert_ne!(left.workspace_path(root), right.workspace_path(root));
        assert_ne!(left.container_name_prefix(), right.container_name_prefix());
    }

    #[test]
    fn scope_key_does_not_collapse_path_special_characters() {
        let root = Path::new("/tmp/reborn-sandbox");
        let left = RebornSandboxScopeKey::from_scope(&scope("tenant:a", "user", Some("p"), None));
        let right = RebornSandboxScopeKey::from_scope(&scope("tenant_a", "user", Some("p"), None));

        assert_ne!(left.workspace_path(root), right.workspace_path(root));
        assert_ne!(left.container_name_prefix(), right.container_name_prefix());
    }

    #[test]
    fn relative_workdir_rejects_escape() {
        let workspace = Path::new("/tmp/reborn-sandbox/tenant/user");
        let error =
            RebornScopedSandboxCommandTransport::resolve_container_workdir(workspace, Some("../x"))
                .unwrap_err();

        assert!(format!("{error}").contains("scoped workspace"));
    }
}
