//! Observer facts emitted by `Observer` hooks.
//!
//! Observers cannot change driver-visible outcomes. The dispatcher collects
//! their facts and forwards them to the audit/observability backend. As with
//! gates and mutators, the type is sealed: only the sink path can mint an
//! `ObserverFact`, so a Trusted/Installed hook cannot smuggle in a payload
//! that bypasses the redaction policy.

use crate::error::SanitizedReason;

/// A fact observed by a hook. Routed to audit/observability, never to
/// driver state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserverFact {
    pub(crate) inner: ObserverFactInner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ObserverFactInner {
    /// A bare structured note keyed by a static category. Most observer hooks
    /// will use this; richer event shapes can be added as the integration
    /// surface grows.
    Note {
        category: NoteCategory,
        summary: SanitizedReason,
    },
}

/// Closed vocabulary of observer-note categories. Limits the surface a
/// misbehaving observer can use to flood audit logs with adversarial labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteCategory {
    HookFired,
    HookSkipped,
    HookSlow,
    HookProtocolViolation,
}

impl ObserverFact {
    pub(crate) fn note(category: NoteCategory, summary: SanitizedReason) -> Self {
        Self {
            inner: ObserverFactInner::Note { category, summary },
        }
    }

    pub fn view(&self) -> ObserverFactView<'_> {
        match &self.inner {
            ObserverFactInner::Note { category, summary } => ObserverFactView::Note {
                category: *category,
                summary,
            },
        }
    }
}

#[derive(Debug)]
pub enum ObserverFactView<'a> {
    Note {
        category: NoteCategory,
        summary: &'a SanitizedReason,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_round_trips() {
        let fact = ObserverFact::note(
            NoteCategory::HookFired,
            SanitizedReason::from_static("alpha"),
        );
        match fact.view() {
            ObserverFactView::Note { category, summary } => {
                assert_eq!(category, NoteCategory::HookFired);
                assert_eq!(summary.as_str(), "alpha");
            }
        }
    }
}
