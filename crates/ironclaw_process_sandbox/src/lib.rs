//! Docker process sandbox process executor for IronClaw Reborn.
//!
//! This crate owns the dynamic process compatibility lane: a trusted host can
//! execute a typed [`SandboxProcessPlan`] through [`ProcessExecutor`] while keeping
//! host paths in executor configuration and secret material behind broker
//! policy seams.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use ironclaw_host_api::{ProcessId, ResourceScope, RuntimeCredentialTarget, SecretHandle};
use ironclaw_processes::{
    ProcessCancellationToken, ProcessExecutionError, ProcessExecutionRequest,
    ProcessExecutionResult, ProcessExecutor,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time,
};

pub const DEFAULT_PROCESS_SANDBOX_IMAGE: &str = "ironclaw-process-sandbox:dev";
pub const PROCESS_SANDBOX_CAPABILITY_ID: &str = "system.process_sandbox.run";
pub const DEFAULT_WORKSPACE_MOUNT: &str = "/workspace";
pub const DEFAULT_TOOLS_MOUNT: &str = "/ironclaw/state/tools";
pub const DEFAULT_CACHE_MOUNT: &str = "/ironclaw/state/cache";
const DEFAULT_STDOUT_LIMIT: u64 = 1024 * 1024;
const DEFAULT_STDERR_LIMIT: u64 = 256 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxProcessPlan {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub install: Option<SandboxInstallPlan>,
    pub run: SandboxCommandPlan,
    #[serde(default)]
    pub mounts: SandboxMounts,
    #[serde(default)]
    pub network: SandboxNetworkPlan,
    #[serde(default)]
    pub credentials: Vec<SandboxCredentialBinding>,
}

impl SandboxProcessPlan {
    pub fn validate(&self) -> Result<(), SandboxPlanError> {
        if let Some(image) = &self.image {
            validate_docker_image_reference(image)?;
        }
        if let Some(install) = &self.install {
            install.command.validate("install")?;
            for host in &install.allowed_hosts {
                validate_host(host)?;
            }
            validate_env_has_no_raw_sensitive_values(&install.command.env, &[])?;
        }
        self.run.validate("run")?;
        self.mounts.validate()?;
        self.network.validate()?;

        let placeholders = self
            .credentials
            .iter()
            .map(|binding| binding.placeholder_value.as_str())
            .collect::<Vec<_>>();
        validate_env_has_no_raw_sensitive_values(&self.run.env, &placeholders)?;

        let runtime_hosts = self.network.runtime_allowed_hosts();
        let mut seen = HashSet::new();
        for binding in &self.credentials {
            binding.validate()?;
            let approved_host = binding.approved_host.to_ascii_lowercase();
            if !runtime_hosts.contains(&approved_host) {
                return Err(SandboxPlanError::CredentialHostNotAllowed {
                    host: binding.approved_host.clone(),
                });
            }
            if !seen.insert((
                binding.approved_host.to_ascii_lowercase(),
                binding.header_name(),
            )) {
                return Err(SandboxPlanError::DuplicateCredentialTarget {
                    host: binding.approved_host.clone(),
                    header: binding.header_name(),
                });
            }
            if let Some(env_name) = &binding.placeholder_env {
                match self.run.env.get(env_name) {
                    Some(value) if value == &binding.placeholder_value => {}
                    Some(_) => {
                        return Err(SandboxPlanError::InvalidPlaceholderEnv {
                            env: env_name.clone(),
                        });
                    }
                    None => {
                        return Err(SandboxPlanError::MissingPlaceholderEnv {
                            env: env_name.clone(),
                        });
                    }
                }
            }
        }

        if !self.credentials.is_empty() {
            if !self.network.direct_egress_lockdown {
                return Err(SandboxPlanError::CredentialedRunWithoutLockdown);
            }
            if self.network.runtime_hosts.is_empty() {
                return Err(SandboxPlanError::CredentialedRunWithoutRuntimeNetwork);
            }
            if self.mounts.tools.writable || self.mounts.cache.writable {
                return Err(SandboxPlanError::WritableStateDuringCredentialedRun);
            }
        }

        Ok(())
    }

