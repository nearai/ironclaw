//! Container lifecycle management for sandboxed jobs.
//!
//! Extends the existing `SandboxManager` infrastructure to support persistent
//! containers with their own agent loops (as opposed to ephemeral per-command containers).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::bootstrap::ironclaw_base_dir;
use crate::error::OrchestratorError;
use crate::orchestrator::auth::{CredentialGrant, TokenStore};
use crate::sandbox::connect_docker;

/// Path to the master worker MCP config on the host.
const WORKER_MCP_CONFIG_PATH: &str = "/opt/ironclaw/config/worker/mcp-servers.json";

/// Which mode a sandbox container runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobMode {
    /// Standard IronClaw worker with proxied LLM calls.
    Worker,
    /// Claude Code bridge that spawns the `claude` CLI directly.
    ClaudeCode,
}

impl JobMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::ClaudeCode => "claude_code",
        }
    }
}

impl std::fmt::Display for JobMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Configuration for the container job manager.
#[derive(Debug, Clone)]
pub struct ContainerJobConfig {
    /// Docker image for worker containers.
    pub image: String,
    /// Default memory limit in MB.
    pub memory_limit_mb: u64,
    /// Default CPU shares.
    pub cpu_shares: u32,
    /// Port the orchestrator internal API listens on.
    pub orchestrator_port: u16,
    /// Anthropic API key for Claude Code containers (read from ANTHROPIC_API_KEY).
    /// Takes priority over OAuth token.
    pub claude_code_api_key: Option<String>,
    /// OAuth access token extracted from the host's `claude login` session.
    /// Passed as CLAUDE_CODE_OAUTH_TOKEN to containers. Falls back to this
    /// when no ANTHROPIC_API_KEY is available.
    pub claude_code_oauth_token: Option<String>,
    /// Claude model to use in ClaudeCode mode.
    pub claude_code_model: String,
    /// Maximum turns for Claude Code.
    pub claude_code_max_turns: u32,
    /// Memory limit in MB for Claude Code containers (heavier than workers).
    pub claude_code_memory_limit_mb: u64,
    /// Allowed tool patterns for Claude Code (passed as CLAUDE_CODE_ALLOWED_TOOLS env var).
    pub claude_code_allowed_tools: Vec<String>,
    /// Whether per-job MCP server filtering is enabled.
    /// When false, `mcp_servers` param on `create_job` is ignored.
    pub mcp_per_job_enabled: bool,
}

impl Default for ContainerJobConfig {
    fn default() -> Self {
        Self {
            image: "ironclaw-worker:latest".to_string(),
            memory_limit_mb: 2048,
            cpu_shares: 1024,
            orchestrator_port: 50051,
            claude_code_api_key: None,
            claude_code_oauth_token: None,
            claude_code_model: "sonnet".to_string(),
            claude_code_max_turns: 50,
            claude_code_memory_limit_mb: 4096,
            claude_code_allowed_tools: crate::config::ClaudeCodeConfig::default().allowed_tools,
            mcp_per_job_enabled: false,
        }
    }
}

/// State of a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerState {
    Creating,
    Running,
    Stopped,
    Failed,
}

