//! Stage-aware runtime capability descriptions.
//!
//! These capabilities describe what a runtime can honestly promise to the
//! rest of the product and to users. They are intentionally broader than the
//! low-level mechanics of a specific backend so later stages can evolve
//! without forcing Docker-shaped assumptions onto Kubernetes.

/// High-level maturity stage for a runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStage {
    /// Limited worker runtime with explicit unsupported project-backed flows.
    Stage1Runtime,
    /// Project-backed tasks work through orchestrator-delivered content.
    Stage2ProjectBacked,
    /// User-facing behavior is close to Docker for normal work.
    Stage3NearDocker,
    /// Full reference sandbox support.
    FullSandbox,
}

impl RuntimeStage {
    pub fn as_slug(&self) -> &'static str {
        match self {
            RuntimeStage::Stage1Runtime => "stage1-runtime",
            RuntimeStage::Stage2ProjectBacked => "stage2-project-backed",
            RuntimeStage::Stage3NearDocker => "stage3-near-docker",
            RuntimeStage::FullSandbox => "full-sandbox",
        }
    }

    pub fn as_contract_label(&self) -> &'static str {
        match self {
            RuntimeStage::Stage1Runtime => "Stage 1 worker runtime",
            RuntimeStage::Stage2ProjectBacked => "Stage 2 project-backed runtime",
            RuntimeStage::Stage3NearDocker => "Stage 3 near-Docker runtime",
            RuntimeStage::FullSandbox => "full sandbox runtime",
        }
    }
}

/// How project workspace content reaches a runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceDelivery {
    Unsupported,
    HostMount,
    OrchestratorBootstrap,
}

impl WorkspaceDelivery {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkspaceDelivery::Unsupported => "unsupported",
            WorkspaceDelivery::HostMount => "host-mount",
            WorkspaceDelivery::OrchestratorBootstrap => "orchestrator-bootstrap",
        }
    }

    pub fn supports_bind_mounts(&self) -> bool {
        matches!(self, WorkspaceDelivery::HostMount)
    }
}

/// How runtime-scoped configuration is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigDelivery {
    Unsupported,
    HostMount,
    OrchestratorBootstrap,
    ProjectedVolume,
}

impl ConfigDelivery {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConfigDelivery::Unsupported => "unsupported",
            ConfigDelivery::HostMount => "host-mount",
            ConfigDelivery::OrchestratorBootstrap => "orchestrator-bootstrap",
            ConfigDelivery::ProjectedVolume => "projected-volume",
        }
    }
}

/// How outbound network constraints are enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkIsolation {
    HostProxyAllowlist,
    PodDirect,
    KubernetesNativeControls,
}

impl NetworkIsolation {
    pub fn as_str(&self) -> &'static str {
        match self {
            NetworkIsolation::HostProxyAllowlist => "host-proxy-allowlist",
            NetworkIsolation::PodDirect => "pod-direct",
            NetworkIsolation::KubernetesNativeControls => "kubernetes-native-controls",
        }
    }

    pub fn supports_host_proxy(&self) -> bool {
        matches!(self, NetworkIsolation::HostProxyAllowlist)
    }
}

/// Canonical capability profile for a container runtime backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilities {
    pub stage: RuntimeStage,
    pub workspace_delivery: WorkspaceDelivery,
    pub config_delivery: ConfigDelivery,
    pub network_isolation: NetworkIsolation,
    pub limitations: &'static [&'static str],
}

