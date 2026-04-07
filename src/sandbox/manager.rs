//! Main sandbox manager coordinating proxy and containers.
//!
//! The `SandboxManager` is the primary entry point for sandboxed execution.
//! It coordinates:
//! - Docker container creation and lifecycle
//! - HTTP proxy for network access control
//! - Credential injection for API calls
//! - Resource limits and timeouts
//!
//! # Architecture
//!
//! ```text
//! ┌───────────────────────────────────────────────────────────────────────────┐
//! │                           SandboxManager                                   │
//! │                                                                            │
//! │   execute(cmd, cwd, policy)                                                │
//! │         │                                                                  │
//! │         ▼                                                                  │
//! │   ┌──────────────┐     ┌──────────────┐     ┌──────────────────────────┐  │
//! │   │ Start Proxy  │────▶│ Create       │────▶│ Execute & Collect Output │  │
//! │   │ (if needed)  │     │ Container    │     │                          │  │
//! │   └──────────────┘     └──────────────┘     └──────────────────────────┘  │
//! │                                                        │                   │
//! │                                                        ▼                   │
//! │                                              ┌──────────────────────────┐  │
//! │                                              │ Cleanup Container        │  │
//! │                                              └──────────────────────────┘  │
//! └───────────────────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use bollard::Docker;
use bollard::container::RemoveContainerOptions;

use crate::sandbox::config::{ResourceLimits, SandboxConfig, SandboxPolicy};
use crate::sandbox::container::{ContainerOutput, ContainerRunner, connect_docker};
use crate::sandbox::error::{Result, SandboxError};
use crate::sandbox::proxy::{HttpProxy, NetworkProxyBuilder};

/// Output from sandbox execution.
#[derive(Debug, Clone)]
pub struct ExecOutput {
    /// Exit code from the command.
    pub exit_code: i64,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// Combined output (stdout + stderr).
    pub output: String,
    /// How long the command ran.
    pub duration: Duration,
    /// Whether output was truncated.
    pub truncated: bool,
}

impl From<ContainerOutput> for ExecOutput {
    fn from(c: ContainerOutput) -> Self {
        let output = if c.stderr.is_empty() {
            c.stdout.clone()
        } else if c.stdout.is_empty() {
            c.stderr.clone()
        } else {
            format!("{}\n\n--- stderr ---\n{}", c.stdout, c.stderr)
        };

        Self {
            exit_code: c.exit_code,
            stdout: c.stdout,
            stderr: c.stderr,
            output,
            duration: c.duration,
            truncated: c.truncated,
        }
    }
}

/// State for a persistent session container.
#[derive(Clone)]
struct SessionContainer {
    id: String,
    /// Host path mounted at `/workspace` inside the container.
    base_cwd: PathBuf,
}

/// Main sandbox manager.
pub struct SandboxManager {
    config: SandboxConfig,
    proxy: Arc<RwLock<Option<HttpProxy>>>,
    docker: Arc<RwLock<Option<Docker>>>,
    initialized: std::sync::atomic::AtomicBool,
    /// Container ID and base workspace path for persistent session mode.
    session_container: Arc<RwLock<Option<SessionContainer>>>,
}

