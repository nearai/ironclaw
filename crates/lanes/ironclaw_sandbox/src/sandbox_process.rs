//! Reborn-native user sandbox command transport.
//!
//! Host workspaces and local Docker containers derive from tenant plus user and
//! persist across threads. Container-local process state is therefore shared by
//! all threads owned by the same authenticated user.
//! The per-user lifecycle gate serializes commands for one user while allowing
//! different users to run in parallel.

use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use bollard::{
    Docker,
    container::Config,
    models::{HostConfig, HostConfigLogConfig},
};
use fs2::FileExt;
use ironclaw_host_api::{
    process::{
        CommandExecutionOutput, CommandExecutionRequest, CredentialedSandboxCommandRequest,
        RuntimeProcessError, SandboxCommandCredential, SandboxCommandTransport,
    },
    resource::ResourceScope,
};

mod broker;
mod ca;
mod connect;
mod container_identity;
mod credential_firewall;
mod key_codec;
mod managed_egress;
mod mounts;
mod network_allowlist;
mod railway;
mod scope_key;
pub(crate) mod shell_limits;
mod user_container;
mod worker_spec;

use crate::validation::{validate_env_has_no_raw_sensitive_values, validate_env_name};
use managed_egress::{ManagedEgressBundle, ManagedEgressConfig, ManagedEgressRuntime};

// `user_key` is the shared per-user workspace and local Docker container
// identity. Railway keeps its existing backend-specific lifecycle.
mod attribution;
mod registry;
mod user_key;

use mounts::RebornSandboxMountSources;

pub use broker::{RebornSandboxNetworkBroker, RebornSandboxSecretBroker};
pub use connect::{SandboxDockerReadiness, connect_docker_with_retry, sandbox_docker_readiness};
pub use container_identity::{RebornSandboxContainerIdentity, RebornSandboxWorkspaceMode};
pub use network_allowlist::{
    DEFAULT_SANDBOX_ALLOWED_DOMAINS, SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV, sandbox_allowed_domains,
    sandbox_extra_allowed_domains, sandbox_network_policy,
};
pub use railway::{RailwayPreviewSandboxConfig, RailwayPreviewSandboxTransport};
pub use registry::SandboxActivityRegistry;
pub use scope_key::RebornSandboxScopeKey;
pub use user_key::RebornSandboxUserKey;

/// Stable Docker container-name prefix used by the local per-user sandbox.
pub const USER_SANDBOX_CONTAINER_NAME_PREFIX: &str = user_key::USER_CONTAINER_NAME_PREFIX;
/// Number of hexadecimal digest characters in a local per-user container name.
pub const USER_SANDBOX_CONTAINER_DIGEST_HEX_LEN: usize = user_key::USER_CONTAINER_DIGEST_HEX_LEN;
/// Stable tenant label on local per-user sandbox containers.
pub const USER_SANDBOX_LABEL_TENANT: &str = registry::USER_CONTAINER_LABEL_TENANT;
/// Stable user label on local per-user sandbox containers.
pub const USER_SANDBOX_LABEL_USER: &str = registry::USER_CONTAINER_LABEL_USER;
/// Stable immutable-image label on local per-user sandbox containers.
pub const USER_SANDBOX_LABEL_IMAGE: &str = registry::USER_CONTAINER_LABEL_IMAGE;
/// Stable security-posture label on local per-user sandbox containers.
pub const USER_SANDBOX_LABEL_SECURITY_POSTURE: &str =
    registry::USER_CONTAINER_LABEL_SECURITY_POSTURE;
/// Stable tenant label on managed-egress proxy containers.
pub const USER_SANDBOX_PROXY_LABEL_TENANT: &str = "ironclaw.proxy.tenant";
/// Stable user label on managed-egress proxy containers.
pub const USER_SANDBOX_PROXY_LABEL_USER: &str = "ironclaw.proxy.user";
/// Stable tenant label on managed-egress private networks.
pub const USER_SANDBOX_NETWORK_LABEL_TENANT: &str = "ironclaw.network.tenant";
/// Stable user label on managed-egress private networks.
pub const USER_SANDBOX_NETWORK_LABEL_USER: &str = "ironclaw.network.user";
/// Digest-pinned managed-egress proxy image used by sandbox profiles.
pub const DEFAULT_SANDBOX_PROXY_IMAGE: &str = managed_egress::DEFAULT_PROXY_IMAGE;

const DEFAULT_IMAGE: &str = "ironclaw-worker:latest";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(shell_limits::SHELL_TIMEOUT_DEFAULT_SECS);
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const DEFAULT_RETENTION_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
// Cover the longest admitted shell command plus host-kill and reconcile grace.
const USER_LIFECYCLE_GATE_ACQUIRE_TIMEOUT: Duration =
    Duration::from_secs(shell_limits::SHELL_TIMEOUT_MAX_SECS + 10);
