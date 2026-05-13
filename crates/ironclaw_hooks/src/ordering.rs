//! Deterministic ordering of hooks at a point.
//!
//! Two-level: **phase** (coarse, restricted by trust class) → **priority**
//! (fine, author-chosen) → **hook id** (stable tiebreak). The phases for
//! gate-class points are:
//!
//! 1. [`HookPhase::Validation`] — input shape, schema, well-formedness.
//!    Builtin only.
//! 2. [`HookPhase::Authorization`] — capability is in user's surface; no
//!    scope violation. Builtin only.
//! 3. [`HookPhase::Policy`] — restrictive policy hooks (e.g., "no shell
//!    exec while on mobile"). Trusted and Installed (with grant).
//! 4. [`HookPhase::Telemetry`] — observers; always run regardless of
//!    short-circuiting in earlier phases so audit consumers can see the
//!    final decision.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::identity::HookId;
use crate::trust::HookTrustClass;

/// Coarse-grained phase for hook ordering at a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPhase {
    Validation,
    Authorization,
    Policy,
    Telemetry,
}

impl HookPhase {
    /// `true` if a hook of the given trust class is permitted to register at
    /// this phase. Validation and Authorization are reserved for Builtin
    /// hooks because they enforce host-defined contracts; Policy is open to
    /// Trusted and (grant-gated) Installed.
    pub fn permits_trust(self, trust: HookTrustClass) -> bool {
        match self {
            Self::Validation | Self::Authorization => matches!(trust, HookTrustClass::Builtin),
            Self::Policy => true,
            Self::Telemetry => true,
        }
    }
}

/// Author-chosen priority within a phase. Lower numbers run first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HookPriority(pub i32);

impl HookPriority {
    pub const DEFAULT: Self = Self(100);
    pub const FIRST: Self = Self(0);
    pub const LAST: Self = Self(i32::MAX);
}

/// Sort key combining phase, priority, and hook id. The hook-id component
/// makes the order *stable*: two hooks at the same phase + priority always
/// sort the same way across runs.
#[derive(Debug, Clone, Copy)]
pub struct HookOrderKey {
    pub phase: HookPhase,
    pub priority: HookPriority,
    pub hook_id: HookId,
}

impl HookOrderKey {
    pub fn new(phase: HookPhase, priority: HookPriority, hook_id: HookId) -> Self {
        Self {
            phase,
            priority,
            hook_id,
        }
    }
}

impl PartialEq for HookOrderKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for HookOrderKey {}

impl PartialOrd for HookOrderKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HookOrderKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.phase
            .cmp(&other.phase)
            .then(self.priority.cmp(&other.priority))
            .then_with(|| self.hook_id.as_bytes().cmp(other.hook_id.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::HookVersion;

    fn key(phase: HookPhase, priority: i32, builtin_path: &str) -> HookOrderKey {
        HookOrderKey::new(
            phase,
            HookPriority(priority),
            HookId::for_builtin(builtin_path, HookVersion::ONE),
        )
    }

    #[test]
    fn phase_orders_before_priority() {
        let a = key(HookPhase::Validation, 500, "a");
        let b = key(HookPhase::Authorization, 0, "b");
        assert!(a < b);
    }

    #[test]
    fn priority_orders_before_hook_id() {
        let a = key(HookPhase::Policy, 0, "z");
        let b = key(HookPhase::Policy, 100, "a");
        assert!(a < b);
    }

    #[test]
    fn hook_id_breaks_ties_stably() {
        let a = key(HookPhase::Telemetry, 100, "alpha");
        let b = key(HookPhase::Telemetry, 100, "beta");
        let ordering = a.cmp(&b);
        // Whichever way alpha vs beta sort, it must be deterministic.
        assert_ne!(ordering, Ordering::Equal);
        assert_eq!(a.cmp(&b), ordering);
    }

    #[test]
    fn validation_phase_restricted_to_builtin() {
        assert!(HookPhase::Validation.permits_trust(HookTrustClass::Builtin));
        assert!(!HookPhase::Validation.permits_trust(HookTrustClass::Trusted));
        assert!(!HookPhase::Validation.permits_trust(HookTrustClass::Installed));
    }

    #[test]
    fn policy_phase_open_to_all() {
        for class in [
            HookTrustClass::Builtin,
            HookTrustClass::Trusted,
            HookTrustClass::Installed,
        ] {
            assert!(HookPhase::Policy.permits_trust(class));
        }
    }
}