    fn image(&self) -> &str {
        self.image
            .as_deref()
            .unwrap_or(DEFAULT_PROCESS_SANDBOX_IMAGE)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxInstallPlan {
    pub command: SandboxCommandPlan,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxCommandPlan {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub max_stdout_bytes: Option<u64>,
    #[serde(default)]
    pub max_stderr_bytes: Option<u64>,
}

impl SandboxCommandPlan {
    fn validate(&self, phase: &'static str) -> Result<(), SandboxPlanError> {
        if self.command.trim().is_empty() {
            return Err(SandboxPlanError::EmptyCommand { phase });
        }
        if self.command.starts_with('-') || self.command.chars().any(char::is_whitespace) {
            return Err(SandboxPlanError::UnsafeCommand { phase });
        }
        if let Some(working_dir) = &self.working_dir
            && !is_container_absolute_path(working_dir)
        {
            return Err(SandboxPlanError::InvalidContainerPath {
                path: working_dir.clone(),
            });
        }
        for (name, value) in &self.env {
            validate_env_name(name)?;
            if value.contains('\0') {
                return Err(SandboxPlanError::InvalidEnvValue { env: name.clone() });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxMounts {
    pub workspace: SandboxMount,
    pub tools: SandboxMount,
    pub cache: SandboxMount,
}

impl Default for SandboxMounts {
    fn default() -> Self {
        Self {
            workspace: SandboxMount {
                container_path: DEFAULT_WORKSPACE_MOUNT.to_string(),
                writable: true,
            },
            tools: SandboxMount {
                container_path: DEFAULT_TOOLS_MOUNT.to_string(),
                writable: false,
            },
            cache: SandboxMount {
                container_path: DEFAULT_CACHE_MOUNT.to_string(),
                writable: false,
            },
        }
    }
}

impl SandboxMounts {
    fn validate(&self) -> Result<(), SandboxPlanError> {
        self.workspace.validate()?;
        self.tools.validate()?;
        self.cache.validate()?;
        if self.workspace.container_path == self.tools.container_path
            || self.workspace.container_path == self.cache.container_path
            || self.tools.container_path == self.cache.container_path
        {
            return Err(SandboxPlanError::DuplicateMountPath);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxMount {
    pub container_path: String,
    #[serde(default)]
    pub writable: bool,
}

impl SandboxMount {
    fn validate(&self) -> Result<(), SandboxPlanError> {
        if !is_container_absolute_path(&self.container_path) {
            return Err(SandboxPlanError::InvalidContainerPath {
                path: self.container_path.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxNetworkPlan {
    #[serde(default)]
    pub runtime_hosts: Vec<String>,
    #[serde(default)]
    pub direct_egress_lockdown: bool,
}

impl SandboxNetworkPlan {
    fn validate(&self) -> Result<(), SandboxPlanError> {
        for host in &self.runtime_hosts {
            validate_host(host)?;
        }
        Ok(())
    }

    fn runtime_allowed_hosts(&self) -> HashSet<String> {
        self.runtime_hosts
            .iter()
            .map(|host| host.to_ascii_lowercase())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxCredentialBinding {
    pub handle: SecretHandle,
    pub approved_host: String,
    pub target: RuntimeCredentialTarget,
    #[serde(default)]
    pub placeholder_env: Option<String>,
    pub placeholder_value: String,
    #[serde(default = "default_required")]
    pub required: bool,
}

impl SandboxCredentialBinding {
    fn validate(&self) -> Result<(), SandboxPlanError> {
        validate_host(&self.approved_host)?;
        if self.placeholder_value.trim().is_empty()
            || self.placeholder_value.contains(char::is_whitespace)
        {
            return Err(SandboxPlanError::InvalidCredentialPlaceholder);
        }
        if let Some(env) = &self.placeholder_env {
            validate_env_name(env)?;
        }
        match &self.target {
            RuntimeCredentialTarget::Header { name, prefix } => {
                validate_header_name(name)?;
                if let Some(prefix) = prefix
                    && prefix.contains('\n')
                {
                    return Err(SandboxPlanError::InvalidCredentialTarget);
                }
            }
            RuntimeCredentialTarget::QueryParam { .. } => {
                return Err(SandboxPlanError::UnsupportedCredentialTarget);
            }
        }
        Ok(())
    }

    fn header_name(&self) -> String {
        match &self.target {
            RuntimeCredentialTarget::Header { name, .. } => name.to_ascii_lowercase(),
            RuntimeCredentialTarget::QueryParam { name } => name.to_ascii_lowercase(),
        }
    }
}

fn default_required() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxProcessApprovalSummary {
    pub install_command: Option<Vec<String>>,
    pub run_command: Vec<String>,
    pub mounts: Vec<SandboxApprovalMount>,
    pub allowed_network_hosts: Vec<String>,
    pub credentials: Vec<SandboxApprovalCredential>,
    pub direct_egress_lockdown: bool,
}

impl SandboxProcessApprovalSummary {
    pub fn from_plan(plan: &SandboxProcessPlan) -> Result<Self, SandboxPlanError> {
        plan.validate()?;
        Ok(Self {
            install_command: plan
                .install
                .as_ref()
                .map(|install| command_line(&install.command)),
            run_command: command_line(&plan.run),
            mounts: vec![
                SandboxApprovalMount::from_mount("workspace", &plan.mounts.workspace),
                SandboxApprovalMount::from_mount("tools", &plan.mounts.tools),
                SandboxApprovalMount::from_mount("cache", &plan.mounts.cache),
            ],
            allowed_network_hosts: plan.network.runtime_hosts.clone(),
            credentials: plan
                .credentials
                .iter()
                .map(SandboxApprovalCredential::from_binding)
                .collect(),
            direct_egress_lockdown: plan.network.direct_egress_lockdown,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxApprovalMount {
    pub name: String,
    pub container_path: String,
    pub writable: bool,
}

impl SandboxApprovalMount {
    fn from_mount(name: &str, mount: &SandboxMount) -> Self {
        Self {
            name: name.to_string(),
            container_path: mount.container_path.clone(),
            writable: mount.writable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxApprovalCredential {
    pub secret_alias: SecretHandle,
    pub approved_host: String,
    pub placeholder_env: Option<String>,
    pub placeholder_value: String,
    pub target: String,
    pub required: bool,
}

impl SandboxApprovalCredential {
    fn from_binding(binding: &SandboxCredentialBinding) -> Self {
        Self {
            secret_alias: binding.handle.clone(),
            approved_host: binding.approved_host.clone(),
            placeholder_env: binding.placeholder_env.clone(),
            placeholder_value: binding.placeholder_value.clone(),
            target: credential_target_summary(&binding.target),
            required: binding.required,
        }
    }
}

fn command_line(command: &SandboxCommandPlan) -> Vec<String> {
    let mut line = vec![command.command.clone()];
    line.extend(command.args.clone());
    line
}

fn credential_target_summary(target: &RuntimeCredentialTarget) -> String {
    match target {
        RuntimeCredentialTarget::Header { name, prefix } => {
            format!(
                "header:{name}={}<secret>",
                prefix.as_deref().unwrap_or_default()
            )
        }
        RuntimeCredentialTarget::QueryParam { name } => format!("query:{name}=<secret>"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SandboxPlanError {
    #[error("{phase} command must not be empty")]
    EmptyCommand { phase: &'static str },
    #[error("{phase} command must be a single executable name or path")]
    UnsafeCommand { phase: &'static str },
    #[error("invalid Docker image reference: {reason}")]
    InvalidImage { reason: String },
    #[error("invalid host {host}: {reason}")]
    InvalidHost { host: String, reason: String },
    #[error("invalid container path {path}")]
    InvalidContainerPath { path: String },
    #[error("mount container paths must be unique")]
    DuplicateMountPath,
    #[error("invalid environment variable name {env}")]
    InvalidEnvName { env: String },
    #[error("invalid environment variable value for {env}")]
    InvalidEnvValue { env: String },
    #[error("sensitive environment variable {env} must use an approved placeholder")]
    RawSecretEnvValue { env: String },
    #[error("invalid credential placeholder")]
    InvalidCredentialPlaceholder,
    #[error("credential target is invalid")]
    InvalidCredentialTarget,
    #[error("credential target is not supported by Docker process sandbox MVP")]
    UnsupportedCredentialTarget,
    #[error("credential host {host} is not allowed by runtime network plan")]
    CredentialHostNotAllowed { host: String },
    #[error("duplicate credential target {host}/{header}")]
    DuplicateCredentialTarget { host: String, header: String },
    #[error("credentialed run requires direct egress lockdown")]
    CredentialedRunWithoutLockdown,
    #[error("credentialed run requires runtime network hosts")]
    CredentialedRunWithoutRuntimeNetwork,
    #[error("credentialed run requires a configured broker")]
    CredentialedRunWithoutBroker,
    #[error("credentialed run must not mount tool/cache state writable")]
    WritableStateDuringCredentialedRun,
    #[error("placeholder env {env} is missing from run env")]
    MissingPlaceholderEnv { env: String },
    #[error("placeholder env {env} must equal the approved placeholder value")]
    InvalidPlaceholderEnv { env: String },
}

#[derive(Debug, Clone)]
pub struct DockerProcessSandboxConfig {
    pub docker_bin: String,
    pub image: String,
    pub workspace_host_path: PathBuf,
    pub tools_host_path: PathBuf,
    pub cache_host_path: PathBuf,
    pub broker: Option<DockerBrokerConfig>,
}

impl DockerProcessSandboxConfig {
    pub fn new(
        workspace_host_path: impl Into<PathBuf>,
        tools_host_path: impl Into<PathBuf>,
        cache_host_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            docker_bin: "docker".to_string(),
            image: DEFAULT_PROCESS_SANDBOX_IMAGE.to_string(),
            workspace_host_path: workspace_host_path.into(),
            tools_host_path: tools_host_path.into(),
            cache_host_path: cache_host_path.into(),
            broker: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerBrokerConfig {
    pub proxy_url: String,
    pub ca_cert_host_path: PathBuf,
    pub ca_cert_container_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxProcessPhase {
    Install,
    Run,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerInvocation {
    pub docker_bin: String,
    pub phase: SandboxProcessPhase,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerRunOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub wall_clock_ms: u64,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone)]
pub struct SandboxProcessRequest {
    pub process_id: ProcessId,
    pub scope: ResourceScope,
    pub plan: SandboxProcessPlan,
    pub cancellation: ProcessCancellationToken,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SandboxProcessResult {
    pub output: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("process sandbox execution failed: {kind}")]
pub struct SandboxProcessError {
    pub kind: String,
}

impl SandboxProcessError {
    pub fn new(kind: impl Into<String>) -> Self {
        Self { kind: kind.into() }
    }
}

#[async_trait]
pub trait ProcessSandboxBackend: Send + Sync {
    async fn execute(
        &self,
        request: SandboxProcessRequest,
    ) -> Result<SandboxProcessResult, SandboxProcessError>;
}

#[derive(Clone)]
pub struct ProcessSandboxExecutor {
    backend: Arc<dyn ProcessSandboxBackend>,
}

impl ProcessSandboxExecutor {
    pub fn new(backend: Arc<dyn ProcessSandboxBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl ProcessExecutor for ProcessSandboxExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        let plan = serde_json::from_value::<SandboxProcessPlan>(request.input)
            .map_err(|_| ProcessExecutionError::new("invalid_process_sandbox_plan"))?;
        plan.validate()
            .map_err(|_| ProcessExecutionError::new("invalid_process_sandbox_plan"))?;
        let result = self
            .backend
            .execute(SandboxProcessRequest {
                process_id: request.process_id,
                scope: request.scope,
                plan,
                cancellation: request.cancellation,
            })
            .await
            .map_err(|error| ProcessExecutionError::new(error.kind))?;
        Ok(ProcessExecutionResult {
            output: result.output,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DockerRunError {
    #[error("Docker process failed to start")]
    Spawn,
    #[error("Docker process I/O failed")]
    Io,
    #[error("Docker process was cancelled")]
    Cancelled,
    #[error("Docker process timed out")]
    Timeout,
}

#[async_trait]
pub trait DockerRunner: Send + Sync {
    async fn run(
        &self,
        invocation: DockerInvocation,
        command: &SandboxCommandPlan,
        cancellation: ProcessCancellationToken,
    ) -> Result<DockerRunOutput, DockerRunError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemDockerRunner;

#[async_trait]
impl DockerRunner for SystemDockerRunner {
    async fn run(
        &self,
        invocation: DockerInvocation,
        command_plan: &SandboxCommandPlan,
        cancellation: ProcessCancellationToken,
    ) -> Result<DockerRunOutput, DockerRunError> {
        let started = Instant::now();
        let mut command = Command::new(&invocation.docker_bin);
        command
            .args(&invocation.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|_| DockerRunError::Spawn)?;
        let stdout = child.stdout.take().ok_or(DockerRunError::Io)?;
        let stderr = child.stderr.take().ok_or(DockerRunError::Io)?;
        let stdout_limit = command_plan
            .max_stdout_bytes
            .unwrap_or(DEFAULT_STDOUT_LIMIT);
        let stderr_limit = command_plan
            .max_stderr_bytes
            .unwrap_or(DEFAULT_STDERR_LIMIT);
        let stdout_reader = tokio::spawn(read_bounded_async(stdout, stdout_limit));
        let stderr_reader = tokio::spawn(read_bounded_async(stderr, stderr_limit));
        let timeout = Duration::from_millis(command_plan.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));

        let status = tokio::select! {
            status = child.wait() => status.map_err(|_| DockerRunError::Io)?,
            _ = cancellation.cancelled() => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(DockerRunError::Cancelled);
            }
            _ = time::sleep(timeout) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(DockerRunError::Timeout);
            }
        };

        let (stdout, stdout_truncated) = stdout_reader
            .await
            .map_err(|_| DockerRunError::Io)?
            .map_err(|_| DockerRunError::Io)?;
        let (stderr, stderr_truncated) = stderr_reader
            .await
            .map_err(|_| DockerRunError::Io)?
            .map_err(|_| DockerRunError::Io)?;
        Ok(DockerRunOutput {
            exit_code: status.code().unwrap_or(-1),
            stdout,
            stderr,
            wall_clock_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            stdout_truncated,
            stderr_truncated,
        })
    }
}

async fn read_bounded_async<R>(mut reader: R, limit: u64) -> Result<(Vec<u8>, bool), std::io::Error>
where
    R: AsyncRead + Unpin,
{
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            return Ok((output, truncated));
        }
        let remaining = limit.saturating_sub(output.len());
        if remaining == 0 {
            truncated = true;
            continue;
        }
        let take = read.min(remaining);
        output.extend_from_slice(&buffer[..take]);
        if take < read {
            truncated = true;
        }
    }
}

#[derive(Clone)]
pub struct DockerProcessSandboxBackend {
    config: DockerProcessSandboxConfig,
    runner: Arc<dyn DockerRunner>,
}

impl DockerProcessSandboxBackend {
    pub fn new(config: DockerProcessSandboxConfig) -> Self {
        Self {
            config,
            runner: Arc::new(SystemDockerRunner),
        }
    }

    pub fn with_runner(config: DockerProcessSandboxConfig, runner: Arc<dyn DockerRunner>) -> Self {
        Self { config, runner }
    }
}

#[async_trait]
impl ProcessSandboxBackend for DockerProcessSandboxBackend {
    async fn execute(
        &self,
        request: SandboxProcessRequest,
    ) -> Result<SandboxProcessResult, SandboxProcessError> {
        request
            .plan
            .validate()
            .map_err(|_| SandboxProcessError::new("invalid_process_sandbox_plan"))?;
        let mut phase_results = Vec::new();
        if let Some(install) = &request.plan.install {
            let invocation = docker_invocation_for_phase(
                &self.config,
                &request.plan,
                SandboxProcessPhase::Install,
                &install.command,
            )
            .map_err(|_| SandboxProcessError::new("invalid_process_sandbox_plan"))?;
            let output = self
                .runner
                .run(invocation, &install.command, request.cancellation.clone())
                .await
                .map_err(sandbox_process_error)?;
            phase_results.push(phase_output(SandboxProcessPhase::Install, output));
        }

        let invocation = docker_invocation_for_phase(
            &self.config,
            &request.plan,
            SandboxProcessPhase::Run,
            &request.plan.run,
        )
        .map_err(|_| SandboxProcessError::new("invalid_process_sandbox_plan"))?;
        let output = self
            .runner
            .run(invocation, &request.plan.run, request.cancellation)
            .await
            .map_err(sandbox_process_error)?;
        phase_results.push(phase_output(SandboxProcessPhase::Run, output));

        Ok(SandboxProcessResult {
            output: json!({
                "kind": "process_sandbox_result",
                "phases": phase_results,
            }),
        })
    }
}

fn sandbox_process_error(error: DockerRunError) -> SandboxProcessError {
    SandboxProcessError::new(match error {
        DockerRunError::Spawn => "docker_spawn_failed",
        DockerRunError::Io => "docker_io_failed",
        DockerRunError::Cancelled => "cancelled",
        DockerRunError::Timeout => "timeout",
    })
}

fn phase_output(phase: SandboxProcessPhase, output: DockerRunOutput) -> Value {
    json!({
        "phase": phase,
        "exit_code": output.exit_code,
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
        "stdout_truncated": output.stdout_truncated,
        "stderr_truncated": output.stderr_truncated,
        "wall_clock_ms": output.wall_clock_ms,
    })
}

pub fn docker_invocation_for_phase(
    config: &DockerProcessSandboxConfig,
    plan: &SandboxProcessPlan,
    phase: SandboxProcessPhase,
    command: &SandboxCommandPlan,
) -> Result<DockerInvocation, SandboxPlanError> {
    plan.validate()?;
    if phase == SandboxProcessPhase::Run && !plan.credentials.is_empty() && config.broker.is_none()
    {
        return Err(SandboxPlanError::CredentialedRunWithoutBroker);
    }
    let mut args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--init".to_string(),
        "--security-opt".to_string(),
        "no-new-privileges".to_string(),
        "--cap-drop".to_string(),
        "ALL".to_string(),
    ];
    if phase == SandboxProcessPhase::Run && !plan.credentials.is_empty() {
        args.push("--cap-add".to_string());
        args.push("NET_ADMIN".to_string());
    }

    args.extend(network_args_for_phase(config, plan, phase));
    args.extend(mount_args_for_phase(config, plan, phase));
    args.extend(env_args_for_phase(config, plan, phase, command)?);
    if let Some(working_dir) = &command.working_dir {
        args.push("--workdir".to_string());
        args.push(working_dir.clone());
    }
    args.push(plan.image().to_string());
    args.push(command.command.clone());
    args.extend(command.args.clone());
    Ok(DockerInvocation {
        docker_bin: config.docker_bin.clone(),
        phase,
        args,
    })
}

fn network_args_for_phase(
    config: &DockerProcessSandboxConfig,
    plan: &SandboxProcessPlan,
    phase: SandboxProcessPhase,
) -> Vec<String> {
    match phase {
        SandboxProcessPhase::Install => {
            if plan
                .install
                .as_ref()
                .is_some_and(|install| !install.allowed_hosts.is_empty())
            {
                vec!["--network".to_string(), "bridge".to_string()]
            } else {
                vec!["--network".to_string(), "none".to_string()]
            }
        }
        SandboxProcessPhase::Run => {
            if !plan.credentials.is_empty() || !plan.network.runtime_hosts.is_empty() {
                let mut args = vec!["--network".to_string(), "bridge".to_string()];
                if !plan.credentials.is_empty() && config.broker.is_some() {
                    args.extend([
                        "--env".to_string(),
                        "IRONCLAW_EGRESS_LOCKDOWN=broker-only".to_string(),
                    ]);
                }
                args
            } else {
                vec!["--network".to_string(), "none".to_string()]
            }
        }
    }
}

fn mount_args_for_phase(
    config: &DockerProcessSandboxConfig,
    plan: &SandboxProcessPlan,
    phase: SandboxProcessPhase,
) -> Vec<String> {
    let tools_readonly = phase == SandboxProcessPhase::Run && !plan.mounts.tools.writable;
    let cache_readonly = phase == SandboxProcessPhase::Run && !plan.mounts.cache.writable;
    let mut args = Vec::new();
    args.extend(bind_mount_arg(
        &config.workspace_host_path,
        &plan.mounts.workspace.container_path,
        !plan.mounts.workspace.writable,
    ));
    args.extend(bind_mount_arg(
        &config.tools_host_path,
        &plan.mounts.tools.container_path,
        tools_readonly,
    ));
    args.extend(bind_mount_arg(
        &config.cache_host_path,
        &plan.mounts.cache.container_path,
        cache_readonly,
    ));
    if phase == SandboxProcessPhase::Run
        && !plan.credentials.is_empty()
        && let Some(broker) = &config.broker
    {
        args.extend(bind_mount_arg(
            &broker.ca_cert_host_path,
            &broker.ca_cert_container_path,
            true,
        ));
    }
    args
}

fn bind_mount_arg(host_path: &Path, container_path: &str, readonly: bool) -> Vec<String> {
    let mut spec = format!(
        "type=bind,src={},dst={}",
        host_path.display(),
        container_path
    );
    if readonly {
        spec.push_str(",readonly");
    }
    vec!["--mount".to_string(), spec]
}

fn env_args_for_phase(
    config: &DockerProcessSandboxConfig,
    plan: &SandboxProcessPlan,
    phase: SandboxProcessPhase,
    command: &SandboxCommandPlan,
) -> Result<Vec<String>, SandboxPlanError> {
    let mut env = command.env.clone();
    if phase == SandboxProcessPhase::Run
        && !plan.credentials.is_empty()
        && let Some(broker) = &config.broker
    {
        env.insert("HTTP_PROXY".to_string(), broker.proxy_url.clone());
        env.insert("HTTPS_PROXY".to_string(), broker.proxy_url.clone());
        env.insert(
            "SSL_CERT_FILE".to_string(),
            broker.ca_cert_container_path.clone(),
        );
        env.insert(
            "REQUESTS_CA_BUNDLE".to_string(),
            broker.ca_cert_container_path.clone(),
        );
        env.insert(
            "NODE_EXTRA_CA_CERTS".to_string(),
            broker.ca_cert_container_path.clone(),
        );
        env.insert(
            "GIT_SSL_CAINFO".to_string(),
            broker.ca_cert_container_path.clone(),
        );
        env.insert(
            "CURL_CA_BUNDLE".to_string(),
            broker.ca_cert_container_path.clone(),
        );
        env.insert(
            "IRONCLAW_BROKER_PROXY".to_string(),
            broker.proxy_url.clone(),
        );
    }
    let placeholders = plan
        .credentials
        .iter()
        .map(|binding| binding.placeholder_value.as_str())
        .collect::<Vec<_>>();
    validate_env_has_no_raw_sensitive_values(&env, &placeholders)?;
    let mut args = Vec::new();
    for (name, value) in env {
        validate_env_name(&name)?;
        args.push("--env".to_string());
        args.push(format!("{name}={value}"));
    }
    Ok(args)
}

#[derive(Debug, Clone)]
pub struct BrokerHeaderRewrite {
    pub name: String,
    pub old_value: String,
    pub new_value: SecretString,
    pub secret_alias: SecretHandle,
}

#[derive(Debug, Clone)]
pub struct BrokerRewriteResult {
    pub headers: Vec<(String, String)>,
    pub rewrites: Vec<BrokerHeaderRewrite>,
}

#[derive(Debug, Clone)]
pub struct SandboxBrokerPolicy {
    bindings: Vec<SandboxCredentialBinding>,
}

impl SandboxBrokerPolicy {
    pub fn new(bindings: Vec<SandboxCredentialBinding>) -> Result<Self, SandboxPlanError> {
        let policy = Self { bindings };
        for binding in &policy.bindings {
            binding.validate()?;
        }
        Ok(policy)
    }

    pub fn rewrite_headers(
        &self,
        host: &str,
        headers: Vec<(String, String)>,
        secrets: &HashMap<SecretHandle, SecretString>,
    ) -> BrokerRewriteResult {
        let mut rewrites = Vec::new();
        let rewritten_headers = headers
            .into_iter()
            .map(|(name, value)| {
                let Some(binding) = self.matching_header_binding(host, &name, &value) else {
                    return (name, value);
                };
                let Some(secret) = secrets.get(&binding.handle) else {
                    return (name, value);
                };
                let RuntimeCredentialTarget::Header { prefix, .. } = &binding.target else {
                    return (name, value);
                };
                let prefix = prefix.as_deref().unwrap_or_default();
                let new_plain = format!("{prefix}{}", secret.expose_secret());
                rewrites.push(BrokerHeaderRewrite {
                    name: name.clone(),
                    old_value: value,
                    new_value: SecretString::from(new_plain.clone()),
                    secret_alias: binding.handle.clone(),
                });
                (name, new_plain)
            })
            .collect();
        BrokerRewriteResult {
            headers: rewritten_headers,
            rewrites,
        }
    }

    pub fn sanitize_text(
        &self,
        text: &str,
        secrets: &HashMap<SecretHandle, SecretString>,
    ) -> String {
        secrets.values().fold(text.to_string(), |acc, secret| {
            let value = secret.expose_secret();
            if value.is_empty() {
                acc
            } else {
                acc.replace(value, "[REDACTED]")
            }
        })
    }

    fn matching_header_binding(
        &self,
        host: &str,
        header_name: &str,
        header_value: &str,
    ) -> Option<&SandboxCredentialBinding> {
        self.bindings.iter().find(|binding| {
            binding.approved_host.eq_ignore_ascii_case(host)
                && match &binding.target {
                    RuntimeCredentialTarget::Header { name, prefix } => {
                        name.eq_ignore_ascii_case(header_name)
                            && header_value
                                == format!(
                                    "{}{}",
                                    prefix.as_deref().unwrap_or_default(),
                                    binding.placeholder_value
                                )
                    }
                    RuntimeCredentialTarget::QueryParam { .. } => false,
                }
        })
    }
}

fn validate_docker_image_reference(image: &str) -> Result<(), SandboxPlanError> {
    if image.is_empty() {
        return Err(SandboxPlanError::InvalidImage {
            reason: "must not be empty".to_string(),
        });
    }
    if image.starts_with('-') {
        return Err(SandboxPlanError::InvalidImage {
            reason: "must not start with '-'".to_string(),
        });
    }
    if image.chars().any(char::is_whitespace) {
        return Err(SandboxPlanError::InvalidImage {
            reason: "must not contain whitespace".to_string(),
        });
    }
    Ok(())
}

fn validate_host(host: &str) -> Result<(), SandboxPlanError> {
    if host.is_empty() {
        return Err(SandboxPlanError::InvalidHost {
            host: host.to_string(),
            reason: "must not be empty".to_string(),
        });
    }
    if host.contains('/') || host.contains(':') || host.chars().any(char::is_whitespace) {
        return Err(SandboxPlanError::InvalidHost {
            host: host.to_string(),
            reason: "must be a host name without scheme, port, path, or whitespace".to_string(),
        });
    }
    Ok(())
}

fn validate_header_name(name: &str) -> Result<(), SandboxPlanError> {
    if name.is_empty()
        || name
            .bytes()
            .any(|byte| !matches!(byte, b'!' | b'#'..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'^' | b'_' | b'`' | b'a'..=b'z' | b'|' | b'~'))
    {
        return Err(SandboxPlanError::InvalidCredentialTarget);
    }
    Ok(())
}

fn validate_env_name(name: &str) -> Result<(), SandboxPlanError> {
    if name.is_empty()
        || name.starts_with(|ch: char| ch.is_ascii_digit())
        || !name
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
    {
        return Err(SandboxPlanError::InvalidEnvName {
            env: name.to_string(),
        });
    }
    Ok(())
}

fn validate_env_has_no_raw_sensitive_values(
    env: &HashMap<String, String>,
    allowed_placeholders: &[&str],
) -> Result<(), SandboxPlanError> {
    for (name, value) in env {
        if is_sensitive_env_name(name)
            && !allowed_placeholders
                .iter()
                .any(|placeholder| value == placeholder)
        {
            return Err(SandboxPlanError::RawSecretEnvValue { env: name.clone() });
        }
    }
    Ok(())
}

fn is_sensitive_env_name(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "API_KEY",
        "ACCESS_KEY",
        "AUTH",
    ]
    .iter()
    .any(|marker| name.contains(marker))
}

fn is_container_absolute_path(path: &str) -> bool {
    path.starts_with('/') && !path.contains('\0') && !path.split('/').any(|segment| segment == "..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        AgentId, CapabilityId, ExtensionId, InvocationId, MountView, ProcessId, ProjectId,
        ResourceEstimate, ResourceScope, RuntimeKind, TenantId, ThreadId, UserId,
    };
    use std::sync::Mutex;

    fn sample_plan() -> SandboxProcessPlan {
        let mut env = HashMap::new();
        env.insert("NOTION_API_KEY".to_string(), "NOTION_API_KEY".to_string());
        SandboxProcessPlan {
            image: None,
            install: Some(SandboxInstallPlan {
                command: SandboxCommandPlan {
                    command: "npm".to_string(),
                    args: vec![
                        "install".to_string(),
                        "-g".to_string(),
                        "notion-cli".to_string(),
                    ],
                    env: HashMap::new(),
                    working_dir: None,
                    timeout_ms: None,
                    max_stdout_bytes: None,
                    max_stderr_bytes: None,
                },
                allowed_hosts: vec!["registry.npmjs.org".to_string()],
            }),
            run: SandboxCommandPlan {
                command: "notion".to_string(),
                args: vec!["list".to_string()],
                env,
                working_dir: Some("/workspace".to_string()),
                timeout_ms: Some(5_000),
                max_stdout_bytes: Some(4096),
                max_stderr_bytes: Some(4096),
            },
            mounts: SandboxMounts::default(),
            network: SandboxNetworkPlan {
                runtime_hosts: vec!["api.notion.com".to_string()],
                direct_egress_lockdown: true,
            },
            credentials: vec![SandboxCredentialBinding {
                handle: SecretHandle::new("notion_token").unwrap(),
                approved_host: "api.notion.com".to_string(),
                target: RuntimeCredentialTarget::Header {
                    name: "Authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                placeholder_env: Some("NOTION_API_KEY".to_string()),
                placeholder_value: "NOTION_API_KEY".to_string(),
                required: true,
            }],
        }
    }

    fn sample_config(root: &Path) -> DockerProcessSandboxConfig {
        DockerProcessSandboxConfig {
            docker_bin: "docker".to_string(),
            image: DEFAULT_PROCESS_SANDBOX_IMAGE.to_string(),
            workspace_host_path: root.join("workspace"),
            tools_host_path: root.join("tools"),
            cache_host_path: root.join("cache"),
            broker: Some(DockerBrokerConfig {
                proxy_url: "http://host.docker.internal:4489".to_string(),
                ca_cert_host_path: root.join("broker-ca.pem"),
                ca_cert_container_path: "/ironclaw/broker/ca.pem".to_string(),
            }),
        }
    }

    #[test]
    fn plan_validation_rejects_raw_secret_env_values() {
        let mut plan = sample_plan();
        plan.run.env.insert(
            "NOTION_API_KEY".to_string(),
            "real-secret-token".to_string(),
        );

        let error = plan.validate().unwrap_err();

        assert!(matches!(error, SandboxPlanError::RawSecretEnvValue { .. }));
    }

    #[test]
    fn plan_validation_rejects_credential_host_missing_from_runtime_network() {
        let mut plan = sample_plan();
        plan.network.runtime_hosts.clear();

        let error = plan.validate().unwrap_err();

        assert!(matches!(
            error,
            SandboxPlanError::CredentialHostNotAllowed { .. }
                | SandboxPlanError::CredentialedRunWithoutRuntimeNetwork
        ));
    }

    #[test]
    fn plan_validation_rejects_credentialed_run_without_lockdown() {
        let mut plan = sample_plan();
        plan.network.direct_egress_lockdown = false;

        let error = plan.validate().unwrap_err();

        assert_eq!(error, SandboxPlanError::CredentialedRunWithoutLockdown);
    }

    #[test]
    fn plan_validation_rejects_writable_quarantine_during_credentialed_run() {
        let mut plan = sample_plan();
        plan.mounts.tools.writable = true;

        let error = plan.validate().unwrap_err();

        assert_eq!(error, SandboxPlanError::WritableStateDuringCredentialedRun);
    }

    #[test]
    fn docker_args_never_include_secret_material() {
        let temp = tempfile::tempdir().unwrap();
        let plan = sample_plan();
        let config = sample_config(temp.path());

        let invocation =
            docker_invocation_for_phase(&config, &plan, SandboxProcessPhase::Run, &plan.run)
                .unwrap();
        let joined = invocation.args.join("\n");

        assert!(!joined.contains("real-notion-secret"));
        assert!(joined.contains("NOTION_API_KEY=NOTION_API_KEY"));
        assert!(joined.contains("IRONCLAW_EGRESS_LOCKDOWN=broker-only"));
        assert!(joined.contains("HTTP_PROXY=http://host.docker.internal:4489"));
    }

    #[test]
    fn docker_builder_rejects_credentialed_run_without_broker() {
        let temp = tempfile::tempdir().unwrap();
        let plan = sample_plan();
        let mut config = sample_config(temp.path());
        config.broker = None;

        let error =
            docker_invocation_for_phase(&config, &plan, SandboxProcessPhase::Run, &plan.run)
                .unwrap_err();

        assert_eq!(error, SandboxPlanError::CredentialedRunWithoutBroker);
    }

    #[test]
    fn install_and_run_phases_have_different_mount_and_network_policies() {
        let temp = tempfile::tempdir().unwrap();
        let plan = sample_plan();
        let config = sample_config(temp.path());
        let install = docker_invocation_for_phase(
            &config,
            &plan,
            SandboxProcessPhase::Install,
            &plan.install.as_ref().unwrap().command,
        )
        .unwrap();
        let run = docker_invocation_for_phase(&config, &plan, SandboxProcessPhase::Run, &plan.run)
            .unwrap();
        let install_args = install.args.join("\n");
        let run_args = run.args.join("\n");

        assert!(install_args.contains("--network\nbridge"));
        assert!(!install_args.contains("IRONCLAW_EGRESS_LOCKDOWN=broker-only"));
        assert!(!install_args.contains("dst=/ironclaw/state/tools,readonly"));
        assert!(run_args.contains("IRONCLAW_EGRESS_LOCKDOWN=broker-only"));
        assert!(run_args.contains("dst=/ironclaw/state/tools,readonly"));
        assert!(run_args.contains("dst=/ironclaw/state/cache,readonly"));
    }

    #[test]
    fn broker_policy_rewrites_only_approved_host_and_header() {
        let plan = sample_plan();
        let policy = SandboxBrokerPolicy::new(plan.credentials).unwrap();
        let mut secrets = HashMap::new();
        secrets.insert(
            SecretHandle::new("notion_token").unwrap(),
            SecretString::from("real-notion-secret"),
        );

        let approved = policy.rewrite_headers(
            "api.notion.com",
            vec![(
                "Authorization".to_string(),
                "Bearer NOTION_API_KEY".to_string(),
            )],
            &secrets,
        );
        let denied = policy.rewrite_headers(
            "example.com",
            vec![(
                "Authorization".to_string(),
                "Bearer NOTION_API_KEY".to_string(),
            )],
            &secrets,
        );

        assert_eq!(
            approved.headers,
            vec![(
                "Authorization".to_string(),
                "Bearer real-notion-secret".to_string()
            )]
        );
        assert_eq!(approved.rewrites.len(), 1);
        assert_eq!(
            denied.headers,
            vec![(
                "Authorization".to_string(),
                "Bearer NOTION_API_KEY".to_string()
            )]
        );
        assert!(denied.rewrites.is_empty());
    }

    #[test]
    fn broker_redacts_secret_values_from_error_paths() {
        let plan = sample_plan();
        let policy = SandboxBrokerPolicy::new(plan.credentials).unwrap();
        let mut secrets = HashMap::new();
        secrets.insert(
            SecretHandle::new("notion_token").unwrap(),
            SecretString::from("real-notion-secret"),
        );

        let sanitized = policy.sanitize_text("upstream echoed real-notion-secret", &secrets);

        assert_eq!(sanitized, "upstream echoed [REDACTED]");
    }

    #[test]
    fn approval_summary_contains_only_sanitized_authority_details() {
        let plan = sample_plan();

        let summary = SandboxProcessApprovalSummary::from_plan(&plan).unwrap();
        let serialized = serde_json::to_string(&summary).unwrap();

        assert_eq!(
            summary.install_command,
            Some(vec![
                "npm".to_string(),
                "install".to_string(),
                "-g".to_string(),
                "notion-cli".to_string()
            ])
        );
        assert_eq!(
            summary.run_command,
            vec!["notion".to_string(), "list".to_string()]
        );
        assert_eq!(summary.allowed_network_hosts, vec!["api.notion.com"]);
        assert!(summary.direct_egress_lockdown);
        assert_eq!(summary.credentials[0].secret_alias.as_str(), "notion_token");
        assert_eq!(
            summary.credentials[0].target,
            "header:Authorization=Bearer <secret>"
        );
        assert!(serialized.contains("NOTION_API_KEY"));
        assert!(!serialized.contains("real-notion-secret"));
    }

    #[derive(Default)]
    struct RecordingRunner {
        invocations: Mutex<Vec<DockerInvocation>>,
    }

    #[async_trait]
    impl DockerRunner for RecordingRunner {
        async fn run(
            &self,
            invocation: DockerInvocation,
            _command: &SandboxCommandPlan,
            _cancellation: ProcessCancellationToken,
        ) -> Result<DockerRunOutput, DockerRunError> {
            self.invocations.lock().unwrap().push(invocation);
            Ok(DockerRunOutput {
                exit_code: 0,
                stdout: b"{\"ok\":true}".to_vec(),
                stderr: Vec::new(),
                wall_clock_ms: 12,
                stdout_truncated: false,
                stderr_truncated: false,
            })
        }
    }

    #[tokio::test]
    async fn executor_returns_sanitized_phase_output_json() {
        let temp = tempfile::tempdir().unwrap();
        let runner = Arc::new(RecordingRunner::default());
        let backend = DockerProcessSandboxBackend::with_runner(
            sample_config(temp.path()),
            runner.clone() as Arc<dyn DockerRunner>,
        );
        let executor = ProcessSandboxExecutor::new(Arc::new(backend));

        let result = executor
            .execute(sample_request(serde_json::to_value(sample_plan()).unwrap()))
            .await
            .unwrap();

        assert_eq!(result.output["kind"], "process_sandbox_result");
        assert_eq!(result.output["phases"].as_array().unwrap().len(), 2);
        assert_eq!(runner.invocations.lock().unwrap().len(), 2);
    }

    struct CancellingRunner;

    #[async_trait]
    impl DockerRunner for CancellingRunner {
        async fn run(
            &self,
            _invocation: DockerInvocation,
            _command: &SandboxCommandPlan,
            cancellation: ProcessCancellationToken,
        ) -> Result<DockerRunOutput, DockerRunError> {
            cancellation.cancel();
            Err(DockerRunError::Cancelled)
        }
    }

    #[tokio::test]
    async fn executor_surfaces_cancellation_as_stable_error_kind() {
        let temp = tempfile::tempdir().unwrap();
        let backend = DockerProcessSandboxBackend::with_runner(
            sample_config(temp.path()),
            Arc::new(CancellingRunner),
        );
        let executor = ProcessSandboxExecutor::new(Arc::new(backend));

        let error = executor
            .execute(sample_request(serde_json::to_value(sample_plan()).unwrap()))
            .await
            .unwrap_err();

        assert_eq!(error.kind, "cancelled");
    }

    fn sample_request(input: Value) -> ProcessExecutionRequest {
        ProcessExecutionRequest {
            process_id: ProcessId::new(),
            invocation_id: InvocationId::new(),
            scope: ResourceScope {
                tenant_id: TenantId::new("tenant").unwrap(),
                user_id: UserId::new("user").unwrap(),
                agent_id: Some(AgentId::new("agent").unwrap()),
                project_id: Some(ProjectId::new("project").unwrap()),
                mission_id: None,
                thread_id: Some(ThreadId::new("thread").unwrap()),
                invocation_id: InvocationId::new(),
            },
            extension_id: ExtensionId::new("system.process_sandbox").unwrap(),
            capability_id: CapabilityId::new("system.process_sandbox.run").unwrap(),
            runtime: RuntimeKind::System,
            estimate: ResourceEstimate::default(),
            mounts: MountView::default(),
            resource_reservation: None,
            input,
            cancellation: ProcessCancellationToken::new(),
        }
    }
}