const DEFAULT_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DEFAULT_CPU_SHARES: u32 = 1024;
const DEFAULT_MAX_OUTPUT_BYTES: usize = shell_limits::SHELL_OUTPUT_LIMIT_DEFAULT_BYTES as usize;
const CONTAINER_WORKSPACE_ROOT: &str = "/workspace";

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
    idle_timeout: Duration,
    retention_timeout: Duration,
    memory_bytes: u64,
    cpu_shares: u32,
    max_output_bytes: usize,
    network_broker: Option<RebornSandboxNetworkBroker>,
    secret_broker: Option<RebornSandboxSecretBroker>,
    managed_egress: Option<ManagedEgressConfig>,
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
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            retention_timeout: DEFAULT_RETENTION_TIMEOUT,
            memory_bytes: DEFAULT_MEMORY_BYTES,
            cpu_shares: DEFAULT_CPU_SHARES,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            network_broker: None,
            secret_broker: None,
            managed_egress: None,
            container_identity: RebornSandboxContainerIdentity::workspace_owner(),
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
    pub fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    pub fn with_retention_timeout(mut self, timeout: Duration) -> Self {
        self.retention_timeout = timeout;
        self
    }

    pub fn with_managed_egress_proxy(mut self) -> Result<Self, RuntimeProcessError> {
        if self.network_broker.is_some() {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox managed egress and network broker are mutually exclusive".to_string(),
            ));
        }
        let policy = sandbox_network_policy().map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox managed egress policy is invalid: {error}"
            ))
        })?;
        self.managed_egress = Some(ManagedEgressConfig::from_policy(
            policy,
            self.workspace_root.join(".managed-egress"),
        )?);
        Ok(self)
    }

    pub fn with_network_broker_unix_socket(
        mut self,
        host_socket: impl Into<PathBuf>,
    ) -> Result<Self, RuntimeProcessError> {
        if self.managed_egress.is_some() {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox managed egress and network broker are mutually exclusive".to_string(),
            ));
        }
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
        virtual_root: ironclaw_host_api::path::VirtualPath,
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
        Some("none".to_string())
    }

    fn managed_container_network_mode(
        &self,
        bundle: Option<&ManagedEgressBundle>,
    ) -> Option<String> {
        bundle
            .map(|bundle| bundle.network_name.clone())
            .or_else(|| self.container_network_mode())
    }

    fn command_env(
        &self,
        extra_env: HashMap<String, String>,
    ) -> Result<Vec<String>, RuntimeProcessError> {
        let mut env = validate_env(&extra_env)?;
        broker::push_broker_env(
            self.network_broker.as_ref(),
            self.secret_broker.as_ref(),
            &mut env,
        )?;
        Ok(env)
    }

    fn command_env_for_bundle(
        &self,
        extra_env: HashMap<String, String>,
        credentials: &[SandboxCommandCredential],
        managed: Option<&ManagedEgressRuntime>,
        bundle: Option<&ManagedEgressBundle>,
    ) -> Result<Vec<String>, RuntimeProcessError> {
        let (Some(managed), Some(bundle)) = (managed, bundle) else {
            return self.command_env(extra_env);
        };
        validate_credential_env(&extra_env, credentials)?;
        let mut env = extra_env
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>();
        env.sort();
        broker::push_broker_env(None, self.secret_broker.as_ref(), &mut env)?;
        let mode_prefix = format!("{}=", broker::REBORN_NETWORK_MODE_ENV);
        env.retain(|entry| !entry.starts_with(&mode_prefix));
        env.extend(managed.user_environment(bundle)?);
        let brokered_mode = format!("{}=brokered", broker::REBORN_NETWORK_MODE_ENV);
        if env.iter().filter(|entry| *entry == &brokered_mode).count() != 1 {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox managed egress did not establish exactly one brokered network mode"
                    .to_string(),
            ));
        }
        Ok(env)
    }

    fn append_broker_binds(&self, binds: &mut Vec<String>) -> Result<(), RuntimeProcessError> {
        if self.managed_egress.is_some() && self.network_broker.is_some() {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox managed egress and network broker are mutually exclusive".to_string(),
            ));
        }
        broker::append_broker_binds(
            self.network_broker.as_ref(),
            self.secret_broker.as_ref(),
            binds,
        )
    }
}

struct LocalDockerOwnerLock {
    _file: std::fs::File,
}

impl LocalDockerOwnerLock {
    async fn acquire(workspace_root: &Path) -> Result<Arc<Self>, RuntimeProcessError> {
        tokio::fs::create_dir_all(workspace_root)
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox workspace root could not be initialized: {error}"
                ))
            })?;
        let lock_path = workspace_root.join(".ironclaw-sandbox-owner.lock");
        let file = tokio::task::spawn_blocking(move || {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(lock_path)
                .map_err(|error| {
                    RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox workspace ownership could not be opened: {error}"
                    ))
                })?;
            file.try_lock_exclusive().map_err(|error| {
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    RuntimeProcessError::ExecutionFailed(
                        "sandbox Docker workspace is already owned by another IronClaw process"
                            .to_string(),
                    )
                } else {
                    RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox workspace ownership could not be acquired: {error}"
                    ))
                }
            })?;
            Ok(file)
        })
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox workspace ownership task failed: {error}"
            ))
        })??;
        Ok(Arc::new(Self { _file: file }))
    }
}

#[derive(Clone)]
pub struct RebornScopedSandboxCommandTransport {
    docker: Docker,
    config: RebornSandboxConfig,
    activity: Arc<SandboxActivityRegistry>,
    managed_egress: Option<Arc<ManagedEgressRuntime>>,
    sweeper: Arc<user_container::UserContainerSweeper>,
    _owner_lock: Arc<LocalDockerOwnerLock>,
}
struct SandboxExecutionRequest {
    scope: ResourceScope,
    mounts: Option<ironclaw_host_api::mount::MountView>,
    executable: String,
    args: Vec<String>,
    workdir: Option<String>,
    timeout_secs: Option<u64>,
    extra_env: HashMap<String, String>,
}

struct CredentialCleanupGuard {
    managed: Option<Arc<ManagedEgressRuntime>>,
    docker: Docker,
    bundle: Option<ManagedEgressBundle>,
    lifecycle: Option<tokio::sync::OwnedMutexGuard<()>>,
    armed: bool,
}

