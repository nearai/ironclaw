//! Main sandbox manager coordinating proxy and containers.
//!
//! The `SandboxManager` is the primary entry point for sandboxed execution.
//! It coordinates:
//! - Container runtime lifecycle (Docker, Kubernetes, or future backends)
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
//! │   │ (if needed)  │     │ Workload     │     │                          │  │
//! │   └──────────────┘     └──────────────┘     └──────────────────────────┘  │
//! │                                                        │                   │
//! │                                                        ▼                   │
//! │                                              ┌──────────────────────────┐  │
//! │                                              │ Cleanup Workload         │  │
//! │                                              └──────────────────────────┘  │
//! └───────────────────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use crate::sandbox::config::{SandboxConfig, SandboxPolicy};
use crate::sandbox::error::{Result, SandboxError};
use crate::sandbox::proxy::{HttpProxy, NetworkProxyBuilder};
use crate::sandbox::runtime::{ContainerRuntime, VolumeMount, WorkloadOutput, WorkloadSpec};

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

impl From<WorkloadOutput> for ExecOutput {
    fn from(w: WorkloadOutput) -> Self {
        let output = if w.stderr.is_empty() {
            w.stdout.clone()
        } else if w.stdout.is_empty() {
            w.stderr.clone()
        } else {
            format!("{}\n\n--- stderr ---\n{}", w.stdout, w.stderr)
        };

        Self {
            exit_code: w.exit_code,
            stdout: w.stdout,
            stderr: w.stderr,
            output,
            duration: w.duration,
            truncated: w.truncated,
        }
    }
}

