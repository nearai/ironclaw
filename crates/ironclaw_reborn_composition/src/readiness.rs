use serde::{Deserialize, Serialize};

use crate::RebornCompositionProfile;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RebornReadinessState {
    #[default]
    Disabled,
    DevOnly,
    ProductionValidated,
    MigrationDryRunValidated,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornFacadeReadiness {
    pub host_runtime: bool,
    pub turn_coordinator: bool,
    pub product_auth: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornWorkerReadiness {
    pub turn_runner: bool,
    pub trigger_poller: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornReadiness {
    pub profile: RebornCompositionProfile,
    pub state: RebornReadinessState,
    pub facades: RebornFacadeReadiness,
    #[serde(default)]
    pub workers: RebornWorkerReadiness,
}

impl RebornReadiness {
    pub const fn disabled() -> Self {
        Self {
            profile: RebornCompositionProfile::Disabled,
            state: RebornReadinessState::Disabled,
            facades: RebornFacadeReadiness {
                host_runtime: false,
                turn_coordinator: false,
                product_auth: false,
            },
            workers: RebornWorkerReadiness {
                turn_runner: false,
                trigger_poller: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_deserializes_without_workers_for_older_payloads() {
        let readiness: RebornReadiness = serde_json::from_str(
            r#"{
                "profile": "local-dev",
                "state": "dev-only",
                "facades": {
                    "host_runtime": true,
                    "turn_coordinator": true,
                    "product_auth": false
                }
            }"#,
        )
        .expect("readiness deserializes");

        assert_eq!(readiness.profile, RebornCompositionProfile::LocalDev);
        assert_eq!(readiness.state, RebornReadinessState::DevOnly);
        assert_eq!(readiness.workers, RebornWorkerReadiness::default());
    }
}