impl CredentialCleanupGuard {
    fn new(docker: Docker, lifecycle: tokio::sync::OwnedMutexGuard<()>) -> Self {
        Self {
            managed: None,
            docker,
            bundle: None,
            lifecycle: Some(lifecycle),
            armed: false,
        }
    }

    fn arm(&mut self, managed: Arc<ManagedEgressRuntime>, bundle: ManagedEgressBundle) {
        self.managed = Some(managed);
        self.bundle = Some(bundle);
        self.armed = true;
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    async fn cleanup(&mut self) -> Result<(), RuntimeProcessError> {
        if !self.armed {
            return Ok(());
        }
        let managed = self.managed.as_ref().ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "sandbox credential cleanup guard is incomplete".to_string(),
            )
        })?;
        let bundle = self.bundle.as_ref().ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "sandbox credential cleanup guard is incomplete".to_string(),
            )
        })?;
        let result = managed.clear_credentials(&self.docker, bundle).await;
        self.armed = false;
        result
    }
}

impl Drop for CredentialCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let (Some(managed), Some(bundle), Some(lifecycle)) = (
            self.managed.take(),
            self.bundle.take(),
            self.lifecycle.take(),
        ) else {
            tracing::error!("sandbox credential cleanup guard dropped without cleanup state");
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::error!("sandbox credential cleanup could not start after runtime shutdown");
            return;
        };
        let docker = self.docker.clone();
        // Tokio cancellation and Rust panic unwinding both run this Drop path.
        // The owned lifecycle gate moves into the detached task, so another
        // invocation cannot race credential replacement with cleanup. Process
        // aborts do not run destructors and therefore remain outside this
        // in-process guarantee.
        runtime.spawn(async move {
            let _lifecycle = lifecycle;
            if let Err(error) = managed.clear_credentials(&docker, &bundle).await {
                tracing::error!(?error, "sandbox detached credential cleanup failed");
            }
        });
    }
}

impl From<CommandExecutionRequest> for SandboxExecutionRequest {
    fn from(request: CommandExecutionRequest) -> Self {
        Self {
            scope: request.scope,
            mounts: request.mounts,
            executable: "sh".to_string(),
            args: vec!["-c".to_string(), request.command],
            workdir: request.workdir,
            timeout_secs: request.timeout_secs,
            extra_env: request.extra_env,
        }
    }
}

impl From<CredentialedSandboxCommandRequest> for SandboxExecutionRequest {
    fn from(request: CredentialedSandboxCommandRequest) -> Self {
        CommandExecutionRequest {
            scope: request.scope,
            mounts: request.mounts,
            command: request.command,
            workdir: request.workdir,
            timeout_secs: request.timeout_secs,
            extra_env: request.extra_env,
        }
        .into()
    }
}

#[cfg(test)]
mod test_support {
    use super::*;

    pub(super) fn transport(
        docker: Docker,
        config: RebornSandboxConfig,
    ) -> RebornScopedSandboxCommandTransport {
        RebornScopedSandboxCommandTransport {
            docker,
            config,
            managed_egress: None,
            activity: Arc::new(SandboxActivityRegistry::new()),
            sweeper: user_container::test_support::disabled_sweeper(),
            _owner_lock: Arc::new(LocalDockerOwnerLock {
                _file: tempfile::tempfile().expect("create test-only ownership handle"),
            }),
        }
    }
}

impl std::fmt::Debug for RebornScopedSandboxCommandTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornScopedSandboxCommandTransport")
            .field("workspace_root", &self.config.workspace_root)
            .field("image", &self.config.image)
            .field("network_broker", &self.config.network_broker)
            .field("secret_broker", &self.config.secret_broker)
            .field("managed_egress", &self.managed_egress)
            .field("container_identity", &self.config.container_identity)
            .finish_non_exhaustive()
    }
}

impl RebornScopedSandboxCommandTransport {
    pub async fn connect(config: RebornSandboxConfig) -> Result<Self, RuntimeProcessError> {
        let owner_lock = LocalDockerOwnerLock::acquire(&config.workspace_root).await?;
        let docker = connect_docker().await?;
        let managed_egress = match config.managed_egress.clone() {
            Some(managed) => Some(ManagedEgressRuntime::connect(&docker, managed).await?),
            None => None,
        };
        let activity = Arc::new(SandboxActivityRegistry::new());
        user_container::reconcile_labeled_user_containers(
            &docker,
            &activity,
            managed_egress.as_deref(),
        )
        .await;
        let sweeper = user_container::UserContainerSweeper::spawn(
            docker.clone(),
            Arc::clone(&activity),
            managed_egress.clone(),
            config.idle_timeout,
            config.retention_timeout,
        );
        Ok(Self {
            docker,
            config,
            managed_egress,
            activity,
            sweeper,
            _owner_lock: owner_lock,
        })
    }

    pub async fn shutdown(&self) {
        self.sweeper.shutdown().await;
    }

    // `into_process_port` was deleted with the lane merge: it returned
    // `ironclaw_host_runtime::UserSandboxProcessPort`, a kernel type this
    // runtimes-layer crate may not name. It had zero callers workspace-wide;
    // the kernel wraps the transport (`UserSandboxProcessPort::new`), which
    // is the direction the port inversion requires.

