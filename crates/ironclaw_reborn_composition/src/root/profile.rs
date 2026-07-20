use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RebornCompositionProfile {
    #[default]
    Disabled,
    LocalDev,
    LocalDevYolo,
    HostedSingleTenant,
    HostedSingleTenantVolume,
    Production,
    MigrationDryRun,
}

impl RebornCompositionProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::LocalDev => "local-dev",
            Self::LocalDevYolo => "local-dev-yolo",
            Self::HostedSingleTenant => "hosted-single-tenant",
            Self::HostedSingleTenantVolume => "hosted-single-tenant-volume",
            Self::Production => "production",
            Self::MigrationDryRun => "migration-dry-run",
        }
    }

    pub fn is_active(self) -> bool {
        self != Self::Disabled
    }

    /// The deployment data this profile name selects.
    ///
    /// Every predicate below reads this rather than `match`ing on `self`:
    /// `DeploymentConfig::for_profile` is the one profile match in the crate
    /// (§4.4). The `confirm_host_access` argument only affects the yolo
    /// *policy request*, which none of these predicates read, so passing
    /// `false` here cannot change any answer.
    fn deployment(self) -> crate::deployment::DeploymentConfig {
        crate::deployment::DeploymentConfig::for_profile(self, false)
    }

    pub fn requires_production_shape(self) -> bool {
        self.deployment().substrate() == crate::deployment::RuntimeSubstrate::ProductionShaped
    }

    pub fn uses_local_runtime_substrate(self) -> bool {
        self.deployment().substrate() == crate::deployment::RuntimeSubstrate::Local
    }

    pub fn uses_local_dev_storage_input(self) -> bool {
        self.deployment().uses_local_dev_storage_input()
    }

    pub fn starts_live_runtime(self) -> bool {
        self.deployment().traffic().starts_live_runtime()
    }

    pub fn uses_hosted_extension_installation_state(self) -> bool {
        self.deployment().uses_hosted_extension_installation_state()
    }

    pub fn to_event_store_profile(self) -> ironclaw_reborn_event_store::RebornProfile {
        self.deployment().event_store_profile()
    }
}

impl FromStr for RebornCompositionProfile {
    type Err = RebornCompositionProfileParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "disabled" => Ok(Self::Disabled),
            "local-dev" => Ok(Self::LocalDev),
            "local-dev-yolo" => Ok(Self::LocalDevYolo),
            "hosted-single-tenant" => Ok(Self::HostedSingleTenant),
            "hosted-single-tenant-volume" => Ok(Self::HostedSingleTenantVolume),
            "production" => Ok(Self::Production),
            "migration-dry-run" => Ok(Self::MigrationDryRun),
            _ => Err(RebornCompositionProfileParseError { value: normalized }),
        }
    }
}

impl std::fmt::Display for RebornCompositionProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid reborn composition profile '{value}'")]
pub struct RebornCompositionProfileParseError {
    value: String,
}
