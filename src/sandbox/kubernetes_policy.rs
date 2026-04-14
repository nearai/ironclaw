//! Kubernetes-native isolation readiness reporting.
//!
//! This module tracks which cluster-side controls are configured for the
//! Kubernetes runtime. The current runtime contract stays at Stage 2 even
//! after read-only one-shot sandbox commands gain uploaded workspace delivery,
//! because the broader near-Docker experience still depends on cluster-side
//! network controls plus workspace write-back semantics.

const ENV_NATIVE_NETWORK_CONTROLS: &str = "IRONCLAW_K8S_NATIVE_NETWORK_CONTROLS";
const ENV_PROJECTED_RUNTIME_CONFIG: &str = "IRONCLAW_K8S_PROJECTED_RUNTIME_CONFIG";

const MISSING_NONE: &[&str] = &[];
const MISSING_NETWORK_ONLY: &[&str] = &["kubernetes-native network controls"];
const MISSING_PROJECTED_CONFIG_ONLY: &[&str] = &["projected runtime config delivery"];
const MISSING_BOTH: &[&str] = &[
    "kubernetes-native network controls",
    "projected runtime config delivery",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KubernetesIsolationReadiness {
    native_network_controls: bool,
    projected_runtime_config: bool,
}

impl KubernetesIsolationReadiness {
    pub const fn new(native_network_controls: bool, projected_runtime_config: bool) -> Self {
        Self {
            native_network_controls,
            projected_runtime_config,
        }
    }

    pub fn from_env() -> Self {
        Self::new(
            env_flag_enabled(ENV_NATIVE_NETWORK_CONTROLS),
            env_flag_enabled(ENV_PROJECTED_RUNTIME_CONFIG),
        )
    }

    pub fn stage3_prerequisites_ready(&self) -> bool {
        self.native_network_controls && self.projected_runtime_config
    }

    pub fn native_network_controls_enabled(&self) -> bool {
        self.native_network_controls
    }

    pub fn projected_runtime_config_enabled(&self) -> bool {
        self.projected_runtime_config
    }

    pub fn missing_stage3_prerequisites(&self) -> &'static [&'static str] {
        match (self.native_network_controls, self.projected_runtime_config) {
            (true, true) => MISSING_NONE,
            (false, true) => MISSING_NETWORK_ONLY,
            (true, false) => MISSING_PROJECTED_CONFIG_ONLY,
            (false, false) => MISSING_BOTH,
        }
    }

    pub fn doctor_note(&self) -> String {
        if self.stage3_prerequisites_ready() {
            "stage3-prereqs=ready, read-only one-shot commands can use uploaded workspaces and runtime config can use projected files, workspace-write one-shot commands still need Docker until workspace write-back exists".to_string()
        } else {
            format!(
                "stage3-missing={}",
                self.missing_stage3_prerequisites().join(", ")
            )
        }
    }
}

fn env_flag_enabled(key: &str) -> bool {
    matches!(
        std::env::var(key).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_reports_both_missing_by_default() {
        let readiness = KubernetesIsolationReadiness::new(false, false);
        assert!(!readiness.stage3_prerequisites_ready());
        assert_eq!(readiness.missing_stage3_prerequisites(), MISSING_BOTH);
        assert_eq!(
            readiness.doctor_note(),
            "stage3-missing=kubernetes-native network controls, projected runtime config delivery"
        );
    }

    #[test]
    fn readiness_reports_ready_when_both_controls_exist() {
        let readiness = KubernetesIsolationReadiness::new(true, true);
        assert!(readiness.stage3_prerequisites_ready());
        assert!(readiness.missing_stage3_prerequisites().is_empty());
        assert_eq!(
            readiness.doctor_note(),
            "stage3-prereqs=ready, read-only one-shot commands can use uploaded workspaces and runtime config can use projected files, workspace-write one-shot commands still need Docker until workspace write-back exists"
        );
    }
}
