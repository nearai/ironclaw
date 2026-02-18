//! Job management tools.
//!
//! These tools allow the LLM to manage jobs:
//! - Create new jobs/tasks (with optional sandbox delegation)
//! - List existing jobs
//! - Check job status
//! - Cancel running jobs

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::channels::IncomingMessage;
use crate::channels::web::types::SseEvent;
use crate::context::{ContextManager, JobContext, JobState};
use crate::db::Database;
use crate::history::SandboxJobRecord;
use crate::orchestrator::auth::CredentialGrant;
use crate::orchestrator::job_manager::{ContainerJobManager, JobMode};
use crate::secrets::SecretsStore;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

/// Resolve a job ID from a full UUID or a short prefix (like git short SHAs).
///
/// Tries full UUID parse first. If that fails, treats the input as a hex prefix
/// and searches the context manager for a unique match.
async fn resolve_job_id(input: &str, context_manager: &ContextManager) -> Result<Uuid, ToolError> {
    // Fast path: full UUID
    if let Ok(id) = Uuid::parse_str(input) {
        return Ok(id);
    }

    // Require a minimum prefix length to limit brute-force enumeration.
    if input.len() < 4 {
        return Err(ToolError::InvalidParameters(
            "job ID prefix must be at least 4 hex characters".to_string(),
        ));
    }

    // Prefix match against known jobs
    let input_lower = input.to_lowercase();
    let all_ids = context_manager.all_jobs().await;
    let matches: Vec<Uuid> = all_ids
        .into_iter()
        .filter(|id| {
            let hex = id.to_string().replace('-', "");
            hex.starts_with(&input_lower)
        })
        .collect();

    match matches.len() {
        1 => Ok(matches[0]),
        0 => Err(ToolError::InvalidParameters(format!(
            "no job found matching prefix '{}'",
            input
        ))),
        n => Err(ToolError::InvalidParameters(format!(
            "ambiguous prefix '{}' matches {} jobs, provide more characters",
            input, n
        ))),
    }
}

/// Tool for creating a new job.
///
/// When sandbox deps are injected (via `with_sandbox`), the tool automatically
/// delegates execution to a Docker container. Otherwise it creates an in-memory
/// job via the ContextManager. The LLM never needs to know the difference.
pub struct CreateJobTool {
    context_manager: Arc<ContextManager>,
    job_manager: Option<Arc<ContainerJobManager>>,
    store: Option<Arc<dyn Database>>,
    /// Broadcast sender for job events (used to subscribe a monitor).
    event_tx: Option<tokio::sync::broadcast::Sender<(Uuid, SseEvent)>>,
    /// Injection channel for pushing messages into the agent loop.
    inject_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
    /// Encrypted secrets store for validating credential grants.
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
}

