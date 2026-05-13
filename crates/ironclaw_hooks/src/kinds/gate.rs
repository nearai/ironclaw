//! Gate decisions for the `before_capability` hook point.
//!
//! The outer type [`BeforeCapabilityHookDecision`] is `pub` so callers can
//! match on it for read-only inspection, but the inner enum is `pub(crate)`
//! and the constructors are `pub(crate)`. The sink traits in
//! [`crate::sink`] are the only public path that mints decisions, and the
//! `InstalledHookSink` impl deliberately does not expose `allow` — an
//! `Installed`-tier hook cannot mint a permissive override.

use crate::error::SanitizedReason;

/// Decision returned by a `before_capability` hook. Sealed; constructable only
/// via the sink traits in [`crate::sink`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeforeCapabilityHookDecision {
    pub(crate) inner: GateDecisionInner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GateDecisionInner {
    /// Allow the capability invocation to proceed. Only Builtin and Trusted
    /// hooks may produce this variant.
    Allow,
    /// Deny the capability invocation. Fail-closed for all trust tiers.
    Deny { reason: SanitizedReason },
    /// Pause the run waiting for explicit user approval through the host's
    /// approval channel. The dispatcher promotes this to
    /// `CapabilityOutcome::ApprovalRequired` on the way out.
    PauseApproval { reason: SanitizedReason },
    /// Pause the run waiting for the user to complete an auth flow.
    PauseAuth { reason: SanitizedReason },
}

impl BeforeCapabilityHookDecision {
    pub(crate) fn allow() -> Self {
        Self {
            inner: GateDecisionInner::Allow,
        }
    }

    pub(crate) fn deny(reason: SanitizedReason) -> Self {
        Self {
            inner: GateDecisionInner::Deny { reason },
        }
    }

    pub(crate) fn pause_approval(reason: SanitizedReason) -> Self {
        Self {
            inner: GateDecisionInner::PauseApproval { reason },
        }
    }

    pub(crate) fn pause_auth(reason: SanitizedReason) -> Self {
        Self {
            inner: GateDecisionInner::PauseAuth { reason },
        }
    }

    /// Public read-only view for callers needing to react to the decision (the
    /// dispatcher, the Reborn middleware that translates into
    /// `CapabilityOutcome`).
    pub fn view(&self) -> GateDecisionView<'_> {
        match &self.inner {
            GateDecisionInner::Allow => GateDecisionView::Allow,
            GateDecisionInner::Deny { reason } => GateDecisionView::Deny { reason },
            GateDecisionInner::PauseApproval { reason } => {
                GateDecisionView::PauseApproval { reason }
            }
            GateDecisionInner::PauseAuth { reason } => GateDecisionView::PauseAuth { reason },
        }
    }

    /// `true` if the decision permits the capability to execute. Convenience
    /// wrapper around `view()`.
    pub fn permits(&self) -> bool {
        matches!(self.inner, GateDecisionInner::Allow)
    }
}

/// Read-only public projection of a gate decision. Carries borrowed references
/// to the underlying reason payloads so consumers don't need to clone.
#[derive(Debug)]
pub enum GateDecisionView<'a> {
    Allow,
    Deny { reason: &'a SanitizedReason },
    PauseApproval { reason: &'a SanitizedReason },
    PauseAuth { reason: &'a SanitizedReason },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_permits() {
        let d = BeforeCapabilityHookDecision::allow();
        assert!(d.permits());
        assert!(matches!(d.view(), GateDecisionView::Allow));
    }

    #[test]
    fn deny_does_not_permit() {
        let d = BeforeCapabilityHookDecision::deny(SanitizedReason::from_static("over budget"));
        assert!(!d.permits());
        match d.view() {
            GateDecisionView::Deny { reason } => assert_eq!(reason.as_str(), "over budget"),
            other => panic!("unexpected view: {other:?}"),
        }
    }

    #[test]
    fn pause_variants_do_not_permit() {
        for d in [
            BeforeCapabilityHookDecision::pause_approval(SanitizedReason::from_static("need ok")),
            BeforeCapabilityHookDecision::pause_auth(SanitizedReason::from_static("need auth")),
        ] {
            assert!(!d.permits());
        }
    }
}
