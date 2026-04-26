//! Script runner contracts for IronClaw Reborn.
//!
//! `ironclaw_scripts` executes declared script/CLI capabilities through a
//! host-selected backend. Extension manifests describe the command metadata, but
//! extensions do not receive raw Docker flags, host paths, ambient environment,
//! secrets, or network by default.

use std::{
    io::Write,
    process::{Command, Stdio},
    time::Instant,
};

use ironclaw_extensions::{ExtensionPackage, ExtensionRuntime};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, ResourceEstimate, ResourceReservationId, ResourceScope,
    ResourceUsage, RuntimeKind,
};
use ironclaw_resources::{ResourceError, ResourceGovernor, ResourceReceipt};
use serde_json::Value;
use thiserror::Error;

/// Script runner limits owned by the host runtime, not by extension manifests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRuntimeConfig {
    pub max_stdout_bytes: u64,
    pub max_stderr_bytes: u64,
}

impl Default for ScriptRuntimeConfig {
    fn default() -> Self {
        Self {
            max_stdout_bytes: 1024 * 1024,
            max_stderr_bytes: 64 * 1024,
        }
    }
}

impl ScriptRuntimeConfig {
    pub fn for_testing() -> Self {
        Self {
            max_stdout_bytes: 64 * 1024,
            max_stderr_bytes: 16 * 1024,
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
            .as_deref()
            .ok_or_else(|| "DockerScriptBackend requires an image".to_string())?;

        let started = Instant::now();
        let mut child = Command::new("docker")
            .arg("run")
            .arg("--rm")
            .arg("-i")
            .arg("--network")
            .arg("none")
            .arg(image)
            .arg(&request.command)
            .args(&request.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| error.to_string())?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(request.stdin_json.as_bytes())
                .map_err(|error| error.to_string())?;
        }

        let output = child
            .wait_with_output()
            .map_err(|error| error.to_string())?;
        Ok(ScriptBackendOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: output.stderr,
            wall_clock_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        })
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
        let reservation = governor.reserve(request.scope, request.estimate)?;

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

fn release_after_failure<G>(
    governor: &G,
    reservation_id: ResourceReservationId,
    original: ScriptError,
) -> ScriptError
where
    G: ResourceGovernor + ?Sized,
{
    match governor.release(reservation_id) {
        Ok(_) => original,
        Err(error) => ScriptError::Resource(Box::new(error)),
    }
}

fn bounded_lossy(bytes: &[u8], limit: u64) -> String {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let end = bytes.len().min(limit);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}
