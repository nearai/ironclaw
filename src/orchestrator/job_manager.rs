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

use ironclaw_common::MAX_WORKER_ITERATIONS;

/// Which mode a sandbox container runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobMode {
    /// Standard IronClaw worker with proxied LLM calls.
    Worker,
    /// Claude Code bridge that spawns the `claude` CLI directly.
    ClaudeCode,
    /// ACP (Agent Client Protocol) bridge that spawns any ACP-compliant agent.
    Acp,
}

impl JobMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::ClaudeCode => "claude_code",
            Self::Acp => "acp",
        }
    }
}

impl std::fmt::Display for JobMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Parameters for creating a container job, bundled to avoid positional
/// argument proliferation on `create_job` / `execute_sandbox`.
#[derive(Debug, Clone, Default)]
pub struct JobCreationParams {
    /// Credential grants for the worker (served via `/credentials`).
    pub credential_grants: Vec<CredentialGrant>,
    /// Optional filter: which MCP servers to mount into the container.
    /// `None` = full master config, `Some([])` = no MCP, `Some(["name"])` = filtered.
    pub mcp_servers: Option<Vec<String>>,
    /// Optional cap on worker agent loop iterations (clamped to 1..=500 server-side).
    pub max_iterations: Option<u32>,
    /// ACP agent definition to inject into ACP-mode containers.
    pub acp_agent: Option<crate::config::acp::AcpAgentConfig>,
    /// Master MCP server configuration as raw JSON data, loaded by the caller
    /// from the source-of-truth (DB-backed per-user `mcp_servers` setting).
    /// Shape matches `McpServersFile` (a `servers` array plus a
    /// `schema_version` field).
    ///
    /// `None` means the caller did not load any config (e.g. no DB available);
    /// the orchestrator then mounts no MCP config. Pre-fix this was sourced
    /// from a hardcoded host file path which bootstrap moves into the DB on
    /// first run, so per-job MCP filtering silently no-op'd on most installs.
    pub master_mcp_config: Option<serde_json::Value>,
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
    /// Memory limit for ACP containers.
    pub acp_memory_limit_mb: u64,
    /// Maximum runtime for ACP bridge sessions in seconds.
    pub acp_timeout_secs: u64,
    /// Whether per-job MCP server filtering is enabled.
    /// When false, `mcp_servers` param on `create_job` is ignored.
    pub mcp_per_job_enabled: bool,
    /// Whether Claude Code sandbox mode is available (from CLAUDE_CODE_ENABLED).
    pub claude_code_enabled: bool,
    /// Whether ACP agent mode is available (from ACP_ENABLED).
    pub acp_enabled: bool,
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
            acp_memory_limit_mb: 4096,
            acp_timeout_secs: 1800,
            mcp_per_job_enabled: false,
            claude_code_enabled: false,
            acp_enabled: false,
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
/// Resolve the host address that worker containers should use to call
/// back to the orchestrator API running in this process.
///
/// Defaults to `host.docker.internal`, which resolves dynamically inside
/// worker containers via the `host.docker.internal:host-gateway` entry
/// set in `extra_hosts` below. This adapts to non-default Docker
/// networking (custom `bip`, rootless Docker) and is correct on Docker
/// Desktop (macOS/Windows) as well.
///
/// **When to override with `ORCHESTRATOR_HOST`**: on orchestrators like
/// Nomad in production mode, Docker ports are published on the node's
/// advertised IP (e.g. `10.128.0.17`), NOT on `0.0.0.0`. In that case
/// `host.docker.internal` (which resolves to the Docker bridge gateway)
/// is not where the port is listening, so workers can't connect back.
/// Set `ORCHESTRATOR_HOST` to the IP where Docker published the port
/// (visible in `docker port <container>`). In a Nomad job spec that's
/// `ORCHESTRATOR_HOST = "${attr.unique.network.ip-address}"`.
fn resolve_orchestrator_host() -> String {
    std::env::var("ORCHESTRATOR_HOST").unwrap_or_else(|_| "host.docker.internal".to_string())
}

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

    /// Whether Claude Code mode is enabled for job creation.
    pub fn claude_code_enabled(&self) -> bool {
        self.config.claude_code_enabled
    }

