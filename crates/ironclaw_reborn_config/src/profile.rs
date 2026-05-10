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
    /// Production startup. Future runtime composition must fail closed here if
    /// required durable services are absent.
    Production,
    /// Validate production-shaped boot/config without accepting production
    /// traffic or performing migration side effects.
    MigrationDryRun,
}

impl RebornProfile {
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
            Self::Production => "production",
            Self::MigrationDryRun => "migration-dry-run",
        }
    }
}

impl FromStr for RebornProfile {
    type Err = RebornConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "local-dev" => Ok(Self::LocalDev),
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
