//! Script runner contracts for IronClaw Reborn.
//!
//! `ironclaw_scripts` executes declared script/CLI capabilities through a
//! host-selected backend. Extension manifests describe the command metadata, but
//! extensions do not receive raw Docker flags, host paths, ambient environment,
//! secrets, or network by default.

use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use ironclaw_extensions::{ExtensionPackage, ExtensionRuntime};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, MountView, ResourceEstimate, ResourceReservation,
    ResourceReservationId, ResourceScope, ResourceUsage, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeKind,
};
use ironclaw_resources::{ResourceError, ResourceGovernor, ResourceReceipt};
use serde_json::Value;
use thiserror::Error;

/// Script runner limits owned by the host runtime, not by extension manifests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRuntimeConfig {
    pub max_stdout_bytes: u64,
    pub max_stderr_bytes: u64,
    pub max_wall_clock_ms: u64,
}

impl Default for ScriptRuntimeConfig {
    fn default() -> Self {
        Self {
            max_stdout_bytes: 1024 * 1024,
            max_stderr_bytes: 64 * 1024,
            max_wall_clock_ms: 30_000,
        }
    }
}

impl ScriptRuntimeConfig {
    pub fn for_testing() -> Self {
        Self {
            max_stdout_bytes: 64 * 1024,
            max_stderr_bytes: 16 * 1024,
            max_wall_clock_ms: 5_000,
        }
    }
}

/// JSON invocation passed to a script capability.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptInvocation {
    pub input: Value,
}

/// Full resource-governed script execution request.
#[derive(Debug)]
pub struct ScriptExecutionRequest<'a> {
    pub package: &'a ExtensionPackage,
    pub capability_id: &'a CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub mounts: Option<MountView>,
    pub resource_reservation: Option<ResourceReservation>,
    pub invocation: ScriptInvocation,
}

/// Host-normalized request handed to the configured backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptBackendRequest {
    pub provider: ExtensionId,
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub runner: String,
    pub image: Option<String>,
    pub command: String,
    pub args: Vec<String>,
    pub stdin_json: String,
    pub max_stdout_bytes: u64,
    pub max_stderr_bytes: u64,
    pub max_wall_clock_ms: u64,
}

/// Raw backend output before the script runtime parses stdout as JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptBackendOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub wall_clock_ms: u64,
}

impl ScriptBackendOutput {
    pub fn json(value: Value) -> Self {
        Self {
            exit_code: 0,
            stdout: serde_json::to_vec(&value).unwrap_or_else(|_| b"null".to_vec()),
            stderr: Vec::new(),
            wall_clock_ms: 0,
        }
    }
}

/// Backend interface for sandboxed script execution.
pub trait ScriptBackend: Send + Sync {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String>;
}

/// Docker CLI backend for V1 script execution.
///
/// This backend intentionally accepts only normalized manifest-derived command
/// fields. It does not expose raw Docker flags to extensions, does not mount host
/// paths, does not pass host environment variables, and disables container
/// network access by default.
#[derive(Debug, Clone, Copy, Default)]
pub struct DockerScriptBackend;

impl ScriptBackend for DockerScriptBackend {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        if request.runner != "docker" {
            return Err(format!(
                "DockerScriptBackend cannot execute runner {}",
                request.runner
            ));
        }
        let image = request
            .image
            .clone()
            .ok_or_else(|| "DockerScriptBackend requires an image".to_string())?;
        validate_docker_image_reference(&image)?;

        execute_docker_request(request, &image)
    }
}

/// Parsed script capability result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptCapabilityResult {
    pub output: Value,
    pub reservation_id: ResourceReservationId,
    pub usage: ResourceUsage,
    pub output_bytes: u64,
}

/// Full resource-governed script execution result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptExecutionResult {
    pub result: ScriptCapabilityResult,
    pub receipt: ResourceReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptHostHttpRequest {
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
    pub method: ironclaw_host_api::NetworkMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub network_policy: ironclaw_host_api::NetworkPolicy,
    pub credential_injections: Vec<ironclaw_host_api::RuntimeCredentialInjection>,
    pub response_body_limit: Option<u64>,
    pub timeout_ms: Option<u32>,
}

pub type ScriptHostHttpResponse = RuntimeHttpEgressResponse;

#[derive(Debug, Error)]
pub enum ScriptHostHttpError {
    #[error("script host HTTP error: {reason}")]
    Egress { reason: String },
}

