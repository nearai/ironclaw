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
    time::Duration,
};

use async_trait::async_trait;
use bollard::Docker;
use ironclaw_host_api::{MountView, ResourceScope};

use crate::{
    CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError, SandboxCommandTransport,
    TenantSandboxProcessPort,
};

mod broker;
mod connect;
mod container_identity;
mod egress_proxy;
mod exec_transport;
mod mounts;
mod network_allowlist;
mod reaper;
mod registry;
mod scope_key;
pub(crate) mod shell_limits;
mod user_key;

use mounts::RebornSandboxMountSources;
use shell_limits::{clamp_shell_output_limit_bytes, clamp_shell_timeout_secs};

pub use broker::{RebornSandboxNetworkBroker, RebornSandboxSecretBroker};
pub use connect::{SandboxDockerReadiness, connect_docker_with_retry, sandbox_docker_readiness};
pub use container_identity::{RebornSandboxContainerIdentity, RebornSandboxWorkspaceMode};
pub use egress_proxy::{BoundEgressAllowlistProxy, EgressAllowlistProxy, EgressProxyError};
pub use network_allowlist::{
    DEFAULT_SANDBOX_ALLOWED_DOMAINS, SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV, sandbox_allowed_domains,
    sandbox_extra_allowed_domains, sandbox_network_policy,
};
pub use reaper::{ReapSummary, SandboxReaper, SandboxReaperConfig};
use registry::BackgroundJobRegistry;
pub use registry::SandboxActivityRegistry;
pub use scope_key::RebornSandboxScopeKey;
pub use user_key::RebornSandboxUserKey;

/// Docker label prefix for container metadata attached by
/// [`RebornScopedSandboxCommandTransport`] — shared with [`reaper`] so the
/// reaper's container-listing filter and this transport's container-creation
/// labels never drift apart.
const LABEL_PREFIX: &str = "ironclaw";

const DEFAULT_IMAGE: &str = "ironclaw-worker:latest";
// Sourced from `shell_limits` so the config-level default and the per-call
// clamp default (used when the model omits `timeout`/`output_limit`) can
// never drift apart. The per-call ceilings (`SHELL_TIMEOUT_MAX_SECS`,
// `SHELL_OUTPUT_LIMIT_MAX_BYTES`) are applied in `execute_in_container`.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(shell_limits::SHELL_TIMEOUT_DEFAULT_SECS);
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

    /// Docker `--network` value for the sandbox container.
    ///
    /// - `disable_network: false` (`with_network_enabled`, unused in
    ///   production today): `None` (Docker default bridge) — a deliberate
    ///   fully-open mode, unrelated to the brokered-egress cases below.
    /// - `disable_network: true` with no broker, or a Unix-socket broker
    ///   (`requires_docker_network() == false`): `Some("none")` — no network
    ///   interfaces at all.
    /// - `disable_network: true` with an HTTP-proxy broker
    ///   (`requires_docker_network() == true`): joins the pinned internal
    ///   network (`broker::SANDBOX_EGRESS_NETWORK_NAME`) instead of the
    ///   default bridge. **E1**: the default bridge NATs to the internet, so
    ///   a container there could dial out directly and ignore the proxy env
    ///   — "proxy-allowlist egress" would be advisory, not enforced. The
    ///   internal network has no route off-host except back to its own
    ///   gateway, where the proxy is reached (see
    ///   `broker::SANDBOX_EGRESS_NETWORK_GATEWAY` and
    ///   `exec_transport::ensure_egress_network`, which creates the network
    ///   idempotently before a container joins it).
    fn container_network_mode(&self) -> Option<String> {
        if !self.disable_network {
            return None;
        }
        let requires_docker_network = self
            .network_broker
            .as_ref()
            .is_some_and(RebornSandboxNetworkBroker::requires_docker_network);
        if requires_docker_network {
            Some(broker::SANDBOX_EGRESS_NETWORK_NAME.to_string())
        } else {
            Some("none".to_string())
        }
    }

    fn command_env(
        &self,
        extra_env: HashMap<String, String>,
    ) -> Result<Vec<String>, RuntimeProcessError> {
        let mut env = validate_env(extra_env)?;
        broker::push_broker_env(
            self.network_broker.as_ref(),
            self.secret_broker.as_ref(),
            &mut env,
        )?;
        Ok(env)
    }

    fn append_broker_binds(&self, binds: &mut Vec<String>) -> Result<(), RuntimeProcessError> {
        broker::append_broker_binds(
            self.network_broker.as_ref(),
            self.secret_broker.as_ref(),
            binds,
        )
    }
}

