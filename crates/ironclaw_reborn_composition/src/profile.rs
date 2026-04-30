//! Reborn composition profile.
//!
//! Lives in this crate (not `ironclaw_host_api`) because it is a startup
//! orchestration concern: the value picks which factory branch runs, what
//! degree of fail-closed validation applies, and how readiness reports the
//! resulting graph. Lower substrate crates do not branch on the profile —
//! they expose their own typed contracts and let the composition root do the
//! switching.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Explicit composition profile selected by the operator.
///
/// The default is [`Disabled`] so that adding this crate to the workspace
/// does not change production behavior. The cutover decision flips it to
/// [`Production`] only after the full required service graph is wired and
/// validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RebornProfile {
    /// Reborn composition disabled. Legacy startup path is authoritative.
    /// This is the safe default while substrate crates land incrementally.
    #[default]
    Disabled,
    /// Explicit local/dev/test profile. In-memory and reference filesystem
    /// backends are allowed. Readiness reports `LocalDev`, never
    /// "production-ready".
    LocalDev,
    /// Full fail-closed graph required. Missing substrate, invalid config,
    /// or any in-memory/dev fallback aborts startup with a sanitized
    /// diagnostic before traffic-serving surfaces are exposed.
    Production,
    /// Build the Production graph but do not expose channels, loops, or
    /// HTTP routes. Used to validate factories/migrations against a
    /// candidate config without flipping the live deployment.
    MigrationDryRun,
}

impl RebornProfile {
    /// True when this profile expects a fully-validated production graph.
    /// Used by factories to decide whether missing substrate is fatal or
    /// tolerable.
    pub fn requires_full_graph(self) -> bool {
        matches!(self, Self::Production | Self::MigrationDryRun)
    }

    /// True when in-memory / reference backends are allowed.
    pub fn allows_in_memory_backends(self) -> bool {
        matches!(self, Self::Disabled | Self::LocalDev)
    }

    /// Wire id used in env/config. Mirrors serde's `kebab-case` rendering so
    /// hand-written env values match the deserialised representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::LocalDev => "local-dev",
            Self::Production => "production",
            Self::MigrationDryRun => "migration-dry-run",
        }
    }
}

impl fmt::Display for RebornProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Failure parsing a [`RebornProfile`] from a string.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(
    "invalid reborn profile '{value}'; expected one of: disabled, local-dev, production, migration-dry-run"
)]
pub struct RebornProfileParseError {
    pub value: String,
}

impl std::str::FromStr for RebornProfile {
    type Err = RebornProfileParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Accept both kebab-case (canonical) and snake_case (env-friendly)
        // and underscore variants, normalising to lowercase. This stays
        // narrower than serde's parse so an unknown value is a hard error
        // rather than a silent coerce.
        let normalized = s.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "disabled" | "off" => Ok(Self::Disabled),
            "local-dev" | "local" | "dev" => Ok(Self::LocalDev),
            "production" | "prod" => Ok(Self::Production),
            "migration-dry-run" | "dry-run" => Ok(Self::MigrationDryRun),
            _ => Err(RebornProfileParseError {
                value: s.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn default_is_disabled() {
        assert_eq!(RebornProfile::default(), RebornProfile::Disabled);
    }

    #[test]
    fn requires_full_graph_only_for_production_and_dry_run() {
        assert!(!RebornProfile::Disabled.requires_full_graph());
        assert!(!RebornProfile::LocalDev.requires_full_graph());
        assert!(RebornProfile::Production.requires_full_graph());
        assert!(RebornProfile::MigrationDryRun.requires_full_graph());
    }

    #[test]
    fn allows_in_memory_only_for_disabled_and_local_dev() {
        assert!(RebornProfile::Disabled.allows_in_memory_backends());
        assert!(RebornProfile::LocalDev.allows_in_memory_backends());
        assert!(!RebornProfile::Production.allows_in_memory_backends());
        assert!(!RebornProfile::MigrationDryRun.allows_in_memory_backends());
    }

    #[test]
    fn parse_accepts_canonical_and_env_friendly_forms() {
        assert_eq!(
            RebornProfile::from_str("disabled").unwrap(),
            RebornProfile::Disabled
        );
        assert_eq!(
            RebornProfile::from_str("local-dev").unwrap(),
            RebornProfile::LocalDev
        );
        assert_eq!(
            RebornProfile::from_str("local_dev").unwrap(),
            RebornProfile::LocalDev
        );
        assert_eq!(
            RebornProfile::from_str("Production").unwrap(),
            RebornProfile::Production
        );
        assert_eq!(
            RebornProfile::from_str("migration_dry_run").unwrap(),
            RebornProfile::MigrationDryRun
        );
        assert_eq!(
            RebornProfile::from_str("dry-run").unwrap(),
            RebornProfile::MigrationDryRun
        );
    }

    #[test]
    fn parse_rejects_unknown_values() {
        let err = RebornProfile::from_str("staging").expect_err("unknown profile must fail");
        assert_eq!(err.value, "staging");
    }

    #[test]
    fn display_matches_canonical_kebab() {
        assert_eq!(RebornProfile::LocalDev.to_string(), "local-dev");
        assert_eq!(
            RebornProfile::MigrationDryRun.to_string(),
            "migration-dry-run"
        );
    }

    #[test]
    fn serde_round_trips_in_kebab_case() {
        let json = serde_json::to_string(&RebornProfile::LocalDev).unwrap();
        assert_eq!(json, "\"local-dev\"");
        let parsed: RebornProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, RebornProfile::LocalDev);
    }
}