#[derive(Debug, Clone)]
pub struct ScriptRuntimeHttpAdapter<E> {
    egress: E,
}

impl<E> ScriptRuntimeHttpAdapter<E>
where
    E: RuntimeHttpEgress,
{
    pub fn new(egress: E) -> Self {
        Self { egress }
    }

    pub fn request(
        &self,
        request: ScriptHostHttpRequest,
    ) -> Result<ScriptHostHttpResponse, ScriptHostHttpError> {
        self.egress
            .execute(RuntimeHttpEgressRequest {
                runtime: RuntimeKind::Script,
                scope: request.scope,
                capability_id: request.capability_id,
                method: request.method,
                url: request.url,
                headers: request.headers,
                body: request.body,
                network_policy: request.network_policy,
                credential_injections: request.credential_injections,
                response_body_limit: request.response_body_limit,
                timeout_ms: request.timeout_ms,
            })
            .map_err(script_http_error)
    }
}

fn script_http_error(error: RuntimeHttpEgressError) -> ScriptHostHttpError {
    ScriptHostHttpError::Egress {
        reason: error.stable_runtime_reason().to_string(),
    }
}

/// Script runtime failures.
#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("resource governor error: {0}")]
    Resource(Box<ResourceError>),
    #[error("script backend error: {reason}")]
    Backend { reason: String },
    #[error("unsupported script runner {runner}")]
    UnsupportedRunner { runner: String },
    #[error("extension {extension} uses runtime {actual:?}, not RuntimeKind::Script")]
    ExtensionRuntimeMismatch {
        extension: ExtensionId,
        actual: RuntimeKind,
    },
    #[error("capability {capability} is not declared by this extension package")]
    CapabilityNotDeclared { capability: CapabilityId },
    #[error("script descriptor mismatch: {reason}")]
    DescriptorMismatch { reason: String },
    #[error("invalid script invocation: {reason}")]
    InvalidInvocation { reason: String },
    #[error("script exited with code {code}: {stderr}")]
    ExitFailure { code: i32, stderr: String },
    #[error("script output limit exceeded: limit {limit}, actual {actual}")]
    OutputLimitExceeded { limit: u64, actual: u64 },
    #[error("script timed out after {limit_ms} ms")]
    Timeout { limit_ms: u64 },
    #[error("script stdout is invalid JSON: {reason}")]
    InvalidOutput { reason: String },
}

impl From<ResourceError> for ScriptError {
    fn from(error: ResourceError) -> Self {
        Self::Resource(Box::new(error))
    }
}

/// Runtime for executing manifest-declared script capabilities.
#[derive(Debug, Clone)]
pub struct ScriptRuntime<B> {
    config: ScriptRuntimeConfig,
    backend: B,
}