impl SandboxManager {
    /// Create a new sandbox manager.
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config,
            proxy: Arc::new(RwLock::new(None)),
            docker: Arc::new(RwLock::new(None)),
            initialized: std::sync::atomic::AtomicBool::new(false),
            session_container: Arc::new(RwLock::new(None)),
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(SandboxConfig::default())
    }

    /// Check if the sandbox is available (Docker running, etc.).
    pub async fn is_available(&self) -> bool {
        if !self.config.enabled {
            return false;
        }

        match connect_docker().await {
            Ok(docker) => docker.ping().await.is_ok(),
            Err(_) => false,
        }
    }

    /// Initialize the sandbox (connect to Docker, start proxy).
    pub async fn initialize(&self) -> Result<()> {
        if self.initialized.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }

        if !self.config.enabled {
            return Err(SandboxError::Config {
                reason: "sandbox is disabled".to_string(),
            });
        }

        // Connect to Docker
        let docker = connect_docker().await?;

        // Check if Docker is responsive
        docker
            .ping()
            .await
            .map_err(|e| SandboxError::DockerNotAvailable {
                reason: e.to_string(),
            })?;

        // Check for / pull image using a temporary runner
        let checker = ContainerRunner::new(
            docker.clone(),
            self.config.image.clone(),
            self.config.proxy_port,
        );
        if !checker.image_exists().await {
            if self.config.auto_pull_image {
                checker.pull_image().await?;
            } else {
                return Err(SandboxError::ContainerCreationFailed {
                    reason: format!(
                        "image {} not found and auto_pull is disabled",
                        self.config.image
                    ),
                });
            }
        }

        *self.docker.write().await = Some(docker);

        // Start the network proxy if we're using a sandboxed policy.
        // Skip when host networking is enabled — the container shares the
        // host's network namespace, so the proxy is unnecessary.
        if self.config.policy.is_sandboxed() && !self.config.host_network {
            let proxy = NetworkProxyBuilder::from_config(&self.config)
                .build_and_start(self.config.proxy_port)
                .await?;

            *self.proxy.write().await = Some(proxy);
        }

        self.initialized
            .store(true, std::sync::atomic::Ordering::SeqCst);

        tracing::info!("Sandbox initialized");
        Ok(())
    }

    /// Shutdown the sandbox (stop proxy, remove session container, clean up).
    pub async fn shutdown(&self) {
        // Remove persistent session container if one was created.
        if let Some(session) = self.session_container.write().await.take()
            && let Some(docker) = self.docker.read().await.as_ref()
        {
            let _ = Self::force_remove_container(docker, &session.id).await;
            tracing::info!(
                container = &session.id[..session.id.len().min(12)],
                "Removed persistent session container"
            );
        }

        if let Some(proxy) = self.proxy.write().await.take() {
            proxy.stop().await;
        }

        self.initialized
            .store(false, std::sync::atomic::Ordering::SeqCst);

        tracing::debug!("Sandbox shut down");
    }

    /// Execute a command in the sandbox.
    pub async fn execute(
        &self,
        command: &str,
        cwd: &Path,
        env: HashMap<String, String>,
    ) -> Result<ExecOutput> {
        self.execute_with_policy(command, cwd, self.config.policy, env)
            .await
    }

    /// Execute a command with a specific policy.
    pub async fn execute_with_policy(
        &self,
        command: &str,
        cwd: &Path,
        policy: SandboxPolicy,
        env: HashMap<String, String>,
    ) -> Result<ExecOutput> {
        // FullAccess policy bypasses the sandbox entirely.
        // Double-check the allow_full_access guard at execution time as well,
        // in case the policy was overridden per-call via execute_with_policy().
        if policy == SandboxPolicy::FullAccess {
            if !self.config.allow_full_access {
                tracing::error!(
                    "FullAccess execution requested but SANDBOX_ALLOW_FULL_ACCESS is not \
                     enabled. Refusing to execute on host. Falling back to error."
                );
                return Err(SandboxError::Config {
                    reason: "FullAccess policy requires SANDBOX_ALLOW_FULL_ACCESS=true".to_string(),
                });
            }
            // Log only the binary name to avoid leaking secrets embedded in
            // command arguments (e.g. tokens in curl headers).
            let binary = command.split_whitespace().next().unwrap_or("<empty>");
            tracing::warn!(
                binary = %binary,
                cwd = %cwd.display(),
                "[FullAccess] Executing command directly on host (no sandbox isolation)"
            );
            return self.execute_direct(command, cwd, env).await;
        }

        // Ensure we're initialized
        if !self.initialized.load(std::sync::atomic::Ordering::SeqCst) {
            self.initialize().await?;
        }

        // Persistent mode: reuse a long-lived session container.
        if self.config.persistent {
            return self
                .retry_transient(|| self.execute_in_session_container(command, cwd, env.clone()))
                .await;
        }

        // Ephemeral mode: one container per command, with transient retry.
        self.retry_transient(|| self.try_execute_in_container(command, cwd, policy, env.clone()))
            .await
    }

    /// Single attempt at container execution (no retry logic).
    async fn try_execute_in_container(
        &self,
        command: &str,
        cwd: &Path,
        policy: SandboxPolicy,
        env: HashMap<String, String>,
    ) -> Result<ExecOutput> {
        let runner = self.make_runner().await?;
        let limits = self.resource_limits();

        let container_output = runner
            .execute(command, cwd, policy, &limits, env, &self.config)
            .await?;
        Ok(container_output.into())
    }

    /// Execute a command directly on the host (no sandbox).
    async fn execute_direct(
        &self,
        command: &str,
        cwd: &Path,
        env: HashMap<String, String>,
    ) -> Result<ExecOutput> {
        use tokio::process::Command;

        let start = std::time::Instant::now();

        let mut cmd = if cfg!(target_os = "windows") {
            let mut c = Command::new("cmd");
            c.args(["/C", command]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", command]);
            c
        };

        cmd.current_dir(cwd);
        cmd.envs(env);

        let output = tokio::time::timeout(self.config.timeout, cmd.output())
            .await
            .map_err(|_| SandboxError::Timeout(self.config.timeout))?
            .map_err(|e| SandboxError::ExecutionFailed {
                reason: e.to_string(),
            })?;

        let max_output: usize = 64 * 1024; // 64 KB, matching container path
        let half_max = max_output / 2;

        let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let mut truncated = false;

        if stdout.len() > half_max {
            let end = crate::util::floor_char_boundary(&stdout, half_max);
            stdout.truncate(end);
            truncated = true;
        }
        if stderr.len() > half_max {
            let end = crate::util::floor_char_boundary(&stderr, half_max);
            stderr.truncate(end);
            truncated = true;
        }

        let combined = if stderr.is_empty() {
            stdout.clone()
        } else if stdout.is_empty() {
            stderr.clone()
        } else {
            format!("{}\n\n--- stderr ---\n{}", stdout, stderr)
        };

        Ok(ExecOutput {
            exit_code: output.status.code().unwrap_or(-1) as i64,
            stdout,
            stderr,
            output: combined,
            duration: start.elapsed(),
            truncated,
        })
    }

    /// Execute a build command (convenience method using WorkspaceWrite policy).
    pub async fn build(
        &self,
        command: &str,
        project_dir: &Path,
        env: HashMap<String, String>,
    ) -> Result<ExecOutput> {
        self.execute_with_policy(command, project_dir, SandboxPolicy::WorkspaceWrite, env)
            .await
    }

    /// Get the current configuration.
    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }

    /// Check if the sandbox is initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Get the proxy port if running.
    pub async fn proxy_port(&self) -> Option<u16> {
        if let Some(proxy) = self.proxy.read().await.as_ref() {
            proxy.addr().await.map(|a| a.port())
        } else {
            None
        }
    }

    /// Whether persistent sandbox mode is enabled.
    pub fn is_persistent(&self) -> bool {
        self.config.persistent
    }

    async fn require_docker(&self) -> Result<Docker> {
        self.docker
            .read()
            .await
            .clone()
            .ok_or_else(|| SandboxError::DockerNotAvailable {
                reason: "Docker connection not initialized".to_string(),
            })
    }

    fn resource_limits(&self) -> ResourceLimits {
        ResourceLimits {
            memory_bytes: self.config.memory_limit_mb * 1024 * 1024,
            cpu_shares: self.config.cpu_shares,
            timeout: self.config.timeout,
            max_output_bytes: 64 * 1024,
        }
    }

    /// Retry an async operation on transient sandbox errors with exponential backoff.
    async fn retry_transient<F, Fut>(&self, mut op: F) -> Result<ExecOutput>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<ExecOutput>>,
    {
        const MAX_RETRIES: u32 = 2;
        let mut last_err: Option<SandboxError> = None;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_secs(1 << attempt); // 2s, 4s
                tracing::warn!(
                    attempt = attempt + 1,
                    max_attempts = MAX_RETRIES + 1,
                    delay_secs = delay.as_secs(),
                    "Retrying sandbox execution after transient failure"
                );
                tokio::time::sleep(delay).await;
            }

            match op().await {
                Ok(output) => return Ok(output),
                Err(e) if is_transient_sandbox_error(&e) => {
                    tracing::warn!(
                        attempt = attempt + 1,
                        error = %e,
                        "Transient sandbox error, will retry"
                    );
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }

        Err(last_err.unwrap_or_else(|| SandboxError::ExecutionFailed {
            reason: "all retry attempts exhausted".to_string(),
        }))
    }

    async fn make_runner(&self) -> Result<ContainerRunner> {
        let docker = self.require_docker().await?;
        let proxy_port = self.proxy_port().await.unwrap_or(0);
        Ok(ContainerRunner::new(
            docker,
            self.config.image.clone(),
            proxy_port,
        ))
    }

    /// Ensure the persistent session container is running, creating it if needed.
    ///
    /// Uses a read-lock fast path when the container is already running.
    /// Falls back to a write-lock to create or recreate the container.
    async fn ensure_session_container(
        &self,
        cwd: &Path,
        env: HashMap<String, String>,
    ) -> Result<SessionContainer> {
        let docker = self.require_docker().await?;

        // Fast path: read-lock to check existing container.
        {
            let guard = self.session_container.read().await;
            if let Some(session) = guard.as_ref()
                && Self::is_container_running(&docker, &session.id).await
            {
                return Ok(session.clone());
            }
        }

        // Slow path: write-lock to create or recreate.
        let mut guard = self.session_container.write().await;

        // Double-check: another task may have recreated while we waited for the lock.
        if let Some(session) = guard.as_ref() {
            if Self::is_container_running(&docker, &session.id).await {
                return Ok(session.clone());
            }

            tracing::warn!(
                container = &session.id[..session.id.len().min(12)],
                "Persistent session container died; state will be lost. Recreating."
            );
            let _ = Self::force_remove_container(&docker, &session.id).await;
        }

        // Create a new session container.
        let proxy_port = self.proxy_port().await.unwrap_or(0);
        let runner = ContainerRunner::new(docker.clone(), self.config.image.clone(), proxy_port);
        let limits = self.resource_limits();

        let container_id = runner
            .create_persistent_container(cwd, self.config.policy, &limits, env, &self.config)
            .await?;

        docker
            .start_container(
                &container_id,
                None::<bollard::container::StartContainerOptions<String>>,
            )
            .await
            .map_err(|e| SandboxError::ContainerStartFailed {
                reason: e.to_string(),
            })?;

        tracing::info!(
            container = &container_id[..container_id.len().min(12)],
            "Created persistent session container"
        );

        let session = SessionContainer {
            id: container_id,
            base_cwd: cwd.to_path_buf(),
        };
        *guard = Some(session.clone());
        Ok(session)
    }

    /// Check whether a container is still running via the Docker API.
    async fn is_container_running(docker: &Docker, container_id: &str) -> bool {
        match docker.inspect_container(container_id, None).await {
            Ok(info) => info.state.and_then(|s| s.running).unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Force-remove a container, ignoring errors.
    async fn force_remove_container(
        docker: &Docker,
        container_id: &str,
    ) -> std::result::Result<(), bollard::errors::Error> {
        docker
            .remove_container(
                container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
    }

    /// Execute a command in the persistent session container.
    async fn execute_in_session_container(
        &self,
        command: &str,
        cwd: &Path,
        env: HashMap<String, String>,
    ) -> Result<ExecOutput> {
        let env_vec: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let session = self.ensure_session_container(cwd, env).await?;
        let runner = self.make_runner().await?;
        let limits = self.resource_limits();

        let working_dir = map_host_to_container_path(cwd, &session.base_cwd);

        let container_output = runner
            .exec_in_container(&session.id, command, &working_dir, &limits, &env_vec)
            .await?;
        Ok(container_output.into())
    }
}

/// Map a host cwd to a container working directory relative to `/workspace`.
///
/// The container mounts `base_cwd` at `/workspace`, so a host path like
/// `/home/user/project/src` becomes `/workspace/src`. Paths outside `base_cwd`
/// fall back to `/workspace`.
fn map_host_to_container_path(cwd: &Path, base_cwd: &Path) -> String {
    cwd.strip_prefix(base_cwd)
        .map(|rel| {
            if rel.as_os_str().is_empty() {
                "/workspace".to_string()
            } else {
                format!("/workspace/{}", rel.display())
            }
        })
        .unwrap_or_else(|_| "/workspace".to_string())
}

impl Drop for SandboxManager {
    fn drop(&mut self) {
        // Note: async cleanup should be done via shutdown() before dropping
        if self.initialized.load(std::sync::atomic::Ordering::SeqCst) {
            tracing::warn!("SandboxManager dropped without shutdown(), resources may leak");
        }
    }
}

/// Check whether a sandbox error is transient and worth retrying.
///
/// Transient errors are those caused by Docker daemon glitches, container
/// creation race conditions, or container start failures — not by command
/// execution failures, timeouts, or policy violations.
fn is_transient_sandbox_error(err: &SandboxError) -> bool {
    matches!(
        err,
        SandboxError::DockerNotAvailable { .. }
            | SandboxError::ContainerCreationFailed { .. }
            | SandboxError::ContainerStartFailed { .. }
    )
}

/// Builder for creating a sandbox manager.
pub struct SandboxManagerBuilder {
    config: SandboxConfig,
}

impl SandboxManagerBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            config: SandboxConfig::default(),
        }
    }

    /// Enable the sandbox.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.config.enabled = enabled;
        self
    }

    /// Set the sandbox policy.
    ///
    /// **Note:** `SandboxPolicy::FullAccess` additionally requires
    /// `allow_full_access(true)` to be set, or the manager will return
    /// `SandboxError::Config` at execution time. This is an intentional
    /// double opt-in to prevent accidental host execution.
    pub fn policy(mut self, policy: SandboxPolicy) -> Self {
        self.config.policy = policy;
        self
    }

    /// Explicitly allow FullAccess policy (double opt-in).
    pub fn allow_full_access(mut self, allow: bool) -> Self {
        self.config.allow_full_access = allow;
        self
    }

    /// Set the command timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Set the memory limit in MB.
    pub fn memory_limit_mb(mut self, mb: u64) -> Self {
        self.config.memory_limit_mb = mb;
        self
    }

    /// Set the Docker image.
    pub fn image(mut self, image: &str) -> Self {
        self.config.image = image.to_string();
        self
    }

    /// Add domains to the network allowlist.
    pub fn allow_domains(mut self, domains: Vec<String>) -> Self {
        self.config.network_allowlist.extend(domains);
        self
    }

    /// Enable persistent session container mode.
    pub fn persistent(mut self, persistent: bool) -> Self {
        self.config.persistent = persistent;
        self
    }

    /// Set extra bind-mount volumes.
    pub fn extra_volumes(mut self, volumes: Vec<String>) -> Self {
        self.config.extra_volumes = volumes;
        self
    }

    /// Enable host networking.
    pub fn host_network(mut self, host_network: bool) -> Self {
        self.config.host_network = host_network;
        self
    }

    /// Run as root inside the container.
    pub fn run_as_root(mut self, run_as_root: bool) -> Self {
        self.config.run_as_root = run_as_root;
        self
    }

    /// Set extra environment variables.
    pub fn extra_env(mut self, extra_env: Vec<String>) -> Self {
        self.config.extra_env = extra_env;
        self
    }

    /// Build the sandbox manager.
    pub fn build(self) -> SandboxManager {
        SandboxManager::new(self.config)
    }

    /// Build and initialize the sandbox manager.
    pub async fn build_and_init(self) -> Result<SandboxManager> {
        let manager = self.build();
        manager.initialize().await?;
        Ok(manager)
    }
}