#[derive(Clone)]
pub struct RebornScopedSandboxCommandTransport {
    docker: Docker,
    config: RebornSandboxConfig,
    activity: Arc<SandboxActivityRegistry>,
    background_jobs: Arc<BackgroundJobRegistry>,
    /// Gates the egress network's idempotent-but-not-free create attempt
    /// (see `exec_transport::ensure_egress_network_once`) to once per
    /// process instead of once per command dispatch.
    network_ready: Arc<tokio::sync::OnceCell<()>>,
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
        let docker = connect_docker_with_retry().await?;
        Ok(Self::new(docker, config))
    }

    pub fn new(docker: Docker, config: RebornSandboxConfig) -> Self {
        Self {
            docker,
            config,
            activity: Arc::new(SandboxActivityRegistry::new()),
            background_jobs: Arc::new(BackgroundJobRegistry::new()),
            network_ready: Arc::new(tokio::sync::OnceCell::new()),
        }
    }

    /// Overrides the default activity registry with one shared elsewhere
    /// (e.g. with a [`SandboxReaper`] instance), so both observe the same
    /// per-user last-activity timestamps. Composition wiring is the
    /// expected caller.
    pub fn with_activity_registry(mut self, activity: Arc<SandboxActivityRegistry>) -> Self {
        self.activity = activity;
        self
    }

    pub fn into_process_port(self) -> TenantSandboxProcessPort {
        TenantSandboxProcessPort::new(Arc::new(self))
    }

    /// Initializes (and returns) the per-user host workspace directory that
    /// backs the persistent container's flat `/workspace` bind — every
    /// thread/project/agent for the same `{tenant, user}` pair shares this
    /// one directory, matching the container reuse in `exec_transport`. Also
    /// seeds `.home` (owner-only) so `HOME=/workspace/.home` (set in
    /// `exec_transport::user_container_launch_config`) always resolves to a
    /// real, private directory.
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
        let home = workspace.join(".home");
        tokio::fs::create_dir_all(&home).await.map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox workspace HOME could not be initialized: {error}"
            ))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700))
                .await
                .map_err(|error| {
                    RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox workspace HOME permissions could not be set: {error}"
                    ))
                })?;
        }
        // Pre-create npm's redirected global prefix (see
        // `exec_transport::user_container_launch_config`'s NPM_CONFIG_PREFIX)
        // so `npm install -g` never trips over a missing directory. It lives
        // inside the already-0o700 `.home`, so no separate chmod is needed.
        let npm_global = home.join(".npm-global");
        tokio::fs::create_dir_all(&npm_global)
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox workspace npm global prefix could not be initialized: {error}"
                ))
            })?;
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
}

#[async_trait]
impl SandboxCommandTransport for RebornScopedSandboxCommandTransport {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        reject_nul("sandbox command", &request.command)?;
        // Phase A scope narrowing: a persistent container's binds are fixed
        // at creation time from the flat per-user `/workspace` bind only
        // (`exec_transport::user_container_launch_config` always resolves
        // binds with `mounts: None`) — a later request naming a scoped
        // `MountView` grant can never be retrofitted onto an already-running
        // container, so it is rejected up front rather than silently
        // ignored.
        reject_non_workspace_mount_grants(request.mounts.as_ref())?;