impl<B> ScriptRuntime<B>
where
    B: ScriptBackend,
{
    pub fn new(config: ScriptRuntimeConfig, backend: B) -> Self {
        Self { config, backend }
    }

    pub fn config(&self) -> &ScriptRuntimeConfig {
        &self.config
    }

    pub fn execute_extension_json<G>(
        &self,
        governor: &G,
        request: ScriptExecutionRequest<'_>,
    ) -> Result<ScriptExecutionResult, ScriptError>
    where
        G: ResourceGovernor + ?Sized,
    {
        let backend_request = self.prepare_backend_request(&request)?;
        let reservation = reserve_or_use_existing(
            governor,
            request.scope.clone(),
            request.estimate.clone(),
            request.resource_reservation.clone(),
        )?;

        let output = match self.backend.execute(backend_request) {
            Ok(output) => output,
            Err(reason) => {
                return Err(release_after_failure(
                    governor,
                    reservation.id,
                    ScriptError::Backend { reason },
                ));
            }
        };

        if output.stdout.len() as u64 > self.config.max_stdout_bytes {
            return Err(release_after_failure(
                governor,
                reservation.id,
                ScriptError::OutputLimitExceeded {
                    limit: self.config.max_stdout_bytes,
                    actual: output.stdout.len() as u64,
                },
            ));
        }

        if output.exit_code != 0 {
            return Err(release_after_failure(
                governor,
                reservation.id,
                ScriptError::ExitFailure {
                    code: output.exit_code,
                    stderr: bounded_lossy(&output.stderr, self.config.max_stderr_bytes),
                },
            ));
        }

        let parsed = match serde_json::from_slice::<Value>(&output.stdout) {
            Ok(parsed) => parsed,
            Err(error) => {
                return Err(release_after_failure(
                    governor,
                    reservation.id,
                    ScriptError::InvalidOutput {
                        reason: error.to_string(),
                    },
                ));
            }
        };

        let output_bytes = output.stdout.len() as u64;
        let usage = ResourceUsage {
            wall_clock_ms: output.wall_clock_ms,
            output_bytes,
            process_count: 1,
            ..ResourceUsage::default()
        };
        let receipt = governor.reconcile(reservation.id, usage.clone())?;
        Ok(ScriptExecutionResult {
            result: ScriptCapabilityResult {
                output: parsed,
                reservation_id: reservation.id,
                usage,
                output_bytes,
            },
            receipt,
        })
    }

    fn prepare_backend_request(
        &self,
        request: &ScriptExecutionRequest<'_>,
    ) -> Result<ScriptBackendRequest, ScriptError> {
        let descriptor = request
            .package
            .capabilities
            .iter()
            .find(|descriptor| &descriptor.id == request.capability_id)
            .cloned()
            .ok_or_else(|| ScriptError::CapabilityNotDeclared {
                capability: request.capability_id.clone(),
            })?;

        if descriptor.runtime != RuntimeKind::Script {
            return Err(ScriptError::ExtensionRuntimeMismatch {
                extension: request.package.id.clone(),
                actual: descriptor.runtime,
            });
        }
        if descriptor.provider != request.package.id {
            return Err(ScriptError::DescriptorMismatch {
                reason: format!(
                    "descriptor {} provider {} does not match package {}",
                    descriptor.id, descriptor.provider, request.package.id
                ),
            });
        }

        let (runner, image, command, args) = match &request.package.manifest.runtime {
            ExtensionRuntime::Script {
                runner,
                image,
                command,
                args,
            } => (runner, image, command, args),
            other => {
                return Err(ScriptError::ExtensionRuntimeMismatch {
                    extension: request.package.id.clone(),
                    actual: other.kind(),
                });
            }
        };
        if runner == "docker" && image.is_none() {
            return Err(ScriptError::UnsupportedRunner {
                runner: runner.clone(),
            });
        }

        let stdin_json = serde_json::to_string(&request.invocation.input).map_err(|error| {
            ScriptError::InvalidInvocation {
                reason: error.to_string(),
            }
        })?;

        Ok(ScriptBackendRequest {
            provider: request.package.id.clone(),
            capability_id: request.capability_id.clone(),
            scope: request.scope.clone(),
            runner: runner.clone(),
            image: image.clone(),
            command: command.clone(),
            args: args.clone(),
            stdin_json,
            max_stdout_bytes: self.config.max_stdout_bytes,
            max_stderr_bytes: self.config.max_stderr_bytes,
            max_wall_clock_ms: self.config.max_wall_clock_ms,
        })
    }
}

/// Object-safe script executor interface used by the kernel composition layer.
pub trait ScriptExecutor: Send + Sync {
    fn execute_extension_json(
        &self,
        governor: &dyn ResourceGovernor,
        request: ScriptExecutionRequest<'_>,
    ) -> Result<ScriptExecutionResult, ScriptError>;
}

impl<B> ScriptExecutor for ScriptRuntime<B>
where
    B: ScriptBackend,
{
    fn execute_extension_json(
        &self,
        governor: &dyn ResourceGovernor,
        request: ScriptExecutionRequest<'_>,
    ) -> Result<ScriptExecutionResult, ScriptError> {
        ScriptRuntime::execute_extension_json(self, governor, request)
    }
}

fn validate_docker_image_reference(image: &str) -> Result<(), String> {
    if image.is_empty() {
        return Err("Docker image reference must not be empty".to_string());
    }
    if image.starts_with('-') {
        return Err("Docker image reference must not start with '-'".to_string());
    }
    if image.chars().any(char::is_whitespace) {
        return Err("Docker image reference must not contain whitespace".to_string());
    }
    Ok(())
}