#[cfg(feature = "docker")]
impl From<crate::sandbox::container::ContainerOutput> for ExecOutput {
    fn from(c: crate::sandbox::container::ContainerOutput) -> Self {
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

/// Main sandbox manager.
pub struct SandboxManager {
    config: SandboxConfig,
    proxy: Arc<RwLock<Option<HttpProxy>>>,
    runtime: Arc<RwLock<Option<Arc<dyn ContainerRuntime>>>>,
    initialized: std::sync::atomic::AtomicBool,
}

impl SandboxManager {
    /// Create a new sandbox manager.
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config,
            proxy: Arc::new(RwLock::new(None)),
            runtime: Arc::new(RwLock::new(None)),
            initialized: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(SandboxConfig::default())
    }

    /// Create a sandbox manager with a pre-initialized runtime.
    pub fn with_runtime(config: SandboxConfig, runtime: Arc<dyn ContainerRuntime>) -> Self {
        Self {
            config,
            proxy: Arc::new(RwLock::new(None)),
            runtime: Arc::new(RwLock::new(Some(runtime))),
            initialized: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Check if the sandbox is available (runtime running, etc.).
    pub async fn is_available(&self) -> bool {
        if !self.config.enabled {
            return false;
        }

        if let Some(ref rt) = *self.runtime.read().await {
            return rt.is_available().await;
        }

        // No runtime set yet — try to create one and check
        match crate::sandbox::runtime::connect_runtime(
            self.config.container_runtime.as_deref(),
            &self.config.k8s_namespace,
        )
        .await
        {
            Ok(rt) => rt.is_available().await,
            Err(_) => false,
        }
    }

    /// Initialize the sandbox (connect to runtime, start proxy).
    pub async fn initialize(&self) -> Result<()> {
        if self.initialized.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }

        if !self.config.enabled {
            return Err(SandboxError::Config {
                reason: "sandbox is disabled".to_string(),
            });
        }

        // Connect to the runtime if not already set
        if self.runtime.read().await.is_none() {
            let rt = self.create_runtime().await?;
            *self.runtime.write().await = Some(rt);
        }

        {
            let guard = self.runtime.read().await;
            let rt = guard.as_ref().ok_or_else(|| SandboxError::Config {
                reason: "runtime initialization failed".to_string(),
            })?;

            if !rt.is_available().await {
                return Err(SandboxError::DockerNotAvailable {
                    reason: format!("{} runtime is not available", rt.name()),
                });
            }

            // Check for / pull image
            if !rt.image_exists(&self.config.image).await {
                if self.config.auto_pull_image {
                    rt.pull_image(&self.config.image).await?;
                } else {
                    return Err(SandboxError::ContainerCreationFailed {
                        reason: format!(
                            "image {} not found and auto_pull is disabled",
                            self.config.image
                        ),
                    });
                }
            }

            if self.config.policy.is_sandboxed() {
                let policy_name = match self.config.policy {
                    SandboxPolicy::ReadOnly => "read_only",
                    SandboxPolicy::WorkspaceWrite => "workspace_write",
                    SandboxPolicy::FullAccess => "full_access",
                };
                let mut unsupported = Vec::new();
                if !rt.supports_host_proxy() {
                    unsupported.push("allowlist-only networking");
                }
                if !rt.supports_bind_mounts() {
                    unsupported.push("workspace bind mounts");
                }
                if !unsupported.is_empty() {
                    return Err(SandboxError::Config {
                        reason: format!(
                            "{} runtime cannot satisfy {} sandbox requirements (missing {}). \
                             Use Docker for sandboxed command execution until Kubernetes-native \
                             enforcement is implemented.",
                            rt.name(),
                            policy_name,
                            unsupported.join(" and "),
                        ),
                    });
                }
            }
        }

        // Start the network proxy if we're using a sandboxed policy
        if self.config.policy.is_sandboxed() {
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

    /// Create a container runtime based on config override, env var, and
    /// compiled feature flags via the shared factory.
    async fn create_runtime(&self) -> Result<Arc<dyn ContainerRuntime>> {
        crate::sandbox::runtime::connect_runtime(
            self.config.container_runtime.as_deref(),
            &self.config.k8s_namespace,
        )
        .await
    }

    /// Shutdown the sandbox (stop proxy, clean up).
    pub async fn shutdown(&self) {
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

        // Retry transient failures with exponential backoff.
        const MAX_SANDBOX_RETRIES: u32 = 2;
        let mut last_err: Option<SandboxError> = None;

        for attempt in 0..=MAX_SANDBOX_RETRIES {
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(1 << attempt);
                tracing::warn!(
                    attempt = attempt + 1,
                    max_attempts = MAX_SANDBOX_RETRIES + 1,
                    delay_secs = delay.as_secs(),
                    "Retrying sandbox execution after transient failure"
                );
                tokio::time::sleep(delay).await;
            }

            match self
                .try_execute_in_container(command, cwd, policy, env.clone())
                .await
            {
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

    /// Single attempt at container execution via the runtime trait.
    async fn try_execute_in_container(
        &self,
        command: &str,
        cwd: &Path,
        policy: SandboxPolicy,
        env: HashMap<String, String>,
    ) -> Result<ExecOutput> {
        let proxy_port = if let Some(proxy) = self.proxy.read().await.as_ref() {
            proxy.addr().await.map(|a| a.port()).unwrap_or(0)
        } else {
            0
        };

        let rt_guard = self.runtime.read().await;
        let rt = rt_guard
            .as_ref()
            .ok_or_else(|| SandboxError::DockerNotAvailable {
                reason: "runtime not initialized".to_string(),
            })?;

        let orchestrator_host = rt.orchestrator_host();

        if policy.is_sandboxed() {
            let policy_name = match policy {
                SandboxPolicy::ReadOnly => "read_only",
                SandboxPolicy::WorkspaceWrite => "workspace_write",
                SandboxPolicy::FullAccess => "full_access",
            };
            let mut unsupported = Vec::new();
            if !rt.supports_host_proxy() {
                unsupported.push("allowlist-only networking");
            }
            if !rt.supports_bind_mounts() {
                unsupported.push("workspace bind mounts");
            }
            if !unsupported.is_empty() {
                return Err(SandboxError::Config {
                    reason: format!(
                        "{} runtime cannot satisfy {} sandbox requirements (missing {}). \
                         Use Docker for sandboxed command execution until Kubernetes-native \
                         enforcement is implemented.",
                        rt.name(),
                        policy_name,
                        unsupported.join(" and "),
                    ),
                });
            }
        }

        // Build environment
        let mut env_vec: Vec<String> = env
            .into_iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        if proxy_port > 0 && policy.is_sandboxed() {
            env_vec.push(format!(
                "http_proxy=http://{}:{}",
                orchestrator_host, proxy_port
            ));
            env_vec.push(format!(
                "https_proxy=http://{}:{}",
                orchestrator_host, proxy_port
            ));
            env_vec.push(format!(
                "HTTP_PROXY=http://{}:{}",
                orchestrator_host, proxy_port
            ));
            env_vec.push(format!(
                "HTTPS_PROXY=http://{}:{}",
                orchestrator_host, proxy_port
            ));
        }

        // Build volume mounts based on policy
        let working_dir_str = cwd.display().to_string();
        let mounts = match policy {
            SandboxPolicy::ReadOnly => {
                vec![VolumeMount {
                    source: working_dir_str,
                    target: "/workspace".to_string(),
                    read_only: true,
                }]
            }
            SandboxPolicy::WorkspaceWrite => {
                vec![VolumeMount {
                    source: working_dir_str,
                    target: "/workspace".to_string(),
                    read_only: false,
                }]
            }
            SandboxPolicy::FullAccess => {
                vec![
                    VolumeMount {
                        source: working_dir_str,
                        target: "/workspace".to_string(),
                        read_only: false,
                    },
                    VolumeMount {
                        source: "/tmp".to_string(),
                        target: "/tmp".to_string(),
                        read_only: false,
                    },
                ]
            }
        };

        let spec = WorkloadSpec {
            name: format!("sandbox-{}", uuid::Uuid::new_v4()),
            image: self.config.image.clone(),
            command: vec!["sh".to_string(), "-c".to_string(), command.to_string()],
            env: env_vec,
            working_dir: "/workspace".to_string(),
            mounts,
            tmpfs_mounts: [
                ("/tmp".to_string(), "size=512M".to_string()),
                (
                    "/home/sandbox/.cargo/registry".to_string(),
                    "size=1G".to_string(),
                ),
            ]
            .into_iter()
            .collect(),
            memory_bytes: Some((self.config.memory_limit_mb * 1024 * 1024) as i64),
            cpu_shares: Some(self.config.cpu_shares as i64),
            extra_hosts: vec!["host.docker.internal:host-gateway".to_string()],
            readonly_rootfs: policy != SandboxPolicy::FullAccess,
            auto_remove: true,
            ..Default::default()
        };

        let start_time = std::time::Instant::now();

        let workload_id = rt.create_and_start_workload(&spec).await?;

        let result = tokio::time::timeout(self.config.timeout, async {
            let exit_code = rt.wait_workload(&workload_id).await?;
            let (stdout, stderr, truncated) = rt.collect_logs(&workload_id, 64 * 1024).await?;
            Ok::<WorkloadOutput, SandboxError>(WorkloadOutput {
                exit_code,
                stdout,
                stderr,
                duration: start_time.elapsed(),
                truncated,
            })
        })
        .await;

        // Always attempt cleanup
        if let Err(e) = rt.remove_workload(&workload_id).await {
            tracing::warn!(workload_id = %workload_id, error = %e, "failed to remove workload after execution");
        }

        match result {
            Ok(Ok(output)) => Ok(output.into()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(SandboxError::Timeout(self.config.timeout)),
        }
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

        let max_output: usize = 64 * 1024;
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
}

impl Drop for SandboxManager {
    fn drop(&mut self) {
        if self.initialized.load(std::sync::atomic::Ordering::SeqCst) {
            tracing::warn!("SandboxManager dropped without shutdown(), resources may leak");
        }
    }
}

/// Check whether a sandbox error is transient and worth retrying.
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
    runtime: Option<Arc<dyn ContainerRuntime>>,
}

impl SandboxManagerBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            config: SandboxConfig::default(),
            runtime: None,
        }
    }

    /// Enable the sandbox.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.config.enabled = enabled;
        self
    }

    /// Set the sandbox policy.
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

    /// Set the container image.
    pub fn image(mut self, image: &str) -> Self {
        self.config.image = image.to_string();
        self
    }

    /// Add domains to the network allowlist.
    pub fn allow_domains(mut self, domains: Vec<String>) -> Self {
        self.config.network_allowlist.extend(domains);
        self
    }

    /// Provide a pre-created runtime.
    pub fn runtime(mut self, runtime: Arc<dyn ContainerRuntime>) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// Build the sandbox manager.
    pub fn build(self) -> SandboxManager {
        if let Some(rt) = self.runtime {
            SandboxManager::with_runtime(self.config, rt)
        } else {
            SandboxManager::new(self.config)
        }
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
    use crate::sandbox::runtime::{RuntimeDetection, RuntimeStatus};
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_exec_output_from_workload_output() {
        let workload = WorkloadOutput {
            exit_code: 0,
            stdout: "hello".to_string(),
            stderr: String::new(),
            duration: Duration::from_secs(1),
            truncated: false,
        };

        let exec: ExecOutput = workload.into();
        assert_eq!(exec.exit_code, 0);
        assert_eq!(exec.output, "hello");
    }

    #[test]
    fn test_exec_output_combined() {
        let workload = WorkloadOutput {
            exit_code: 1,
            stdout: "out".to_string(),
            stderr: "err".to_string(),
            duration: Duration::from_secs(1),
            truncated: false,
        };

        let exec: ExecOutput = workload.into();
        assert!(exec.output.contains("out"));
        assert!(exec.output.contains("err"));
        assert!(exec.output.contains("stderr"));
    }

    #[test]
    fn test_builder_defaults() {
        let manager = SandboxManagerBuilder::new().build();
        assert!(manager.config.enabled);
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

    struct RecordingRuntime {
        supports_host_proxy: bool,
        supports_bind_mounts: bool,
        spec: Mutex<Option<WorkloadSpec>>,
    }

    impl RecordingRuntime {
        fn new(supports_host_proxy: bool, supports_bind_mounts: bool) -> Self {
            Self {
                supports_host_proxy,
                supports_bind_mounts,
                spec: Mutex::new(None),
            }
        }

        fn captured_spec(&self) -> Option<WorkloadSpec> {
            self.spec
                .lock()
                .expect("recording runtime mutex poisoned")
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl ContainerRuntime for RecordingRuntime {
        fn name(&self) -> &'static str {
            "recording"
        }

        async fn is_available(&self) -> bool {
            true
        }

        async fn detect(&self) -> RuntimeDetection {
            RuntimeDetection {
                status: RuntimeStatus::Available,
                runtime_name: "recording",
                install_hint: String::new(),
                start_hint: String::new(),
            }
        }

        async fn image_exists(&self, _image: &str) -> bool {
            true
        }

        async fn pull_image(&self, _image: &str) -> Result<()> {
            Ok(())
        }

        async fn build_image(&self, _image: &str, _dockerfile_path: &Path) -> Result<()> {
            Ok(())
        }

        async fn create_and_start_workload(&self, spec: &WorkloadSpec) -> Result<String> {
            *self.spec.lock().expect("recording runtime mutex poisoned") = Some(spec.clone());
            Ok("recording-workload".to_string())
        }

        async fn wait_workload(&self, _workload_id: &str) -> Result<i64> {
            Ok(0)
        }

        async fn stop_workload(&self, _workload_id: &str, _grace_period_secs: u32) -> Result<()> {
            Ok(())
        }

        async fn remove_workload(&self, _workload_id: &str) -> Result<()> {
            Ok(())
        }

        async fn exec_in_workload(
            &self,
            _workload_id: &str,
            _command: &[&str],
            _working_dir: &str,
            _max_output: usize,
            _timeout: Duration,
        ) -> Result<WorkloadOutput> {
            Ok(WorkloadOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::from_secs(0),
                truncated: false,
            })
        }

        async fn collect_logs(
            &self,
            _workload_id: &str,
            _max_output: usize,
        ) -> Result<(String, String, bool)> {
            Ok(("hello".to_string(), String::new(), false))
        }

        async fn list_managed_workloads(
            &self,
            _label_key: &str,
        ) -> Result<Vec<crate::sandbox::runtime::ManagedWorkload>> {
            Ok(Vec::new())
        }

        fn orchestrator_host(&self) -> &str {
            "host.docker.internal"
        }

        fn supports_host_proxy(&self) -> bool {
            self.supports_host_proxy
        }

        fn supports_bind_mounts(&self) -> bool {
            self.supports_bind_mounts
        }
    }

    #[tokio::test]
    async fn test_sandboxed_execution_rejects_runtime_without_host_proxy() {
        let runtime = Arc::new(RecordingRuntime::new(false, true));
        let manager = SandboxManager::with_runtime(SandboxConfig::default(), runtime.clone());

        let err = manager
            .execute("echo hello", Path::new("."), HashMap::new())
            .await
            .expect_err("sandboxed execution should fail closed without host proxy")
            .to_string();

        assert!(
            err.contains("allowlist-only networking"),
            "expected host-proxy failure, got: {err}"
        );
        assert!(
            runtime.captured_spec().is_none(),
            "workload should not be created when proxy contract cannot be met"
        );
    }

    #[tokio::test]
    async fn test_sandboxed_execution_rejects_runtime_without_bind_mounts() {
        let runtime = Arc::new(RecordingRuntime::new(true, false));
        let manager = SandboxManager::with_runtime(SandboxConfig::default(), runtime.clone());

        let err = manager
            .execute("echo hello", Path::new("."), HashMap::new())
            .await
            .expect_err("sandboxed execution should fail closed without bind mounts")
            .to_string();

        assert!(
            err.contains("workspace bind mounts"),
            "expected bind-mount failure, got: {err}"
        );
        assert!(
            runtime.captured_spec().is_none(),
            "workload should not be created when workspace mounts are unsupported"
        );
    }

    #[tokio::test]
    async fn test_sandboxed_execution_adds_host_gateway_mapping() {
        let runtime = Arc::new(RecordingRuntime::new(true, true));
        let manager = SandboxManager::with_runtime(SandboxConfig::default(), runtime.clone());

        let output = manager
            .execute("echo hello", Path::new("."), HashMap::new())
            .await
            .expect("sandboxed execution should succeed with recording runtime");
        assert!(output.stdout.contains("hello"));

        let spec = runtime
            .captured_spec()
            .expect("successful execution should capture workload spec");
        assert!(
            spec.extra_hosts
                .contains(&"host.docker.internal:host-gateway".to_string()),
            "sandbox workloads must map host.docker.internal for Linux Docker reachability"
        );
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

        assert!(result.is_ok());
        let output = result.unwrap(); // safety: test
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

        assert!(result.is_err());
        let err = result.unwrap_err().to_string(); // safety: test
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
            .build();

        let result = manager
            .execute("echo hello", Path::new("."), HashMap::new())
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string(); // safety: test
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

        let result = manager
            .execute(
                "printf 'A%.0s' $(seq 1 40000)",
                Path::new("."),
                HashMap::new(),
            )
            .await;

        assert!(result.is_ok());
        let output = result.unwrap(); // safety: test
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
}