        let key = RebornSandboxUserKey::from_scope(&request.scope);
        let workspace = self.prepare_workspace(&request.scope).await?;
        let workdir = Self::resolve_container_workdir(request.workdir.as_deref())?;
        // Clamp to `[SHELL_TIMEOUT_MIN_SECS, SHELL_TIMEOUT_MAX_SECS]` — the
        // model-adjustable `timeout` field is bounded by the operator
        // ceiling here rather than rejected when it overshoots.
        let requested_secs = request
            .timeout_secs
            .unwrap_or(self.config.default_timeout.as_secs());
        let timeout = clamp_shell_timeout_secs(Some(requested_secs));
        // Clamp to `[SHELL_OUTPUT_LIMIT_MIN_BYTES, SHELL_OUTPUT_LIMIT_MAX_BYTES]`,
        // falling back to the configured default when the model omits
        // `output_limit`.
        let output_limit = clamp_shell_output_limit_bytes(Some(
            request
                .output_limit_bytes
                .unwrap_or(self.config.max_output_bytes as u64),
        ));
        let env = self.config.command_env(request.extra_env)?;

        let container_id = exec_transport::ensure_container(
            &self.docker,
            &self.config,
            &key,
            &request.scope.tenant_id,
            &request.scope.user_id,
            &workspace,
            &self.network_ready,
        )
        .await?;

        if request.background {
            let command_preview = request.command.clone();
            let launch = exec_transport::exec_background_in_container(
                &self.docker,
                &container_id,
                workdir,
                env,
                request.command,
            )
            .await?;
            self.background_jobs
                .record(&key, launch.pid, command_preview);
            self.activity.touch(&key);
            return Ok(CommandExecutionOutput {
                output: format!(
                    "Started in background: pid {}, log {}",
                    launch.pid, launch.log_path
                ),
                saved_output: None,
                exit_code: 0,
                sandboxed: true,
                duration: Duration::from_secs(0),
            });
        }

        let mut output = exec_transport::exec_in_container(
            &self.docker,
            &container_id,
            workdir,
            env,
            request.command,
            timeout,
            output_limit,
        )
        .await?;
        self.activity.touch(&key);
        self.reconcile_background_jobs(&container_id, &key).await;
        output
            .output
            .push_str(&exec_transport::render_background_footer(
                &self.background_jobs.jobs_for(&key),
            ));
        Ok(output)
    }
}

impl RebornScopedSandboxCommandTransport {
    /// Cheap `ps -o pid=` sweep against the tracked background pids so a
    /// process that has since exited drops off the footer instead of being
    /// reported as still live forever.
    async fn reconcile_background_jobs(&self, container_id: &str, key: &RebornSandboxUserKey) {
        let tracked = self.background_jobs.jobs_for(key);
        if tracked.is_empty() {
            return;
        }
        let alive = exec_transport::exec_in_container(
            &self.docker,
            container_id,
            ContainerWorkdir::workspace_root(),
            Vec::new(),
            "ps -o pid= --no-headers".to_string(),
            Duration::from_secs(5),
            4096,
        )
        .await;
        let Ok(alive) = alive else {
            return;
        };
        let alive_pids: Vec<u32> = alive
            .output
            .split_whitespace()
            .filter_map(|token| token.trim().parse::<u32>().ok())
            .collect();
        self.background_jobs.drop_dead(key, &alive_pids);
    }
}

/// Phase A scope-narrowing guard: persistent per-user containers only
/// support the default `/workspace` bind (see `run_command` above), so any
/// caller-supplied `MountView` grant — scoped or not — is rejected before
/// the container is ever touched, with a clear error, rather than being
/// silently dropped by `exec_transport::user_container_launch_config`'s
/// hardcoded `mounts: None`.
fn reject_non_workspace_mount_grants(
    mounts: Option<&MountView>,
) -> Result<(), RuntimeProcessError> {
    let Some(mounts) = mounts else {
        return Ok(());
    };
    if mounts.mounts.is_empty() {
        return Ok(());
    }
    Err(RuntimeProcessError::ExecutionFailed(
        "sandbox command rejected: persistent per-user sandbox containers only support the \
         default /workspace bind in Phase A; scoped mount grants are not supported"
            .to_string(),
    ))
}

