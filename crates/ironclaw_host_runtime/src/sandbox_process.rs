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
mod mounts;
mod scope_key;

use mounts::RebornSandboxMountSources;

pub use container_identity::{RebornSandboxContainerIdentity, RebornSandboxWorkspaceMode};
pub use scope_key::RebornSandboxScopeKey;

const DEFAULT_IMAGE: &str = "ironclaw-worker:latest";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DEFAULT_CPU_SHARES: u32 = 1024;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;
const CONTAINER_WORKSPACE_ROOT: &str = "/workspace";
const REBORN_NETWORK_MODE_ENV: &str = "IRONCLAW_REBORN_NETWORK_MODE";
const REBORN_HTTP_PROXY_ENV: &str = "IRONCLAW_REBORN_HTTP_PROXY";
const REBORN_HTTP_BROKER_SOCKET_ENV: &str = "IRONCLAW_REBORN_HTTP_BROKER_SOCKET";
const REBORN_HTTP_BROKER_URL_ENV: &str = "IRONCLAW_REBORN_HTTP_BROKER_URL";
const REBORN_SECRET_MODE_ENV: &str = "IRONCLAW_REBORN_SECRET_MODE";
const REBORN_SECRET_BROKER_ENV: &str = "IRONCLAW_REBORN_SECRET_BROKER_URL";
const REBORN_SECRET_BROKER_SOCKET_ENV: &str = "IRONCLAW_REBORN_SECRET_BROKER_SOCKET";
const HTTP_PROXY_ENV_KEYS: &[&str] = &["http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY"];
const CONTAINER_HTTP_BROKER_SOCKET: &str = "/tmp/ironclaw-http-broker.sock";
const CONTAINER_SECRET_BROKER_SOCKET: &str = "/tmp/ironclaw-secret-broker.sock";
const CONTAINER_BROKER_URL: &str = "http://ironclaw-broker";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContainerWorkdir(String);

impl ContainerWorkdir {
    fn workspace_root() -> Self {
        Self(CONTAINER_WORKSPACE_ROOT.to_string())
    }

    fn from_relative(relative: impl AsRef<Path>) -> Self {
        let relative = relative.as_ref().to_string_lossy();
        if relative.is_empty() || relative == "." {
            return Self::workspace_root();
        }
        Self(format!(
            "{CONTAINER_WORKSPACE_ROOT}/{}",
            relative.trim_start_matches('/')
        ))
    }

    fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone)]
pub struct RebornSandboxConfig {
    workspace_root: PathBuf,
    mount_sources: RebornSandboxMountSources,
    image: String,
    default_timeout: Duration,
    memory_bytes: u64,
    cpu_shares: u32,
    max_output_bytes: usize,
    disable_network: bool,
    network_broker: Option<RebornSandboxNetworkBroker>,
    secret_broker: Option<RebornSandboxSecretBroker>,
    container_identity: RebornSandboxContainerIdentity,
}

impl RebornSandboxConfig {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            mount_sources: RebornSandboxMountSources::default(),
            image: std::env::var("IRONCLAW_REBORN_SANDBOX_IMAGE")
                .or_else(|_| std::env::var("IRONCLAW_SANDBOX_IMAGE"))
                .unwrap_or_else(|_| DEFAULT_IMAGE.to_string()),
            default_timeout: DEFAULT_TIMEOUT,
            memory_bytes: DEFAULT_MEMORY_BYTES,
            cpu_shares: DEFAULT_CPU_SHARES,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            disable_network: true,
            network_broker: None,
            secret_broker: None,
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

    pub fn with_network_broker_proxy_url(
        mut self,
        proxy_url: impl Into<String>,
    ) -> Result<Self, RuntimeProcessError> {
        self.network_broker = Some(RebornSandboxNetworkBroker::new(proxy_url)?);
        Ok(self)
    }

    pub fn with_network_broker_port(mut self, port: u16) -> Self {
        self.network_broker = Some(RebornSandboxNetworkBroker::from_port(port));
        self
    }

    pub fn with_network_broker_unix_socket(
        mut self,
        host_socket: impl Into<PathBuf>,
    ) -> Result<Self, RuntimeProcessError> {
        self.network_broker = Some(RebornSandboxNetworkBroker::unix_socket(host_socket)?);
        Ok(self)
    }