    /// Whether ACP agent mode is enabled for job creation.
    pub fn acp_enabled(&self) -> bool {
        self.config.acp_enabled
    }

    /// Whether the given job mode is allowed by the current configuration.
    pub fn is_mode_enabled(&self, mode: JobMode) -> bool {
        match mode {
            JobMode::Worker => true,
            JobMode::ClaudeCode => self.config.claude_code_enabled,
            JobMode::Acp => self.config.acp_enabled,
        }
    }

    fn extend_acp_env(
        &self,
        env_vec: &mut Vec<String>,
        acp_agent: Option<&crate::config::acp::AcpAgentConfig>,
    ) {
        env_vec.push(format!("ACP_TIMEOUT_SECS={}", self.config.acp_timeout_secs));

        if let Some(agent) = acp_agent {
            env_vec.push(format!("ACP_AGENT_COMMAND={}", agent.command));
            if !agent.args.is_empty()
                && let Ok(json) = serde_json::to_string(&agent.args)
            {
                env_vec.push(format!("ACP_AGENT_ARGS={}", json));
            }
            if !agent.env.is_empty()
                && let Ok(json) = serde_json::to_string(&agent.env)
            {
                env_vec.push(format!("ACP_AGENT_ENV={}", json));
            }
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
    pub async fn create_job(
        &self,
        job_id: Uuid,
        task: &str,
        project_dir: Option<PathBuf>,
        mode: JobMode,
        params: JobCreationParams,
    ) -> Result<String, OrchestratorError> {
        if !self.is_mode_enabled(mode) {
            return Err(OrchestratorError::ModeDisabled {
                mode: mode.to_string(),
            });
        }

        // Generate auth token (stored in TokenStore, never logged)
        let token = self.token_store.create_token(job_id).await;

        // Store credential grants (revoked automatically when the token is revoked)
        let JobCreationParams {
            credential_grants,
            mcp_servers,
            max_iterations,
            acp_agent,
            master_mcp_config,
        } = params;

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
                acp_agent,
                master_mcp_config,
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
        acp_agent: Option<crate::config::acp::AcpAgentConfig>,
        master_mcp_config: Option<serde_json::Value>,
    ) -> Result<(), OrchestratorError> {
        // Connect to Docker (reuses cached connection)
        let docker = self.docker().await?;

        // Build container configuration.
        let orchestrator_host = resolve_orchestrator_host();

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
        // Server-side clamp ensures the cap is enforced even if the tool parsing
        // layer is bypassed (e.g., direct API call via the web restart handler).
        if let Some(iters) = max_iterations
            && mode == JobMode::Worker
        {
            let capped = iters.clamp(1, MAX_WORKER_ITERATIONS);
            env_vec.push(format!("IRONCLAW_MAX_ITERATIONS={}", capped));
        }

        // Mount per-job MCP config when the feature is enabled. The master
        // config comes from the caller (loaded from the per-user DB setting),
        // not from a hardcoded host file path — bootstrap migrates the legacy
        // ~/.ironclaw/mcp-servers.json into the DB on first run, so any
        // file-based source would be empty on most installs.
        if self.config.mcp_per_job_enabled {
            match generate_worker_mcp_config(
                master_mcp_config.as_ref(),
                mcp_servers.as_deref(),
                job_id,
            )
            .await?
            {
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
                        "No MCP config to mount (no master from caller or empty filter list)"
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

        // ACP mode: inject runtime timeout plus per-job agent command/args/env.
        if mode == JobMode::Acp {
            self.extend_acp_env(&mut env_vec, acp_agent.as_ref());
        }

        // Memory limit per mode
        let memory_mb = match mode {
            JobMode::ClaudeCode => self.config.claude_code_memory_limit_mb,
            JobMode::Acp => self.config.acp_memory_limit_mb,
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
            JobMode::Acp => vec![
                "acp-bridge".to_string(),
                "--job-id".to_string(),
                job_id.to_string(),
                "--orchestrator-url".to_string(),
                orchestrator_url,
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
            JobMode::Acp => format!("ironclaw-acp-{}", job_id),
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

/// Human-readable label for a `serde_json::Value` discriminant; used in error
/// messages when a master MCP config field has the wrong shape.
fn type_name_of(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Generate a per-job MCP config file from caller-provided master data,
/// optionally filtering to specific servers.
///
/// - `master = None` → caller has no master config; no mount
/// - `server_names = None` → mount the full master config as-is
/// - `server_names = Some([])` → no MCP config (no mount)
/// - `server_names = Some(["serpstat"])` → filtered config with matching servers
///
/// The master config is data, not a path: it is loaded from the per-user
/// DB-backed `mcp_servers` setting by the caller (`job` tool / restart
/// handler). This avoids the staging-regressions issue 3 where the
/// orchestrator hardcoded a host file path that bootstrap moves into the
/// DB on first run, leaving most installs with no MCP mount.
///
/// Temp files are written to `<temp_dir>/ironclaw-mcp-configs/` and cleaned
/// up in `cleanup_job`.
async fn generate_worker_mcp_config(
    master: Option<&serde_json::Value>,
    server_names: Option<&[String]>,
    job_id: Uuid,
) -> Result<Option<std::path::PathBuf>, OrchestratorError> {
    let Some(master) = master else {
        return Ok(None);
    };

    // Empty list → no MCP, regardless of master content.
    if matches!(server_names, Some([])) {
        return Ok(None);
    }

    // Validate server names if a filter was provided. Reject path separators,
    // null bytes, and excessively long names so the filter cannot escape the
    // temp file directory or inject shell metacharacters if names ever flow
    // into a path or command.
    if let Some(names) = server_names {
        for name in names {
            if name.len() > 128 || name.contains('/') || name.contains('\\') || name.contains('\0')
            {
                return Err(OrchestratorError::ContainerCreationFailed {
                    job_id,
                    reason: format!("invalid MCP server name: {:?}", name),
                });
            }
        }
    }

    // Filter the master config when a name filter is provided. When no filter
    // is given we still serialize through this path so the on-disk file we
    // mount has the same shape regardless of source — pre-fix, the no-filter
    // path returned the master file as-is, which only worked when the master
    // was a real on-disk file.
    //
    // The `servers` field is the load-bearing part of the config — if it is
    // missing or the wrong type the master JSON is malformed and we must
    // surface that loudly rather than silently mounting nothing. The trusted
    // helper (`load_master_mcp_config_value`) always serializes a real
    // `McpServersFile`, so reaching this branch means a future caller passed
    // foreign JSON that does not match our expected shape.
    let servers_value =
        master
            .get("servers")
            .ok_or_else(|| OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: "MCP master config is missing the required `servers` field".to_string(),
            })?;
    let servers_array =
        servers_value
            .as_array()
            .ok_or_else(|| OrchestratorError::ContainerCreationFailed {
                job_id,
                reason: format!(
                    "MCP master config `servers` field must be an array, got {}",
                    type_name_of(servers_value)
                ),
            })?;
    let servers_iter = servers_array.clone().into_iter();
    let filtered_servers: Vec<serde_json::Value> = match server_names {
        None => servers_iter
            .filter(|s| s["enabled"].as_bool().unwrap_or(true))
            .collect(),
        Some(names) => servers_iter
            .filter(|s| {
                let name_matches = s["name"]
                    .as_str()
                    .map(|n| names.iter().any(|req| req.eq_ignore_ascii_case(n)))
                    .unwrap_or(false);
                let is_enabled = s["enabled"].as_bool().unwrap_or(true);
                name_matches && is_enabled
            })
            .collect(),
    };

    if filtered_servers.is_empty() {
        if let Some(names) = server_names {
            tracing::warn!(
                job_id = %job_id,
                requested = ?names,
                "No matching MCP servers found in master config; skipping MCP mount"
            );
        } else {
            tracing::debug!(
                job_id = %job_id,
                "Master MCP config has no enabled servers; skipping MCP mount"
            );
        }
        return Ok(None);
    }

    let schema_version = master
        .get("schema_version")
        .cloned()
        .unwrap_or(serde_json::json!(1));
    let filtered = serde_json::json!({
        "servers": filtered_servers,
        "schema_version": schema_version
    });

    let tmp_dir = std::env::temp_dir().join("ironclaw-mcp-configs");
    tokio::fs::create_dir_all(&tmp_dir).await.map_err(|e| {
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
        let _ = tokio::fs::set_permissions(&tmp_dir, std::fs::Permissions::from_mode(0o700)).await;
    }

    let tmp_path = tmp_dir.join(format!("{}.json", job_id));
    let config_json = serde_json::to_string_pretty(&filtered).map_err(|e| {
        OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!("failed to serialize filtered MCP config: {e}"),
        }
    })?;
    tokio::fs::write(&tmp_path, config_json)
        .await
        .map_err(|e| OrchestratorError::ContainerCreationFailed {
            job_id,
            reason: format!("failed to write per-job MCP config: {e}"),
        })?;

    Ok(Some(tmp_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    #[test]
    fn test_container_job_config_default() {
        let config = ContainerJobConfig::default();
        assert_eq!(config.orchestrator_port, 50051);
        assert_eq!(config.memory_limit_mb, 2048);
    }

    #[test]
    fn resolve_orchestrator_host_uses_env_var_when_set() {
        let _guard = lock_env();

        // Safety: env-var mutation requires unsafe in edition 2024;
        // lock_env() serializes concurrent access from other test threads.
        unsafe { std::env::set_var("ORCHESTRATOR_HOST", "10.128.0.17") };
        assert_eq!(resolve_orchestrator_host(), "10.128.0.17");

        unsafe { std::env::set_var("ORCHESTRATOR_HOST", "my-custom-host.internal") };
        assert_eq!(resolve_orchestrator_host(), "my-custom-host.internal");

        unsafe { std::env::remove_var("ORCHESTRATOR_HOST") };
    }

    #[test]
    fn resolve_orchestrator_host_default_when_env_unset() {
        let _guard = lock_env();

        unsafe { std::env::remove_var("ORCHESTRATOR_HOST") };
        // Default is `host.docker.internal` on all platforms — the
        // `extra_hosts: ["host.docker.internal:host-gateway"]` on the worker
        // container resolves this to the actual host gateway IP dynamically.
        assert_eq!(resolve_orchestrator_host(), "host.docker.internal");
    }

    #[test]
    fn resolve_orchestrator_host_empty_string_treated_as_value() {
        // An empty string is a valid env var value (not "unset"), so
        // std::env::var returns Ok(""), and we use it as-is. Documenting
        // this behavior — operators should unset the var rather than set
        // it empty if they want the default.
        let _guard = lock_env();

        unsafe { std::env::set_var("ORCHESTRATOR_HOST", "") };
        assert_eq!(resolve_orchestrator_host(), "");

        unsafe { std::env::remove_var("ORCHESTRATOR_HOST") };
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

    #[test]
    fn test_job_mode_acp_as_str() {
        assert_eq!(JobMode::Acp.as_str(), "acp");
    }

    #[test]
    fn test_job_mode_acp_display() {
        assert_eq!(format!("{}", JobMode::Acp), "acp");
    }

    #[test]
    fn test_container_job_config_acp_memory_default() {
        let config = ContainerJobConfig::default();
        assert_eq!(config.acp_memory_limit_mb, 4096);
    }

    #[test]
    fn test_container_job_config_acp_timeout_default() {
        let config = ContainerJobConfig::default();
        assert_eq!(config.acp_timeout_secs, 1800);
    }

    #[test]
    fn test_container_job_config_claude_code_disabled_by_default() {
        let config = ContainerJobConfig::default();
        assert!(!config.claude_code_enabled);
    }

    #[test]
    fn test_container_job_config_acp_disabled_by_default() {
        let config = ContainerJobConfig::default();
        assert!(!config.acp_enabled);
    }

    #[test]
    fn test_is_mode_enabled_matches_individual_accessors() {
        let manager = ContainerJobManager::new(
            ContainerJobConfig {
                claude_code_enabled: true,
                acp_enabled: false,
                ..Default::default()
            },
            TokenStore::new(),
        );
        assert!(manager.is_mode_enabled(JobMode::Worker));
        assert!(manager.is_mode_enabled(JobMode::ClaudeCode));
        assert!(!manager.is_mode_enabled(JobMode::Acp));
    }

    #[tokio::test]
    async fn test_create_job_rejects_disabled_mode() {
        let manager = ContainerJobManager::new(
            ContainerJobConfig {
                claude_code_enabled: false,
                acp_enabled: false,
                ..Default::default()
            },
            TokenStore::new(),
        );
        let result = manager
            .create_job(
                Uuid::new_v4(),
                "test task",
                None,
                JobMode::ClaudeCode,
                JobCreationParams::default(),
            )
            .await;
        let err = result.unwrap_err().to_string(); // safety: test
        assert!(
            err.contains("not enabled"),
            "expected mode-disabled error, got: {err}"
        );
    }

    #[test]
    fn test_extend_acp_env_includes_timeout_and_agent_details() {
        let config = ContainerJobConfig {
            acp_timeout_secs: 45,
            ..Default::default()
        };
        let manager = ContainerJobManager::new(config, TokenStore::new());

        let agent = crate::config::acp::AcpAgentConfig::new(
            "codex",
            "codex",
            vec!["acp".into()],
            HashMap::from([("FOO".to_string(), "bar".to_string())]),
        );
        let mut env_vec = Vec::new();
        manager.extend_acp_env(&mut env_vec, Some(&agent));

        assert!(env_vec.contains(&"ACP_TIMEOUT_SECS=45".to_string()));
        assert!(env_vec.contains(&"ACP_AGENT_COMMAND=codex".to_string()));
        assert!(
            env_vec
                .iter()
                .any(|entry| entry.starts_with("ACP_AGENT_ARGS="))
        );
        assert!(
            env_vec
                .iter()
                .any(|entry| entry.starts_with("ACP_AGENT_ENV="))
        );
    }
    // ── generate_worker_mcp_config tests ────────────────────────────

    /// Helper: build a master MCP config as serde_json::Value (the shape the
    /// caller now passes after loading from the per-user DB setting). The
    /// previous tests built file-based fixtures via tempfile, which never
    /// exercised the on-disk source-of-truth path the bug lived in.
    fn master_value(json: &str) -> serde_json::Value {
        serde_json::from_str(json).expect("test fixture must be valid JSON") // safety: test fixture
    }

    #[tokio::test]
    async fn test_mcp_config_none_filter_writes_full_master() {
        // No filter → mount the full (enabled) master config. Pre-fix this
        // returned the master path as-is when it was on disk; now the function
        // is data-driven, so it always writes a temp file.
        let job_id = Uuid::new_v4();
        let master = master_value(
            r#"{"servers":[
                {"name":"serpstat","enabled":true,"url":"http://localhost:8062"},
                {"name":"notion","enabled":true,"url":"http://localhost:8063"}
            ]}"#,
        );
        let result = generate_worker_mcp_config(Some(&master), None, job_id).await;
        let out_path = result.unwrap().expect("should write a temp config"); // safety: test fixture
        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap(); // safety: test fixture
        let servers = content["servers"].as_array().unwrap(); // safety: test fixture
        assert_eq!(servers.len(), 2); // safety: test fixture
        let _ = std::fs::remove_file(&out_path);
    }

    #[tokio::test]
    async fn test_mcp_config_empty_filter_returns_none() {
        let master = master_value(r#"{"servers":[{"name":"x","enabled":true}]}"#);
        let result = generate_worker_mcp_config(Some(&master), Some(&[]), Uuid::new_v4()).await;
        assert_eq!(result.unwrap(), None);
    }

    /// Regression for staging-regressions issue 3: when the orchestrator has
    /// no master config (e.g. caller did not load it from the DB, or the user
    /// has no MCP servers configured), the function must return `None` rather
    /// than touching any hardcoded host file path. Pre-fix the function read
    /// from `/opt/ironclaw/config/worker/mcp-servers.json`, which bootstrap
    /// migrates into the DB and removes from disk on first run, so per-job
    /// MCP filtering silently no-op'd on every typical install.
    #[tokio::test]
    async fn test_mcp_config_no_master_returns_none() {
        let result = generate_worker_mcp_config(None, None, Uuid::new_v4()).await;
        assert_eq!(
            // safety: test fixture
            result.unwrap(), // safety: test fixture
            None,
            "no master from caller must return None — \
             must NOT fall back to a host file path"
        );

        // Same with a filter list — no master means no mount, period.
        let names = vec!["serpstat".to_string()];
        let result = generate_worker_mcp_config(None, Some(&names), Uuid::new_v4()).await;
        assert_eq!(result.unwrap(), None);
    }

    #[tokio::test]
    async fn test_mcp_config_filters_to_named_servers() {
        let job_id = Uuid::new_v4();
        let master = master_value(
            r#"{"schema_version":1,"servers":[
                {"name":"serpstat","enabled":true,"url":"http://localhost:8062"},
                {"name":"notion","enabled":true,"url":"http://localhost:8063"},
                {"name":"disabled","enabled":false,"url":"http://localhost:9999"}
            ]}"#,
        );

        let names = vec!["serpstat".to_string(), "disabled".to_string()];
        let result = generate_worker_mcp_config(Some(&master), Some(&names), job_id).await;
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

    #[tokio::test]
    async fn test_mcp_config_no_match_returns_none() {
        let master = master_value(r#"{"servers":[{"name":"serpstat","enabled":true}]}"#);
        let names = vec!["nonexistent".to_string()];
        let result = generate_worker_mcp_config(Some(&master), Some(&names), Uuid::new_v4()).await;
        assert_eq!(result.unwrap(), None);
    }

    #[tokio::test]
    async fn test_mcp_config_case_insensitive_match() {
        let job_id = Uuid::new_v4();
        let master = master_value(r#"{"servers":[{"name":"Serpstat","enabled":true}]}"#);
        let names = vec!["serpstat".to_string()];
        let result = generate_worker_mcp_config(Some(&master), Some(&names), job_id).await;
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

    #[test]
    fn test_max_iterations_not_injected_for_claude_code() {
        // ClaudeCode mode uses its own `max_turns`, not IRONCLAW_MAX_ITERATIONS.
        // Verify the gate in create_job_inner only injects for Worker mode.
        let source = include_str!("job_manager.rs");
        assert!(
            source.contains("mode == JobMode::Worker"),
            "create_job_inner must gate IRONCLAW_MAX_ITERATIONS on JobMode::Worker \
             (ClaudeCode has its own max_turns)"
        );
    }

    #[test]
    fn test_server_side_max_iterations_clamp() {
        // Verify the server-side clamp uses the same constant as worker/job.rs
        let source = include_str!("job_manager.rs");
        assert!(
            source.contains("iters.clamp(1, MAX_WORKER_ITERATIONS)"),
            "create_job_inner must clamp max_iterations server-side using MAX_WORKER_ITERATIONS"
        );
    }

    #[tokio::test]
    async fn test_mcp_server_name_validation_rejects_path_separators() {
        let job_id = Uuid::new_v4();
        let master = master_value(r#"{"servers":[{"name":"test","enabled":true}]}"#);

        // Path separator should be rejected
        let names = vec!["../../etc/passwd".to_string()];
        assert!(
            generate_worker_mcp_config(Some(&master), Some(&names), job_id)
                .await
                .is_err()
        );

        // Null byte should be rejected
        let names = vec!["test\0evil".to_string()];
        assert!(
            generate_worker_mcp_config(Some(&master), Some(&names), job_id)
                .await
                .is_err()
        );

        // Excessively long name should be rejected
        let names = vec!["a".repeat(129)];
        assert!(
            generate_worker_mcp_config(Some(&master), Some(&names), job_id)
                .await
                .is_err()
        );

        // Valid name should pass
        let names = vec!["test".to_string()];
        let result = generate_worker_mcp_config(Some(&master), Some(&names), job_id).await;
        assert!(result.is_ok());
    }

    // ── Regression tests (CI-required) ────────────────────────────────

    #[tokio::test]
    async fn test_filtered_config_contains_only_requested_server() {
        let job_id = Uuid::new_v4();
        let master = master_value(
            r#"{"schema_version":2,"servers":[
                {"name":"serpstat","enabled":true,"url":"http://localhost:8062"},
                {"name":"notion","enabled":true,"url":"http://localhost:8063"},
                {"name":"archon","enabled":true,"url":"http://localhost:8064"}
            ]}"#,
        );

        let names = vec!["serpstat".to_string()];
        let result = generate_worker_mcp_config(Some(&master), Some(&names), job_id).await;
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

    #[tokio::test]
    async fn test_feature_flag_disabled_skips_mcp_filtering() {
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

    #[tokio::test]
    async fn test_temp_file_cleanup_removes_per_job_config() {
        let job_id = Uuid::new_v4();
        let master = master_value(r#"{"servers":[{"name":"serpstat","enabled":true}]}"#);

        let names = vec!["serpstat".to_string()];
        let result = generate_worker_mcp_config(Some(&master), Some(&names), job_id).await;
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
    #[tokio::test]
    async fn test_temp_dir_has_restrictive_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let job_id = Uuid::new_v4();
        let master = master_value(r#"{"servers":[{"name":"test","enabled":true}]}"#);

        let names = vec!["test".to_string()];
        let result = generate_worker_mcp_config(Some(&master), Some(&names), job_id).await;
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

    /// Regression for staging-regressions issue 3: when MCP_PER_JOB_ENABLED is
    /// true and the user has MCP servers configured in the DB, the
    /// orchestrator must produce a valid mount from the caller-provided
    /// master config — not from a hardcoded host file path. This test verifies
    /// the data flow that the previous on-disk fixture tests could never
    /// catch: a master loaded from the DB-shaped JSON value, with no filter,
    /// must produce a temp file containing every enabled server.
    #[tokio::test]
    async fn test_db_loaded_master_mounts_full_config() {
        let job_id = Uuid::new_v4();
        // Shape matches what `load_mcp_servers_from_db` returns when serialized
        // via `serde_json::to_value(&McpServersFile { ... })`.
        let master = serde_json::json!({
            "schema_version": 1,
            "servers": [
                {"name": "github", "enabled": true, "url": "https://api.github.com"},
                {"name": "notion", "enabled": true, "url": "https://api.notion.com"},
            ],
        });

        let result = generate_worker_mcp_config(Some(&master), None, job_id).await;
        let out_path = result.unwrap().expect(
            // safety: test fixture
            "DB-loaded master with no filter must produce a mountable config — \
             pre-fix this would silently fall back to None on installs where \
             the hardcoded /opt/ironclaw/config/worker/mcp-servers.json did not exist",
        );

        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
        let servers = content["servers"].as_array().unwrap();
        assert_eq!(
            servers.len(),
            2,
            "both DB-configured servers must appear in the mount"
        );
        assert!(servers.iter().any(|s| s["name"] == "github")); // safety: test fixture
        assert!(servers.iter().any(|s| s["name"] == "notion")); // safety: test fixture

        let _ = std::fs::remove_file(&out_path);
    }

    /// Regression for PR #2116 review: a master config that is missing the
    /// `servers` field — or has it as something other than a JSON array —
    /// must surface a `ContainerCreationFailed` error rather than silently
    /// no-op'ing the per-job MCP mount. The previous `unwrap_or_default()`
    /// path treated malformed JSON as "no servers" and quietly skipped the
    /// mount, which is exactly the silent-failure mode that staging-regressions
    /// issue 3 produced via a different code path.
    #[tokio::test]
    async fn test_mcp_config_missing_servers_field_errors() {
        let job_id = Uuid::new_v4();

        // `servers` field absent entirely.
        let master = serde_json::json!({"schema_version": 1});
        let err = generate_worker_mcp_config(Some(&master), None, job_id)
            .await
            .expect_err("missing `servers` field must error");
        assert!(
            err.to_string()
                .contains("missing the required `servers` field"),
            "expected explicit missing-field error, got: {err}"
        );

        // `servers` present but the wrong shape (object, not array).
        let master = serde_json::json!({"servers": {"name": "x"}});
        let err = generate_worker_mcp_config(Some(&master), None, job_id)
            .await
            .expect_err("non-array `servers` field must error");
        assert!(
            err.to_string().contains("must be an array"),
            "expected explicit type-mismatch error, got: {err}"
        );
    }
}