impl std::fmt::Display for ContainerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Creating => write!(f, "creating"),
            Self::Running => write!(f, "running"),
            Self::Stopped => write!(f, "stopped"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// Handle to a running container job.
#[derive(Debug, Clone)]
pub struct ContainerHandle {
    pub job_id: Uuid,
    pub container_id: String,
    pub state: ContainerState,
    pub mode: JobMode,
    pub created_at: DateTime<Utc>,
    pub project_dir: Option<PathBuf>,
    pub task_description: String,
    /// Last status message reported by the worker (iteration count, progress, etc.).
    pub last_worker_status: Option<String>,
    /// Which iteration the worker is on (updated via status reports).
    pub worker_iteration: u32,
    /// Completion result from the worker (set when the worker reports done).
    pub completion_result: Option<CompletionResult>,
    // NOTE: auth_token is intentionally NOT in this struct.
    // It lives only in the TokenStore (never logged, serialized, or persisted).
}

/// Result reported by a worker on completion.
#[derive(Debug, Clone)]
pub struct CompletionResult {
    pub success: bool,
    pub message: Option<String>,
}

/// Validate that a project directory is under `~/.ironclaw/projects/`.
///
/// Returns the canonicalized path if valid. Creates the base directory if
/// it doesn't exist (so the prefix check always runs).
///
/// # TOCTOU note
///
/// There is a time-of-check/time-of-use gap between `canonicalize()` here
/// and the actual Docker `binds.push()` in the caller. In a multi-tenant
/// system a malicious actor could swap a symlink after validation. This is
/// acceptable in IronClaw's single-tenant design where the user controls
/// the filesystem.
fn validate_bind_mount_path(
    dir: &std::path::Path,
    job_id: Uuid,
) -> Result<PathBuf, OrchestratorError> {
    let canonical = dir
        .canonicalize()
        .map_err(|e| OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!(
                "failed to canonicalize project dir {}: {}",
                dir.display(),
                e
            ),
        })?;

    let projects_base = ironclaw_base_dir().join("projects");

    if !projects_base.is_absolute() {
        return Err(OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: "base directory is not absolute; cannot safely validate bind mounts".into(),
        });
    }

    // Ensure the base exists so canonicalize always succeeds.
    std::fs::create_dir_all(&projects_base).map_err(|e| {
        OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!(
                "failed to create projects base {}: {}",
                projects_base.display(),
                e
            ),
        }
    })?;

    let canonical_base =
        projects_base
            .canonicalize()
            .map_err(|e| OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: format!(
                    "failed to canonicalize projects base {}: {}",
                    projects_base.display(),
                    e
                ),
            })?;

    if !canonical.starts_with(&canonical_base) {
        return Err(OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!(
                "project directory {} is outside allowed base {}",
                canonical.display(),
                canonical_base.display()
            ),
        });
    }

    Ok(canonical)
}

/// Manages the lifecycle of Docker containers for sandboxed job execution.
pub struct ContainerJobManager {
    config: ContainerJobConfig,
    token_store: TokenStore,
    pub(crate) containers: Arc<RwLock<HashMap<Uuid, ContainerHandle>>>,
    /// Cached Docker connection (created on first use).
    docker: Arc<RwLock<Option<bollard::Docker>>>,
}

impl ContainerJobManager {
    pub fn new(config: ContainerJobConfig, token_store: TokenStore) -> Self {
        Self {
            config,
            token_store,
            containers: Arc::new(RwLock::new(HashMap::new())),
            docker: Arc::new(RwLock::new(None)),
        }
    }

    /// Get or create a Docker connection.
    async fn docker(&self) -> Result<bollard::Docker, OrchestratorError> {
        {
            let guard = self.docker.read().await;
            if let Some(ref d) = *guard {
                return Ok(d.clone());
            }
        }
        let docker = connect_docker()
            .await
            .map_err(|e| OrchestratorError::Docker {
                reason: e.to_string(),
            })?;
        *self.docker.write().await = Some(docker.clone());
        Ok(docker)
    }