    pub fn with_secret_broker_url(
        mut self,
        broker_url: impl Into<String>,
    ) -> Result<Self, RuntimeProcessError> {
        self.secret_broker = Some(RebornSandboxSecretBroker::new(broker_url)?);
        Ok(self)
    }

    pub fn with_secret_broker_unix_socket(
        mut self,
        host_socket: impl Into<PathBuf>,
    ) -> Result<Self, RuntimeProcessError> {
        self.secret_broker = Some(RebornSandboxSecretBroker::unix_socket(host_socket)?);
        Ok(self)
    }

    pub fn with_local_mount_source(
        mut self,
        virtual_root: ironclaw_host_api::VirtualPath,
        host_root: impl Into<PathBuf>,
    ) -> Result<Self, RuntimeProcessError> {
        self.mount_sources
            .add_local_source(virtual_root, host_root)?;
        Ok(self)
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

    fn container_network_mode(&self) -> Option<String> {
        if self.disable_network
            && !self
                .network_broker
                .as_ref()
                .is_some_and(RebornSandboxNetworkBroker::requires_docker_network)
        {
            Some("none".to_string())
        } else {
            None
        }
    }

    fn command_env(
        &self,
        extra_env: HashMap<String, String>,
    ) -> Result<Vec<String>, RuntimeProcessError> {
        let mut env = validate_env(extra_env)?;
        if let Some(broker) = &self.network_broker {
            push_reserved_env(&mut env, REBORN_NETWORK_MODE_ENV, "brokered")?;
            match broker {
                RebornSandboxNetworkBroker::HttpProxy { proxy_url } => {
                    push_reserved_env(&mut env, REBORN_HTTP_PROXY_ENV, proxy_url)?;
                    for key in HTTP_PROXY_ENV_KEYS {
                        push_reserved_env(&mut env, key, proxy_url)?;
                    }
                }
                RebornSandboxNetworkBroker::UnixSocket { .. } => {
                    push_reserved_env(
                        &mut env,
                        REBORN_HTTP_BROKER_SOCKET_ENV,
                        CONTAINER_HTTP_BROKER_SOCKET,
                    )?;
                    push_reserved_env(&mut env, REBORN_HTTP_BROKER_URL_ENV, CONTAINER_BROKER_URL)?;
                }
            }
        } else {
            push_reserved_env(&mut env, REBORN_NETWORK_MODE_ENV, "disabled")?;
        }
        if let Some(broker) = &self.secret_broker {
            push_reserved_env(&mut env, REBORN_SECRET_MODE_ENV, "brokered")?;
            match broker {
                RebornSandboxSecretBroker::HttpEndpoint { broker_url } => {
                    push_reserved_env(&mut env, REBORN_SECRET_BROKER_ENV, broker_url)?;
                }
                RebornSandboxSecretBroker::UnixSocket { .. } => {
                    push_reserved_env(
                        &mut env,
                        REBORN_SECRET_BROKER_SOCKET_ENV,
                        CONTAINER_SECRET_BROKER_SOCKET,
                    )?;
                }
            }
        } else {
            push_reserved_env(&mut env, REBORN_SECRET_MODE_ENV, "disabled")?;
        }
        Ok(env)
    }

    fn append_broker_binds(&self, binds: &mut Vec<String>) -> Result<(), RuntimeProcessError> {
        if let Some(RebornSandboxNetworkBroker::UnixSocket { host_socket }) = &self.network_broker {
            binds.push(docker_file_bind(
                host_socket,
                CONTAINER_HTTP_BROKER_SOCKET,
                "network broker socket",
            )?);
        }
        if let Some(RebornSandboxSecretBroker::UnixSocket { host_socket }) = &self.secret_broker {
            binds.push(docker_file_bind(
                host_socket,
                CONTAINER_SECRET_BROKER_SOCKET,
                "secret broker socket",
            )?);
        }
        Ok(())
    }
}

/// Broker affordance exposed to tenant sandbox commands.
///
/// The Unix-socket variant preserves Docker `--network none`; the HTTP-proxy
/// variant intentionally requires Docker network attachment and is for
/// compositions that accept proxy-enforced rather than Docker-enforced egress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornSandboxNetworkBroker {
    HttpProxy { proxy_url: String },
    UnixSocket { host_socket: PathBuf },
}