    async fn prepare_workspace(
        &self,
        scope: &ResourceScope,
    ) -> Result<PathBuf, RuntimeProcessError> {
        let key = RebornSandboxUserKey::from_scope(scope);
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

    async fn resolve_worker_image(&self) -> Result<String, RuntimeProcessError> {
        self.docker
            .inspect_image(&self.config.image)
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox worker image could not be resolved: {error}"
                ))
            })?
            .id
            .ok_or_else(|| {
                RuntimeProcessError::ExecutionFailed(
                    "sandbox worker image resolved without an immutable image id".to_string(),
                )
            })
    }

    fn user_container_launch_config(
        &self,
        scope: &ResourceScope,
        resolved_image: &str,
        bundle: Option<&ManagedEgressBundle>,
        container_user: String,
        mut binds: Vec<String>,
    ) -> Result<user_container::UserContainerLaunch, RuntimeProcessError> {
        let env = self.config.command_env_for_bundle(
            HashMap::new(),
            &[],
            self.managed_egress.as_deref(),
            bundle,
        )?;
        if let (Some(managed), Some(bundle)) = (self.managed_egress.as_deref(), bundle) {
            binds.push(managed.user_ca_bind(bundle)?);
        }
        let security = worker_spec::DockerWorkerSecuritySpec::new(
            self.config.managed_container_network_mode(bundle),
        );
        let posture = security_posture_stamp(
            &self.config,
            &container_user,
            &security,
            &binds,
            &env,
            bundle.map(|bundle| bundle.posture.as_str()),
        );
        let labels = registry::build_user_container_launch_labels(
            user_container::LABEL_PREFIX,
            &scope.tenant_id,
            &scope.user_id,
            resolved_image,
            &posture,
        );
        let tmpfs = HashMap::from([("/tmp".to_string(), security.tmpfs_options())]);
        let host_config = HostConfig {
            binds: Some(binds),
            memory: Some(self.config.memory_bytes as i64),
            cpu_shares: Some(self.config.cpu_shares as i64),
            auto_remove: Some(false),
            network_mode: security.network_mode(),
            dns: bundle.map(|bundle| vec![bundle.proxy_ip.clone()]),
            cap_drop: Some(security.cap_drop()),
            security_opt: Some(security.security_options()),
            readonly_rootfs: Some(security.readonly_rootfs()),
            pids_limit: Some(security.pids_limit()),
            nano_cpus: Some(security.nano_cpus()),
            log_config: Some(HostConfigLogConfig {
                typ: Some(security.log_driver()),
                config: Some(security.log_options().into_iter().collect()),
            }),
            tmpfs: Some(tmpfs),
            ..Default::default()
        };
        let config = Config {
            image: Some(resolved_image.to_string()),
            working_dir: Some(CONTAINER_WORKSPACE_ROOT.to_string()),
            env: Some(env),
            labels: Some(labels.clone()),
            host_config: Some(host_config),
            user: Some(container_user),
            attach_stdout: Some(false),
            attach_stderr: Some(false),
            open_stdin: Some(false),
            ..Default::default()
        };
        Ok(user_container::UserContainerLaunch { config, labels })
    }

    async fn run_command_owned(
        self,
        request: SandboxExecutionRequest,
        credentials: Vec<SandboxCommandCredential>,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        reject_nul("sandbox executable", &request.executable)?;
        let user_key = RebornSandboxUserKey::from_scope(&request.scope);
        let workdir = Self::resolve_container_workdir(request.workdir.as_deref())?;
        let timeout = request
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(self.config.default_timeout);
        if timeout.is_zero() {
            return Err(RuntimeProcessError::Timeout(timeout));
        }
        for argument in &request.args {
            reject_nul("sandbox command argument", argument)?;
        }
        user_container::exec_helper_timeout_secs(timeout)?;
        validate_credential_env(&request.extra_env, &credentials)?;
        let workspace = self.prepare_workspace(&request.scope).await?;
        let container_user = self
            .config
            .container_identity
            .container_user(&workspace)
            .await?;
        let mut binds = self
            .config
            .mount_sources
            .prepare_container_binds(&workspace, request.mounts.as_ref())
            .await?
            .into_iter()
            .map(|bind| bind.into_docker_bind())
            .collect::<Vec<_>>();
        self.config.append_broker_binds(&mut binds)?;
        binds.sort();
        let activity = self.activity.begin(&user_key)?;
        let gate = self.activity.gate(&user_key).ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "sandbox user container lifecycle gate disappeared".to_string(),
            )
        })?;
        let resolved_image = self.resolve_worker_image().await?;
        let user_lifecycle =
            acquire_user_lifecycle_gate(gate, USER_LIFECYCLE_GATE_ACQUIRE_TIMEOUT).await?;
        let mut credential_cleanup =
            CredentialCleanupGuard::new(self.docker.clone(), user_lifecycle);
        let bundle = match self.managed_egress.as_ref() {
            Some(managed) => Some(
                managed
                    .ensure_bundle(
                        &self.docker,
                        &user_key,
                        &request.scope.tenant_id,
                        &request.scope.user_id,
                    )
                    .await?,
            ),
            None => None,
        };
        if !credentials.is_empty()
            && let (Some(managed), Some(bundle)) = (self.managed_egress.as_ref(), bundle.as_ref())
        {
            credential_cleanup.arm(Arc::clone(managed), bundle.clone());
        }
        let setup = async {
            if let (Some(managed), Some(bundle)) = (self.managed_egress.as_ref(), bundle.as_ref()) {
                managed
                    .set_invocation(bundle, &request.scope.invocation_id)
                    .await?;
                if !credentials.is_empty() {
                    managed
                        .configure_credentials(&self.docker, bundle, &credentials)
                        .await?;
                }
            }
            let launch = self.user_container_launch_config(
                &request.scope,
                &resolved_image,
                bundle.as_ref(),
                container_user,
                binds,
            )?;
            let container_name =
                user_container::ensure_user_container(&self, &user_key, launch).await?;
            let exec_env = self.config.command_env_for_bundle(
                request.extra_env.clone(),
                &credentials,
                self.managed_egress.as_deref(),
                bundle.as_ref(),
            )?;
            Ok::<_, RuntimeProcessError>((container_name, exec_env))
        }
        .await;
        let (container_name, exec_env) = match setup {
            Ok(setup) => setup,
            Err(setup_error) => {
                if let Some(managed) = self.managed_egress.as_ref() {
                    match managed
                        .rollback_provisioned_bundle(
                            &self.docker,
                            &user_key,
                            &user_key.container_name(),
                        )
                        .await
                    {
                        Ok(()) => credential_cleanup.disarm(),
                        Err(cleanup_error) => {
                            return Err(RuntimeProcessError::ExecutionFailed(format!(
                                "sandbox user container setup failed: {setup_error}; managed-egress rollback failed: {cleanup_error}"
                            )));
                        }
                    }
                }
                return Err(setup_error);
            }
        };
        let result = user_container::execute_in_user_container(
            &self,
            &user_key,
            &container_name,
            user_container::UserContainerCommand {
                command: request.executable,
                args: request.args,
                workdir,
                env: exec_env,
                timeout,
            },
        )
        .await;
        let cleanup = credential_cleanup.cleanup().await;
        drop(activity);
        match (result, cleanup) {
            (Ok(output), Ok(())) => Ok(output),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(cleanup_error)) => Err(RuntimeProcessError::ExecutionFailed(format!(
                "{error}; sandbox credential cleanup failed: {cleanup_error}"
            ))),
        }
    }
}

