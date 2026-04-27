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

    pub fn allowlist_networking_ready(&self) -> bool {
        self.native_network_controls
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

    pub fn allowlist_note(&self) -> String {
        if self.allowlist_networking_ready() {
            "allowlist-networking=ready, one-shot sandbox commands can use Kubernetes for allowlist-constrained networking".to_string()
        } else {
            format!(
                "allowlist-networking=missing:{}, one-shot sandbox commands that need allowlist-constrained networking still need Docker",
                MISSING_NETWORK_ONLY.join(", ")
            )
        }
    }

    pub fn projected_runtime_config_note(&self) -> String {
        if self.projected_runtime_config_enabled() {
            "projected-runtime-config=ready, runtime config files can use projected file delivery"
                .to_string()
        } else {
            "projected-runtime-config=missing:projected runtime config delivery, runtime config files still use orchestrator bootstrap delivery".to_string()
        }
    }

    pub fn doctor_note(&self) -> String {
        format!(
            "{}; {}",
            self.allowlist_note(),
            self.projected_runtime_config_note()
        )
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
        assert!(!readiness.allowlist_networking_ready());
        assert_eq!(readiness.missing_stage3_prerequisites(), MISSING_BOTH);
        assert_eq!(
            readiness.doctor_note(),
            "allowlist-networking=missing:kubernetes-native network controls, one-shot sandbox commands that need allowlist-constrained networking still need Docker; projected-runtime-config=missing:projected runtime config delivery, runtime config files still use orchestrator bootstrap delivery"
        );
    }

    #[test]
    fn readiness_reports_ready_when_both_controls_exist() {
        let readiness = KubernetesIsolationReadiness::new(true, true);
        assert!(readiness.stage3_prerequisites_ready());
        assert!(readiness.allowlist_networking_ready());
        assert!(readiness.missing_stage3_prerequisites().is_empty());
        assert_eq!(
            readiness.doctor_note(),
            "allowlist-networking=ready, one-shot sandbox commands can use Kubernetes for allowlist-constrained networking; projected-runtime-config=ready, runtime config files can use projected file delivery"
        );
    }

    #[test]
    fn readiness_reports_allowlist_ready_without_projected_config() {
        let readiness = KubernetesIsolationReadiness::new(true, false);
        assert!(readiness.allowlist_networking_ready());
        assert!(!readiness.stage3_prerequisites_ready());
        assert_eq!(
            readiness.doctor_note(),
            "allowlist-networking=ready, one-shot sandbox commands can use Kubernetes for allowlist-constrained networking; projected-runtime-config=missing:projected runtime config delivery, runtime config files still use orchestrator bootstrap delivery"
        );
    }

    #[test]
    fn readiness_reports_projected_config_separately_when_network_missing() {
        let readiness = KubernetesIsolationReadiness::new(false, true);
        assert!(!readiness.allowlist_networking_ready());
        assert!(!readiness.stage3_prerequisites_ready());
        assert_eq!(
            readiness.doctor_note(),
            "allowlist-networking=missing:kubernetes-native network controls, one-shot sandbox commands that need allowlist-constrained networking still need Docker; projected-runtime-config=ready, runtime config files can use projected file delivery"
        );
    }
}
