use ironclaw_host_api::Timestamp;

use crate::{SanitizedFailureReason, TriggerError, TriggerRecord};

use super::TriggerPollerFailureReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SubmitFailureKind {
    Retryable,
    Permanent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FireFailureDisposition {
    Retryable,
    PermanentReschedule(Timestamp),
    PermanentTerminal,
}

impl FireFailureDisposition {
    pub(super) fn from_kind(kind: SubmitFailureKind, next_run_at: Timestamp) -> Self {
        match kind {
            SubmitFailureKind::Retryable => Self::Retryable,
            SubmitFailureKind::Permanent => Self::PermanentReschedule(next_run_at),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FailureClassification {
    pub(super) kind: SubmitFailureKind,
    pub(super) reason: TriggerPollerFailureReason,
    /// Sanitized human-readable detail persisted on the error run-history row,
    /// derived from the error's own (already boundary-mapped) reason text, with
    /// the category label as a fallback. Never carries internal ids/paths — the
    /// reason text is chosen at the mapping boundary (see
    /// `trigger_poller_trusted_submit::trigger_authorization_error`).
    pub(super) detail: Option<SanitizedFailureReason>,
}

pub(super) fn classify_failure(error: &TriggerError) -> FailureClassification {
    let (kind, reason) = match error {
        TriggerError::Backend { .. } => (
            SubmitFailureKind::Retryable,
            TriggerPollerFailureReason::Backend,
        ),
        TriggerError::InvalidTriggerId { .. } => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::InvalidTriggerId,
        ),
        TriggerError::InvalidFireIdentityComponent { .. } => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::InvalidFireIdentityComponent,
        ),
        TriggerError::InvalidRecord { .. } => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::InvalidRecord,
        ),
        TriggerError::InvalidPollerConfig { .. } => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::InvalidPollerConfig,
        ),
        TriggerError::InvalidSchedule { .. } => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::InvalidSchedule,
        ),
        TriggerError::InvalidMaterialization { .. } => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::InvalidMaterialization,
        ),
        TriggerError::NotFound => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::NotFound,
        ),
    };
    let detail = trigger_error_reason_text(error)
        .and_then(SanitizedFailureReason::sanitize)
        .or_else(|| SanitizedFailureReason::sanitize(failure_category_label(reason)));
    FailureClassification {
        kind,
        reason,
        detail,
    }
}

/// The already-mapped reason text carried on a [`TriggerError`], if any. These
/// strings are chosen at the error's construction boundary and must stay free
/// of internal ids/paths; [`SanitizedFailureReason`] is the length/charset
/// backstop over them.
fn trigger_error_reason_text(error: &TriggerError) -> Option<&str> {
    match error {
        // `Backend.reason` is built by `backend_error()` as
        // `format!("{operation}: {raw libsql/pg error}")`, which can carry SQL
        // fragments, file paths, or connection detail. Drop it and fall back to
        // the curated category label ("trigger backend temporarily
        // unavailable") so raw backend errors never reach the persisted reason.
        TriggerError::Backend { .. } | TriggerError::NotFound => None,
        TriggerError::InvalidTriggerId { reason }
        | TriggerError::InvalidFireIdentityComponent { reason, .. }
        | TriggerError::InvalidRecord { reason }
        | TriggerError::InvalidPollerConfig { reason }
        | TriggerError::InvalidSchedule { reason }
        | TriggerError::InvalidMaterialization { reason } => Some(reason.as_str()),
    }
}

/// Sanitized detail for a failure category that carries no underlying error
/// (e.g. `SourceNoFire`), so every persisted error row has a human reason.
pub(super) fn category_detail(
    reason: TriggerPollerFailureReason,
) -> Option<SanitizedFailureReason> {
    SanitizedFailureReason::sanitize(failure_category_label(reason))
}

/// Stable human label for a failure category, used when the underlying error
/// carried no reason text (e.g. `NotFound`, `SourceNoFire`).
fn failure_category_label(reason: TriggerPollerFailureReason) -> &'static str {
    match reason {
        TriggerPollerFailureReason::Backend => "trigger backend temporarily unavailable",
        TriggerPollerFailureReason::InvalidTriggerId => "trigger has an invalid id",
        TriggerPollerFailureReason::InvalidFireIdentityComponent => {
            "trigger fire identity is invalid"
        }
        TriggerPollerFailureReason::InvalidRecord => "trigger record is invalid",
        TriggerPollerFailureReason::InvalidPollerConfig => "trigger poller config is invalid",
        TriggerPollerFailureReason::InvalidSchedule => "trigger schedule is invalid",
        TriggerPollerFailureReason::InvalidMaterialization => {
            "trigger prompt could not be prepared"
        }
        TriggerPollerFailureReason::NotFound => "trigger was not found",
        TriggerPollerFailureReason::SourceNoFire => "trigger source produced no fire",
        TriggerPollerFailureReason::ActiveRunLookup => "active run lookup failed",
    }
}

pub(super) fn next_run_at_after_fire(
    record: &TriggerRecord,
    fire_slot: Timestamp,
) -> Result<Timestamp, TriggerError> {
    record
        .schedule
        .next_slot_after(fire_slot)?
        .ok_or_else(|| TriggerError::InvalidSchedule {
            reason: "schedule has no next fire slot after claimed fire".to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Locks F1: a `Backend` error's raw reason (which `backend_error()` builds
    /// as `"{operation}: {raw libsql/pg error}"`) must NOT leak into the
    /// persisted detail. The classifier must fall back to the curated category
    /// label instead.
    #[test]
    fn classify_backend_error_uses_curated_label_not_raw_reason() {
        let classification = classify_failure(&TriggerError::Backend {
            reason: "connect trigger repository: no such file /tmp/db.sqlite (code 14)".to_string(),
        });
        assert_eq!(classification.kind, SubmitFailureKind::Retryable);
        assert_eq!(classification.reason, TriggerPollerFailureReason::Backend);
        assert_eq!(
            classification.detail.as_ref().map(|d| d.as_str()),
            Some("trigger backend temporarily unavailable")
        );
    }

    /// A curated `InvalidMaterialization` reason (chosen at the mapping boundary)
    /// is preserved verbatim through the classifier.
    #[test]
    fn classify_invalid_materialization_preserves_curated_reason() {
        let classification = classify_failure(&TriggerError::InvalidMaterialization {
            reason: "trigger fire not authorized: creator access revoked".to_string(),
        });
        assert_eq!(classification.kind, SubmitFailureKind::Permanent);
        assert_eq!(
            classification.reason,
            TriggerPollerFailureReason::InvalidMaterialization
        );
        assert_eq!(
            classification.detail.as_ref().map(|d| d.as_str()),
            Some("trigger fire not authorized: creator access revoked")
        );
    }

    /// `category_detail` returns the curated label for a category that carries
    /// no underlying error reason (e.g. `SourceNoFire`).
    #[test]
    fn category_detail_returns_curated_label_for_source_no_fire() {
        let detail = category_detail(TriggerPollerFailureReason::SourceNoFire)
            .expect("source-no-fire has a curated label");
        assert_eq!(detail.as_str(), "trigger source produced no fire");
    }

    /// `NotFound` carries no reason text, so the classifier falls back to the
    /// curated label.
    #[test]
    fn classify_not_found_uses_curated_label() {
        let classification = classify_failure(&TriggerError::NotFound);
        assert_eq!(
            classification.detail.as_ref().map(|d| d.as_str()),
            Some("trigger was not found")
        );
    }
}
