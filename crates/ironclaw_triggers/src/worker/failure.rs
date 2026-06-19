use ironclaw_host_api::Timestamp;

use crate::{TriggerError, TriggerRecord};

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
