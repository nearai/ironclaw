use std::{ffi::OsString, str::FromStr};

use crate::RebornConfigError;

/// Environment variable that selects the standalone Reborn boot profile.
pub const REBORN_PROFILE_ENV: &str = "IRONCLAW_REBORN_PROFILE";

/// Coarse boot profile for the standalone Reborn binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RebornProfile {
    /// Explicit local/developer mode. This is the safe default for a separately
    /// invoked binary until production composition is wired and verified.
    #[default]
    LocalDev,
    /// Trusted single-user local development mode with full host shell
    /// environment inheritance. Never selected by default.
    LocalDevYolo,
    /// Hosted single-tenant startup. Uses the local-runtime product surface
    /// with durable PostgreSQL storage.
    HostedSingleTenant,
    /// Single-tenant hosted preview using the local-runtime substrate on a
    /// persistent volume. Intended for SSO-only Railway-style deployments while
    /// the full PostgreSQL production composition continues to mature.
    HostedSingleTenantVolume,
    /// Production startup. Future runtime composition must fail closed here if
    /// required durable services are absent.
    Production,
    /// Validate production-shaped boot/config without accepting production
    /// traffic or performing migration side effects.
    MigrationDryRun,
}

impl RebornProfile {
    const ALL: [Self; 6] = [
        Self::LocalDev,
        Self::LocalDevYolo,
        Self::HostedSingleTenant,
        Self::HostedSingleTenantVolume,
        Self::Production,
        Self::MigrationDryRun,
    ];

    pub fn all() -> &'static [Self] {
        &Self::ALL
    }

    pub fn from_env_value(value: Option<OsString>) -> Result<Self, RebornConfigError> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        let value = value.to_string_lossy();
        Self::from_str(value.as_ref())
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalDev => "local-dev",
            Self::LocalDevYolo => "local-dev-yolo",
            Self::HostedSingleTenant => "hosted-single-tenant",
            Self::HostedSingleTenantVolume => "hosted-single-tenant-volume",
            Self::Production => "production",
            Self::MigrationDryRun => "migration-dry-run",
        }
    }

    pub fn starts_hosted_single_tenant_listener(self) -> bool {
        matches!(
            self,
            Self::HostedSingleTenant | Self::HostedSingleTenantVolume
        )
    }

    pub fn uses_standalone_local_runtime_volume(self) -> bool {
        matches!(
            self,
            Self::LocalDev | Self::LocalDevYolo | Self::HostedSingleTenantVolume
        )
    }

    pub fn local_runtime_storage_subdir(self) -> &'static str {
        match self {
            Self::HostedSingleTenant => "hosted-single-tenant",
            Self::HostedSingleTenantVolume => "hosted-single-tenant-volume",
            Self::LocalDev | Self::LocalDevYolo | Self::Production | Self::MigrationDryRun => {
                "local-dev"
            }
        }
    }

    pub fn supports_local_runtime_skill_management(self) -> bool {
        matches!(
            self,
            Self::LocalDev
                | Self::LocalDevYolo
                | Self::HostedSingleTenant
                | Self::HostedSingleTenantVolume
        )
    }

    /// Whether the WebUI `serve` listener may auto-provision a local bearer
    /// token + operator user id when the operator has not supplied them via
    /// environment variables.
    ///
    /// True only for the trusted-laptop developer profiles. Hosted and
    /// production profiles must fail closed: their session-signing secret and
    /// operator identity are operator-owned and never generated for them.
    pub fn allows_dev_credential_autoprovision(self) -> bool {
        matches!(self, Self::LocalDev | Self::LocalDevYolo)
    }
}

impl FromStr for RebornProfile {
    type Err = RebornConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "local-dev" => Ok(Self::LocalDev),
            "local-dev-yolo" => Ok(Self::LocalDevYolo),
            "hosted-single-tenant" => Ok(Self::HostedSingleTenant),
            "hosted-single-tenant-volume" => Ok(Self::HostedSingleTenantVolume),
            "production" => Ok(Self::Production),
            "migration-dry-run" => Ok(Self::MigrationDryRun),
            other => Err(RebornConfigError::InvalidProfile {
                name: REBORN_PROFILE_ENV,
                value: other.to_string(),
            }),
        }
    }
}

impl std::fmt::Display for RebornProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_dev_profiles_allow_credential_autoprovision() {
        for profile in RebornProfile::all() {
            let allowed = profile.allows_dev_credential_autoprovision();
            match profile {
                RebornProfile::LocalDev | RebornProfile::LocalDevYolo => {
                    assert!(
                        allowed,
                        "{profile} must allow dev credential auto-provision"
                    )
                }
                RebornProfile::HostedSingleTenant
                | RebornProfile::HostedSingleTenantVolume
                | RebornProfile::Production
                | RebornProfile::MigrationDryRun => assert!(
                    !allowed,
                    "{profile} must fail closed and require operator-supplied credentials"
                ),
            }
        }
    }
}
