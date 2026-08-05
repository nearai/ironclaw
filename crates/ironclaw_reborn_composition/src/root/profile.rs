use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RebornCompositionProfile {
    #[default]
    Disabled,
    #[serde(rename = "local-dev")]
    Standalone,
    #[serde(rename = "local-dev-yolo")]
    StandaloneUnrestricted,
    HostedSingleTenant,
    HostedSingleTenantVolume,
    HostedSingleTenantVolumeSandboxed,
    HostedSingleTenantVolumeSandboxedRailway,
    Production,
    MigrationDryRun,
}

impl RebornCompositionProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Standalone => "local-dev",
            Self::StandaloneUnrestricted => "local-dev-yolo",
            Self::HostedSingleTenant => "hosted-single-tenant",
            Self::HostedSingleTenantVolume => "hosted-single-tenant-volume",
            Self::HostedSingleTenantVolumeSandboxed => "hosted-single-tenant-volume-sandboxed",
            Self::HostedSingleTenantVolumeSandboxedRailway => {
                "hosted-single-tenant-volume-sandboxed-railway"
            }
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

    pub fn uses_local_filesystem_storage(self) -> bool {
        self.deployment().uses_local_filesystem_storage()
    }

    pub fn starts_live_runtime(self) -> bool {
        self.deployment().traffic().starts_live_runtime()
    }

    pub fn uses_hosted_extension_installation_state(self) -> bool {
        self.deployment().uses_hosted_extension_installation_state()
    }

    /// Whether this profile keys workspace mounts per caller.
    ///
    /// One decision, read by every workspace write lane in composition and by
    /// the CLI's WebUI workspace-projection flag, so the browser and the agent
    /// address the same subtree.
    pub fn workspace_scoped_per_caller(self) -> bool {
        self.deployment().workspace_scoped_per_caller()
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
            "local-dev" => Ok(Self::Standalone),
            "local-dev-yolo" => Ok(Self::StandaloneUnrestricted),
            "hosted-single-tenant" => Ok(Self::HostedSingleTenant),
            "hosted-single-tenant-volume" => Ok(Self::HostedSingleTenantVolume),
            "hosted-single-tenant-volume-sandboxed" => Ok(Self::HostedSingleTenantVolumeSandboxed),
            "hosted-single-tenant-volume-sandboxed-railway" => {
                Ok(Self::HostedSingleTenantVolumeSandboxedRailway)
            }
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