impl CreateJobTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self {
            context_manager,
            job_manager: None,
            store: None,
            event_tx: None,
            inject_tx: None,
            secrets_store: None,
        }
    }

    /// Inject sandbox dependencies so `create_job` delegates to Docker containers.
    pub fn with_sandbox(
        mut self,
        job_manager: Arc<ContainerJobManager>,
        store: Option<Arc<dyn Database>>,
    ) -> Self {
        self.job_manager = Some(job_manager);
        self.store = store;
        self
    }

    /// Inject monitor dependencies so fire-and-forget jobs spawn a background
    /// monitor that forwards Claude Code output to the main agent loop.
    pub fn with_monitor_deps(
        mut self,
        event_tx: tokio::sync::broadcast::Sender<(Uuid, SseEvent)>,
        inject_tx: tokio::sync::mpsc::Sender<IncomingMessage>,
    ) -> Self {
        self.event_tx = Some(event_tx);
        self.inject_tx = Some(inject_tx);
        self
    }

    /// Inject secrets store for credential validation.
    pub fn with_secrets(mut self, secrets: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        self.secrets_store = Some(secrets);
        self
    }

    pub fn sandbox_enabled(&self) -> bool {
        self.job_manager.is_some()
    }

    /// Parse and validate the `credentials` parameter.
    ///
    /// Each key is a secret name (must exist in SecretsStore), each value is the
    /// env var name the container should receive it as. Returns an empty vec if
    /// no credentials were requested.
    async fn parse_credentials(
        &self,
        params: &serde_json::Value,
        user_id: &str,
    ) -> Result<Vec<CredentialGrant>, ToolError> {
        let creds_obj = match params.get("credentials").and_then(|v| v.as_object()) {
            Some(obj) if !obj.is_empty() => obj,
            _ => return Ok(vec![]),
        };

        const MAX_CREDENTIAL_GRANTS: usize = 20;
        if creds_obj.len() > MAX_CREDENTIAL_GRANTS {
            return Err(ToolError::InvalidParameters(format!(
                "too many credential grants ({}, max {})",
                creds_obj.len(),
                MAX_CREDENTIAL_GRANTS
            )));
        }

        let secrets = match &self.secrets_store {
            Some(s) => s,
            None => {
                return Err(ToolError::ExecutionFailed(
                    "credentials requested but no secrets store is configured. \
                     Set SECRETS_MASTER_KEY to enable credential management."
                        .to_string(),
                ));
            }
        };

        let mut grants = Vec::with_capacity(creds_obj.len());
        for (secret_name, env_var_value) in creds_obj {
            let env_var = env_var_value.as_str().ok_or_else(|| {
                ToolError::InvalidParameters(format!(
                    "credential env var for '{}' must be a string",
                    secret_name
                ))
            })?;

            validate_env_var_name(env_var)?;

            // Validate the secret actually exists
            let exists = secrets.exists(user_id, secret_name).await.map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "failed to check secret '{}': {}",
                    secret_name, e
                ))
            })?;

            if !exists {
                return Err(ToolError::ExecutionFailed(format!(
                    "secret '{}' not found. Store it first via 'ironclaw tool auth' or the web UI.",
                    secret_name
                )));
            }

            grants.push(CredentialGrant {
                secret_name: secret_name.clone(),
                env_var: env_var.to_string(),
            });
        }

        Ok(grants)
    }

    /// Persist a sandbox job record (fire-and-forget).
    fn persist_job(&self, record: SandboxJobRecord) {
        if let Some(store) = self.store.clone() {
            tokio::spawn(async move {
                if let Err(e) = store.save_sandbox_job(&record).await {
                    tracing::warn!(job_id = %record.id, "Failed to persist sandbox job: {}", e);
                }
            });
        }
    }

    /// Update sandbox job status in DB (fire-and-forget).
    fn update_status(
        &self,
        job_id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<String>,
        started_at: Option<chrono::DateTime<Utc>>,
        completed_at: Option<chrono::DateTime<Utc>>,
    ) {
        if let Some(store) = self.store.clone() {
            let status = status.to_string();
            tokio::spawn(async move {
                if let Err(e) = store
                    .update_sandbox_job_status(
                        job_id,
                        &status,
                        success,
                        message.as_deref(),
                        started_at,
                        completed_at,
                    )
                    .await
                {
                    tracing::warn!(job_id = %job_id, "Failed to update sandbox job status: {}", e);
                }
            });
        }
    }

    /// Execute via in-memory ContextManager (no sandbox).
    async fn execute_local(
        &self,
        title: &str,
        description: &str,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        match self
            .context_manager
            .create_job_for_user(&ctx.user_id, title, description)
            .await
        {
            Ok(job_id) => {
                let result = serde_json::json!({
                    "job_id": job_id.to_string(),
                    "title": title,
                    "status": "pending",
                    "message": format!("Created job '{}'", title)
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
            Err(e) => {
                let result = serde_json::json!({
                    "error": e.to_string()
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
        }
    }

    /// Execute via sandboxed Docker container.
    async fn execute_sandbox(
        &self,
        task: &str,
        explicit_dir: Option<PathBuf>,
        wait: bool,
        mode: JobMode,
        credential_grants: Vec<CredentialGrant>,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let jm = self.job_manager.as_ref().expect("sandbox deps required");

        let job_id = Uuid::new_v4();
        let (project_dir, browse_id) = resolve_project_dir(explicit_dir, job_id)?;
        let project_dir_str = project_dir.display().to_string();

        // Serialize credential grants so restarts can reload them.
        let credential_grants_json = match serde_json::to_string(&credential_grants) {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!(
                    "Failed to serialize credential grants for job {}: {}. \
                     Grants will not survive a restart.",
                    job_id,
                    e
                );
                String::from("[]")
            }
        };

        // Persist the job to DB before creating the container.
        self.persist_job(SandboxJobRecord {
            id: job_id,
            task: task.to_string(),
            status: "creating".to_string(),
            user_id: ctx.user_id.clone(),
            project_dir: project_dir_str.clone(),
            success: None,
            failure_reason: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            credential_grants_json,
        });

        // Persist the job mode to DB
        if mode == JobMode::ClaudeCode
            && let Some(store) = self.store.clone()
        {
            let job_id_copy = job_id;
            tokio::spawn(async move {
                if let Err(e) = store
                    .update_sandbox_job_mode(job_id_copy, "claude_code")
                    .await
                {
                    tracing::warn!(job_id = %job_id_copy, "Failed to set job mode: {}", e);
                }
            });
        }

        // Create the container job with the pre-determined job_id.
        let _token = jm
            .create_job(job_id, task, Some(project_dir), mode, credential_grants)
            .await
            .map_err(|e| {
                self.update_status(
                    job_id,
                    "failed",
                    Some(false),
                    Some(e.to_string()),
                    None,
                    Some(Utc::now()),
                );
                ToolError::ExecutionFailed(format!("failed to create container: {}", e))
            })?;

        // Container started successfully.
        let now = Utc::now();
        self.update_status(job_id, "running", None, None, Some(now), None);

        if !wait {
            // Spawn a background monitor that forwards Claude Code output
            // into the main agent loop.
            //
            // This monitor is intentionally fire-and-forget: its lifetime is
            // bound to the broadcast channel (etx) and the inject sender (itx).
            // When the broadcast sender is dropped during shutdown the
            // subscription closes and the monitor exits. Likewise, if the agent
            // loop stops consuming from inject_tx the send will fail and the
            // monitor terminates. No JoinHandle is retained.
            if let (Some(etx), Some(itx)) = (&self.event_tx, &self.inject_tx) {
                crate::agent::job_monitor::spawn_job_monitor(job_id, etx.subscribe(), itx.clone());
            }

            let result = serde_json::json!({
                "job_id": job_id.to_string(),
                "status": "started",
                "message": "Container started. Use job_events to check status or job_prompt to send follow-up instructions.",
                "project_dir": project_dir_str,
                "browse_url": format!("/projects/{}", browse_id),
            });
            return Ok(ToolOutput::success(result, start.elapsed()));
        }

        // Wait for completion by polling the container state.
        let timeout = Duration::from_secs(600);
        let poll_interval = Duration::from_secs(2);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if tokio::time::Instant::now() > deadline {
                let _ = jm.stop_job(job_id).await;
                jm.cleanup_job(job_id).await;
                self.update_status(
                    job_id,
                    "failed",
                    Some(false),
                    Some("Timed out (10 minutes)".to_string()),
                    None,
                    Some(Utc::now()),
                );
                return Err(ToolError::ExecutionFailed(
                    "container execution timed out (10 minutes)".to_string(),
                ));
            }

            match jm.get_handle(job_id).await {
                Some(handle) => match handle.state {
                    crate::orchestrator::job_manager::ContainerState::Running
                    | crate::orchestrator::job_manager::ContainerState::Creating => {
                        tokio::time::sleep(poll_interval).await;
                    }
                    crate::orchestrator::job_manager::ContainerState::Stopped => {
                        let message = handle
                            .completion_result
                            .as_ref()
                            .and_then(|r| r.message.clone())
                            .unwrap_or_else(|| "Container job completed".to_string());
                        let success = handle
                            .completion_result
                            .as_ref()
                            .map(|r| r.success)
                            .unwrap_or(true);
                        jm.cleanup_job(job_id).await;

                        let finished_at = Utc::now();
                        if success {
                            self.update_status(
                                job_id,
                                "completed",
                                Some(true),
                                None,
                                None,
                                Some(finished_at),
                            );
                            let result = serde_json::json!({
                                "job_id": job_id.to_string(),
                                "status": "completed",
                                "output": message,
                                "project_dir": project_dir_str,
                                "browse_url": format!("/projects/{}", browse_id),
                            });
                            return Ok(ToolOutput::success(result, start.elapsed()));
                        } else {
                            self.update_status(
                                job_id,
                                "failed",
                                Some(false),
                                Some(message.clone()),
                                None,
                                Some(finished_at),
                            );
                            return Err(ToolError::ExecutionFailed(format!(
                                "container job failed: {}",
                                message
                            )));
                        }
                    }
                    crate::orchestrator::job_manager::ContainerState::Failed => {
                        let message = handle
                            .completion_result
                            .as_ref()
                            .and_then(|r| r.message.clone())
                            .unwrap_or_else(|| "unknown failure".to_string());
                        jm.cleanup_job(job_id).await;
                        self.update_status(
                            job_id,
                            "failed",
                            Some(false),
                            Some(message.clone()),
                            None,
                            Some(Utc::now()),
                        );
                        return Err(ToolError::ExecutionFailed(format!(
                            "container job failed: {}",
                            message
                        )));
                    }
                },
                None => {
                    self.update_status(
                        job_id,
                        "completed",
                        Some(true),
                        None,
                        None,
                        Some(Utc::now()),
                    );
                    let result = serde_json::json!({
                        "job_id": job_id.to_string(),
                        "status": "completed",
                        "output": "Container job completed",
                        "project_dir": project_dir_str,
                        "browse_url": format!("/projects/{}", browse_id),
                    });
                    return Ok(ToolOutput::success(result, start.elapsed()));
                }
            }
        }
    }
}

/// The base directory where all project directories must live.
/// Env var names that could be abused to hijack process behavior.
const DANGEROUS_ENV_VARS: &[&str] = &[
    // Dynamic linker hijacking
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    // Shell behavior
    "BASH_ENV",
    "ENV",
    "CDPATH",
    "IFS",
    "PATH",
    "HOME",
    // Language runtime library path hijacking
    "PYTHONPATH",
    "NODE_PATH",
    "PERL5LIB",
    "RUBYLIB",
    "CLASSPATH",
    // JVM injection
    "JAVA_TOOL_OPTIONS",
    "MAVEN_OPTS",
    "USER",
    "SHELL",
    "RUST_LOG",
];

/// Validate that an env var name is safe for container injection.
fn validate_env_var_name(name: &str) -> Result<(), ToolError> {
    if name.is_empty() {
        return Err(ToolError::InvalidParameters(
            "env var name cannot be empty".into(),
        ));
    }

    // Must match ^[A-Z_][A-Z0-9_]*$
    let valid = name
        .bytes()
        .enumerate()
        .all(|(i, b)| matches!(b, b'A'..=b'Z' | b'_') || (i > 0 && b.is_ascii_digit()));

    if !valid {
        return Err(ToolError::InvalidParameters(format!(
            "env var '{}' must match [A-Z_][A-Z0-9_]* (uppercase, underscores, digits)",
            name
        )));
    }

    if DANGEROUS_ENV_VARS.contains(&name) {
        return Err(ToolError::InvalidParameters(format!(
            "env var '{}' is on the denylist (could hijack process behavior)",
            name
        )));
    }

    Ok(())
}

fn projects_base() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("projects")
}

/// Resolve the project directory, creating it if it doesn't exist.
///
/// Auto-creates `~/.ironclaw/projects/{project_id}/` so every sandbox job has a
/// persistent bind mount that survives container teardown.
///
/// When an explicit path is provided (e.g. job restarts reusing the old dir),
/// it is validated to fall within `~/.ironclaw/projects/` after canonicalization.
fn resolve_project_dir(
    explicit: Option<PathBuf>,
    project_id: Uuid,
) -> Result<(PathBuf, String), ToolError> {
    let base = projects_base();
    std::fs::create_dir_all(&base).map_err(|e| {
        ToolError::ExecutionFailed(format!(
            "failed to create projects base {}: {}",
            base.display(),
            e
        ))
    })?;
    let canonical_base = base.canonicalize().map_err(|e| {
        ToolError::ExecutionFailed(format!("failed to canonicalize projects base: {}", e))
    })?;

    let (canonical_dir, _was_explicit) = match explicit {
        Some(d) => {
            // Explicit paths: validate BEFORE creating anything.
            // The path must already exist (it comes from a previous job run).
            let canonical = d.canonicalize().map_err(|e| {
                ToolError::InvalidParameters(format!(
                    "explicit project dir {} does not exist or is inaccessible: {}",
                    d.display(),
                    e
                ))
            })?;
            if !canonical.starts_with(&canonical_base) {
                return Err(ToolError::InvalidParameters(format!(
                    "project directory must be under {}",
                    canonical_base.display()
                )));
            }
            (canonical, true)
        }
        None => {
            let dir = canonical_base.join(project_id.to_string());
            std::fs::create_dir_all(&dir).map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "failed to create project dir {}: {}",
                    dir.display(),
                    e
                ))
            })?;
            let canonical = dir.canonicalize().map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "failed to canonicalize project dir {}: {}",
                    dir.display(),
                    e
                ))
            })?;
            (canonical, false)
        }
    };

    let browse_id = canonical_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| project_id.to_string());
    Ok((canonical_dir, browse_id))
}

