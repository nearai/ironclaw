//! Failure-policy taxonomy.
//!
//! When a hook misbehaves, the dispatcher classifies the failure into one of
//! the [`FailureCategory`] variants and applies a [`FailureDisposition`] that
//! depends on both the category *and* the kind of decision the hook was meant
//! to produce. The rule is:
//!
//! - Gate / Mutator failures **fail closed** — the dispatcher behaves as if
//!   the hook had returned the most restrictive decision it can mint.
//! - Observer / Effect failures **fail isolated** — the dispatcher drops the
//!   result and emits an audit record.
//!
//! In both cases, the hook's slot in the registry is **poisoned for the rest
//! of the current turn run**. A flapping hook does not silently downgrade to
//! permissive behavior on the next iteration.

use serde::{Deserialize, Serialize};

use crate::trust::DecisionKind;

/// Categorization of a hook's misbehavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    /// The hook exceeded its dispatch budget.
    Timeout,
    /// The hook panicked or otherwise crashed during invocation.
    Panic,
    /// The hook returned a value that does not match the dispatch contract
    /// (wrong decision kind for the point, invalid patch, etc.).
    Malformed,
    /// The hook attempted to mint a decision its trust class does not permit
    /// (should be unreachable when the sink traits are used; the variant
    /// exists for the future WASM surface that bypasses Rust's type checker).
    AttenuationViolation,
}

/// What the dispatcher does in response to a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureDisposition {
    /// Treat the hook as if it had produced the most restrictive decision it
    /// can mint. For `Gate`, this is `Deny`. For `Mutator`, this is "no patch
    /// applied." Hook slot poisoned for the rest of the run.
    FailClosed,
    /// Drop the result, emit an audit record. Continue execution. Hook slot
    /// poisoned for the rest of the run.
    FailIsolated,
}

impl FailureCategory {
    /// Disposition for a hook of the given decision kind when this category
    /// of failure occurs.
    pub fn disposition_for(self, kind: DecisionKind) -> FailureDisposition {
        match kind {
            DecisionKind::Gate | DecisionKind::Mutator => FailureDisposition::FailClosed,
            DecisionKind::Observer | DecisionKind::Effect => FailureDisposition::FailIsolated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_and_mutator_fail_closed() {
        for category in [
            FailureCategory::Timeout,
            FailureCategory::Panic,
            FailureCategory::Malformed,
            FailureCategory::AttenuationViolation,
        ] {
            assert_eq!(
                category.disposition_for(DecisionKind::Gate),
                FailureDisposition::FailClosed
            );
            assert_eq!(
                category.disposition_for(DecisionKind::Mutator),
                FailureDisposition::FailClosed
            );
        }
    }

    #[test]
    fn observer_and_effect_fail_isolated() {
        for category in [
            FailureCategory::Timeout,
            FailureCategory::Panic,
            FailureCategory::Malformed,
        ] {
            assert_eq!(
                category.disposition_for(DecisionKind::Observer),
                FailureDisposition::FailIsolated
            );
            assert_eq!(
                category.disposition_for(DecisionKind::Effect),
                FailureDisposition::FailIsolated
            );
        }
    }
}