pub(super) fn append_with_limit(buffer: &mut String, text: &str, limit: usize) {
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

/// Wraps `value` in single quotes, escaping any embedded single quote so the
/// result is safe to interpolate into a `sh -c '...'` argument. The one
/// shell-quoting implementation in this crate — `exec_transport`'s
/// pgid-isolation wrapper calls this instead of hand-rolling its own.
pub(crate) fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod shell_quote_tests {
    use super::*;

    #[test]
    fn shell_single_quote_escapes_embedded_single_quotes() {
        assert_eq!(shell_single_quote("echo hi"), "'echo hi'");
        assert_eq!(shell_single_quote("it's"), "'it'\\''s'");
    }
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
    fn validate_env_rejects_empty_equals_and_nul_values() {
        for (key, value) in [
            ("", "value"),
            ("BAD=KEY", "value"),
            ("BAD\0KEY", "value"),
            ("GOOD_KEY", "bad\0value"),
        ] {
            let error = validate_env(HashMap::from([(key.to_string(), value.to_string())]))
                .expect_err("invalid env should be rejected");
            assert!(matches!(error, RuntimeProcessError::ExecutionFailed(_)));
        }
    }

    #[test]
    fn network_broker_proxy_url_joins_internal_egress_network_not_default_bridge() {
        let config = RebornSandboxConfig::new("/tmp/reborn-sandbox")
            .with_network_broker_proxy_url("http://broker.internal:8181")
            .unwrap();
        let env = config.command_env(HashMap::new()).unwrap();

        // E1: an HTTP-proxy broker must NOT leave the container on Docker's
        // default bridge (which NATs to the internet) — it joins the
        // pinned internal network instead.
        assert_eq!(
            config.container_network_mode(),
            Some(broker::SANDBOX_EGRESS_NETWORK_NAME.to_string())
        );
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
    fn network_broker_port_uses_pinned_internal_network_gateway_proxy_url() {
        let config = RebornSandboxConfig::new("/tmp/reborn-sandbox").with_network_broker_port(8181);
        let env = config.command_env(HashMap::new()).unwrap();
        let proxy_url = format!("http://{}:8181", broker::SANDBOX_EGRESS_NETWORK_GATEWAY);

        // E1: the default-port broker's proxy URL must point at the pinned
        // internal network's gateway (reachable once the container joins
        // `SANDBOX_EGRESS_NETWORK_NAME`), not the Docker default-bridge
        // host-gateway address — that was the E1 hole (default bridge NATs
        // to the internet).
        assert!(env.contains(&format!("IRONCLAW_REBORN_HTTP_PROXY={proxy_url}")));
        assert!(env.contains(&format!("http_proxy={proxy_url}")));
        assert_eq!(
            config.container_network_mode(),
            Some(broker::SANDBOX_EGRESS_NETWORK_NAME.to_string())
        );
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
    fn broker_env_rejects_all_reserved_user_overrides() {
        let config = RebornSandboxConfig::new("/tmp/reborn-sandbox")
            .with_network_broker_proxy_url("http://broker.internal:8181")
            .unwrap()
            .with_secret_broker_url("https://broker.internal/secrets")
            .unwrap();
        for key in broker::RESERVED_BROKER_ENV_KEYS {
            let error = config
                .command_env(HashMap::from([(
                    (*key).to_string(),
                    "caller-controlled".to_string(),
                )]))
                .unwrap_err();

            assert!(format!("{error}").contains("reserved"), "{key}");
        }
    }

    #[test]
    fn broker_urls_reject_credentials_fragments_control_characters_and_non_http_schemes() {
        assert!(RebornSandboxNetworkBroker::new("unix:///tmp/broker.sock").is_err());
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
    async fn user_container_launch_config_applies_unix_socket_broker_env_binds_and_none_network() {
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
        let tenant = ironclaw_host_api::TenantId::new("tenant-a").unwrap();
        let user = ironclaw_host_api::UserId::new("user-a").unwrap();

        let launch =
            exec_transport::user_container_launch_config(&config, &tenant, &user, &workspace)
                .await
                .unwrap();
        let host_config = launch.host_config.unwrap();
        let binds = host_config.binds.unwrap();
        let env = launch.env.unwrap();

        assert_eq!(host_config.network_mode, Some("none".to_string()));
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
    async fn user_container_launch_config_applies_http_proxy_broker_env_and_joins_internal_egress_network()
     {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let config = RebornSandboxConfig::new(temp.path().join("workspaces"))
            .with_network_broker_proxy_url("http://broker.internal:8181")
            .unwrap();
        let tenant = ironclaw_host_api::TenantId::new("tenant-a").unwrap();
        let user = ironclaw_host_api::UserId::new("user-a").unwrap();

        let launch =
            exec_transport::user_container_launch_config(&config, &tenant, &user, &workspace)
                .await
                .unwrap();
        let host_config = launch.host_config.unwrap();
        let binds = host_config.binds.unwrap();
        let env = launch.env.unwrap();

        // E1: the applied Docker HostConfig must attach to the pinned
        // internal egress network, never silently fall back to the default
        // bridge (which would NAT to the internet and defeat the proxy
        // allowlist).
        assert_eq!(
            host_config.network_mode,
            Some(broker::SANDBOX_EGRESS_NETWORK_NAME.to_string())
        );
        assert!(env.contains(&"IRONCLAW_REBORN_NETWORK_MODE=brokered".to_string()));
        assert!(env.contains(&"http_proxy=http://broker.internal:8181".to_string()));
        assert!(env.contains(&"HTTPS_PROXY=http://broker.internal:8181".to_string()));
        assert!(binds.contains(&format!("{}:/workspace:rw", workspace.display())));
        assert!(
            binds
                .iter()
                .all(|bind| !bind.contains("ironclaw-http-broker.sock"))
        );
    }

    #[test]
    fn reject_non_workspace_mount_grants_allows_none_and_empty_but_rejects_any_grant() {
        assert!(reject_non_workspace_mount_grants(None).is_ok());
        assert!(reject_non_workspace_mount_grants(Some(&MountView::default())).is_ok());

        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/app").unwrap(),
            process_read_only_permissions(),
        )])
        .unwrap();
        let error = reject_non_workspace_mount_grants(Some(&mounts)).unwrap_err();

        assert!(format!("{error}").contains("scoped mount grants are not supported"));
    }

    #[tokio::test]
    async fn run_command_rejects_any_scoped_mount_grant_before_container_touch() {
        let temp = tempfile::tempdir().unwrap();
        // `run_command` must reject a scoped `MountView` grant as a pure
        // precondition, before any Docker client use — so this test must
        // not require a live daemon either. `connect_with_local_defaults`
        // stats the Unix socket at construction and fails immediately
        // without one; the HTTP-transport client performs no I/O until a
        // request is sent (see `ensure_egress_network_is_a_no_op_for_none_
        // network_configs` in `exec_transport.rs` for the same pattern).
        let docker =
            Docker::connect_with_http("http://127.0.0.1:0", 120, bollard::API_DEFAULT_VERSION)
                .expect("HTTP-transport client construction performs no I/O");
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
                output_limit_bytes: None,
                background: false,
            })
            .await
            .unwrap_err();

        assert!(format!("{error}").contains("scoped mount grants are not supported"));
    }

    fn process_read_only_permissions() -> MountPermissions {
        MountPermissions {
            execute: true,
            ..MountPermissions::read_only()
        }
    }
}
