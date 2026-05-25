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
    Production,
    MigrationDryRun,
}

impl RebornCompositionProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::LocalDev => "local-dev",
            Self::LocalDevYolo => "local-dev-yolo",
            Self::Production => "production",
            Self::MigrationDryRun => "migration-dry-run",
        }
    }

    pub fn is_active(self) -> bool {
        self != Self::Disabled
    }

    pub fn requires_production_shape(self) -> bool {
        matches!(self, Self::Production | Self::MigrationDryRun)
    }

    pub fn to_event_store_profile(self) -> ironclaw_reborn_event_store::RebornProfile {
        match self {
            Self::Disabled | Self::LocalDev | Self::LocalDevYolo => {
                ironclaw_reborn_event_store::RebornProfile::LocalDev
            }
            Self::Production | Self::MigrationDryRun => {
                ironclaw_reborn_event_store::RebornProfile::Production
            }
        }
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
