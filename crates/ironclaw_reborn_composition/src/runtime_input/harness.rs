//! Harness selection input (forward-compat for epic #3036).
//!
//! Epic #3036 ("Configuration-as-Code") introduces `HarnessManifest` —
//! a named composition of extensions + skills + prompt overlay + runtime
//! constraints + capability-surface filter + memory schema + required
//! exit artifacts. At most one harness is active per session/thread.
//!
//! Harness substrate (typed `HarnessRepo`, activation service, prompt
//! overlay assembler, capability-surface filter that bumps surface
//! version) is **upstream work that does not exist yet**. This DTO ships
//! today so that:
//!
//! 1. The composed runtime's input contract carries a `harness` slot, so
//!    no `RebornRuntimeInput` shape change is required when the substrate
//!    lands.
//! 2. The CLI's harness-related subcommands (`harness install/list/...`)
//!    have a typed shape to read/write — they print
//!    "not-yet-wired" today but accept the same identifiers they will
//!    accept after the substrate lands.
//! 3. The "active harness" concept appears explicitly in the audit/log
//!    surface from day one, so operators have a stable mental model.
//!
//! `RebornHarnessId` is defined here locally rather than imported from a
//! future `ironclaw_harness` crate; once that crate lands, this type
//! becomes a re-export. The string identifier shape (kebab-case, name
//! segment validation) is chosen to match the v1 extension/skill id
//! pattern so a future migration is purely a re-export, not a value
//! transformation.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Identifier for a Reborn use-case harness as described in epic #3036.
/// Same syntactic rules as `ironclaw_host_api::PackageId`: lowercase ASCII
/// letters, digits, hyphens; must start with a letter; max 64 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RebornHarnessId(String);

impl RebornHarnessId {
    pub fn new(value: impl Into<String>) -> Result<Self, RebornHarnessIdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(RebornHarnessIdError::Empty);
        }
        if value.len() > 64 {
            return Err(RebornHarnessIdError::TooLong { len: value.len() });
        }
        if !value.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(RebornHarnessIdError::InvalidStart);
        }
        for character in value.chars() {
            if !(character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-')
            {
                return Err(RebornHarnessIdError::InvalidChar { character });
            }
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for RebornHarnessId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for RebornHarnessId {
    type Error = RebornHarnessIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RebornHarnessId> for String {
    fn from(value: RebornHarnessId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RebornHarnessIdError {
    #[error("harness id must not be empty")]
    Empty,
    #[error("harness id is {len} bytes; must be at most 64")]
    TooLong { len: usize },
    #[error("harness id must start with an ASCII lowercase letter")]
    InvalidStart,
    #[error(
        "harness id may only contain ASCII lowercase letters, digits, or hyphens; got `{character}`"
    )]
    InvalidChar { character: char },
}

/// Default harness selection applied to new conversations on the runtime.
///
/// `None` (the value carried inside `RebornRuntimeInput::harness`) means
/// "no harness overlay". A `Some(selection)` value asks the composition
/// root to record an intent that **today** is a no-op-with-warning: the
/// runtime logs the harness id once at boot and proceeds without an
/// overlay, because the harness substrate hasn't landed yet (epic #3036
/// sub-issue "HarnessManifest"). When the substrate lands, the same
/// field starts driving `HarnessActivationService::activate` and the
/// active harness flows into `build_instruction_bundle` and
/// `visible_capabilities`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornHarnessSelection {
    pub harness_id: RebornHarnessId,
}

impl RebornHarnessSelection {
    pub fn new(harness_id: RebornHarnessId) -> Self {
        Self { harness_id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_ids_accepted() {
        for ok in ["red-team", "chain-incident-response", "x1", "alpha-beta-9"] {
            assert!(RebornHarnessId::new(ok).is_ok(), "{ok} should be valid");
        }
    }

    #[test]
    fn invalid_ids_rejected() {
        assert!(matches!(
            RebornHarnessId::new(""),
            Err(RebornHarnessIdError::Empty)
        ));
        // Leading uppercase fails the start check (which runs before the
        // per-char loop), so this is `InvalidStart`, not `InvalidChar`.
        assert!(matches!(
            RebornHarnessId::new("Red-Team"),
            Err(RebornHarnessIdError::InvalidStart)
        ));
        assert!(matches!(
            RebornHarnessId::new("red_team"),
            Err(RebornHarnessIdError::InvalidChar { character: '_' })
        ));
        assert!(matches!(
            RebornHarnessId::new("9-leading-digit"),
            Err(RebornHarnessIdError::InvalidStart)
        ));
        let huge = "a".repeat(65);
        assert!(matches!(
            RebornHarnessId::new(huge),
            Err(RebornHarnessIdError::TooLong { len: 65 })
        ));
    }

    #[test]
    fn serde_round_trip() {
        let id = RebornHarnessId::new("red-team").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        let parsed: RebornHarnessId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn serde_rejects_invalid_string() {
        let result: Result<RebornHarnessId, _> = serde_json::from_str("\"BadId\"");
        assert!(result.is_err());
    }
}
