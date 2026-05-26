use std::collections::{HashMap, HashSet};

use ironclaw_host_api::{RuntimeCredentialTarget, SecretHandle};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    DEFAULT_CACHE_MOUNT, DEFAULT_TOOLS_MOUNT, DEFAULT_WORKSPACE_MOUNT, MAX_OUTPUT_LIMIT,
    MAX_TIMEOUT_MS,
    validation::{
        is_container_absolute_path, validate_env_has_no_raw_sensitive_values, validate_env_name,
        validate_header_name, validate_host,
    },
};

/// Serialized process sandbox request accepted from the host runtime.
///
/// The plan describes logical command, mount, network, and credential intent.
/// It never carries Docker flags, host paths, inherited host environment, or
/// raw secret values; callers must validate it before handing it to a backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxProcessPlan {
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
    /// Validates the plan before backend execution.
    pub fn validate(&self) -> Result<(), ProcessSandboxPlanError> {
        validate_plan(self)
    }
}

/// A sandbox process plan that has passed local validation.
///
/// Backends accept this wrapper so validation-sensitive decisions, such as
/// mount construction and credential broker setup, cannot accidentally consume
/// untrusted raw plan JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedSandboxProcessPlan {
    plan: SandboxProcessPlan,
}

impl ValidatedSandboxProcessPlan {
    /// Validates and wraps a raw sandbox process plan.
    pub fn new(plan: SandboxProcessPlan) -> Result<Self, ProcessSandboxPlanError> {
        validate_plan(&plan)?;
        Ok(Self { plan })
    }

    /// Returns the validated plan without dropping the validation marker.
    pub fn as_plan(&self) -> &SandboxProcessPlan {
        &self.plan
    }

    /// Consumes the marker and returns the underlying plan.
    pub fn into_plan(self) -> SandboxProcessPlan {
        self.plan
    }
}

impl TryFrom<SandboxProcessPlan> for ValidatedSandboxProcessPlan {
    type Error = ProcessSandboxPlanError;

    fn try_from(plan: SandboxProcessPlan) -> Result<Self, Self::Error> {
        Self::new(plan)
    }
}

impl std::ops::Deref for ValidatedSandboxProcessPlan {
    type Target = SandboxProcessPlan;

    fn deref(&self) -> &Self::Target {
        self.as_plan()
    }
}

/// Optional setup phase run before the main sandbox command.
///
/// `allowed_hosts` records requested install-time network authority. The Docker
/// MVP currently fails closed when this list is non-empty because install egress
/// filtering is not yet enforced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxInstallPlan {
    pub command: SandboxCommandPlan,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

/// A single command phase inside the sandbox container.
///
/// Commands are serialized as an executable plus argument vector, not a shell
/// line. Validation rejects empty, shell-word, and option-like executables,
/// unsafe environment names, relative working directories, and resource limits
/// above the host caps.
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
    pub(crate) fn validate(&self, phase: &'static str) -> Result<(), ProcessSandboxPlanError> {
        if self.command.trim().is_empty() {
            return Err(ProcessSandboxPlanError::EmptyCommand { phase });
        }
        if self.command.starts_with('-') || self.command.chars().any(char::is_whitespace) {
            return Err(ProcessSandboxPlanError::UnsafeCommand { phase });
        }
        if let Some(working_dir) = &self.working_dir
            && !is_container_absolute_path(working_dir)
        {
            return Err(ProcessSandboxPlanError::InvalidContainerPath {
                path: working_dir.clone(),
            });
        }
        if self
            .timeout_ms
            .is_some_and(|timeout_ms| timeout_ms > MAX_TIMEOUT_MS)
        {
            return Err(ProcessSandboxPlanError::TimeoutLimitTooLarge {
                phase,
                max: MAX_TIMEOUT_MS,
            });
        }
        if self
            .max_stdout_bytes
            .is_some_and(|limit| limit > MAX_OUTPUT_LIMIT)
        {
            return Err(ProcessSandboxPlanError::OutputLimitTooLarge {
                phase,
                stream: "stdout",
                max: MAX_OUTPUT_LIMIT,
            });
        }
        if self
            .max_stderr_bytes
            .is_some_and(|limit| limit > MAX_OUTPUT_LIMIT)
        {
            return Err(ProcessSandboxPlanError::OutputLimitTooLarge {
                phase,
                stream: "stderr",
                max: MAX_OUTPUT_LIMIT,
            });
        }
        for (name, value) in &self.env {
            validate_env_name(name)?;
            if value.contains('\0') {
                return Err(ProcessSandboxPlanError::InvalidEnvValue { env: name.clone() });
            }
        }
        Ok(())
    }
}

