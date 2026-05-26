use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time,
};

use crate::{
    DEFAULT_PROCESS_SANDBOX_IMAGE, DEFAULT_STDERR_LIMIT, DEFAULT_STDOUT_LIMIT, DEFAULT_TIMEOUT_MS,
    ProcessSandboxBackend, ProcessSandboxError, ProcessSandboxErrorKind, SandboxCommandPlan,
    SandboxPhaseOutput, SandboxPlanError, SandboxProcessOutput, SandboxProcessRequest,
    SandboxProcessResult, ValidatedSandboxProcessPlan,
    validation::{validate_env_has_no_raw_sensitive_values, validate_env_name},
};
use ironclaw_processes::ProcessCancellationToken;

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
pub(crate) struct DockerInvocation {
    pub docker_bin: String,
    pub phase: SandboxProcessPhase,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DockerRunOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub wall_clock_ms: u64,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum DockerRunError {
    #[error("Docker process failed to start")]
    Spawn,
    #[error("Docker process I/O failed")]
    Io,
    #[error("Docker process was cancelled")]
    Cancelled,
    #[error("Docker process timed out")]
    Timeout,
}

use thiserror::Error;

#[async_trait]
pub(crate) trait DockerRunner: Send + Sync {
    async fn run(
        &self,
        invocation: DockerInvocation,
        command: &SandboxCommandPlan,
        cancellation: ProcessCancellationToken,
    ) -> Result<DockerRunOutput, DockerRunError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SystemDockerRunner;

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

    #[cfg(test)]
    pub(crate) fn with_runner(
        config: DockerProcessSandboxConfig,
        runner: Arc<dyn DockerRunner>,
    ) -> Self {
        Self { config, runner }
    }
}

#[async_trait]
impl ProcessSandboxBackend for DockerProcessSandboxBackend {
    async fn execute(
        &self,
        request: SandboxProcessRequest,
    ) -> Result<SandboxProcessResult, ProcessSandboxError> {
        let mut phases = Vec::new();
        if let Some(install) = &request.plan.install {
            let invocation = docker_invocation_for_phase(
                &self.config,
                &request.plan,
                SandboxProcessPhase::Install,
                &install.command,
            )?;
            let output = self
                .runner
                .run(invocation, &install.command, request.cancellation.clone())
                .await
                .map_err(sandbox_process_error)?;
            phases.push(phase_output(SandboxProcessPhase::Install, output));
        }

        let invocation = docker_invocation_for_phase(
            &self.config,
            &request.plan,
            SandboxProcessPhase::Run,
            &request.plan.run,
        )?;
        let output = self
            .runner
            .run(invocation, &request.plan.run, request.cancellation)
            .await
            .map_err(sandbox_process_error)?;
        phases.push(phase_output(SandboxProcessPhase::Run, output));

        Ok(SandboxProcessResult {
            output: SandboxProcessOutput { phases },
        })
    }
}

fn sandbox_process_error(error: DockerRunError) -> ProcessSandboxError {
    ProcessSandboxError::new(match error {
        DockerRunError::Spawn => ProcessSandboxErrorKind::DockerSpawnFailed,
        DockerRunError::Io => ProcessSandboxErrorKind::DockerIoFailed,
        DockerRunError::Cancelled => ProcessSandboxErrorKind::Cancelled,
        DockerRunError::Timeout => ProcessSandboxErrorKind::Timeout,
    })
}

fn phase_output(phase: SandboxProcessPhase, output: DockerRunOutput) -> SandboxPhaseOutput {
    SandboxPhaseOutput {
        phase,
        exit_code: output.exit_code,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        stdout_truncated: output.stdout_truncated,
        stderr_truncated: output.stderr_truncated,
        wall_clock_ms: output.wall_clock_ms,
    }
}

pub(crate) fn docker_invocation_for_phase(
    config: &DockerProcessSandboxConfig,
    plan: &ValidatedSandboxProcessPlan,
    phase: SandboxProcessPhase,
    command: &SandboxCommandPlan,
) -> Result<DockerInvocation, SandboxPlanError> {
    let spec = DockerPhaseSpec::new(config, plan, phase, command)?;
    let mut args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--init".to_string(),
        "--security-opt".to_string(),
        "no-new-privileges".to_string(),
        "--cap-drop".to_string(),
        "ALL".to_string(),
    ];
    if spec.needs_net_admin {
        args.push("--cap-add".to_string());
        args.push("NET_ADMIN".to_string());
    }