    /// Create and start a new container for a job.
    ///
    /// The caller provides the `job_id` so it can be persisted to the database
    /// before the container is created. Credential grants are stored in the
    /// TokenStore and served on-demand via the `/credentials` endpoint.
    /// Returns the auth token for the worker.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_job(
        &self,
        job_id: Uuid,
        task: &str,
        project_dir: Option<PathBuf>,
        mode: JobMode,
        credential_grants: Vec<CredentialGrant>,
        mcp_servers: Option<Vec<String>>,
        max_iterations: Option<u32>,
    ) -> Result<String, OrchestratorError> {
        // Generate auth token (stored in TokenStore, never logged)
        let token = self.token_store.create_token(job_id).await;

        // Store credential grants (revoked automatically when the token is revoked)
        self.token_store
            .store_grants(job_id, credential_grants)
            .await;

        // Record the handle
        let handle = ContainerHandle {
            job_id,
            container_id: String::new(), // set after container creation
            state: ContainerState::Creating,
            mode,
            created_at: Utc::now(),
            project_dir: project_dir.clone(),
            task_description: task.to_string(),
            last_worker_status: None,
            worker_iteration: 0,
            completion_result: None,
        };
        self.containers.write().await.insert(job_id, handle);

        // Run the actual container creation. On any failure, revoke the token
        // and remove the handle so we don't leak resources.
        match self
            .create_job_inner(
                job_id,
                &token,
                project_dir,
                mode,
                mcp_servers,
                max_iterations,
            )
            .await
        {
            Ok(()) => Ok(token),
            Err(e) => {
                self.token_store.revoke(job_id).await;
                self.containers.write().await.remove(&job_id);
                Err(e)
            }
        }
    }

    /// Inner implementation of container creation (separated for cleanup).
    #[allow(clippy::too_many_arguments)]
    async fn create_job_inner(
        &self,
        job_id: Uuid,
        token: &str,
        project_dir: Option<PathBuf>,
        mode: JobMode,
        mcp_servers: Option<Vec<String>>,
        max_iterations: Option<u32>,
    ) -> Result<(), OrchestratorError> {
        // Connect to Docker (reuses cached connection)
        let docker = self.docker().await?;

        // Build container configuration
        let orchestrator_host = if cfg!(target_os = "linux") {
            "172.17.0.1"
        } else {
            "host.docker.internal"
        };

        let orchestrator_url = format!(
            "http://{}:{}",
            orchestrator_host, self.config.orchestrator_port
        );

        let mut env_vec = vec![
            format!("IRONCLAW_WORKER_TOKEN={}", token),
            format!("IRONCLAW_JOB_ID={}", job_id),
            format!("IRONCLAW_ORCHESTRATOR_URL={}", orchestrator_url),
        ];

        // Build volume mounts (validate project_dir stays within ~/.ironclaw/projects/)
        let mut binds = Vec::new();
        if let Some(ref dir) = project_dir {
            let canonical = validate_bind_mount_path(dir, job_id)?;
            binds.push(format!("{}:/workspace:rw", canonical.display()));
            env_vec.push("IRONCLAW_WORKSPACE=/workspace".to_string());
        }

        // Inject max_iterations if specified (only for Worker mode — ClaudeCode uses max_turns).
        if let Some(iters) = max_iterations
            && mode == JobMode::Worker
        {
            env_vec.push(format!("IRONCLAW_MAX_ITERATIONS={}", iters));
        }

        // Mount per-job MCP config when the feature is enabled.
        if self.config.mcp_per_job_enabled {
            let mcp_config_host = std::path::Path::new(WORKER_MCP_CONFIG_PATH);
            match generate_worker_mcp_config(mcp_config_host, mcp_servers.as_deref(), job_id)? {
                Some(config_path) => {
                    binds.push(format!(
                        "{}:/home/sandbox/.ironclaw/mcp-servers.json:ro",
                        config_path.display()
                    ));
                    tracing::debug!(
                        job_id = %job_id,
                        filtered = mcp_servers.is_some(),
                        "Mounted MCP config into container"
                    );
                }
                None => {
                    tracing::debug!(
                        job_id = %job_id,
                        "No MCP config to mount (master missing or empty filter list)"
                    );
                }
            }
        }

        // Claude Code mode: auth + tool allowlist.
        //
        // Auth strategies (first match wins):
        //   1. ANTHROPIC_API_KEY: direct API key (pay-as-you-go billing).
        //   2. CLAUDE_CODE_OAUTH_TOKEN: OAuth access token from `claude login`
        //      session, extracted from the host's credential store.
        if mode == JobMode::ClaudeCode {
            if let Some(ref api_key) = self.config.claude_code_api_key {
                env_vec.push(format!("ANTHROPIC_API_KEY={}", api_key));
            } else if let Some(ref oauth_token) = self.config.claude_code_oauth_token {
                env_vec.push(format!("CLAUDE_CODE_OAUTH_TOKEN={}", oauth_token));
            }
            if !self.config.claude_code_allowed_tools.is_empty() {
                env_vec.push(format!(
                    "CLAUDE_CODE_ALLOWED_TOOLS={}",
                    self.config.claude_code_allowed_tools.join(",")
                ));
            }
        }

        // Memory limit: Claude Code gets more memory
        let memory_mb = match mode {
            JobMode::ClaudeCode => self.config.claude_code_memory_limit_mb,
            JobMode::Worker => self.config.memory_limit_mb,
        };

        // Create the container
        use bollard::container::{Config, CreateContainerOptions};
        use bollard::models::HostConfig;

        let host_config = HostConfig {
            binds: if binds.is_empty() { None } else { Some(binds) },
            memory: Some((memory_mb * 1024 * 1024) as i64),
            cpu_shares: Some(self.config.cpu_shares as i64),
            network_mode: Some("bridge".to_string()),
            extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
            cap_drop: Some(vec!["ALL".to_string()]),
            cap_add: Some(vec!["CHOWN".to_string()]),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            tmpfs: Some(
                [("/tmp".to_string(), "size=512M".to_string())]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        };

        // Build CMD based on mode
        let cmd = match mode {
            JobMode::Worker => vec![
                "worker".to_string(),
                "--job-id".to_string(),
                job_id.to_string(),
                "--orchestrator-url".to_string(),
                orchestrator_url,
            ],
            JobMode::ClaudeCode => vec![
                "claude-bridge".to_string(),
                "--job-id".to_string(),
                job_id.to_string(),
                "--orchestrator-url".to_string(),
                orchestrator_url,
                "--max-turns".to_string(),
                self.config.claude_code_max_turns.to_string(),
                "--model".to_string(),
                self.config.claude_code_model.clone(),
            ],
        };

        // Add Docker labels for reaper identification and orphan detection
        let mut labels = std::collections::HashMap::new();
        labels.insert("ironclaw.job_id".to_string(), job_id.to_string());
        labels.insert(
            "ironclaw.created_at".to_string(),
            chrono::Utc::now().to_rfc3339(),
        );

        let container_config = Config {
            image: Some(self.config.image.clone()),
            cmd: Some(cmd),
            env: Some(env_vec),
            host_config: Some(host_config),
            user: Some("1000:1000".to_string()),
            working_dir: Some("/workspace".to_string()),
            labels: Some(labels),
            ..Default::default()
        };

        let container_name = match mode {
            JobMode::Worker => format!("ironclaw-worker-{}", job_id),
            JobMode::ClaudeCode => format!("ironclaw-claude-{}", job_id),
        };
        let options = CreateContainerOptions {
            name: container_name,
            ..Default::default()
        };

        let response = docker
            .create_container(Some(options), container_config)
            .await
            .map_err(|e| OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: e.to_string(),
            })?;

        let container_id = response.id;

        // Start the container
        docker
            .start_container::<String>(&container_id, None)
            .await
            .map_err(|e| OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: format!("failed to start container: {}", e),
            })?;

        // Update handle with container ID
        if let Some(handle) = self.containers.write().await.get_mut(&job_id) {
            handle.container_id = container_id;
            handle.state = ContainerState::Running;
        }

        tracing::info!(
            job_id = %job_id,
            "Created and started worker container"
        );

        Ok(())
    }

    /// Stop a running container job.
    pub async fn stop_job(&self, job_id: Uuid) -> Result<(), OrchestratorError> {
        let container_id = {
            let containers = self.containers.read().await;
            containers
                .get(&job_id)
                .map(|h| h.container_id.clone())
                .ok_or(OrchestratorError::ContainerNotFound { job_id })?
        };

        if container_id.is_empty() {
            return Err(OrchestratorError::InvalidContainerState {
                job_id,
                state: "creating (no container ID yet)".to_string(),
            });
        }

        let docker = self.docker().await?;

        // Stop the container (10 second grace period)
        if let Err(e) = docker
            .stop_container(
                &container_id,
                Some(bollard::container::StopContainerOptions { t: 10 }),
            )
            .await
        {
            tracing::warn!(job_id = %job_id, error = %e, "Failed to stop container (may already be stopped)");
        }

        // Remove the container
        if let Err(e) = docker
            .remove_container(
                &container_id,
                Some(bollard::container::RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            tracing::warn!(job_id = %job_id, error = %e, "Failed to remove container (may require manual cleanup)");
        }

        // Update state
        if let Some(handle) = self.containers.write().await.get_mut(&job_id) {
            handle.state = ContainerState::Stopped;
        }

        // Revoke the auth token
        self.token_store.revoke(job_id).await;

        tracing::info!(job_id = %job_id, "Stopped worker container");

        Ok(())
    }

    /// Mark a job as complete with a result. The container is stopped but the
    /// handle is kept so `CreateJobTool` can read the completion message.
    pub async fn complete_job(
        &self,
        job_id: Uuid,
        result: CompletionResult,
    ) -> Result<(), OrchestratorError> {
        // Store the result before stopping
        {
            let mut containers = self.containers.write().await;
            if let Some(handle) = containers.get_mut(&job_id) {
                handle.completion_result = Some(result);
                handle.state = ContainerState::Stopped;
            }
        }

        // Stop container and revoke token (but keep handle in map)
        let container_id = {
            let containers = self.containers.read().await;
            containers.get(&job_id).map(|h| h.container_id.clone())
        };
        if let Some(cid) = container_id
            && !cid.is_empty()
        {
            match self.docker().await {
                Ok(docker) => {
                    if let Err(e) = docker
                        .stop_container(
                            &cid,
                            Some(bollard::container::StopContainerOptions { t: 5 }),
                        )
                        .await
                    {
                        tracing::warn!(job_id = %job_id, error = %e, "Failed to stop completed container");
                    }
                    if let Err(e) = docker
                        .remove_container(
                            &cid,
                            Some(bollard::container::RemoveContainerOptions {
                                force: true,
                                ..Default::default()
                            }),
                        )
                        .await
                    {
                        tracing::warn!(job_id = %job_id, error = %e, "Failed to remove completed container");
                    }
                }
                Err(e) => {
                    tracing::warn!(job_id = %job_id, error = %e, "Failed to connect to Docker for container cleanup");
                }
            }
        }
        self.token_store.revoke(job_id).await;

        tracing::info!(job_id = %job_id, "Completed worker container");
        Ok(())
    }

    /// Remove a completed job handle from memory (called after result is read).
    pub async fn cleanup_job(&self, job_id: Uuid) {
        // Clean up per-job MCP config temp file if one was written.
        // Use remove_file directly — avoids TOCTOU race with exists() check.
        let tmp_path = std::env::temp_dir()
            .join("ironclaw-mcp-configs")
            .join(format!("{}.json", job_id));
        match std::fs::remove_file(&tmp_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // No temp file — normal
            Err(e) => {
                tracing::warn!(
                    job_id = %job_id,
                    error = %e,
                    "Failed to remove per-job MCP config temp file"
                );
            }
        }

        self.containers.write().await.remove(&job_id);
    }

    /// Update the worker-reported status for a job.
    pub async fn update_worker_status(
        &self,
        job_id: Uuid,
        message: Option<String>,
        iteration: u32,
    ) {
        if let Some(handle) = self.containers.write().await.get_mut(&job_id) {
            handle.last_worker_status = message;
            handle.worker_iteration = iteration;
        }
    }

    /// Get the handle for a job.
    pub async fn get_handle(&self, job_id: Uuid) -> Option<ContainerHandle> {
        self.containers.read().await.get(&job_id).cloned()
    }

    /// List all active container jobs.
    pub async fn list_jobs(&self) -> Vec<ContainerHandle> {
        self.containers.read().await.values().cloned().collect()
    }

    /// Get a reference to the token store.
    pub fn token_store(&self) -> &TokenStore {
        &self.token_store
    }
}

