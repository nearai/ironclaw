//! Hook trust classes and the attenuation rules attached to them.
//!
//! Trust class is *fixed by source*, never declarable in a manifest. The
//! registry installer is the only thing that assigns trust class; everywhere
//! else in the code, it is read-only metadata.

use serde::{Deserialize, Serialize};

/// Where a hook came from. Determines what decision kinds the hook may produce
/// and what hook points it may register at.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookTrustClass {
    /// Compiled into IronClaw; identity is a canonical path + symbol. Full
    /// authority within the hook framework.
    Builtin,
    /// User-placed in `~/.ironclaw/hooks/` or workspace `hooks/`. Cannot
    /// register at `runtime`-class points (the inner side of capability
    /// attenuation); otherwise has the same decision authority as `Builtin`.
    Trusted,
    /// Loaded from the extension registry. Restricted to `Observer` and
    /// `Effect` kinds by default; `Gate` and `Mutator` require an explicit
    /// per-extension grant captured in the registry binding. The
    /// `InstalledHookSink` trait exposes only monotonic-restriction
    /// constructors so an Installed hook cannot mint `Allow`.
    Installed,
    /// Hook authored at runtime by the agent itself (e.g., in response to a
    /// near-miss or repetition). Same default kind permissions as
    /// `Installed` — `Observer` / `Effect` only by default; `Gate` / `Mutator`
    /// require an explicit grant. Because the agent cannot mint persistent
    /// grants for itself, self-authored gates and mutators are only ever
    /// authorized through the *run-scoped* registration path. Durable
    /// self-authored hooks require the unforgeable channel from #3564 and
    /// are not yet implemented.
    SelfAuthored,
}

impl HookTrustClass {
    /// Whether this class is allowed to produce decisions of the given kind by
    /// default (i.e., without an explicit grant). Mirrored by the sink trait
    /// surface so the answer is also checked at compile time, not just here.
    pub fn permits_kind_by_default(self, kind: DecisionKind) -> bool {
        match (self, kind) {
            (Self::Builtin, _) => true,
            (Self::Trusted, _) => true,
            (Self::Installed, DecisionKind::Observer) => true,
            (Self::Installed, DecisionKind::Effect) => true,
            (Self::Installed, DecisionKind::Gate) => false,
            (Self::Installed, DecisionKind::Mutator) => false,
            (Self::SelfAuthored, DecisionKind::Observer) => true,
            (Self::SelfAuthored, DecisionKind::Effect) => true,
            (Self::SelfAuthored, DecisionKind::Gate) => false,
            (Self::SelfAuthored, DecisionKind::Mutator) => false,
        }
    }
}

/// Coarse-grained classification of what a hook returns. Used by the
/// dispatcher's attenuation check and by the registry's grant model. The fine
/// per-point decision types live in [`crate::kinds`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    /// Allows/denies/pauses a behavior. Fail-closed on protocol violation.
    Gate,
    /// Mutates context delivered to the model. Fail-closed on protocol
    /// violation. Always additive and envelope-wrapped for untrusted authors.
    Mutator,
    /// Observes a fact but does not change driver-visible outcomes.
    Observer,
    /// Enqueues a side effect after a durable event. Routes through normal
    /// capability dispatch; never gains ambient authority.
    Effect,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_default_permits_only_observer_and_effect() {
        assert!(HookTrustClass::Installed.permits_kind_by_default(DecisionKind::Observer));
        assert!(HookTrustClass::Installed.permits_kind_by_default(DecisionKind::Effect));
        assert!(!HookTrustClass::Installed.permits_kind_by_default(DecisionKind::Gate));
        assert!(!HookTrustClass::Installed.permits_kind_by_default(DecisionKind::Mutator));
    }

    #[test]
    fn self_authored_mirrors_installed_default_kind_permissions() {
        // Self-authored hooks have the same default kind permissions as
        // Installed — Gate/Mutator require an explicit grant, which for
        // self-authored only ever comes via run-scoped registration.
        assert!(HookTrustClass::SelfAuthored.permits_kind_by_default(DecisionKind::Observer));
        assert!(HookTrustClass::SelfAuthored.permits_kind_by_default(DecisionKind::Effect));
        assert!(!HookTrustClass::SelfAuthored.permits_kind_by_default(DecisionKind::Gate));
        assert!(!HookTrustClass::SelfAuthored.permits_kind_by_default(DecisionKind::Mutator));
    }

    #[test]
    fn trusted_and_builtin_permit_all_kinds_by_default() {
        for class in [HookTrustClass::Trusted, HookTrustClass::Builtin] {
            for kind in [
                DecisionKind::Gate,
                DecisionKind::Mutator,
                DecisionKind::Observer,
                DecisionKind::Effect,
            ] {
                assert!(
                    class.permits_kind_by_default(kind),
                    "{class:?} should permit {kind:?}"
                );
            }
        }
    }
}
