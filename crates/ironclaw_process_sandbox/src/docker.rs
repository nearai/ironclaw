use std::{
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    time,
};

use crate::{
    DEFAULT_PROCESS_SANDBOX_IMAGE, DEFAULT_STDERR_LIMIT, DEFAULT_STDOUT_LIMIT, DEFAULT_TIMEOUT_MS,
    ProcessSandboxBackend, ProcessSandboxError, ProcessSandboxErrorKind, ProcessSandboxPlanError,
    SandboxCommandPlan, SandboxPhaseOutput, SandboxProcessOutput, SandboxProcessRequest,
    SandboxProcessResult, ValidatedSandboxProcessPlan,
    validation::{validate_env_has_no_raw_sensitive_values, validate_env_name},
};
use ironclaw_processes::ProcessCancellationToken;

static CONTAINER_SEQUENCE: AtomicU64 = AtomicU64::new(1);

const DOCKER_MEMORY_LIMIT: &str = "512m";
const DOCKER_PIDS_LIMIT: &str = "256";
const DOCKER_CPU_LIMIT: &str = "2";

/// Trusted Docker backend configuration for the process sandbox.
///
/// Host paths and the image name come from host configuration, not from the
/// runtime-supplied process plan. The backend translates validated logical
/// plans into a restricted `docker run` invocation.
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
    /// Builds a Docker config with the default binary and sandbox image.
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

/// Host-side broker configuration used by credentialed sandbox runs.
///
/// The proxy URL and CA certificate mount are injected by trusted composition
/// so container traffic can be pinned to the broker and sanitized before it
/// leaves the sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerBrokerConfig {
    pub proxy_url: String,
    pub ca_cert_host_path: PathBuf,
    pub ca_cert_container_path: String,
}