async fn acquire_user_lifecycle_gate(
    gate: Arc<tokio::sync::Mutex<()>>,
    timeout: Duration,
) -> Result<tokio::sync::OwnedMutexGuard<()>, RuntimeProcessError> {
    tokio::time::timeout(timeout, gate.lock_owned())
        .await
        .map_err(|error| {
            tracing::debug!(?error, "sandbox user lifecycle gate acquisition timed out");
            RuntimeProcessError::ExecutionFailed(
                "sandbox is busy: timed out waiting for another command for this user".to_string(),
            )
        })
}

#[async_trait]
impl SandboxCommandTransport for RebornScopedSandboxCommandTransport {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        let execution = tokio::spawn(
            self.clone()
                .run_command_owned(SandboxExecutionRequest::from(request), Vec::new()),
        );
        execution.await.map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox command execution task failed: {error}"
            ))
        })?
    }

    async fn run_credentialed_command(
        &self,
        request: CredentialedSandboxCommandRequest,
        credentials: Vec<SandboxCommandCredential>,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        if self.managed_egress.is_none() {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox transport requires managed egress for credential bindings".to_string(),
            ));
        }
        let execution = tokio::spawn(
            self.clone()
                .run_command_owned(SandboxExecutionRequest::from(request), credentials),
        );
        execution.await.map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox credentialed command execution task failed: {error}"
            ))
        })?
    }

    fn supports_credentialed_command(&self) -> bool {
        self.managed_egress.is_some()
    }

    async fn shutdown(&self) -> Result<(), RuntimeProcessError> {
        RebornScopedSandboxCommandTransport::shutdown(self).await;
        Ok(())
    }
}

// Kept as the crate-local seam used by Docker-backed tests. Connection policy
// lives exclusively in the `connect` module; do not add discovery or timeout
// behavior here.
async fn connect_docker() -> Result<Docker, RuntimeProcessError> {
    connect_docker_with_retry().await
}

fn security_posture_stamp(
    config: &RebornSandboxConfig,
    container_user: &str,
    security: &worker_spec::DockerWorkerSecuritySpec,
    binds: &[String],
    env: &[String],
    egress_posture: Option<&str>,
) -> String {
    let framed_binds = frame_string_values("bind", binds);
    let mut sorted_env = env.to_vec();
    sorted_env.sort();
    let framed_env = frame_string_values("env", &sorted_env);
    let posture = key_codec::encode_parts(&[
        ("generation", "user-exec-v1".to_string()),
        ("container_user", container_user.to_string()),
        (
            "workspace_mode",
            config.container_identity.workspace_mode().to_string(),
        ),
        ("memory_bytes", config.memory_bytes.to_string()),
        ("cpu_shares", config.cpu_shares.to_string()),
        (
            "network_mode",
            security
                .network_mode()
                .unwrap_or_else(|| "default".to_string()),
        ),
        ("cap_drop", frame_string_values("cap", &security.cap_drop())),
        (
            "security_opt",
            frame_string_values("option", &security.security_options()),
        ),
        ("readonly_rootfs", security.readonly_rootfs().to_string()),
        ("pids_limit", security.pids_limit().to_string()),
        ("nano_cpus", security.nano_cpus().to_string()),
        ("tmpfs", security.tmpfs_options()),
        ("log_driver", security.log_driver()),
        (
            "log_options",
            frame_string_values(
                "option",
                &security
                    .log_options()
                    .into_iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>(),
            ),
        ),
        ("binds", framed_binds),
        ("env", framed_env),
        (
            "egress_posture",
            egress_posture.unwrap_or("disabled").to_string(),
        ),
    ]);
    key_codec::digest_hex(&posture)
}