/// Generate a per-job MCP config file, optionally filtering to specific servers.
///
/// - `None` → mount the full master config as-is
/// - `Some([])` → no MCP config (no mount)
/// - `Some(["serpstat"])` → filtered config with only matching servers
///
/// Temp files are written to `<temp_dir>/ironclaw-mcp-configs/` and cleaned up
/// in `cleanup_job`.
fn generate_worker_mcp_config(
    master_path: &std::path::Path,
    server_names: Option<&[String]>,
    job_id: Uuid,
) -> Result<Option<std::path::PathBuf>, OrchestratorError> {
    if !master_path.exists() {
        return Ok(None);
    }

    match server_names {
        // No filter → use master config as-is
        None => Ok(Some(master_path.to_path_buf())),

        // Empty list → no MCP
        Some([]) => Ok(None),

        // Filter to specific servers
        Some(names) => {
            let content = std::fs::read_to_string(master_path).map_err(|e| {
                OrchestratorError::ContainerCreationFailed {
                    job_id,
                    reason: format!("failed to read master MCP config: {e}"),
                }
            })?;

            let master: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
                OrchestratorError::ContainerCreationFailed {
                    job_id,
                    reason: format!("failed to parse master MCP config: {e}"),
                }
            })?;

            let servers = master["servers"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|s| {
                    let name_matches = s["name"]
                        .as_str()
                        .map(|n| names.iter().any(|req| req.eq_ignore_ascii_case(n)))
                        .unwrap_or(false);
                    let is_enabled = s["enabled"].as_bool().unwrap_or(true);
                    name_matches && is_enabled
                })
                .collect::<Vec<_>>();

            if servers.is_empty() {
                tracing::warn!(
                    job_id = %job_id,
                    requested = ?names,
                    "No matching MCP servers found in master config; skipping MCP mount"
                );
                return Ok(None);
            }

            let schema_version = master
                .get("schema_version")
                .cloned()
                .unwrap_or(serde_json::json!(1));
            let filtered = serde_json::json!({
                "servers": servers,
                "schema_version": schema_version
            });

            let tmp_dir = std::env::temp_dir().join("ironclaw-mcp-configs");
            std::fs::create_dir_all(&tmp_dir).map_err(|e| {
                OrchestratorError::ContainerCreationFailed {
                    job_id,
                    reason: format!("failed to create MCP config temp dir: {e}"),
                }
            })?;

            // Restrict directory permissions to owner-only (0o700) to prevent
            // other users on the host from reading filtered MCP configs.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&tmp_dir, std::fs::Permissions::from_mode(0o700));
            }

            let tmp_path = tmp_dir.join(format!("{}.json", job_id));
            let config_json = serde_json::to_string_pretty(&filtered).map_err(|e| {
                OrchestratorError::ContainerCreationFailed {
                    job_id,
                    reason: format!("failed to serialize filtered MCP config: {e}"),
                }
            })?;
            std::fs::write(&tmp_path, config_json).map_err(|e| {
                OrchestratorError::ContainerCreationFailed {
                    job_id,
                    reason: format!("failed to write per-job MCP config: {e}"),
                }
            })?;

            Ok(Some(tmp_path))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_job_config_default() {
        let config = ContainerJobConfig::default();
        assert_eq!(config.orchestrator_port, 50051);
        assert_eq!(config.memory_limit_mb, 2048);
    }

    #[test]
    fn test_container_state_display() {
        assert_eq!(ContainerState::Running.to_string(), "running");
        assert_eq!(ContainerState::Stopped.to_string(), "stopped");
    }

    #[test]
    fn test_validate_bind_mount_valid_path() {
        let base = crate::bootstrap::compute_ironclaw_base_dir().join("projects");
        std::fs::create_dir_all(&base).unwrap();

        let test_dir = base.join("test_validate_bind");
        std::fs::create_dir_all(&test_dir).unwrap();

        let result = validate_bind_mount_path(&test_dir, Uuid::new_v4());
        assert!(result.is_ok());
        let canonical = result.unwrap();
        assert!(canonical.starts_with(base.canonicalize().unwrap()));

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_validate_bind_mount_rejects_outside_base() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().to_path_buf();

        let result = validate_bind_mount_path(&outside, Uuid::new_v4());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("outside allowed base"),
            "expected 'outside allowed base', got: {}",
            err
        );
    }

    #[test]
    fn test_validate_bind_mount_rejects_nonexistent() {
        let nonexistent = PathBuf::from("/no/such/path/at/all");
        let result = validate_bind_mount_path(&nonexistent, Uuid::new_v4());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("canonicalize"),
            "expected canonicalize error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_update_worker_status() {
        let store = TokenStore::new();
        let mgr = ContainerJobManager::new(ContainerJobConfig::default(), store);
        let job_id = Uuid::new_v4();

        // Insert a handle
        {
            let mut containers = mgr.containers.write().await;
            containers.insert(
                job_id,
                ContainerHandle {
                    job_id,
                    container_id: "test".to_string(),
                    state: ContainerState::Running,
                    mode: JobMode::Worker,
                    created_at: chrono::Utc::now(),
                    project_dir: None,
                    task_description: "test job".to_string(),
                    last_worker_status: None,
                    worker_iteration: 0,
                    completion_result: None,
                },
            );
        }

        mgr.update_worker_status(job_id, Some("Iteration 3".to_string()), 3)
            .await;

        let handle = mgr.get_handle(job_id).await.unwrap();
        assert_eq!(handle.worker_iteration, 3);
        assert_eq!(handle.last_worker_status.as_deref(), Some("Iteration 3"));
    }

    // ── generate_worker_mcp_config tests ────────────────────────────

    #[test]
    fn test_mcp_config_none_filter_returns_master_path() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), r#"{"servers":[]}"#).unwrap();
        let result = generate_worker_mcp_config(tmp.path(), None, Uuid::new_v4());
        assert_eq!(result.unwrap(), Some(tmp.path().to_path_buf()));
    }

    #[test]
    fn test_mcp_config_empty_filter_returns_none() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), r#"{"servers":[]}"#).unwrap();
        let result = generate_worker_mcp_config(tmp.path(), Some(&[]), Uuid::new_v4());
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn test_mcp_config_missing_master_returns_none() {
        let result = generate_worker_mcp_config(
            std::path::Path::new("/nonexistent/mcp.json"),
            None,
            Uuid::new_v4(),
        );
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn test_mcp_config_filters_to_named_servers() {
        let job_id = Uuid::new_v4();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"{"schema_version":1,"servers":[
                {"name":"serpstat","enabled":true,"url":"http://localhost:8062"},
                {"name":"notion","enabled":true,"url":"http://localhost:8063"},
                {"name":"disabled","enabled":false,"url":"http://localhost:9999"}
            ]}"#,
        )
        .unwrap();

        let names = vec!["serpstat".to_string(), "disabled".to_string()];
        let result = generate_worker_mcp_config(tmp.path(), Some(&names), job_id);
        let out_path = result.unwrap().expect("should produce a filtered config");

        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
        let servers = content["servers"].as_array().unwrap();

        // "disabled" should be excluded because enabled=false
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["name"], "serpstat");
        assert_eq!(content["schema_version"], 1);

        // cleanup
        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn test_mcp_config_no_match_returns_none() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"{"servers":[{"name":"serpstat","enabled":true}]}"#,
        )
        .unwrap();

        let names = vec!["nonexistent".to_string()];
        let result = generate_worker_mcp_config(tmp.path(), Some(&names), Uuid::new_v4());
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn test_mcp_config_case_insensitive_match() {
        let job_id = Uuid::new_v4();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"{"servers":[{"name":"Serpstat","enabled":true}]}"#,
        )
        .unwrap();

        let names = vec!["serpstat".to_string()];
        let result = generate_worker_mcp_config(tmp.path(), Some(&names), job_id);
        let out_path = result.unwrap().expect("case-insensitive match should work");
        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn test_max_iterations_env_var_injected() {
        // Verify the IRONCLAW_MAX_ITERATIONS env var name matches what the
        // worker CLI reads via clap's `env` attribute.
        let config = ContainerJobConfig::default();
        let mgr = ContainerJobManager::new(config, TokenStore::new());
        // We can't test actual container creation without Docker, but we can
        // verify the env var name matches the clap definition.
        // The clap definition uses: #[arg(long, env = "IRONCLAW_MAX_ITERATIONS")]
        // The create_job_inner injects: format!("IRONCLAW_MAX_ITERATIONS={}", iters)
        // This test ensures the constant isn't accidentally changed in either place.
        let env_var_in_job = "IRONCLAW_MAX_ITERATIONS";
        let source = include_str!("../cli/mod.rs");
        assert!(
            source.contains(&format!("env = \"{}\"", env_var_in_job)),
            "cli/mod.rs must have env = \"IRONCLAW_MAX_ITERATIONS\" on the max_iterations arg"
        );
        drop(mgr);
    }

    // ── Regression tests (CI-required) ────────────────────────────────

    #[test]
    fn test_filtered_config_contains_only_requested_server() {
        let job_id = Uuid::new_v4();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"{"schema_version":2,"servers":[
                {"name":"serpstat","enabled":true,"url":"http://localhost:8062"},
                {"name":"notion","enabled":true,"url":"http://localhost:8063"},
                {"name":"archon","enabled":true,"url":"http://localhost:8064"}
            ]}"#,
        )
        .unwrap();

        let names = vec!["serpstat".to_string()];
        let result = generate_worker_mcp_config(tmp.path(), Some(&names), job_id);
        let out_path = result.unwrap().expect("should produce a filtered config");

        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
        let servers = content["servers"].as_array().unwrap();

        assert_eq!(servers.len(), 1, "only serpstat should be present");
        assert_eq!(servers[0]["name"], "serpstat");
        assert!(
            !servers.iter().any(|s| s["name"] == "notion"),
            "notion must not leak into filtered config"
        );
        assert!(
            !servers.iter().any(|s| s["name"] == "archon"),
            "archon must not leak into filtered config"
        );
        assert_eq!(
            content["schema_version"], 2,
            "schema_version must be preserved"
        );

        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn test_feature_flag_disabled_skips_mcp_filtering() {
        // When MCP_PER_JOB_ENABLED is false (the default), the mcp_servers
        // parameter should be ignored and no filtered config should be created.
        let config = ContainerJobConfig::default();
        assert!(
            !config.mcp_per_job_enabled,
            "mcp_per_job_enabled must default to false"
        );

        // Verify the gate in create_job_inner: the mcp_per_job_enabled field
        // controls whether generate_worker_mcp_config is called at all.
        let source = include_str!("job_manager.rs");
        assert!(
            source.contains("if self.config.mcp_per_job_enabled"),
            "create_job_inner must gate MCP filtering on config.mcp_per_job_enabled"
        );
    }

    #[test]
    fn test_temp_file_cleanup_removes_per_job_config() {
        let job_id = Uuid::new_v4();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"{"servers":[{"name":"serpstat","enabled":true}]}"#,
        )
        .unwrap();

        let names = vec!["serpstat".to_string()];
        let result = generate_worker_mcp_config(tmp.path(), Some(&names), job_id);
        let out_path = result.unwrap().expect("should produce a filtered config");
        assert!(
            out_path.exists(),
            "temp config file should exist after creation"
        );

        // Simulate what cleanup_job does
        let expected_path = std::env::temp_dir()
            .join("ironclaw-mcp-configs")
            .join(format!("{}.json", job_id));
        assert_eq!(
            out_path, expected_path,
            "temp path must match cleanup expectation"
        );
        std::fs::remove_file(&out_path).unwrap();
        assert!(!out_path.exists(), "temp file should be gone after cleanup");
    }

    #[tokio::test]
    async fn test_cleanup_job_is_idempotent() {
        let config = ContainerJobConfig::default();
        let mgr = ContainerJobManager::new(config, TokenStore::new());
        let job_id = Uuid::new_v4();

        // cleanup_job should not panic or error when called for a job
        // that has no temp file and no container handle.
        mgr.cleanup_job(job_id).await;
        // Second call should also be fine (idempotent).
        mgr.cleanup_job(job_id).await;
    }

    #[cfg(unix)]
    #[test]
    fn test_temp_dir_has_restrictive_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let job_id = Uuid::new_v4();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"{"servers":[{"name":"test","enabled":true}]}"#,
        )
        .unwrap();

        let names = vec!["test".to_string()];
        let result = generate_worker_mcp_config(tmp.path(), Some(&names), job_id);
        let out_path = result.unwrap().expect("should produce a filtered config");

        let dir_path = out_path.parent().unwrap();
        let mode = std::fs::metadata(dir_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o700,
            "ironclaw-mcp-configs dir must be 0700, got {:o}",
            mode
        );

        let _ = std::fs::remove_file(&out_path);
    }
}