/// Sandbox execution phase represented in Docker invocations and output.
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
    pub container_name: String,
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
                abort_docker_child(&invocation.docker_bin, &invocation.container_name, &mut child).await;
                return Err(DockerRunError::Cancelled);
            }
            _ = time::sleep(timeout) => {
                abort_docker_child(&invocation.docker_bin, &invocation.container_name, &mut child).await;
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

async fn abort_docker_child(docker_bin: &str, container_name: &str, child: &mut Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
    cleanup_container(docker_bin, container_name).await;
}

async fn cleanup_container(docker_bin: &str, container_name: &str) {
    let _ = Command::new(docker_bin)
        .args(["rm", "-f", container_name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
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

/// Docker-backed implementation of the process sandbox backend.
///
/// The backend enforces the Docker-specific security contract: host-owned
/// mount roots, configured image only, no host environment inheritance,
/// resource limits, dropped capabilities, no-new-privileges, and broker-only
/// egress for credentialed runtime phases.
#[derive(Clone)]
pub struct DockerProcessSandboxBackend {
    config: DockerProcessSandboxConfig,
    runner: Arc<dyn DockerRunner>,
}

impl DockerProcessSandboxBackend {
    /// Constructs a Docker sandbox backend using the system Docker runner.
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
            let exit_code = output.exit_code;
            phases.push(phase_output(SandboxProcessPhase::Install, output));
            if exit_code != 0 {
                return Ok(SandboxProcessResult {
                    output: SandboxProcessOutput { phases },
                });
            }
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
) -> Result<DockerInvocation, ProcessSandboxPlanError> {
    let spec = DockerPhaseSpec::new(config, plan, phase, command)?;
    let container_name = next_container_name(phase);
    let mut args = Vec::with_capacity(48 + command.args.len() + command.env.len());
    args.extend([
        "run".to_string(),
        "--name".to_string(),
        container_name.clone(),
        "--rm".to_string(),
        "--init".to_string(),
        "--memory".to_string(),
        DOCKER_MEMORY_LIMIT.to_string(),
        "--memory-swap".to_string(),
        DOCKER_MEMORY_LIMIT.to_string(),
        "--pids-limit".to_string(),
        DOCKER_PIDS_LIMIT.to_string(),
        "--cpus".to_string(),
        DOCKER_CPU_LIMIT.to_string(),
        "--security-opt".to_string(),
        "no-new-privileges".to_string(),
        "--cap-drop".to_string(),
        "ALL".to_string(),
        "--cap-add".to_string(),
        "SETPCAP".to_string(),
        "--cap-add".to_string(),
        "SETUID".to_string(),
        "--cap-add".to_string(),
        "SETGID".to_string(),
    ]);
    if spec.needs_net_admin() {
        args.push("--cap-add".to_string());
        args.push("NET_ADMIN".to_string());
    }

    args.extend(spec.network_args(config));
    args.extend(spec.mount_args(config, plan)?);
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
        container_name,
        args,
    })
}

fn next_container_name(phase: SandboxProcessPhase) -> String {
    let phase = match phase {
        SandboxProcessPhase::Install => "install",
        SandboxProcessPhase::Run => "run",
    };
    let sequence = CONTAINER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("ironclaw-sandbox-{phase}-{}-{sequence}", std::process::id())
}

struct DockerPhaseSpec<'a> {
    command: &'a SandboxCommandPlan,
    phase: SandboxProcessPhase,
    has_credentials: bool,
    network_mode: &'static str,
}

impl<'a> DockerPhaseSpec<'a> {
    fn new(
        config: &DockerProcessSandboxConfig,
        plan: &ValidatedSandboxProcessPlan,
        phase: SandboxProcessPhase,
        command: &'a SandboxCommandPlan,
    ) -> Result<Self, ProcessSandboxPlanError> {
        let has_credentials = !plan.credentials.is_empty();
        if phase == SandboxProcessPhase::Run && has_credentials && config.broker.is_none() {
            return Err(ProcessSandboxPlanError::CredentialedRunWithoutBroker);
        }
        if phase == SandboxProcessPhase::Install && install_needs_network(plan) {
            return Err(ProcessSandboxPlanError::UnenforcedNetworkHosts { phase: "install" });
        }
        if phase == SandboxProcessPhase::Run
            && !has_credentials
            && !plan.network.runtime_hosts.is_empty()
        {
            return Err(ProcessSandboxPlanError::UnenforcedNetworkHosts { phase: "run" });
        }
        let network_mode = match phase {
            SandboxProcessPhase::Install => "none",
            SandboxProcessPhase::Run if has_credentials => "bridge",
            SandboxProcessPhase::Run => "none",
        };
        Ok(Self {
            command,
            phase,
            has_credentials,
            network_mode,
        })
    }

    fn brokered_run(&self) -> bool {
        self.phase == SandboxProcessPhase::Run && self.has_credentials
    }

    fn needs_net_admin(&self) -> bool {
        self.brokered_run()
    }

    fn include_broker_ca(&self) -> bool {
        self.brokered_run()
    }

    fn tools_readonly(&self, plan: &ValidatedSandboxProcessPlan) -> bool {
        self.phase == SandboxProcessPhase::Install || !plan.mounts.tools.writable
    }

    fn cache_readonly(&self, plan: &ValidatedSandboxProcessPlan) -> bool {
        self.phase == SandboxProcessPhase::Install || !plan.mounts.cache.writable
    }

    fn network_args(&self, config: &DockerProcessSandboxConfig) -> Vec<String> {
        let mut args = vec!["--network".to_string(), self.network_mode.to_string()];
        if let Some(host) = self.broker_add_host(config) {
            args.extend(["--add-host".to_string(), format!("{host}:host-gateway")]);
        }
        if self.brokered_run() {
            args.extend([
                "--env".to_string(),
                "IRONCLAW_EGRESS_LOCKDOWN=broker-only".to_string(),
            ]);
        }
        args
    }

    fn broker_add_host(&self, config: &DockerProcessSandboxConfig) -> Option<String> {
        self.brokered_run().then_some(())?;
        config
            .broker
            .as_ref()
            .and_then(|broker| broker_host_for_add_host(&broker.proxy_url))
    }

    fn mount_args(
        &self,
        config: &DockerProcessSandboxConfig,
        plan: &ValidatedSandboxProcessPlan,
    ) -> Result<Vec<String>, ProcessSandboxPlanError> {
        let mut args = Vec::new();
        args.extend(bind_mount_arg(
            &config.workspace_host_path,
            &plan.mounts.workspace.container_path,
            !plan.mounts.workspace.writable,
        )?);
        args.extend(bind_mount_arg(
            &config.tools_host_path,
            &plan.mounts.tools.container_path,
            self.tools_readonly(plan),
        )?);
        args.extend(bind_mount_arg(
            &config.cache_host_path,
            &plan.mounts.cache.container_path,
            self.cache_readonly(plan),
        )?);
        if self.include_broker_ca()
            && let Some(broker) = &config.broker
        {
            args.extend(bind_mount_arg(
                &broker.ca_cert_host_path,
                &broker.ca_cert_container_path,
                true,
            )?);
        }
        Ok(args)
    }

    fn env_args(
        &self,
        config: &DockerProcessSandboxConfig,
        plan: &ValidatedSandboxProcessPlan,
    ) -> Result<Vec<String>, ProcessSandboxPlanError> {
        let mut env = self.command.env.clone();
        if self.include_broker_ca()
            && let Some(broker) = &config.broker
        {
            for name in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
                env.insert(name.to_string(), broker.proxy_url.clone());
            }
            for name in [
                "SSL_CERT_FILE",
                "REQUESTS_CA_BUNDLE",
                "NODE_EXTRA_CA_CERTS",
                "GIT_SSL_CAINFO",
                "CURL_CA_BUNDLE",
            ] {
                env.insert(name.to_string(), broker.ca_cert_container_path.clone());
            }
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
            if !is_broker_proxy_env_name(&name) {
                validate_env_name(&name)?;
            }
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

fn is_broker_proxy_env_name(name: &str) -> bool {
    matches!(
        name,
        "HTTP_PROXY" | "HTTPS_PROXY" | "http_proxy" | "https_proxy"
    )
}

fn bind_mount_arg(
    host_path: &Path,
    container_path: &str,
    readonly: bool,
) -> Result<Vec<String>, ProcessSandboxPlanError> {
    if container_path.contains(',') {
        return Err(ProcessSandboxPlanError::InvalidContainerPath {
            path: container_path.to_string(),
        });
    }
    let host_path = host_path.display().to_string();
    if host_path.contains(',') {
        return Err(ProcessSandboxPlanError::InvalidHostPath { path: host_path });
    }
    let mut spec = format!("type=bind,src={},dst={}", host_path, container_path);
    if readonly {
        spec.push_str(",readonly");
    }
    Ok(vec!["--mount".to_string(), spec])
}

fn broker_host_for_add_host(proxy_url: &str) -> Option<String> {
    let host = broker_host(proxy_url)?;
    if host.parse::<Ipv4Addr>().is_ok() {
        None
    } else {
        Some(host.to_string())
    }
}

pub(crate) fn broker_host(proxy_url: &str) -> Option<&str> {
    let (_, rest) = proxy_url.split_once("://")?;
    let host_port_path = rest.split('/').next().unwrap_or(rest);
    let host = host_port_path.split(':').next().unwrap_or(host_port_path);
    (!host.is_empty()).then_some(host)
}