/// Container mount points exposed to the sandbox.
///
/// Host paths are supplied only by trusted executor configuration. Plan data
/// controls the container destinations and whether the workspace is writable;
/// credentialed runs require tool and cache state to remain read-only.
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
    fn validate(&self) -> Result<(), ProcessSandboxPlanError> {
        self.workspace.validate()?;
        self.tools.validate()?;
        self.cache.validate()?;
        if self.workspace.container_path == self.tools.container_path
            || self.workspace.container_path == self.cache.container_path
            || self.tools.container_path == self.cache.container_path
        {
            return Err(ProcessSandboxPlanError::DuplicateMountPath);
        }
        Ok(())
    }
}

/// A single logical sandbox mount destination.
///
/// The path is a container path only. Validation rejects system directories,
/// traversal, NUL bytes, and Docker mount-spec metacharacters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxMount {
    pub container_path: String,
    #[serde(default)]
    pub writable: bool,
}

impl SandboxMount {
    fn validate(&self) -> Result<(), ProcessSandboxPlanError> {
        if !is_container_absolute_path(&self.container_path) {
            return Err(ProcessSandboxPlanError::InvalidContainerPath {
                path: self.container_path.clone(),
            });
        }
        if is_blocked_container_mount_path(&self.container_path) {
            return Err(ProcessSandboxPlanError::InvalidContainerPath {
                path: self.container_path.clone(),
            });
        }
        Ok(())
    }
}

fn is_blocked_container_mount_path(path: &str) -> bool {
    const BLOCKED_PREFIXES: &[&str] = &[
        "/bin", "/boot", "/dev", "/etc", "/lib", "/lib64", "/proc", "/run", "/sbin", "/sys",
        "/usr", "/var",
    ];
    path == "/"
        || BLOCKED_PREFIXES
            .iter()
            .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

/// Runtime network authority requested for the sandbox command.
///
/// The Docker MVP allows networked execution only for credentialed brokered
/// runs with direct egress lockdown. Non-empty runtime hosts without a broker
/// fail closed until direct host filtering exists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxNetworkPlan {
    #[serde(default)]
    pub runtime_hosts: Vec<String>,
    #[serde(default)]
    pub direct_egress_lockdown: bool,
}

impl SandboxNetworkPlan {
    fn validate(&self) -> Result<(), ProcessSandboxPlanError> {
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

/// Approved credential placeholder rewrite for brokered runtime egress.
///
/// The plan carries a secret handle and placeholder metadata, never raw secret
/// material. The broker rewrites matching placeholder headers only for the
/// approved host and validated target.
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
    pub(crate) fn validate(&self) -> Result<(), ProcessSandboxPlanError> {
        validate_host(&self.approved_host)?;
        if self.placeholder_value.trim().is_empty()
            || self.placeholder_value.contains(char::is_whitespace)
        {
            return Err(ProcessSandboxPlanError::InvalidCredentialPlaceholder);
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
                    return Err(ProcessSandboxPlanError::InvalidCredentialTarget);
                }
            }
            RuntimeCredentialTarget::QueryParam { .. } => {
                return Err(ProcessSandboxPlanError::UnsupportedCredentialTarget);
            }
        }
        Ok(())
    }

    pub(crate) fn header_name(&self) -> String {
        match &self.target {
            RuntimeCredentialTarget::Header { name, .. } => name.to_ascii_lowercase(),
            RuntimeCredentialTarget::QueryParam { name } => name.to_ascii_lowercase(),
        }
    }
}

fn default_required() -> bool {
    true
}

