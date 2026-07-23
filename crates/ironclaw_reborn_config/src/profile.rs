use std::{ffi::OsString, str::FromStr};

use crate::RebornConfigError;

/// Default environment variable that selects the IronClaw boot profile.
pub const IRONCLAW_PROFILE_ENV: &str = "IRONCLAW_PROFILE";
/// Legacy environment variable retained for compatibility.
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
        Self::from_env_value_named(value, IRONCLAW_PROFILE_ENV)
    }

    pub fn from_env_values(
        value: Option<OsString>,
        legacy_value: Option<OsString>,
    ) -> Result<Self, RebornConfigError> {
        match value {
            Some(value) => Self::from_env_value_named(Some(value), IRONCLAW_PROFILE_ENV),
            None => Self::from_env_value_named(legacy_value, REBORN_PROFILE_ENV),
        }
    }

    fn from_env_value_named(
        value: Option<OsString>,
        name: &'static str,
    ) -> Result<Self, RebornConfigError> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        let value = value.to_string_lossy();
        Self::parse_named(value.as_ref(), name)
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
}

impl FromStr for RebornProfile {
    type Err = RebornConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse_named(value, IRONCLAW_PROFILE_ENV)
    }
}

impl RebornProfile {
    fn parse_named(value: &str, name: &'static str) -> Result<Self, RebornConfigError> {
        match value {
            "local-dev" => Ok(Self::LocalDev),
            "local-dev-yolo" => Ok(Self::LocalDevYolo),
            "hosted-single-tenant" => Ok(Self::HostedSingleTenant),
            "hosted-single-tenant-volume" => Ok(Self::HostedSingleTenantVolume),
            "production" => Ok(Self::Production),
            "migration-dry-run" => Ok(Self::MigrationDryRun),
            other => Err(RebornConfigError::InvalidProfile {
                name,
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