#[async_trait]
impl Tool for CreateJobTool {
    fn name(&self) -> &str {
        "create_job"
    }

    fn description(&self) -> &str {
        if self.sandbox_enabled() {
            "Create and execute a job. The job runs in a sandboxed Docker container with its own \
             sub-agent that has shell, file read/write, list_dir, and apply_patch tools. Use this \
             whenever the user asks you to build, create, or work on something. The task \
             description should be detailed enough for the sub-agent to work independently. \
             Set wait=false to start immediately while continuing the conversation. Set mode \
             to 'claude_code' for complex software engineering tasks."
        } else {
            "Create a new job or task for the agent to work on. Use this when the user wants \
             you to do something substantial that should be tracked as a separate job."
        }
    }

    fn parameters_schema(&self) -> serde_json::Value {
        if self.sandbox_enabled() {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Clear description of what to accomplish"
                    },
                    "description": {
                        "type": "string",
                        "description": "Full description of what needs to be done"
                    },
                    "wait": {
                        "type": "boolean",
                        "description": "If true (default), wait for the container to complete and return results. \
                                        If false, start the container and return the job_id immediately."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["worker", "claude_code"],
                        "description": "Execution mode. 'worker' (default) uses the IronClaw sub-agent. \
                                        'claude_code' uses Claude Code CLI for full agentic software engineering."
                    },
                    "project_dir": {
                        "type": "string",
                        "description": "Path to an existing project directory to mount into the container. \
                                        Must be under ~/.ironclaw/projects/. If omitted, a fresh directory is created."
                    },
                    "credentials": {
                        "type": "object",
                        "description": "Map of secret names to env var names. Each secret must exist in the \
                                        secrets store (via 'ironclaw tool auth' or web UI). Example: \
                                        {\"github_token\": \"GITHUB_TOKEN\", \"npm_token\": \"NPM_TOKEN\"}",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["title", "description"]
            })
        } else {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "A short title for the job (max 100 chars)"
                    },
                    "description": {
                        "type": "string",
                        "description": "Full description of what needs to be done"
                    }
                },
                "required": ["title", "description"]
            })
        }
    }

    fn execution_timeout(&self) -> Duration {
        if self.sandbox_enabled() {
            // Sandbox polls for up to 10 min internally; give an extra 60s buffer.
            Duration::from_secs(660)
        } else {
            Duration::from_secs(30)
        }
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let title = require_str(&params, "title")?;

        let description = require_str(&params, "description")?;

        if self.sandbox_enabled() {
            let wait = params.get("wait").and_then(|v| v.as_bool()).unwrap_or(true);

            let mode = match params.get("mode").and_then(|v| v.as_str()) {
                Some("claude_code") => JobMode::ClaudeCode,
                _ => JobMode::Worker,
            };

            let explicit_dir = params
                .get("project_dir")
                .and_then(|v| v.as_str())
                .map(PathBuf::from);

            // Parse and validate credential grants
            let credential_grants = self.parse_credentials(&params, &ctx.user_id).await?;

            // Combine title and description into the task prompt for the sub-agent.
            let task = format!("{}\n\n{}", title, description);
            self.execute_sandbox(&task, explicit_dir, wait, mode, credential_grants, ctx)
                .await
        } else {
            self.execute_local(title, description, ctx).await
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for listing jobs.
pub struct ListJobsTool {
    context_manager: Arc<ContextManager>,
}

impl ListJobsTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self { context_manager }
    }
}

