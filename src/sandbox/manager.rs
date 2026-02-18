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
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use bollard::Docker;

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

/// Main sandbox manager.
pub struct SandboxManager {
    config: SandboxConfig,
    proxy: Arc<RwLock<Option<HttpProxy>>>,
    docker: Arc<RwLock<Option<Docker>>>,
    initialized: std::sync::atomic::AtomicBool,
}

impl SandboxManager {
    /// Create a new sandbox manager.
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config,
            proxy: Arc::new(RwLock::new(None)),
            docker: Arc::new(RwLock::new(None)),
            initialized: std::sync::atomic::AtomicBool::new(false),
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

    /// Shutdown the sandbox (stop proxy, clean up).
    pub async fn shutdown(&self) {
        if let Some(proxy) = self.proxy.write().await.take() {
            proxy.stop().await;
        }

        self.initialized
            .store(false, std::sync::atomic::Ordering::SeqCst);

        tracing::info!("Sandbox shut down");
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
        // FullAccess policy bypasses the sandbox entirely
        if policy == SandboxPolicy::FullAccess {
            return self.execute_direct(command, cwd, env).await;
        }

        // Ensure we're initialized
        if !self.initialized.load(std::sync::atomic::Ordering::SeqCst) {
            self.initialize().await?;
        }

        // Get proxy port if running
        let proxy_port = if let Some(proxy) = self.proxy.read().await.as_ref() {
            proxy.addr().await.map(|a| a.port()).unwrap_or(0)
        } else {
            0
        };

        // Reuse the stored Docker connection, create a runner with the current proxy port
        let docker =
            self.docker
                .read()
                .await
                .clone()
                .ok_or_else(|| SandboxError::DockerNotAvailable {
                    reason: "Docker connection not initialized".to_string(),
                })?;
        let runner = ContainerRunner::new(docker, self.config.image.clone(), proxy_port);

        let limits = ResourceLimits {
            memory_bytes: self.config.memory_limit_mb * 1024 * 1024,
            cpu_shares: self.config.cpu_shares,
            timeout: self.config.timeout,
            max_output_bytes: 64 * 1024,
        };

        let container_output = runner.execute(command, cwd, policy, &limits, env).await?;

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
}

impl Drop for SandboxManager {
    fn drop(&mut self) {
        // Note: async cleanup should be done via shutdown() before dropping
        if self.initialized.load(std::sync::atomic::Ordering::SeqCst) {
            tracing::warn!("SandboxManager dropped without shutdown(), resources may leak");
        }
    }
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
    pub fn policy(mut self, policy: SandboxPolicy) -> Self {
        self.config.policy = policy;
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
        assert!(!manager.config.enabled); // Disabled by default
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
    async fn test_direct_execution_truncates_large_output() {
        let manager = SandboxManager::new(SandboxConfig {
            enabled: true,
            policy: SandboxPolicy::FullAccess,
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
}
