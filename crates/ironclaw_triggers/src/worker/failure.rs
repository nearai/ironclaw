use crate::TriggerError;

use super::TriggerPollerFailureReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SubmitFailureKind {
    Retryable,
    Permanent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FailureClassification {
    pub(super) kind: SubmitFailureKind,
    pub(super) reason: TriggerPollerFailureReason,
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
    FailureClassification { kind, reason }
}
