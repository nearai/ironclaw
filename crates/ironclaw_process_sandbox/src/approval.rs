use serde::{Deserialize, Serialize};

use crate::{
    SandboxCommandPlan, SandboxCredentialBinding, SandboxMount, SandboxPlanError,
    SandboxProcessPlan,
};
use ironclaw_host_api::{RuntimeCredentialTarget, SecretHandle};

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