fn execute_docker_request(
    request: ScriptBackendRequest,
    image: &str,
) -> Result<ScriptBackendOutput, String> {
    let started = Instant::now();
    let mut command = Command::new("docker");
    command.args(docker_run_args(&request, image));
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Docker child stdout was not captured".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Docker child stderr was not captured".to_string())?;
    let stdout_limit = request.max_stdout_bytes;
    let stderr_limit = request.max_stderr_bytes;
    let stdout_reader = thread::spawn(move || read_bounded(stdout, stdout_limit));
    let stderr_reader = thread::spawn(move || read_bounded(stderr, stderr_limit));

    let stdin_json = request.stdin_json.clone();
    let mut stdin_writer = child.stdin.take().map(|mut stdin| {
        thread::spawn(move || {
            stdin
                .write_all(stdin_json.as_bytes())
                .map_err(|error| error.to_string())
        })
    });

    let timeout = Duration::from_millis(request.max_wall_clock_ms);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(stdin_writer) = stdin_writer.take() {
                let _ = stdin_writer.join();
            }
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(format!(
                "script timed out after {} ms",
                request.max_wall_clock_ms
            ));
        }
        thread::sleep(Duration::from_millis(10));
    };

    if let Some(stdin_writer) = stdin_writer.take() {
        stdin_writer
            .join()
            .map_err(|_| "stdin writer panicked".to_string())??;
    }

    let stdout = stdout_reader
        .join()
        .map_err(|_| "stdout reader panicked".to_string())??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "stderr reader panicked".to_string())??;

    Ok(ScriptBackendOutput {
        exit_code: status.code().unwrap_or(-1),
        stdout,
        stderr,
        wall_clock_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    })
}

fn docker_run_args(request: &ScriptBackendRequest, image: &str) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "-i".to_string(),
        "--network".to_string(),
        "none".to_string(),
        image.to_string(),
        request.command.clone(),
    ];
    args.extend(request.args.clone());
    args
}

fn read_bounded<R>(mut reader: R, limit: u64) -> Result<Vec<u8>, String>
where
    R: Read,
{
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(read) > limit {
            return Err(format!("output exceeded {limit} bytes"));
        }
        output.extend_from_slice(&buffer[..read]);
    }
}

fn reserve_or_use_existing<G>(
    governor: &G,
    scope: ResourceScope,
    estimate: ResourceEstimate,
    reservation: Option<ResourceReservation>,
) -> Result<ResourceReservation, ScriptError>
where
    G: ResourceGovernor + ?Sized,
{
    if let Some(reservation) = reservation {
        if reservation.scope != scope || reservation.estimate != estimate {
            return Err(ScriptError::Resource(Box::new(
                ResourceError::ReservationMismatch { id: reservation.id },
            )));
        }
        return Ok(reservation);
    }
    governor.reserve(scope, estimate).map_err(ScriptError::from)
}

fn release_after_failure<G>(
    governor: &G,
    reservation_id: ResourceReservationId,
    original: ScriptError,
) -> ScriptError
where
    G: ResourceGovernor + ?Sized,
{
    let _ = governor.release(reservation_id);
    original
}

