//! Versioned input contract for the assembled Reborn runtime.
//!
//! The composition root takes an explicit `api_version` stamp on
//! `RebornRuntimeInput` so a future major schema bump can fail closed at
//! `build_reborn_runtime` instead of silently dropping fields the caller
//! intended to set.
//!
//! This mirrors the `api_version = "ironclaw.config/v1"` discipline epic
//! #3036 ("Configuration-as-Code") puts on declarative blueprints: every
//! durable schema entry point is version-stamped, and unknown majors are a
//! hard error, not a silent migration.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Major/minor version of the `RebornRuntimeInput` contract.
///
/// Compatibility rule: the composition root accepts inputs whose `major`
/// equals the composition's compiled `Self::current().major`, regardless of
/// minor. A bump in `minor` reflects backward-compatible field additions;
/// a bump in `major` requires a `try_from` migration path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RebornRuntimeApiVersion {
    major: u32,
    minor: u32,
}

impl RebornRuntimeApiVersion {
    /// First and currently-only versioned shape.
    pub const V1: Self = Self { major: 1, minor: 0 };

    /// The version this build of the composition root speaks.
    pub const fn current() -> Self {
        Self::V1
    }

    pub const fn major(&self) -> u32 {
        self.major
    }

    pub const fn minor(&self) -> u32 {
        self.minor
    }

    /// Returns `Ok` if this version is compatible with `target` per the
    /// major-equal / minor-tolerant rule. Forward-compatibility means a
    /// newer minor on `target` (the composition root version) is fine; a
    /// newer minor on `self` (the caller) is also fine because composition
    /// would simply not exercise fields it doesn't know about — but a
    /// different major fails closed.
    pub fn compatible_with(&self, target: Self) -> Result<(), RebornRuntimeApiVersionError> {
        if self.major == target.major {
            Ok(())
        } else {
            Err(RebornRuntimeApiVersionError::IncompatibleMajor {
                caller: *self,
                composition: target,
            })
        }
    }
}

impl Default for RebornRuntimeApiVersion {
    fn default() -> Self {
        Self::current()
    }
}

impl fmt::Display for RebornRuntimeApiVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ironclaw.runtime/v{}.{}", self.major, self.minor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RebornRuntimeApiVersionError {
    #[error(
        "caller passed api_version {caller}, but this composition root speaks {composition}; \
         major mismatch is a fail-closed error (no implicit migration)"
    )]
    IncompatibleMajor {
        caller: RebornRuntimeApiVersion,
        composition: RebornRuntimeApiVersion,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_is_v1() {
        assert_eq!(RebornRuntimeApiVersion::current(), RebornRuntimeApiVersion::V1);
        assert_eq!(RebornRuntimeApiVersion::current().major(), 1);
    }

    #[test]
    fn same_major_is_compatible() {
        let caller = RebornRuntimeApiVersion { major: 1, minor: 7 };
        assert!(
            caller
                .compatible_with(RebornRuntimeApiVersion::V1)
                .is_ok()
        );
    }

    #[test]
    fn different_major_fails_closed() {
        let caller = RebornRuntimeApiVersion { major: 2, minor: 0 };
        let err = caller
            .compatible_with(RebornRuntimeApiVersion::V1)
            .expect_err("major bump must fail");
        assert!(matches!(
            err,
            RebornRuntimeApiVersionError::IncompatibleMajor { .. }
        ));
    }

    #[test]
    fn display_format_is_wire_stable() {
        assert_eq!(
            RebornRuntimeApiVersion::V1.to_string(),
            "ironclaw.runtime/v1.0"
        );
    }
}