fn frame_string_values(label: &str, values: &[String]) -> String {
    key_codec::encode_parts(
        &values
            .iter()
            .map(|value| (label, value.clone()))
            .collect::<Vec<_>>(),
    )
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

fn validate_env(env: &HashMap<String, String>) -> Result<Vec<String>, RuntimeProcessError> {
    validate_environment_entries(env, &[])?;
    let mut rendered = env
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>();
    rendered.sort();
    Ok(rendered)
}

fn validate_credential_env(
    env: &HashMap<String, String>,
    credentials: &[SandboxCommandCredential],
) -> Result<(), RuntimeProcessError> {
    let placeholders = credentials
        .iter()
        .map(|credential| credential.placeholder.as_str())
        .collect::<Vec<_>>();
    validate_environment_entries(env, &placeholders)?;

    let mut names = HashSet::with_capacity(credentials.len());
    for credential in credentials {
        if credential.placeholder_env.is_empty()
            || credential.placeholder_env.contains(['=', '\0'])
            || credential.placeholder.contains('\0')
            || !names.insert(credential.placeholder_env.as_str())
            || !credential
                .placeholder
                .starts_with(ironclaw_secrets::CREDENTIAL_PLACEHOLDER_PREFIX)
            || env.get(&credential.placeholder_env) != Some(&credential.placeholder)
        {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox credential placeholders must match credential bindings".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_environment_entries(
    env: &HashMap<String, String>,
    allowed_placeholders: &[&str],
) -> Result<(), RuntimeProcessError> {
    for (name, value) in env {
        validate_env_name(name).map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox command environment is invalid: {error}"
            ))
        })?;
        if value.contains('\0') {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "sandbox command environment is invalid for {name}"
            )));
        }
    }
    validate_env_has_no_raw_sensitive_values(env, allowed_placeholders).map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox command environment is invalid: {error}"
        ))
    })
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
    use ironclaw_common::env_helpers::{lock_env, remove_runtime_env, set_runtime_env};
    use ironclaw_host_api::{
        action::{NetworkPolicy, NetworkTargetPattern},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };
    fn inert_docker_client() -> Docker {
        Docker::connect_with_http("http://127.0.0.1:9", 1, bollard::API_DEFAULT_VERSION)
            .expect("construct inert Docker test client")
    }

    #[test]
    fn transport_constructor_uses_canonical_bounded_docker_connector() {
        let _guard = lock_env();
        set_runtime_env(
            "IRONCLAW_REBORN_DOCKER_HOST",
            "/nonexistent/ironclaw-constructor-docker.sock",
        );

        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime for test")
            .block_on(RebornScopedSandboxCommandTransport::connect(
                RebornSandboxConfig::new("/tmp/reborn-sandbox-constructor-test"),
            ));

        remove_runtime_env("IRONCLAW_REBORN_DOCKER_HOST");

        let error = result.expect_err("nonexistent Docker override must fail closed");
        assert!(
            error.to_string().contains("IRONCLAW_REBORN_DOCKER_HOST"),
            "constructor must use the canonical connector, including its bounded retry and override handling: {error}"
        );
    }

    #[test]
    fn test_constructor_works_without_tokio_runtime() {
        let docker = inert_docker_client();
        let _transport = test_support::transport(
            docker,
            RebornSandboxConfig::new("/tmp/reborn-sandbox-pure-constructor"),
        );
    }
    #[test]
    fn command_environment_accepts_valid_non_sensitive_values() {
        let env = HashMap::from([
            ("RUST_LOG".to_string(), "debug".to_string()),
            ("FEATURE_FLAG".to_string(), "enabled".to_string()),
        ]);

        assert_eq!(
            validate_env(&env).unwrap(),
            ["FEATURE_FLAG=enabled", "RUST_LOG=debug"]
        );
    }

    #[test]
    fn credential_environment_allows_benign_values_beside_bound_placeholders() {
        let placeholder = format!("{}test", ironclaw_secrets::CREDENTIAL_PLACEHOLDER_PREFIX);
        let env = HashMap::from([
            ("GH_TOKEN".to_string(), placeholder.clone()),
            ("RUST_LOG".to_string(), "debug".to_string()),
        ]);
        let credentials = [SandboxCommandCredential::new(
            ironclaw_host_api::ids::SecretHandle::new("github_runtime_token").unwrap(),
            "GH_TOKEN".to_string(),
            placeholder,
            "api.github.com".to_string(),
            "Authorization".to_string(),
            Some("token ".to_string()),
            "secret".to_string(),
        )];

        validate_credential_env(&env, &credentials).unwrap();
    }

    #[tokio::test]
    async fn lifecycle_gate_wait_is_bounded() {
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        let _held = gate.clone().lock_owned().await;

        let error = acquire_user_lifecycle_gate(gate, Duration::from_millis(1))
            .await
            .expect_err("contended lifecycle gate must time out");

        assert_eq!(
            error,
            RuntimeProcessError::ExecutionFailed(
                "sandbox is busy: timed out waiting for another command for this user".to_string()
            )
        );
    }

    #[tokio::test]
    async fn cancelled_credential_task_detaches_cleanup_while_holding_lifecycle_gate() {
        let directory = tempfile::tempdir().unwrap();
        let credential_bundle = directory.path().join("credentials.json");
        tokio::fs::write(&credential_bundle, br#"{"atlas":"secret"}"#)
            .await
            .unwrap();
        let managed = ManagedEgressRuntime::test_runtime(
            NetworkPolicy {
                allowed_targets: vec![NetworkTargetPattern {
                    scheme: None,
                    host_pattern: "github.com".to_string(),
                    port: None,
                }],
                deny_private_ip_ranges: true,
                max_egress_bytes: None,
            },
            directory.path().to_path_buf(),
        )
        .unwrap();
        let bundle = ManagedEgressBundle::test_bundle(directory.path());
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        let task_gate = Arc::clone(&gate);
        let (armed_tx, armed_rx) = tokio::sync::oneshot::channel();
        let execution = tokio::spawn(async move {
            let lifecycle = task_gate.lock_owned().await;
            let mut cleanup = CredentialCleanupGuard::new(inert_docker_client(), lifecycle);
            cleanup.arm(managed, bundle);
            armed_tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        armed_rx.await.unwrap();

        execution.abort();
        assert!(execution.await.unwrap_err().is_cancelled());
        let lifecycle = tokio::time::timeout(Duration::from_secs(2), gate.lock_owned())
            .await
            .expect("detached credential cleanup must release the lifecycle gate");
        drop(lifecycle);

        assert!(!credential_bundle.exists());
    }

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
    fn managed_egress_replaces_the_direct_network_posture() {
        let config = RebornSandboxConfig::new("/tmp/reborn-sandbox")
            .with_managed_egress_proxy()
            .unwrap();

        assert_eq!(config.container_network_mode().as_deref(), Some("none"));
        assert!(config.network_broker.is_none());
        assert!(config.managed_egress.is_some());
    }

    #[test]
    fn managed_egress_and_network_broker_are_mutually_exclusive_in_both_orders() {
        let broker_then_managed = RebornSandboxConfig::new("/tmp/reborn-sandbox")
            .with_network_broker_unix_socket("/tmp/reborn-http-broker.sock")
            .unwrap()
            .with_managed_egress_proxy()
            .unwrap_err();
        assert!(
            broker_then_managed
                .to_string()
                .contains("mutually exclusive")
        );

        let managed_then_broker = RebornSandboxConfig::new("/tmp/reborn-sandbox")
            .with_managed_egress_proxy()
            .unwrap()
            .with_network_broker_unix_socket("/tmp/reborn-http-broker.sock")
            .unwrap_err();
        assert!(
            managed_then_broker
                .to_string()
                .contains("mutually exclusive")
        );
    }

    #[test]
    fn validate_credential_env_accepts_valid_caller_values_and_bound_placeholders() {
        let caller_env = HashMap::from([("FEATURE_FLAG".to_string(), "enabled".to_string())]);
        assert_eq!(validate_credential_env(&caller_env, &[]), Ok(()));
        assert_eq!(validate_credential_env(&HashMap::new(), &[]), Ok(()));

        let placeholder = format!("{}test", ironclaw_secrets::CREDENTIAL_PLACEHOLDER_PREFIX);
        let credential = SandboxCommandCredential::new(
            ironclaw_host_api::ids::SecretHandle::new("github_runtime_token").unwrap(),
            "GH_TOKEN".to_string(),
            placeholder.clone(),
            "api.github.com".to_string(),
            "Authorization".to_string(),
            Some("Bearer ".to_string()),
            "real-secret".to_string(),
        );
        let bound_env = HashMap::from([("GH_TOKEN".to_string(), placeholder.clone())]);
        assert_eq!(
            validate_credential_env(&bound_env, std::slice::from_ref(&credential)),
            Ok(())
        );

        let wrong_placeholder =
            HashMap::from([("GH_TOKEN".to_string(), "caller-value".to_string())]);
        assert!(
            validate_credential_env(&wrong_placeholder, std::slice::from_ref(&credential)).is_err()
        );
        let extra_env = HashMap::from([
            ("GH_TOKEN".to_string(), placeholder),
            ("UNBOUND".to_string(), "value".to_string()),
        ]);
        assert_eq!(validate_credential_env(&extra_env, &[credential]), Ok(()));
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
    fn broker_env_rejects_all_caller_overrides_before_reserved_env_is_built() {
        let config = RebornSandboxConfig::new("/tmp/reborn-sandbox")
            .with_network_broker_unix_socket("/tmp/reborn-http-broker.sock")
            .unwrap()
            .with_secret_broker_url("https://broker.internal/secrets")
            .unwrap();
        for key in broker::RESERVED_BROKER_ENV_KEYS {
            assert!(
                config
                    .command_env(HashMap::from([(
                        (*key).to_string(),
                        "caller-controlled".to_string(),
                    )]))
                    .is_err(),
                "{key}"
            );
        }
    }

    #[test]
    fn managed_bundle_rejects_reserved_broker_environment_overrides() {
        let directory = tempfile::tempdir().unwrap();
        let managed = ManagedEgressRuntime::test_runtime(
            NetworkPolicy {
                allowed_targets: vec![NetworkTargetPattern {
                    scheme: None,
                    host_pattern: "example.com".to_string(),
                    port: None,
                }],
                deny_private_ip_ranges: true,
                max_egress_bytes: None,
            },
            directory.path().to_path_buf(),
        )
        .unwrap();
        let bundle = ManagedEgressBundle::test_bundle(directory.path());
        let config = RebornSandboxConfig::new("/tmp/reborn-sandbox");

        for key in broker::RESERVED_BROKER_ENV_KEYS {
            assert!(
                config
                    .command_env_for_bundle(
                        HashMap::from([((*key).to_string(), "caller-controlled".to_string(),)]),
                        &[],
                        Some(&managed),
                        Some(&bundle),
                    )
                    .is_err(),
                "{key}"
            );
        }
    }

    #[test]
    fn broker_endpoints_reject_credentials_fragments_control_characters_and_bad_schemes() {
        assert!(RebornSandboxSecretBroker::new("https://broker.internal/\nsecrets").is_err());
        assert!(RebornSandboxSecretBroker::new("https://token@broker.internal/secrets").is_err());
        assert!(RebornSandboxSecretBroker::new("https://broker.internal/secrets#token").is_err());
        assert!(
            RebornSandboxSecretBroker::new("https://broker.internal/secrets?token=abc").is_err()
        );
        assert!(RebornSandboxNetworkBroker::unix_socket("relative.sock").is_err());
        assert!(RebornSandboxSecretBroker::unix_socket("/tmp/bad:path.sock").is_err());
        assert!(RebornSandboxNetworkBroker::unix_socket("/tmp/bad\npath.sock").is_err());
        assert!(RebornSandboxSecretBroker::unix_socket("/tmp/bad\tpath.sock").is_err());
    }

    #[tokio::test]
    async fn container_launch_config_applies_unix_socket_broker_env_binds_and_none_network() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let network_socket = temp.path().join("network-broker.sock");
        let secret_socket = temp.path().join("secret-broker.sock");
        let config = RebornSandboxConfig::new(temp.path().join("workspaces"))
            .with_network_broker_unix_socket(&network_socket)
            .unwrap()
            .with_secret_broker_unix_socket(&secret_socket)
            .unwrap();
        let transport = test_support::transport(inert_docker_client(), config);
        // Config-shape tests inject an immutable identity directly; only the
        // real run path resolves the configured reference through Docker.
        let container_user = transport
            .config
            .container_identity
            .container_user(&workspace)
            .await
            .unwrap();
        let mut binds = transport
            .config
            .mount_sources
            .prepare_container_binds(&workspace, None)
            .await
            .unwrap()
            .into_iter()
            .map(|bind| bind.into_docker_bind())
            .collect::<Vec<_>>();
        transport.config.append_broker_binds(&mut binds).unwrap();
        binds.sort();
        let launch = transport
            .user_container_launch_config(
                &sandbox_scope(),
                "sha256:test-worker",
                None,
                container_user,
                binds,
            )
            .unwrap();
        let launch = launch.config;
        assert_eq!(launch.image.as_deref(), Some("sha256:test-worker"));
        let host_config = launch.host_config.unwrap();
        let binds = host_config.binds.unwrap();
        let env = launch.env.unwrap();
        assert!(
            launch.cmd.is_none(),
            "image default idle command must be preserved"
        );

        assert_eq!(host_config.network_mode, Some("none".to_string()));
        assert_eq!(
            host_config.nano_cpus,
            Some(worker_spec::DOCKER_WORKER_NANO_CPUS)
        );
        assert_eq!(
            host_config
                .tmpfs
                .as_ref()
                .and_then(|tmpfs| tmpfs.get("/tmp"))
                .map(String::as_str),
            Some(worker_spec::DOCKER_WORKER_TMPFS)
        );
        let log_config = host_config
            .log_config
            .as_ref()
            .expect("bounded Docker logs");
        assert_eq!(log_config.typ.as_deref(), Some("json-file"));
        assert_eq!(
            log_config
                .config
                .as_ref()
                .and_then(|options| options.get("max-size"))
                .map(String::as_str),
            Some("1m")
        );
        assert!(
            launch
                .user
                .as_deref()
                .is_some_and(|user| !user.starts_with("0:"))
        );
        assert!(env.contains(
            &"IRONCLAW_REBORN_HTTP_BROKER_SOCKET=/tmp/ironclaw-http-broker.sock".to_string()
        ));
        assert!(env.contains(
            &"IRONCLAW_REBORN_SECRET_BROKER_SOCKET=/tmp/ironclaw-secret-broker.sock".to_string()
        ));
        assert!(binds.contains(&format!("{}:/workspace:rw", workspace.display())));
        assert!(binds.contains(&format!(
            "{}:/tmp/ironclaw-http-broker.sock:rw",
            network_socket.display()
        )));
        assert!(binds.contains(&format!(
            "{}:/tmp/ironclaw-secret-broker.sock:rw",
            secret_socket.display()
        )));
    }

    #[tokio::test]
    async fn run_command_rejects_timeout_above_exec_helper_limit_before_docker_io() {
        let temp = tempfile::tempdir().unwrap();
        let unavailable = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", unavailable.local_addr().unwrap());
        let docker = Docker::connect_with_http(&endpoint, 1, bollard::API_DEFAULT_VERSION).unwrap();
        let transport = test_support::transport(
            docker,
            RebornSandboxConfig::new(temp.path().join("workspaces")),
        );

        let error = transport
            .run_command(CommandExecutionRequest {
                scope: sandbox_scope(),
                mounts: None,
                command: "true".to_string(),
                workdir: None,
                timeout_secs: Some(86_401),
                extra_env: HashMap::new(),
            })
            .await
            .unwrap_err();

        assert!(format!("{error}").contains("must not exceed 86400 seconds"));
    }

    #[tokio::test]
    async fn run_command_rejects_unconfigured_scoped_mount_before_container_create() {
        let temp = tempfile::tempdir().unwrap();
        let docker = inert_docker_client();
        let transport = test_support::transport(
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
                scope: sandbox_scope(),
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
    fn sandbox_scope() -> ResourceScope {
        ResourceScope::system()
    }

    fn process_read_only_permissions() -> MountPermissions {
        MountPermissions {
            execute: true,
            ..MountPermissions::read_only()
        }
    }
}
