use std::collections::{HashMap, HashSet};

use ironclaw_host_api::{RuntimeCredentialTarget, SecretHandle};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    DEFAULT_CACHE_MOUNT, DEFAULT_TOOLS_MOUNT, DEFAULT_WORKSPACE_MOUNT,
    validation::{
        is_container_absolute_path, validate_env_has_no_raw_sensitive_values, validate_env_name,
        validate_header_name, validate_host,
    },
};

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
    pub fn validate(&self) -> Result<(), SandboxPlanError> {
        validate_plan(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedSandboxProcessPlan {
    plan: SandboxProcessPlan,
}

impl ValidatedSandboxProcessPlan {
    pub fn new(plan: SandboxProcessPlan) -> Result<Self, SandboxPlanError> {
        validate_plan(&plan)?;
        Ok(Self { plan })
    }

    pub fn as_plan(&self) -> &SandboxProcessPlan {
        &self.plan
    }

    pub fn into_plan(self) -> SandboxProcessPlan {
        self.plan
    }
}

impl TryFrom<SandboxProcessPlan> for ValidatedSandboxProcessPlan {
    type Error = SandboxPlanError;

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
    pub(crate) fn validate(&self, phase: &'static str) -> Result<(), SandboxPlanError> {
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
    pub(crate) fn validate(&self) -> Result<(), SandboxPlanError> {
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

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SandboxPlanError {
    #[error("{phase} command must not be empty")]
    EmptyCommand { phase: &'static str },
    #[error("{phase} command must be a single executable name or path")]
    UnsafeCommand { phase: &'static str },
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

fn validate_plan(plan: &SandboxProcessPlan) -> Result<(), SandboxPlanError> {
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
            return Err(SandboxPlanError::CredentialHostNotAllowed {
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
) -> Result<(), SandboxPlanError> {
    let mut seen = HashSet::new();
    for binding in bindings {
        if !seen.insert((
            binding.approved_host.to_ascii_lowercase(),
            binding.header_name(),
        )) {
            return Err(SandboxPlanError::DuplicateCredentialTarget {
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
) -> Result<(), SandboxPlanError> {
    let Some(env_name) = &binding.placeholder_env else {
        return Ok(());
    };
    match plan.run.env.get(env_name) {
        Some(value) if value == &binding.placeholder_value => Ok(()),
        Some(_) => Err(SandboxPlanError::InvalidPlaceholderEnv {
            env: env_name.clone(),
        }),
        None => Err(SandboxPlanError::MissingPlaceholderEnv {
            env: env_name.clone(),
        }),
    }
}

fn validate_credentialed_run_policy(plan: &SandboxProcessPlan) -> Result<(), SandboxPlanError> {
    if !plan.network.direct_egress_lockdown {
        return Err(SandboxPlanError::CredentialedRunWithoutLockdown);
    }
    if plan.network.runtime_hosts.is_empty() {
        return Err(SandboxPlanError::CredentialedRunWithoutRuntimeNetwork);
    }
    if plan.mounts.tools.writable || plan.mounts.cache.writable {
        return Err(SandboxPlanError::WritableStateDuringCredentialedRun);
    }
    Ok(())
}
