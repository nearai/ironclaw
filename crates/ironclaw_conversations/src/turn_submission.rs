//! The narrow turn-submission port this crate needs from the host.
//!
//! `ironclaw_conversations` owns the inbound orchestration — accepted-message
//! idempotency, binding resolution, replay, submit-key rotation — but it must
//! not depend on the turn kernel (`ironclaw_turns`) to run it. The orchestration
//! touches the coordinator at exactly **one** call site (`submit_turn`, inside
//! `submit_or_replay`), so that call site is declared here as a one-method port
//! and implemented above by `ironclaw_reborn_composition` over the
//! `TurnCoordinator` handle it already constructs. Dependency inversion:
//! declared below, implemented above (`.claude/rules/type-placement.md` §2).
//!
//! Everything the port speaks is either `ironclaw_host_api::turn` vocabulary or
//! declared here. Nothing in this module names the kernel.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::turn::{
    AcceptedMessageRef, IdempotencyKey, ReplyTargetBindingRef, RunOriginAdapter, RunProfileRequest,
    SourceBindingRef, SubmitTurnResponse, TurnActor, TurnScope, TurnSurfaceType,
};

/// Trust classification of the ingress that produced a submission.
///
/// This is **this crate's** value, not the kernel's. The distinction is the
/// trust decision the orchestration makes from its own typed binding policy —
/// never re-derived from the adapter-kind string — and it is what
/// `untrusted_trigger_adapter_records_product_inbound_not_scheduled_trigger`
/// pins: an untrusted ingress whose adapter is literally named `"trigger"` must
/// still classify as [`Untrusted`](Self::Untrusted).
///
/// The port implementor is what turns a classification into a product turn
/// origin; a `ScheduledTrigger` origin is reachable only from
/// [`TrustedTrigger`](Self::TrustedTrigger).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationInboundClassification {
    /// Trusted ingress whose adapter is the trusted-trigger adapter.
    TrustedTrigger,
    /// Trusted ingress, non-trigger adapter.
    TrustedOther,
    /// Untrusted ingress. Adapter identity is irrelevant and never mints a
    /// trigger origin.
    Untrusted,
}

/// One inbound turn submission, as the conversation orchestration describes it.
///
/// Every field is either `ironclaw_host_api::turn` vocabulary the accepted
/// message already carries, or the trust classification above. The implementor
/// supplies the kernel-side request shape and the constant lineage fields an
/// inbound submission never sets (no requested model, no requested run id, no
/// parent run, depth 0, no spawn-tree root).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationTurnSubmission {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub requested_run_profile: Option<RunProfileRequest>,
    pub idempotency_key: IdempotencyKey,
    pub received_at: DateTime<Utc>,
    pub classification: ConversationInboundClassification,
    /// The run-origin adapter the accepted message arrived on.
    pub origin_adapter: RunOriginAdapter,
    pub surface_type: Option<TurnSurfaceType>,
}

/// The retry/idempotency partition of a submission failure.
///
/// These are the only three classes the orchestration branches on, and they are
/// two independent facts about a failure: whether it is worth retrying, and
/// whether the accepted message's submit idempotency key must be rotated first.
/// The implementor maps its own denial cone onto them totally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnSubmissionRetry {
    /// Transient, and the submit idempotency key must be rotated before the
    /// next attempt so turn-store replay cannot strand the message.
    RetryableAfterKeyRotation,
    /// Transient, reusing the same submit idempotency key so the turn store can
    /// deduplicate the retry.
    RetryableWithSameKey,
    /// Permanent. Never rotate the key: duplicate retries of a permanently
    /// rejected submission must keep replaying the same terminal outcome.
    Permanent,
}

/// The boundary-facing category of a submission failure.
///
/// Declared here so this crate can classify and project a failure without
/// naming the kernel's error cone. It is deliberately the same partition the
/// turn boundary already exposes, so an implementor maps onto it without
/// inventing new statuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnSubmissionErrorCategory {
    ThreadBusy,
    AdmissionRejected,
    ScopeNotFound,
    Unauthorized,
    InvalidRequest,
    Unavailable,
    Conflict,
    CapacityExceeded,
}