fn bounded_lossy(bytes: &[u8], limit: u64) -> String {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let end = bytes.len().min(limit);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, fs, io::Cursor, sync::Mutex};

    use super::{
        DockerScriptBackend, ScriptBackend, ScriptBackendRequest, docker_run_args, read_bounded,
        validate_docker_image_reference,
    };
    use ironclaw_host_api::{CapabilityId, InvocationId, ResourceScope, TenantId, UserId};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn read_bounded_rejects_output_larger_than_limit() {
        let err = read_bounded(Cursor::new(b"abcdef"), 4).unwrap_err();
        assert!(err.contains("output exceeded 4 bytes"));
    }

    #[test]
    fn read_bounded_allows_output_at_limit() {
        let output = read_bounded(Cursor::new(b"abcd"), 4).unwrap();
        assert_eq!(output, b"abcd");
    }

    #[test]
    fn docker_image_reference_rejects_cli_flag_injection() {
        let err = validate_docker_image_reference("--network=host").unwrap_err();
        assert!(err.contains("must not start with '-'"));
    }

    #[test]
    fn docker_image_reference_rejects_whitespace() {
        let err = validate_docker_image_reference("alpine --privileged").unwrap_err();
        assert!(err.contains("must not contain whitespace"));
    }

    #[test]
    #[cfg(unix)]
    fn docker_backend_disables_ambient_network_before_image_and_manifest_args() {
        let fake_bin = tempfile::tempdir().unwrap();
        let args_file = fake_bin.path().join("docker-args.txt");
        let docker_path = fake_bin.path().join("docker");
        fs::write(
            &docker_path,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$IRONCLAW_DOCKER_ARGS_FILE\"\ncat >/dev/null\nprintf '{\"ok\":true}'\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&docker_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&docker_path, permissions).unwrap();

        let _lock = ENV_LOCK.lock().unwrap();
        let old_path = std::env::var_os("PATH");
        let old_args_file = std::env::var_os("IRONCLAW_DOCKER_ARGS_FILE");
        let _restore = EnvRestore(vec![
            ("PATH", old_path.clone()),
            ("IRONCLAW_DOCKER_ARGS_FILE", old_args_file),
        ]);
        let mut paths = vec![fake_bin.path().to_path_buf()];
        if let Some(old_path) = old_path.as_ref() {
            paths.extend(std::env::split_paths(old_path));
        }
        let path = std::env::join_paths(paths).unwrap();
        unsafe {
            std::env::set_var("PATH", path);
            std::env::set_var("IRONCLAW_DOCKER_ARGS_FILE", &args_file);
        }

        let output = DockerScriptBackend
            .execute(sample_docker_request())
            .unwrap();

        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout, br#"{"ok":true}"#);
        let recorded = fs::read_to_string(args_file).unwrap();
        let args = recorded.lines().collect::<Vec<_>>();
        assert_eq!(
            &args[..6],
            &["run", "--rm", "-i", "--network", "none", "alpine:latest"]
        );
        assert_eq!(args[6], "script-echo");
        assert_eq!(args[7], "--json");
        assert_eq!(args[8], "--network=host");
        let image_index = args
            .iter()
            .position(|arg| *arg == "alpine:latest")
            .expect("image must be present");
        let network_index = args
            .iter()
            .position(|arg| *arg == "--network")
            .expect("network flag must be present");
        let command_index = args
            .iter()
            .position(|arg| *arg == "script-echo")
            .expect("command must be present");
        let manifest_arg_index = args
            .iter()
            .position(|arg| *arg == "--network=host")
            .expect("manifest arg must be preserved after the command");
        assert!(network_index < image_index);
        assert!(image_index < command_index);
        assert!(command_index < manifest_arg_index);
    }

    #[test]
    fn docker_run_args_disable_ambient_network_before_image_and_manifest_args() {
        let request = sample_docker_request();

        let args = docker_run_args(&request, "alpine:latest");

        assert_eq!(
            &args[..6],
            &["run", "--rm", "-i", "--network", "none", "alpine:latest"]
        );
        assert_eq!(args[6], "script-echo");
        assert_eq!(args[7], "--json");
        assert_eq!(args[8], "--network=host");
        let image_index = args
            .iter()
            .position(|arg| arg == "alpine:latest")
            .expect("image must be present");
        let network_index = args
            .iter()
            .position(|arg| arg == "--network")
            .expect("network flag must be present");
        let command_index = args
            .iter()
            .position(|arg| arg == "script-echo")
            .expect("command must be present");
        let manifest_arg_index = args
            .iter()
            .position(|arg| arg == "--network=host")
            .expect("manifest arg must be preserved after the command");
        assert!(network_index < image_index);
        assert!(image_index < command_index);
        assert!(command_index < manifest_arg_index);
    }

    fn sample_docker_request() -> ScriptBackendRequest {
        ScriptBackendRequest {
            provider: ironclaw_host_api::ExtensionId::new("script").unwrap(),
            capability_id: CapabilityId::new("script.echo").unwrap(),
            scope: ResourceScope {
                tenant_id: TenantId::new("tenant1").unwrap(),
                user_id: UserId::new("user1").unwrap(),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            runner: "docker".to_string(),
            image: Some("alpine:latest".to_string()),
            command: "script-echo".to_string(),
            args: vec!["--json".to_string(), "--network=host".to_string()],
            stdin_json: "{}".to_string(),
            max_stdout_bytes: 1024,
            max_stderr_bytes: 1024,
            max_wall_clock_ms: 1_000,
        }
    }

    struct EnvRestore(Vec<(&'static str, Option<OsString>)>);

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            for (key, value) in &self.0 {
                unsafe {
                    if let Some(value) = value {
                        std::env::set_var(key, value);
                    } else {
                        std::env::remove_var(key);
                    }
                }
            }
        }
    }
}
