//! Legacy compatibility bridge mode.
//!
//! `reborn.compatibility.legacy_bridge_mode` from issue #3026's "Legacy
//! compatibility and rollback" section. Picks how Reborn services may
//! interact with the legacy `src/` schemas and managers during the cutover
//! transition.
//!
//! The mode lives on [`crate::RebornBuildInput`] so a substrate factory that
//! understands a legacy schema (e.g. a future memory bridge that reads the
//! existing workspace tables) can branch on it without re-reading config.
//! Default is [`LegacyBridgeMode::Off`] — nothing in Reborn touches legacy
//! state unless the operator names a non-Off mode.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Compatibility bridge between Reborn services and the legacy `src/`
/// schemas/managers.
///
/// Variants are listed least-permissive first. Adding a new variant must
/// keep that ordering: anything that grants Reborn more access to legacy
/// state belongs further down the list, so a `>=` comparison stays
/// meaningful for future "at least read-only" gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LegacyBridgeMode {
    /// No bridge. Reborn services neither read nor write legacy schemas.
    /// This is the cutover-ready state — once every required substrate is
    /// wired and migrations are complete, the legacy path is dead and the
    /// bridge stays off. Default so a fresh deployment cannot accidentally
    /// route around Reborn through a forgotten legacy code path.
    #[default]
    Off,
    /// Reborn services may read legacy schemas for surfacing existing data
    /// (e.g. listing pre-migration threads, hydrating existing memory
    /// rows). Writes through this bridge are rejected. Safe for
    /// `MigrationDryRun` and for staged rollouts where the legacy path
    /// remains authoritative for writes.
    ReadOnly,
    /// Reborn services may read and write legacy schemas for idempotent
    /// backfill and migration tasks. Permissive enough that
    /// [`crate::RebornProfile::Production`] requires an explicit operator
    /// acknowledgement before this mode takes effect — see
    /// [`LegacyBridgeMode::requires_explicit_production_ack`].
    Migrate,
}

impl LegacyBridgeMode {
    /// True when any cross-schema access from Reborn into legacy state is
    /// permitted.
    pub fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// True when this mode is permissive enough that operators must
    /// explicitly acknowledge using it under [`crate::RebornProfile::Production`].
    /// Production with `Migrate` is a transitional state, never the steady
    /// state — requiring an extra acknowledgement keeps an operator who
    /// flipped on Reborn from silently inheriting cross-schema writes from
    /// a stale config.
    pub fn requires_explicit_production_ack(self) -> bool {
        matches!(self, Self::Migrate)
    }

    /// Wire id used in env/config. Mirrors serde's `kebab-case` rendering
    /// so hand-written env values match the deserialised representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::ReadOnly => "read-only",
            Self::Migrate => "migrate",
        }
    }
}

impl fmt::Display for LegacyBridgeMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Failure parsing a [`LegacyBridgeMode`] from a string.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("invalid reborn legacy bridge mode '{value}'; expected one of: off, read-only, migrate")]
pub struct LegacyBridgeModeParseError {
    pub value: String,
}

impl std::str::FromStr for LegacyBridgeMode {
    type Err = LegacyBridgeModeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Accept kebab-case (canonical), snake_case, and a couple of
        // common aliases. Same narrowing-vs-serde rationale as
        // `RebornProfile::from_str` — unknown values are a hard error.
        let normalized = s.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "off" | "disabled" | "none" => Ok(Self::Off),
            "read-only" | "readonly" | "ro" => Ok(Self::ReadOnly),
            "migrate" | "migration" | "rw" => Ok(Self::Migrate),
            _ => Err(LegacyBridgeModeParseError {
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
    fn default_is_off() {
        assert_eq!(LegacyBridgeMode::default(), LegacyBridgeMode::Off);
        assert!(!LegacyBridgeMode::default().is_enabled());
    }

    #[test]
    fn is_enabled_only_for_non_off_modes() {
        assert!(!LegacyBridgeMode::Off.is_enabled());
        assert!(LegacyBridgeMode::ReadOnly.is_enabled());
        assert!(LegacyBridgeMode::Migrate.is_enabled());
    }

    #[test]
    fn migrate_requires_production_ack() {
        // Off and ReadOnly are tame enough to run under Production with no
        // extra operator gate. Migrate alters legacy state, so an
        // additional acknowledgement is required at the AppBuilder seam.
        assert!(!LegacyBridgeMode::Off.requires_explicit_production_ack());
        assert!(!LegacyBridgeMode::ReadOnly.requires_explicit_production_ack());
        assert!(LegacyBridgeMode::Migrate.requires_explicit_production_ack());
    }

    #[test]
    fn parse_accepts_canonical_and_aliases() {
        assert_eq!(
            LegacyBridgeMode::from_str("off").unwrap(),
            LegacyBridgeMode::Off
        );
        assert_eq!(
            LegacyBridgeMode::from_str("read-only").unwrap(),
            LegacyBridgeMode::ReadOnly
        );
        assert_eq!(
            LegacyBridgeMode::from_str("read_only").unwrap(),
            LegacyBridgeMode::ReadOnly
        );
        assert_eq!(
            LegacyBridgeMode::from_str("readonly").unwrap(),
            LegacyBridgeMode::ReadOnly
        );
        assert_eq!(
            LegacyBridgeMode::from_str("migrate").unwrap(),
            LegacyBridgeMode::Migrate
        );
        assert_eq!(
            LegacyBridgeMode::from_str("migration").unwrap(),
            LegacyBridgeMode::Migrate
        );
    }

    #[test]
    fn parse_rejects_unknown_values() {
        let err = LegacyBridgeMode::from_str("forced").expect_err("unknown mode must fail");
        assert_eq!(err.value, "forced");
    }

    #[test]
    fn serde_round_trips_in_kebab_case() {
        let json = serde_json::to_string(&LegacyBridgeMode::ReadOnly).unwrap();
        assert_eq!(json, "\"read-only\"");
        let parsed: LegacyBridgeMode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, LegacyBridgeMode::ReadOnly);
    }

    #[test]
    fn display_matches_canonical_kebab() {
        assert_eq!(LegacyBridgeMode::ReadOnly.to_string(), "read-only");
        assert_eq!(LegacyBridgeMode::Migrate.to_string(), "migrate");
    }
}