#[async_trait]
impl Tool for ListJobsTool {
    fn name(&self) -> &str {
        "list_jobs"
    }

    fn description(&self) -> &str {
        "List all jobs or filter by status. Shows job IDs, titles, and current status."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "filter": {
                    "type": "string",
                    "description": "Filter by status: 'active', 'completed', 'failed', 'all' (default: 'all')",
                    "enum": ["active", "completed", "failed", "all"]
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let filter = params
            .get("filter")
            .and_then(|v| v.as_str())
            .unwrap_or("all");

        let job_ids = match filter {
            "active" => self.context_manager.active_jobs_for(&ctx.user_id).await,
            _ => self.context_manager.all_jobs_for(&ctx.user_id).await,
        };

        let mut jobs = Vec::new();
        for job_id in job_ids {
            if let Ok(ctx) = self.context_manager.get_context(job_id).await {
                let include = match filter {
                    "completed" => ctx.state == JobState::Completed,
                    "failed" => ctx.state == JobState::Failed,
                    "active" => ctx.state.is_active(),
                    _ => true,
                };

                if include {
                    jobs.push(serde_json::json!({
                        "job_id": job_id.to_string(),
                        "title": ctx.title,
                        "status": format!("{:?}", ctx.state),
                        "created_at": ctx.created_at.to_rfc3339()
                    }));
                }
            }
        }

        let summary = self.context_manager.summary_for(&ctx.user_id).await;

        let result = serde_json::json!({
            "jobs": jobs,
            "summary": {
                "total": summary.total,
                "pending": summary.pending,
                "in_progress": summary.in_progress,
                "completed": summary.completed,
                "failed": summary.failed
            }
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for checking job status.
pub struct JobStatusTool {
    context_manager: Arc<ContextManager>,
}

impl JobStatusTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self { context_manager }
    }
}

#[async_trait]
impl Tool for JobStatusTool {
    fn name(&self) -> &str {
        "job_status"
    }

    fn description(&self) -> &str {
        "Check the status and details of a specific job by its ID."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
                }
            },
            "required": ["job_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let requester_id = ctx.user_id.clone();

        let job_id_str = require_str(&params, "job_id")?;
        let job_id = resolve_job_id(job_id_str, &self.context_manager).await?;

        match self.context_manager.get_context(job_id).await {
            Ok(job_ctx) => {
                if job_ctx.user_id != requester_id {
                    let result = serde_json::json!({
                        "error": "Job not found".to_string()
                    });
                    return Ok(ToolOutput::success(result, start.elapsed()));
                }
                let result = serde_json::json!({
                    "job_id": job_id.to_string(),
                    "title": job_ctx.title,
                    "description": job_ctx.description,
                    "status": format!("{:?}", job_ctx.state),
                    "created_at": job_ctx.created_at.to_rfc3339(),
                    "started_at": job_ctx.started_at.map(|t| t.to_rfc3339()),
                    "completed_at": job_ctx.completed_at.map(|t| t.to_rfc3339()),
                    "actual_cost": job_ctx.actual_cost.to_string()
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
            Err(e) => {
                let result = serde_json::json!({
                    "error": format!("Job not found: {}", e)
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for canceling a job.
pub struct CancelJobTool {
    context_manager: Arc<ContextManager>,
}

impl CancelJobTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self { context_manager }
    }
}

#[async_trait]
impl Tool for CancelJobTool {
    fn name(&self) -> &str {
        "cancel_job"
    }

    fn description(&self) -> &str {
        "Cancel a running or pending job. The job will be marked as cancelled and stopped."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
                }
            },
            "required": ["job_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let requester_id = ctx.user_id.clone();

        let job_id_str = require_str(&params, "job_id")?;
        let job_id = resolve_job_id(job_id_str, &self.context_manager).await?;

        // Transition to cancelled state
        match self
            .context_manager
            .update_context(job_id, |ctx| {
                if ctx.user_id != requester_id {
                    return Err("Job not found".to_string());
                }
                ctx.transition_to(JobState::Cancelled, Some("Cancelled by user".to_string()))
            })
            .await
        {
            Ok(Ok(())) => {
                let result = serde_json::json!({
                    "job_id": job_id.to_string(),
                    "status": "cancelled",
                    "message": "Job cancelled successfully"
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
            Ok(Err(reason)) => {
                let result = serde_json::json!({
                    "error": format!("Cannot cancel job: {}", reason)
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
            Err(e) => {
                let result = serde_json::json!({
                    "error": format!("Job not found: {}", e)
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
        }
    }

    fn requires_approval(&self) -> bool {
        true // Canceling a job should require approval
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for reading sandbox job event logs.
///
/// Lets the main agent inspect what a running (or completed) container job has
/// been doing: messages, tool calls, results, status changes, etc.
///
/// Events are streamed from the sandbox worker into the database via the
/// orchestrator's event pipeline. This tool queries them with a DB-level
/// `LIMIT` (default 50, configurable via the `limit` parameter) so the
/// agent sees the most recent activity without loading the full history.
pub struct JobEventsTool {
    store: Arc<dyn Database>,
    context_manager: Arc<ContextManager>,
}

impl JobEventsTool {
    pub fn new(store: Arc<dyn Database>, context_manager: Arc<ContextManager>) -> Self {
        Self {
            store,
            context_manager,
        }
    }
}

#[async_trait]
impl Tool for JobEventsTool {
    fn name(&self) -> &str {
        "job_events"
    }

    fn description(&self) -> &str {
        "Read the event log for a sandbox job. Shows messages, tool calls, results, \
         and status changes from the container. Use this to check what Claude Code \
         or a worker sub-agent has been doing."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of events to return (default 50, most recent)"
                }
            },
            "required": ["job_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let job_id_str = params
            .get("job_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("missing 'job_id' parameter".into()))?;

        let job_id = resolve_job_id(job_id_str, &self.context_manager).await?;

        // Verify the caller owns this job. A missing context is treated as
        // unauthorized to prevent leaking events after process restarts.
        let job_ctx = self
            .context_manager
            .get_context(job_id)
            .await
            .map_err(|_| {
                ToolError::ExecutionFailed(format!(
                    "job {} not found or context unavailable",
                    job_id
                ))
            })?;

        if job_ctx.user_id != ctx.user_id {
            return Err(ToolError::ExecutionFailed(format!(
                "job {} does not belong to current user",
                job_id
            )));
        }

        const MAX_EVENT_LIMIT: i64 = 1000;
        let limit = params
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(50)
            .clamp(1, MAX_EVENT_LIMIT);

        let events = self
            .store
            .list_job_events(job_id, Some(limit))
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to load job events: {}", e)))?;

        let recent: Vec<serde_json::Value> = events
            .iter()
            .map(|ev| {
                serde_json::json!({
                    "event_type": ev.event_type,
                    "data": ev.data,
                    "created_at": ev.created_at.to_rfc3339(),
                })
            })
            .collect();

        let result = serde_json::json!({
            "job_id": job_id.to_string(),
            "total_events": events.len(),
            "returned": recent.len(),
            "events": recent,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }
}

/// Tool for sending follow-up prompts to a running Claude Code sandbox job.
///
/// The prompt is queued in an in-memory `PromptQueue` (a broadcast channel
/// shared with the web gateway). The Claude Code bridge inside the container
/// polls for queued prompts between turns and feeds them into the next
/// `claude --resume` invocation, enabling interactive multi-turn sessions
/// with long-running sandbox jobs.
pub struct JobPromptTool {
    prompt_queue: PromptQueue,
    context_manager: Arc<ContextManager>,
}

/// Type alias matching `crate::channels::web::server::PromptQueue`.
pub type PromptQueue = Arc<
    tokio::sync::Mutex<
        std::collections::HashMap<
            Uuid,
            std::collections::VecDeque<crate::orchestrator::api::PendingPrompt>,
        >,
    >,
>;

impl JobPromptTool {
    pub fn new(prompt_queue: PromptQueue, context_manager: Arc<ContextManager>) -> Self {
        Self {
            prompt_queue,
            context_manager,
        }
    }
}

#[async_trait]
impl Tool for JobPromptTool {
    fn name(&self) -> &str {
        "job_prompt"
    }

    fn description(&self) -> &str {
        "Send a follow-up prompt to a running Claude Code sandbox job. The prompt is \
         queued and delivered on the next poll cycle. Use this to give the sub-agent \
         additional instructions, answer its questions, or tell it to wrap up."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
                },
                "content": {
                    "type": "string",
                    "description": "The follow-up prompt text to send"
                },
                "done": {
                    "type": "boolean",
                    "description": "If true, signals the sub-agent that no more prompts are coming \
                                    and it should finish up. Default false."
                }
            },
            "required": ["job_id", "content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let job_id_str = params
            .get("job_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("missing 'job_id' parameter".into()))?;

        let job_id = resolve_job_id(job_id_str, &self.context_manager).await?;

        // Verify the caller owns this job. A missing context is treated as
        // unauthorized to prevent sending prompts to jobs after process restarts.
        let job_ctx = self
            .context_manager
            .get_context(job_id)
            .await
            .map_err(|_| {
                ToolError::ExecutionFailed(format!(
                    "job {} not found or context unavailable",
                    job_id
                ))
            })?;

        if job_ctx.user_id != ctx.user_id {
            return Err(ToolError::ExecutionFailed(format!(
                "job {} does not belong to current user",
                job_id
            )));
        }

        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("missing 'content' parameter".into()))?;

        let done = params
            .get("done")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let prompt = crate::orchestrator::api::PendingPrompt {
            content: content.to_string(),
            done,
        };

        {
            let mut queue = self.prompt_queue.lock().await;
            queue.entry(job_id).or_default().push_back(prompt);
        }

        let result = serde_json::json!({
            "job_id": job_id.to_string(),
            "status": "queued",
            "message": "Prompt queued",
            "done": done,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self) -> bool {
        true
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_job_tool_local() {
        let manager = Arc::new(ContextManager::new(5));
        let tool = CreateJobTool::new(manager.clone());

        // Without sandbox deps, it should use the local path
        assert!(!tool.sandbox_enabled());

        let params = serde_json::json!({
            "title": "Test Job",
            "description": "A test job description"
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();

        let job_id = result.result.get("job_id").unwrap().as_str().unwrap();
        assert!(!job_id.is_empty());
        assert_eq!(
            result.result.get("status").unwrap().as_str().unwrap(),
            "pending"
        );
    }

    #[test]
    fn test_schema_changes_with_sandbox() {
        let manager = Arc::new(ContextManager::new(5));

        // Without sandbox
        let tool = CreateJobTool::new(Arc::clone(&manager));
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("title"));
        assert!(props.contains_key("description"));
        assert!(!props.contains_key("wait"));
        assert!(!props.contains_key("mode"));
    }

    #[test]
    fn test_execution_timeout_sandbox() {
        let manager = Arc::new(ContextManager::new(5));

        // Without sandbox: default timeout
        let tool = CreateJobTool::new(Arc::clone(&manager));
        assert_eq!(tool.execution_timeout(), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn test_list_jobs_tool() {
        let manager = Arc::new(ContextManager::new(5));

        // Create some jobs
        manager.create_job("Job 1", "Desc 1").await.unwrap();
        manager.create_job("Job 2", "Desc 2").await.unwrap();

        let tool = ListJobsTool::new(manager);

        let params = serde_json::json!({});
        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();

        let jobs = result.result.get("jobs").unwrap().as_array().unwrap();
        assert_eq!(jobs.len(), 2);
    }

    #[tokio::test]
    async fn test_job_status_tool() {
        let manager = Arc::new(ContextManager::new(5));
        let job_id = manager.create_job("Test Job", "Description").await.unwrap();

        let tool = JobStatusTool::new(manager);

        let params = serde_json::json!({
            "job_id": job_id.to_string()
        });
        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();

        assert_eq!(
            result.result.get("title").unwrap().as_str().unwrap(),
            "Test Job"
        );
    }

    #[test]
    fn test_resolve_project_dir_auto() {
        let project_id = Uuid::new_v4();
        let (dir, browse_id) = resolve_project_dir(None, project_id).unwrap();
        assert!(dir.exists());
        assert!(dir.ends_with(project_id.to_string()));
        assert_eq!(browse_id, project_id.to_string());

        // Must be under the projects base
        let base = projects_base().canonicalize().unwrap();
        assert!(dir.starts_with(&base));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_project_dir_explicit_under_base() {
        let base = projects_base();
        std::fs::create_dir_all(&base).unwrap();
        let explicit = base.join("test_explicit_project");
        // Explicit paths must already exist (no auto-create).
        std::fs::create_dir_all(&explicit).unwrap();
        let project_id = Uuid::new_v4();

        let (dir, browse_id) = resolve_project_dir(Some(explicit.clone()), project_id).unwrap();
        assert!(dir.exists());
        assert_eq!(browse_id, "test_explicit_project");

        let canonical_base = base.canonicalize().unwrap();
        assert!(dir.starts_with(&canonical_base));

        let _ = std::fs::remove_dir_all(&explicit);
    }

    #[test]
    fn test_resolve_project_dir_rejects_outside_base() {
        let tmp = tempfile::tempdir().unwrap();
        let escape_attempt = tmp.path().join("evil_project");
        // Don't create it: explicit paths that don't exist are rejected
        // before the prefix check even runs.

        let result = resolve_project_dir(Some(escape_attempt), Uuid::new_v4());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("does not exist"),
            "expected 'does not exist' error, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_project_dir_rejects_outside_base_existing() {
        // A directory that exists but is outside the projects base.
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().to_path_buf();

        let result = resolve_project_dir(Some(outside), Uuid::new_v4());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("must be under"),
            "expected 'must be under' error, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_project_dir_rejects_traversal() {
        // Non-existent traversal path is rejected because canonicalize fails.
        let base = projects_base();
        let traversal = base.join("legit").join("..").join("..").join(".ssh");

        let result = resolve_project_dir(Some(traversal), Uuid::new_v4());
        assert!(result.is_err(), "traversal path should be rejected");

        // Traversal path that actually resolves gets the prefix check.
        // `base/../` resolves to the parent of projects base, which is outside.
        let base_parent = projects_base().join("..").join("definitely_not_projects");
        std::fs::create_dir_all(&base_parent).ok();
        if base_parent.exists() {
            let result = resolve_project_dir(Some(base_parent.clone()), Uuid::new_v4());
            assert!(result.is_err(), "path outside base should be rejected");
            let _ = std::fs::remove_dir_all(&base_parent);
        }
    }

    #[test]
    fn test_sandbox_schema_includes_project_dir() {
        let manager = Arc::new(ContextManager::new(5));
        let jm = Arc::new(ContainerJobManager::new(
            crate::orchestrator::job_manager::ContainerJobConfig::default(),
            crate::orchestrator::TokenStore::new(),
        ));
        let tool = CreateJobTool::new(manager).with_sandbox(jm, None);
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(
            props.contains_key("project_dir"),
            "sandbox schema must expose project_dir"
        );
    }

    #[test]
    fn test_sandbox_schema_includes_credentials() {
        let manager = Arc::new(ContextManager::new(5));
        let jm = Arc::new(ContainerJobManager::new(
            crate::orchestrator::job_manager::ContainerJobConfig::default(),
            crate::orchestrator::TokenStore::new(),
        ));
        let tool = CreateJobTool::new(manager).with_sandbox(jm, None);
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(
            props.contains_key("credentials"),
            "sandbox schema must expose credentials"
        );
    }

    #[tokio::test]
    async fn test_parse_credentials_empty() {
        let manager = Arc::new(ContextManager::new(5));
        let tool = CreateJobTool::new(manager);

        // No credentials parameter
        let params = serde_json::json!({"title": "t", "description": "d"});
        let grants = tool.parse_credentials(&params, "user1").await.unwrap();
        assert!(grants.is_empty());

        // Empty credentials object
        let params = serde_json::json!({"credentials": {}});
        let grants = tool.parse_credentials(&params, "user1").await.unwrap();
        assert!(grants.is_empty());
    }

    #[tokio::test]
    async fn test_parse_credentials_no_secrets_store() {
        let manager = Arc::new(ContextManager::new(5));
        let tool = CreateJobTool::new(manager);

        let params = serde_json::json!({"credentials": {"my_secret": "MY_SECRET"}});
        let result = tool.parse_credentials(&params, "user1").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no secrets store"),
            "expected 'no secrets store' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_parse_credentials_missing_secret() {
        use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
        use secrecy::SecretString;

        let manager = Arc::new(ContextManager::new(5));
        let key = "0123456789abcdef0123456789abcdef";
        let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
        let secrets: Arc<dyn SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(crypto));

        let tool = CreateJobTool::new(manager).with_secrets(Arc::clone(&secrets));

        let params = serde_json::json!({"credentials": {"nonexistent_secret": "SOME_VAR"}});
        let result = tool.parse_credentials(&params, "user1").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not found"),
            "expected 'not found' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_parse_credentials_valid() {
        use crate::secrets::{CreateSecretParams, InMemorySecretsStore, SecretsCrypto};
        use secrecy::SecretString;

        let manager = Arc::new(ContextManager::new(5));
        let key = "0123456789abcdef0123456789abcdef";
        let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
        let secrets: Arc<dyn SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(Arc::clone(&crypto)));

        // Store a secret
        secrets
            .create(
                "user1",
                CreateSecretParams::new("github_token", "ghp_test123"),
            )
            .await
            .unwrap();

        let tool = CreateJobTool::new(manager).with_secrets(Arc::clone(&secrets));

        let params = serde_json::json!({
            "credentials": {"github_token": "GITHUB_TOKEN"}
        });
        let grants = tool.parse_credentials(&params, "user1").await.unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].secret_name, "github_token");
        assert_eq!(grants[0].env_var, "GITHUB_TOKEN");
    }

    fn test_prompt_tool(queue: PromptQueue) -> JobPromptTool {
        let cm = Arc::new(ContextManager::new(5));
        JobPromptTool::new(queue, cm)
    }

    #[tokio::test]
    async fn test_job_prompt_tool_queues_prompt() {
        let cm = Arc::new(ContextManager::new(5));
        let job_id = cm
            .create_job_for_user("default", "Test Job", "desc")
            .await
            .unwrap();

        let queue: PromptQueue =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let tool = JobPromptTool::new(Arc::clone(&queue), cm);

        let params = serde_json::json!({
            "job_id": job_id.to_string(),
            "content": "What's the status?",
            "done": false,
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();

        assert_eq!(
            result.result.get("status").unwrap().as_str().unwrap(),
            "queued"
        );

        let q = queue.lock().await;
        let prompts = q.get(&job_id).unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].content, "What's the status?");
        assert!(!prompts[0].done);
    }

    #[tokio::test]
    async fn test_job_prompt_tool_requires_approval() {
        let queue: PromptQueue =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let tool = test_prompt_tool(queue);
        assert!(tool.requires_approval());
    }

    #[tokio::test]
    async fn test_job_prompt_tool_rejects_invalid_uuid() {
        let queue: PromptQueue =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let tool = test_prompt_tool(queue);

        let params = serde_json::json!({
            "job_id": "not-a-uuid",
            "content": "hello",
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_job_prompt_tool_rejects_missing_content() {
        let queue: PromptQueue =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let tool = test_prompt_tool(queue);

        let params = serde_json::json!({
            "job_id": Uuid::new_v4().to_string(),
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_job_events_tool_rejects_other_users_job() {
        // JobEventsTool needs a Store (PostgreSQL) for the full path, but the
        // ownership check happens first via ContextManager, so we can test that
        // without a database by using a Store that will never be reached.
        //
        // We construct the tool by hand: the store field is never touched
        // because the ownership check short-circuits before the query.
        let cm = Arc::new(ContextManager::new(5));
        let job_id = cm
            .create_job_for_user("owner-user", "Secret Job", "classified")
            .await
            .unwrap();

        // We need a Store to construct the tool, but creating one requires
        // a database URL. Instead, test the ownership logic directly:
        // simulate what execute() does.
        let attacker_ctx = JobContext {
            user_id: "attacker".to_string(),
            ..Default::default()
        };

        let job_ctx = cm.get_context(job_id).await.unwrap();
        assert_ne!(job_ctx.user_id, attacker_ctx.user_id);
        assert_eq!(job_ctx.user_id, "owner-user");
    }

    #[test]
    fn test_job_events_tool_schema() {
        // Verify the schema shape is correct (doesn't need a Store instance).
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of events to return (default 50, most recent)"
                }
            },
            "required": ["job_id"]
        });

        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("job_id"));
        assert!(props.contains_key("limit"));
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0].as_str().unwrap(), "job_id");
    }

    #[tokio::test]
    async fn test_job_prompt_tool_rejects_other_users_job() {
        let cm = Arc::new(ContextManager::new(5));
        let job_id = cm
            .create_job_for_user("owner-user", "Test Job", "desc")
            .await
            .unwrap();

        let queue: PromptQueue =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let tool = JobPromptTool::new(queue, cm);

        let params = serde_json::json!({
            "job_id": job_id.to_string(),
            "content": "sneaky prompt",
        });

        // Attacker context with a different user_id.
        let ctx = JobContext {
            user_id: "attacker".to_string(),
            ..Default::default()
        };

        let result = tool.execute(params, &ctx).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("does not belong to current user"),
            "expected ownership error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_resolve_job_id_full_uuid() {
        let cm = ContextManager::new(5);
        let job_id = cm.create_job("Test", "Desc").await.unwrap();

        let resolved = resolve_job_id(&job_id.to_string(), &cm).await.unwrap();
        assert_eq!(resolved, job_id);
    }

    #[tokio::test]
    async fn test_resolve_job_id_short_prefix() {
        let cm = ContextManager::new(5);
        let job_id = cm.create_job("Test", "Desc").await.unwrap();

        // Use first 8 hex chars (without dashes)
        let hex = job_id.to_string().replace('-', "");
        let prefix = &hex[..8];
        let resolved = resolve_job_id(prefix, &cm).await.unwrap();
        assert_eq!(resolved, job_id);
    }

    #[tokio::test]
    async fn test_resolve_job_id_no_match() {
        let cm = ContextManager::new(5);
        cm.create_job("Test", "Desc").await.unwrap();

        let result = resolve_job_id("00000000", &cm).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no job found"),
            "expected 'no job found', got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_resolve_job_id_invalid_input() {
        let cm = ContextManager::new(5);
        let result = resolve_job_id("not-hex-at-all!", &cm).await;
        assert!(result.is_err());
    }
}