    args.extend(spec.network_args());
    args.extend(spec.mount_args(config, plan));
    args.extend(spec.env_args(config, plan)?);
    if let Some(working_dir) = &command.working_dir {
        args.push("--workdir".to_string());
        args.push(working_dir.clone());
    }
    args.push(config.image.clone());
    args.push(command.command.clone());
    args.extend(command.args.clone());
    Ok(DockerInvocation {
        docker_bin: config.docker_bin.clone(),
        phase,
        args,
    })
}

struct DockerPhaseSpec<'a> {
    command: &'a SandboxCommandPlan,
    network_mode: &'static str,
    lockdown: bool,
    needs_net_admin: bool,
    include_broker_ca: bool,
    tools_readonly: bool,
    cache_readonly: bool,
}

impl<'a> DockerPhaseSpec<'a> {
    fn new(
        config: &DockerProcessSandboxConfig,
        plan: &ValidatedSandboxProcessPlan,
        phase: SandboxProcessPhase,
        command: &'a SandboxCommandPlan,
    ) -> Result<Self, SandboxPlanError> {
        let has_credentials = !plan.credentials.is_empty();
        if phase == SandboxProcessPhase::Run && has_credentials && config.broker.is_none() {
            return Err(SandboxPlanError::CredentialedRunWithoutBroker);
        }
        let network_mode = match phase {
            SandboxProcessPhase::Install if install_needs_network(plan) => "bridge",
            SandboxProcessPhase::Install => "none",
            SandboxProcessPhase::Run
                if has_credentials || !plan.network.runtime_hosts.is_empty() =>
            {
                "bridge"
            }
            SandboxProcessPhase::Run => "none",
        };
        let brokered_run = phase == SandboxProcessPhase::Run && has_credentials;
        Ok(Self {
            command,
            network_mode,
            lockdown: brokered_run,
            needs_net_admin: brokered_run,
            include_broker_ca: brokered_run,
            tools_readonly: phase == SandboxProcessPhase::Run && !plan.mounts.tools.writable,
            cache_readonly: phase == SandboxProcessPhase::Run && !plan.mounts.cache.writable,
        })
    }

    fn network_args(&self) -> Vec<String> {
        let mut args = vec!["--network".to_string(), self.network_mode.to_string()];
        if self.lockdown {
            args.extend([
                "--env".to_string(),
                "IRONCLAW_EGRESS_LOCKDOWN=broker-only".to_string(),
            ]);
        }
        args
    }

    fn mount_args(
        &self,
        config: &DockerProcessSandboxConfig,
        plan: &ValidatedSandboxProcessPlan,
    ) -> Vec<String> {
        let mut args = Vec::new();
        args.extend(bind_mount_arg(
            &config.workspace_host_path,
            &plan.mounts.workspace.container_path,
            !plan.mounts.workspace.writable,
        ));
        args.extend(bind_mount_arg(
            &config.tools_host_path,
            &plan.mounts.tools.container_path,
            self.tools_readonly,
        ));
        args.extend(bind_mount_arg(
            &config.cache_host_path,
            &plan.mounts.cache.container_path,
            self.cache_readonly,
        ));
        if self.include_broker_ca
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

    fn env_args(
        &self,
        config: &DockerProcessSandboxConfig,
        plan: &ValidatedSandboxProcessPlan,
    ) -> Result<Vec<String>, SandboxPlanError> {
        let mut env = self.command.env.clone();
        if self.include_broker_ca
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
}

fn install_needs_network(plan: &ValidatedSandboxProcessPlan) -> bool {
    plan.install
        .as_ref()
        .is_some_and(|install| !install.allowed_hosts.is_empty())
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
