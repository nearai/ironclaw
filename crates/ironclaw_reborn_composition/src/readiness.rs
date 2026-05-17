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
pub struct RebornReadiness {
    pub profile: RebornCompositionProfile,
    pub state: RebornReadinessState,
    pub facades: RebornFacadeReadiness,
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
        }
    }
}