impl RebornSandboxNetworkBroker {
    pub fn new(proxy_url: impl Into<String>) -> Result<Self, RuntimeProcessError> {
        let proxy_url = proxy_url.into();
        validate_broker_url("network broker proxy URL", &proxy_url)?;
        Ok(Self::HttpProxy { proxy_url })
    }

    pub fn from_port(port: u16) -> Self {
        Self::HttpProxy {
            proxy_url: format!("http://{}:{port}", docker_host_gateway()),
        }
    }

    pub fn unix_socket(host_socket: impl Into<PathBuf>) -> Result<Self, RuntimeProcessError> {
        let host_socket = host_socket.into();
        validate_host_socket_path("network broker socket", &host_socket)?;
        Ok(Self::UnixSocket { host_socket })
    }

    fn requires_docker_network(&self) -> bool {
        matches!(self, Self::HttpProxy { .. })
    }
}

/// Secret broker affordance exposed to tenant sandbox commands.
///
/// The value is an endpoint, not secret material. Concrete brokers remain
/// responsible for authentication, one-shot leases, redaction, and audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornSandboxSecretBroker {
    HttpEndpoint { broker_url: String },
    UnixSocket { host_socket: PathBuf },
}

impl RebornSandboxSecretBroker {
    pub fn new(broker_url: impl Into<String>) -> Result<Self, RuntimeProcessError> {
        let broker_url = broker_url.into();
        validate_broker_url("secret broker URL", &broker_url)?;
        Ok(Self::HttpEndpoint { broker_url })
    }

    pub fn unix_socket(host_socket: impl Into<PathBuf>) -> Result<Self, RuntimeProcessError> {
        let host_socket = host_socket.into();
        validate_host_socket_path("secret broker socket", &host_socket)?;
        Ok(Self::UnixSocket { host_socket })
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
            .field("network_broker", &self.config.network_broker)
            .field("secret_broker", &self.config.secret_broker)
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

    async fn prepare_workspace(
        &self,
        scope: &ResourceScope,
    ) -> Result<PathBuf, RuntimeProcessError> {
        let key = RebornSandboxScopeKey::from_scope(scope);
        let workspace = key.workspace_path(&self.config.workspace_root);
        tokio::fs::create_dir_all(&workspace)
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox workspace could not be initialized: {error}"
                ))
            })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(
                &workspace,
                std::fs::Permissions::from_mode(self.config.container_identity.workspace_mode()),
            )
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox workspace permissions could not be set: {error}"
                ))
            })?;
        }
        tokio::fs::canonicalize(&workspace).await.map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox workspace could not be resolved: {error}"
            ))
        })
    }

    fn resolve_container_workdir(
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
            return Ok(ContainerWorkdir::from_relative(relative));
        }

        let requested = Path::new(workdir);
        if requested.is_absolute() {
            Err(RuntimeProcessError::ExecutionFailed(
                "sandbox working directory must be workspace-relative or under /workspace"
                    .to_string(),
            ))
        } else {
            validate_relative_workdir(requested)?;
            Ok(ContainerWorkdir::from_relative(requested))
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
        let env = self.config.command_env(request.extra_env)?;
        let container_user = self.config.container_identity.container_user()?;
        let mut binds = self
            .config
            .mount_sources
            .prepare_container_binds(workspace, request.mounts.as_ref())
            .await?
            .into_iter()
            .map(|bind| bind.into_docker_bind())
            .collect::<Vec<_>>();
        self.config.append_broker_binds(&mut binds)?;
        let host_config = HostConfig {
            binds: Some(binds),
            memory: Some(self.config.memory_bytes as i64),
            cpu_shares: Some(self.config.cpu_shares as i64),
            auto_remove: Some(false),
            network_mode: self.config.container_network_mode(),
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

        let workspace = self.prepare_workspace(&request.scope).await?;
        let workdir = Self::resolve_container_workdir(request.workdir.as_deref())?;
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

fn push_reserved_env(
    env: &mut Vec<String>,
    key: &str,
    value: &str,
) -> Result<(), RuntimeProcessError> {
    if env
        .iter()
        .any(|entry| entry.starts_with(&format!("{key}=")))
    {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "environment variable {key} is reserved for the Reborn sandbox"
        )));
    }
    reject_nul("environment variable name", key)?;
    reject_nul("environment variable value", value)?;
    env.push(format!("{key}={value}"));
    Ok(())
}