/// A typed turn-submission failure.
///
/// It carries the two facts the orchestration branches on ([`retry`] and
/// [`category`]) plus the implementor's own rendered cause, which is preserved
/// verbatim for server-side diagnosis. It is a **typed** value, not a flattened
/// string: callers must branch on `retry()`/`category()` rather than parsing
/// [`Display`](std::fmt::Display).
///
/// [`retry`]: Self::retry
/// [`category`]: Self::category
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub struct TurnSubmissionError {
    category: TurnSubmissionErrorCategory,
    retry: TurnSubmissionRetry,
    detail: String,
}

impl TurnSubmissionError {
    /// Build a failure from the implementor's classification plus the rendered
    /// cause. `detail` is the source failure's own rendering and is never
    /// dropped.
    pub fn new(
        category: TurnSubmissionErrorCategory,
        retry: TurnSubmissionRetry,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            category,
            retry,
            detail: detail.into(),
        }
    }

    pub fn category(&self) -> TurnSubmissionErrorCategory {
        self.category
    }

    pub fn retry(&self) -> TurnSubmissionRetry {
        self.retry
    }

    /// The HTTP-shaped status an adapter surfaces for this failure. Derived
    /// from the category alone, so it cannot drift from it.
    pub fn adapter_status_code(&self) -> u16 {
        match self.category {
            TurnSubmissionErrorCategory::ThreadBusy | TurnSubmissionErrorCategory::Conflict => 409,
            TurnSubmissionErrorCategory::AdmissionRejected
            | TurnSubmissionErrorCategory::CapacityExceeded => 429,
            TurnSubmissionErrorCategory::ScopeNotFound => 404,
            TurnSubmissionErrorCategory::Unauthorized => 403,
            TurnSubmissionErrorCategory::InvalidRequest => 400,
            TurnSubmissionErrorCategory::Unavailable => 503,
        }
    }
}

/// The one host capability the conversation inbound orchestration needs.
///
/// Implemented by `ironclaw_reborn_composition` over the real turn coordinator.
#[async_trait]
pub trait ConversationTurnSubmitter: Send + Sync {
    async fn submit_conversation_turn(
        &self,
        submission: ConversationTurnSubmission,
    ) -> Result<SubmitTurnResponse, TurnSubmissionError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_status_code_is_derived_from_category_for_every_category() {
        for (category, expected) in [
            (TurnSubmissionErrorCategory::ThreadBusy, 409),
            (TurnSubmissionErrorCategory::Conflict, 409),
            (TurnSubmissionErrorCategory::AdmissionRejected, 429),
            (TurnSubmissionErrorCategory::CapacityExceeded, 429),
            (TurnSubmissionErrorCategory::ScopeNotFound, 404),
            (TurnSubmissionErrorCategory::Unauthorized, 403),
            (TurnSubmissionErrorCategory::InvalidRequest, 400),
            (TurnSubmissionErrorCategory::Unavailable, 503),
        ] {
            let error = TurnSubmissionError::new(
                category,
                TurnSubmissionRetry::Permanent,
                "boom".to_string(),
            );
            assert_eq!(error.category(), category);
            assert_eq!(error.adapter_status_code(), expected);
        }
    }

    #[test]
    fn rendered_cause_is_preserved_verbatim() {
        let error = TurnSubmissionError::new(
            TurnSubmissionErrorCategory::Unavailable,
            TurnSubmissionRetry::RetryableAfterKeyRotation,
            "turn service unavailable: turn store unavailable",
        );
        assert_eq!(
            error.to_string(),
            "turn service unavailable: turn store unavailable",
            "the source's rendered cause must survive the port boundary"
        );
        assert_eq!(
            error.retry(),
            TurnSubmissionRetry::RetryableAfterKeyRotation
        );
    }
}