/// Validation errors for raw process sandbox plans.
///
/// These errors are intentionally specific so callers and tests can distinguish
/// policy failures from malformed JSON before a backend is invoked.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProcessSandboxPlanError {
    #[error("{phase} command must not be empty")]
    EmptyCommand { phase: &'static str },
    #[error("{phase} command must be a single executable name or path")]
    UnsafeCommand { phase: &'static str },
    #[error("invalid host {host}: {reason}")]
    InvalidHost { host: String, reason: String },
    #[error("invalid container path {path}")]
    InvalidContainerPath { path: String },
    #[error("invalid host path {path}")]
    InvalidHostPath { path: String },
    #[error("mount container paths must be unique")]
    DuplicateMountPath,
    #[error("{phase} timeout exceeds maximum {max}ms")]
    TimeoutLimitTooLarge { phase: &'static str, max: u64 },
    #[error("{phase} {stream} capture limit exceeds maximum {max} bytes")]
    OutputLimitTooLarge {
        phase: &'static str,
        stream: &'static str,
        max: u64,
    },
    #[error("{phase} network hosts cannot be enforced by Docker process sandbox MVP")]
    UnenforcedNetworkHosts { phase: &'static str },
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

fn validate_plan(plan: &SandboxProcessPlan) -> Result<(), ProcessSandboxPlanError> {
    if let Some(install) = &plan.install {
        install.command.validate("install")?;
        for host in &install.allowed_hosts {
            validate_host(host)?;
        }
        validate_env_has_no_raw_sensitive_values(&install.command.env, &[])?;
    }
    plan.run.validate("run")?;
    plan.mounts.validate()?;
    plan.network.validate()?;

    let placeholders = plan
        .credentials
        .iter()
        .map(|binding| binding.placeholder_value.as_str())
        .collect::<Vec<_>>();
    validate_env_has_no_raw_sensitive_values(&plan.run.env, &placeholders)?;

    let runtime_hosts = plan.network.runtime_allowed_hosts();
    for binding in &plan.credentials {
        binding.validate()?;
        let approved_host = binding.approved_host.to_ascii_lowercase();
        if !runtime_hosts.contains(&approved_host) {
            return Err(ProcessSandboxPlanError::CredentialHostNotAllowed {
                host: binding.approved_host.clone(),
            });
        }
        validate_placeholder_env(plan, binding)?;
    }
    validate_unique_credential_targets(&plan.credentials)?;

    if !plan.credentials.is_empty() {
        validate_credentialed_run_policy(plan)?;
    }

    Ok(())
}

pub(crate) fn validate_unique_credential_targets(
    bindings: &[SandboxCredentialBinding],
) -> Result<(), ProcessSandboxPlanError> {
    let mut seen = HashSet::new();
    for binding in bindings {
        if !seen.insert((
            binding.approved_host.to_ascii_lowercase(),
            binding.header_name(),
        )) {
            return Err(ProcessSandboxPlanError::DuplicateCredentialTarget {
                host: binding.approved_host.clone(),
                header: binding.header_name(),
            });
        }
    }
    Ok(())
}

fn validate_placeholder_env(
    plan: &SandboxProcessPlan,
    binding: &SandboxCredentialBinding,
) -> Result<(), ProcessSandboxPlanError> {
    let Some(env_name) = &binding.placeholder_env else {
        return Ok(());
    };
    match plan.run.env.get(env_name) {
        Some(value) if value == &binding.placeholder_value => Ok(()),
        Some(_) => Err(ProcessSandboxPlanError::InvalidPlaceholderEnv {
            env: env_name.clone(),
        }),
        None => Err(ProcessSandboxPlanError::MissingPlaceholderEnv {
            env: env_name.clone(),
        }),
    }
}

fn validate_credentialed_run_policy(
    plan: &SandboxProcessPlan,
) -> Result<(), ProcessSandboxPlanError> {
    if !plan.network.direct_egress_lockdown {
        return Err(ProcessSandboxPlanError::CredentialedRunWithoutLockdown);
    }
    if plan.network.runtime_hosts.is_empty() {
        return Err(ProcessSandboxPlanError::CredentialedRunWithoutRuntimeNetwork);
    }
    if plan.mounts.tools.writable || plan.mounts.cache.writable {
        return Err(ProcessSandboxPlanError::WritableStateDuringCredentialedRun);
    }
    Ok(())
}