impl Default for SandboxManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exec_output_from_container_output() {
        let container = ContainerOutput {
            exit_code: 0,
            stdout: "hello".to_string(),
            stderr: String::new(),
            duration: Duration::from_secs(1),
            truncated: false,
        };

        let exec: ExecOutput = container.into();
        assert_eq!(exec.exit_code, 0);
        assert_eq!(exec.output, "hello");
    }

    #[test]
    fn test_exec_output_combined() {
        let container = ContainerOutput {
            exit_code: 1,
            stdout: "out".to_string(),
            stderr: "err".to_string(),
            duration: Duration::from_secs(1),
            truncated: false,
        };

        let exec: ExecOutput = container.into();
        assert!(exec.output.contains("out"));
        assert!(exec.output.contains("err"));
        assert!(exec.output.contains("stderr"));
    }

    #[test]
    fn test_builder_defaults() {
        let manager = SandboxManagerBuilder::new().build();
        assert!(manager.config.enabled); // Enabled by default (startup check disables if Docker unavailable)
    }

    #[test]
    fn test_builder_custom() {
        let manager = SandboxManagerBuilder::new()
            .enabled(true)
            .policy(SandboxPolicy::WorkspaceWrite)
            .timeout(Duration::from_secs(60))
            .memory_limit_mb(1024)
            .image("custom:latest")
            .build();

        assert!(manager.config.enabled);
        assert_eq!(manager.config.policy, SandboxPolicy::WorkspaceWrite);
        assert_eq!(manager.config.timeout, Duration::from_secs(60));
        assert_eq!(manager.config.memory_limit_mb, 1024);
        assert_eq!(manager.config.image, "custom:latest");
    }

    #[tokio::test]
    async fn test_direct_execution() {
        let manager = SandboxManager::new(SandboxConfig {
            enabled: true,
            policy: SandboxPolicy::FullAccess,
            allow_full_access: true,
            ..Default::default()
        });

        let result = manager
            .execute("echo hello", Path::new("."), HashMap::new())
            .await;

        // This should work even without Docker since FullAccess runs directly
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_direct_execution_blocked_without_allow() {
        let manager = SandboxManager::new(SandboxConfig {
            enabled: true,
            policy: SandboxPolicy::FullAccess,
            allow_full_access: false,
            ..Default::default()
        });

        let result = manager
            .execute("echo hello", Path::new("."), HashMap::new())
            .await;

        // Should be rejected because allow_full_access is false
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("SANDBOX_ALLOW_FULL_ACCESS"),
            "Error should mention SANDBOX_ALLOW_FULL_ACCESS, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_builder_full_access_without_allow_returns_error() {
        let manager = SandboxManagerBuilder::new()
            .enabled(true)
            .policy(SandboxPolicy::FullAccess)
            // Deliberately omitting .allow_full_access(true)
            .build();

        let result = manager
            .execute("echo hello", Path::new("."), HashMap::new())
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("SANDBOX_ALLOW_FULL_ACCESS"),
            "Error should mention SANDBOX_ALLOW_FULL_ACCESS, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_direct_execution_truncates_large_output() {
        let manager = SandboxManager::new(SandboxConfig {
            enabled: true,
            policy: SandboxPolicy::FullAccess,
            allow_full_access: true,
            ..Default::default()
        });

        // Generate output larger than 32KB (half of 64KB limit)
        // printf repeats a 100-char line 400 times = 40KB
        let result = manager
            .execute(
                "printf 'A%.0s' $(seq 1 40000)",
                Path::new("."),
                HashMap::new(),
            )
            .await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.truncated);
        assert!(output.stdout.len() <= 32 * 1024);
    }

    #[test]
    fn transient_errors_are_retryable() {
        assert!(super::is_transient_sandbox_error(
            &SandboxError::DockerNotAvailable {
                reason: "daemon restarting".to_string()
            }
        ));
        assert!(super::is_transient_sandbox_error(
            &SandboxError::ContainerCreationFailed {
                reason: "image pull glitch".to_string()
            }
        ));
        assert!(super::is_transient_sandbox_error(
            &SandboxError::ContainerStartFailed {
                reason: "cgroup race".to_string()
            }
        ));
    }

    #[test]
    fn non_transient_errors_are_not_retryable() {
        assert!(!super::is_transient_sandbox_error(&SandboxError::Timeout(
            std::time::Duration::from_secs(30)
        )));
        assert!(!super::is_transient_sandbox_error(
            &SandboxError::ExecutionFailed {
                reason: "exit code 1".to_string()
            }
        ));
        assert!(!super::is_transient_sandbox_error(
            &SandboxError::NetworkBlocked {
                reason: "policy violation".to_string()
            }
        ));
        assert!(!super::is_transient_sandbox_error(&SandboxError::Config {
            reason: "bad config".to_string()
        }));
    }

    #[test]
    fn is_persistent_returns_config_value() {
        let manager = SandboxManagerBuilder::new().persistent(false).build();
        assert!(!manager.is_persistent());

        let manager = SandboxManagerBuilder::new().persistent(true).build();
        assert!(manager.is_persistent());
    }

    #[test]
    fn builder_persistent_fields() {
        let manager = SandboxManagerBuilder::new()
            .persistent(true)
            .extra_volumes(vec!["/data:/data:ro".to_string()])
            .host_network(true)
            .run_as_root(true)
            .extra_env(vec!["FOO=bar".to_string()])
            .build();

        assert!(manager.config.persistent);
        assert_eq!(manager.config.extra_volumes, vec!["/data:/data:ro"]);
        assert!(manager.config.host_network);
        assert!(manager.config.run_as_root);
        assert_eq!(manager.config.extra_env, vec!["FOO=bar"]);
    }

    #[test]
    fn working_dir_mapping_subdirectory() {
        let base = PathBuf::from("/home/user/project");
        let cwd = PathBuf::from("/home/user/project/src/lib");
        assert_eq!(
            super::map_host_to_container_path(&cwd, &base),
            "/workspace/src/lib"
        );
    }

    #[test]
    fn working_dir_mapping_exact_match() {
        let base = PathBuf::from("/home/user/project");
        let cwd = PathBuf::from("/home/user/project");
        assert_eq!(super::map_host_to_container_path(&cwd, &base), "/workspace");
    }

    #[test]
    fn working_dir_mapping_outside_base() {
        let base = PathBuf::from("/home/user/project");
        let cwd = PathBuf::from("/tmp/other");
        assert_eq!(super::map_host_to_container_path(&cwd, &base), "/workspace");
    }

    #[test]
    fn session_container_clone() {
        let session = super::SessionContainer {
            id: "abc123".to_string(),
            base_cwd: PathBuf::from("/home/user/project"),
        };
        let cloned = session.clone();
        assert_eq!(cloned.id, "abc123");
        assert_eq!(cloned.base_cwd, PathBuf::from("/home/user/project"));
    }
}