fn validate_broker_url(label: &str, value: &str) -> Result<(), RuntimeProcessError> {
    reject_nul(label, value)?;
    if value.trim() != value
        || value.is_empty()
        || value.chars().any(char::is_control)
        || !(value.starts_with("http://") || value.starts_with("https://"))
    {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "{label} must be an http(s) URL without control characters"
        )));
    }
    Ok(())
}

fn validate_host_socket_path(label: &str, path: &Path) -> Result<(), RuntimeProcessError> {
    let raw = path.to_string_lossy();
    reject_nul(label, &raw)?;
    if !path.is_absolute() || raw.contains(':') || raw.is_empty() {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "{label} must be an absolute host path without ':'"
        )));
    }
    Ok(())
}

fn docker_file_bind(
    host_path: &Path,
    container_path: &str,
    label: &str,
) -> Result<String, RuntimeProcessError> {
    validate_host_socket_path(label, host_path)?;
    reject_nul("container broker path", container_path)?;
    Ok(format!("{}:{container_path}:rw", host_path.display()))
}

fn docker_host_gateway() -> &'static str {
    if cfg!(target_os = "linux") {
        "172.17.0.1"
    } else {
        "host.docker.internal"
    }
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
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    #[test]
    fn relative_workdir_rejects_escape() {
        let error = RebornScopedSandboxCommandTransport::resolve_container_workdir(Some("../x"))
            .unwrap_err();

        assert!(format!("{error}").contains("scoped workspace"));
    }

    #[test]
    fn container_workdir_rejects_host_absolute_paths() {
        let error = RebornScopedSandboxCommandTransport::resolve_container_workdir(Some(
            "/tmp/reborn-sandbox/tenant/user/app",
        ))
        .unwrap_err();

        assert!(format!("{error}").contains("workspace-relative"));
    }

    #[test]
    fn container_workdir_accepts_typed_container_paths() {
        let workdir =
            RebornScopedSandboxCommandTransport::resolve_container_workdir(Some("/workspace/app"))
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

    #[test]
    fn default_sandbox_disables_ambient_network_and_secret_affordance() {
        let config = RebornSandboxConfig::new("/tmp/reborn-sandbox");
        let env = config.command_env(HashMap::new()).unwrap();

        assert_eq!(config.container_network_mode(), Some("none".to_string()));
        assert!(env.contains(&"IRONCLAW_REBORN_NETWORK_MODE=disabled".to_string()));
        assert!(env.contains(&"IRONCLAW_REBORN_SECRET_MODE=disabled".to_string()));
    }

    #[test]
    fn network_broker_exposes_proxy_env_without_none_network_mode() {
        let config = RebornSandboxConfig::new("/tmp/reborn-sandbox")
            .with_network_broker_proxy_url("http://broker.internal:8181")
            .unwrap();
        let env = config.command_env(HashMap::new()).unwrap();

        assert_eq!(config.container_network_mode(), None);
        assert!(env.contains(&"IRONCLAW_REBORN_NETWORK_MODE=brokered".to_string()));
        assert!(
            env.contains(&"IRONCLAW_REBORN_HTTP_PROXY=http://broker.internal:8181".to_string())
        );
        assert!(env.contains(&"http_proxy=http://broker.internal:8181".to_string()));
        assert!(env.contains(&"https_proxy=http://broker.internal:8181".to_string()));
        assert!(env.contains(&"HTTP_PROXY=http://broker.internal:8181".to_string()));
        assert!(env.contains(&"HTTPS_PROXY=http://broker.internal:8181".to_string()));
    }

    #[test]
    fn unix_socket_network_broker_preserves_none_network_mode_and_mounts_socket() {
        let config = RebornSandboxConfig::new("/tmp/reborn-sandbox")
            .with_network_broker_unix_socket("/tmp/reborn-http-broker.sock")
            .unwrap();
        let env = config.command_env(HashMap::new()).unwrap();
        let mut binds = Vec::new();
        config.append_broker_binds(&mut binds).unwrap();

        assert_eq!(config.container_network_mode(), Some("none".to_string()));
        assert!(env.contains(&"IRONCLAW_REBORN_NETWORK_MODE=brokered".to_string()));
        assert!(env.contains(
            &"IRONCLAW_REBORN_HTTP_BROKER_SOCKET=/tmp/ironclaw-http-broker.sock".to_string()
        ));
        assert!(
            env.contains(&"IRONCLAW_REBORN_HTTP_BROKER_URL=http://ironclaw-broker".to_string())
        );
        assert_eq!(
            binds,
            vec!["/tmp/reborn-http-broker.sock:/tmp/ironclaw-http-broker.sock:rw".to_string()]
        );
    }

    #[test]
    fn secret_broker_exposes_endpoint_without_secret_material() {
        let config = RebornSandboxConfig::new("/tmp/reborn-sandbox")
            .with_secret_broker_url("https://broker.internal/secrets")
            .unwrap();
        let env = config.command_env(HashMap::new()).unwrap();

        assert!(env.contains(&"IRONCLAW_REBORN_SECRET_MODE=brokered".to_string()));
        assert!(env.contains(
            &"IRONCLAW_REBORN_SECRET_BROKER_URL=https://broker.internal/secrets".to_string()
        ));
        assert!(
            env.iter()
                .all(|entry| !entry.contains("sk-") && !entry.contains("token="))
        );
    }

    #[test]
    fn unix_socket_secret_broker_exposes_socket_without_secret_material() {
        let config = RebornSandboxConfig::new("/tmp/reborn-sandbox")
            .with_secret_broker_unix_socket("/tmp/reborn-secret-broker.sock")
            .unwrap();
        let env = config.command_env(HashMap::new()).unwrap();
        let mut binds = Vec::new();
        config.append_broker_binds(&mut binds).unwrap();

        assert!(env.contains(&"IRONCLAW_REBORN_SECRET_MODE=brokered".to_string()));
        assert!(env.contains(
            &"IRONCLAW_REBORN_SECRET_BROKER_SOCKET=/tmp/ironclaw-secret-broker.sock".to_string()
        ));
        assert!(
            env.iter()
                .all(|entry| !entry.contains("sk-") && !entry.contains("token="))
        );
        assert_eq!(
            binds,
            vec!["/tmp/reborn-secret-broker.sock:/tmp/ironclaw-secret-broker.sock:rw".to_string()]
        );
    }

    #[test]
    fn broker_env_rejects_user_override() {
        let config = RebornSandboxConfig::new("/tmp/reborn-sandbox")
            .with_network_broker_proxy_url("http://broker.internal:8181")
            .unwrap();
        let error = config
            .command_env(HashMap::from([(
                "http_proxy".to_string(),
                "http://attacker.invalid:1".to_string(),
            )]))
            .unwrap_err();

        assert!(format!("{error}").contains("reserved"));
    }

    #[test]
    fn broker_urls_reject_control_characters_and_non_http_schemes() {
        assert!(RebornSandboxNetworkBroker::new("unix:///tmp/broker.sock").is_err());
        assert!(RebornSandboxSecretBroker::new("https://broker.internal/\nsecrets").is_err());
        assert!(RebornSandboxNetworkBroker::unix_socket("relative.sock").is_err());
        assert!(RebornSandboxSecretBroker::unix_socket("/tmp/bad:path.sock").is_err());
    }

    #[tokio::test]
    async fn run_command_rejects_unconfigured_scoped_mount_before_container_create() {
        let temp = tempfile::tempdir().unwrap();
        let docker = Docker::connect_with_local_defaults().unwrap();
        let transport = RebornScopedSandboxCommandTransport::new(
            docker,
            RebornSandboxConfig::new(temp.path().join("workspaces")),
        );
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/app").unwrap(),
            process_read_only_permissions(),
        )])
        .unwrap();

        let error = transport
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: Some(mounts),
                command: "true".to_string(),
                workdir: None,
                timeout_secs: Some(1),
                extra_env: HashMap::new(),
            })
            .await
            .unwrap_err();

        assert!(format!("{error}").contains("no trusted sandbox mount source"));
    }

    fn process_read_only_permissions() -> MountPermissions {
        MountPermissions {
            execute: true,
            ..MountPermissions::read_only()
        }
    }
}