impl RuntimeCapabilities {
    pub const fn new(
        stage: RuntimeStage,
        workspace_delivery: WorkspaceDelivery,
        config_delivery: ConfigDelivery,
        network_isolation: NetworkIsolation,
        limitations: &'static [&'static str],
    ) -> Self {
        Self {
            stage,
            workspace_delivery,
            config_delivery,
            network_isolation,
            limitations,
        }
    }

    pub fn supports_host_proxy(&self) -> bool {
        self.network_isolation.supports_host_proxy()
    }

    pub fn supports_bind_mounts(&self) -> bool {
        self.workspace_delivery.supports_bind_mounts()
    }

    pub fn supports_sandbox_workspace_delivery(&self) -> bool {
        matches!(
            self.workspace_delivery,
            WorkspaceDelivery::HostMount | WorkspaceDelivery::OrchestratorBootstrap
        )
    }

    pub fn supports_workspace_writeback(&self) -> bool {
        matches!(
            self.workspace_delivery,
            WorkspaceDelivery::HostMount | WorkspaceDelivery::OrchestratorBootstrap
        )
    }

    pub fn supports_allowlist_networking(&self) -> bool {
        !matches!(self.network_isolation, NetworkIsolation::PodDirect)
    }

    pub fn supports_project_content(&self) -> bool {
        !matches!(self.workspace_delivery, WorkspaceDelivery::Unsupported)
    }

    pub fn supports_runtime_config(&self) -> bool {
        !matches!(self.config_delivery, ConfigDelivery::Unsupported)
    }

    pub fn summary_fields(&self) -> [(&'static str, &'static str); 4] {
        [
            ("stage", self.stage.as_slug()),
            ("workspace", self.workspace_delivery.as_str()),
            ("config", self.config_delivery.as_str()),
            ("network", self.network_isolation.as_str()),
        ]
    }
}

pub fn format_capability_gaps(gaps: &[&str]) -> String {
    match gaps {
        [] => "required capabilities".to_string(),
        [one] => (*one).to_string(),
        [first, second] => format!("{first} and {second}"),
        _ => {
            let mut text = gaps[..gaps.len() - 1].join(", ");
            text.push_str(", and ");
            text.push_str(gaps[gaps.len() - 1]);
            text
        }
    }
}

pub fn format_stage_contract_failure(
    runtime_name: &str,
    capabilities: &RuntimeCapabilities,
    attempted: &str,
    gaps: &[&str],
    next_step: &str,
) -> String {
    let gap_text = format_capability_gaps(gaps);
    let verb = if gaps.len() == 1 { "is" } else { "are" };
    match capabilities.stage {
        RuntimeStage::FullSandbox => format!(
            "{runtime_name} runtime cannot provide {attempted} because {gap_text} {verb} unavailable. {next_step}"
        ),
        _ => format!(
            "{runtime_name} runtime is currently {}. It cannot provide {attempted} because {gap_text} {verb} not available yet. {next_step}",
            capabilities.stage.as_contract_label()
        ),
    }
}

pub fn is_capability_contract_violation(reason: &str) -> bool {
    reason.contains("runtime is currently Stage ")
}

pub fn docker_runtime_capabilities() -> RuntimeCapabilities {
    RuntimeCapabilities::new(
        RuntimeStage::FullSandbox,
        WorkspaceDelivery::HostMount,
        ConfigDelivery::HostMount,
        NetworkIsolation::HostProxyAllowlist,
        &[],
    )
}

pub fn kubernetes_runtime_capabilities() -> RuntimeCapabilities {
    kubernetes_runtime_capabilities_with_controls(false, false)
}

pub fn kubernetes_runtime_capabilities_with_controls(
    native_network_controls: bool,
    projected_runtime_config: bool,
) -> RuntimeCapabilities {
    RuntimeCapabilities::new(
        RuntimeStage::Stage2ProjectBacked,
        WorkspaceDelivery::OrchestratorBootstrap,
        if projected_runtime_config {
            ConfigDelivery::ProjectedVolume
        } else {
            ConfigDelivery::OrchestratorBootstrap
        },
        if native_network_controls {
            NetworkIsolation::KubernetesNativeControls
        } else {
            NetworkIsolation::PodDirect
        },
        if native_network_controls {
            &[]
        } else {
            &["allowlist-only networking is unavailable"]
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_capabilities_support_workspace_writeback() {
        assert!(docker_runtime_capabilities().supports_workspace_writeback());
    }

    #[test]
    fn kubernetes_bootstrap_capabilities_support_workspace_writeback() {
        assert!(
            kubernetes_runtime_capabilities_with_controls(true, true)
                .supports_workspace_writeback()
        );
    }
}
