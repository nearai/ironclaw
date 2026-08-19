//! Host adapter for `ironclaw_conversations`' turn-submission port.
//!
//! `ironclaw_conversations` owns the inbound orchestration but declares the one
//! coordinator call it makes as the `ConversationTurnSubmitter` port; this is
//! the implementation, over the `TurnCoordinator` handle composition already
//! constructs for the trigger poller. Nothing here is policy: it resolves the
//! conversation-declared classification into a `ProductTurnContext` and projects
//! the coordinator's `TurnError` onto the port's retry/category partition.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_conversations::{
    ConversationInboundClassification, ConversationTurnSubmission, ConversationTurnSubmitter,
    TurnSubmissionError, TurnSubmissionErrorCategory, TurnSubmissionRetry,
};
use ironclaw_host_api::turn::SubmitTurnResponse;
use ironclaw_turns::{
    AdmissionRejectionReason, SubmitTurnRequest, TurnCoordinator, TurnError, product_context,
};

/// Build the host adapter for `ironclaw_conversations`' turn-submission port.
///
/// The one public seam of this module. Composition's own trigger-poller
/// assembly uses it, and so does out-of-crate test support that hand-wires
/// `trusted_trigger_fire_submitter` — module-owned initialization, so no caller
/// re-derives the classification/error projections below.
pub fn conversation_turn_submitter(
    coordinator: Arc<dyn TurnCoordinator>,
) -> Arc<dyn ConversationTurnSubmitter> {
    Arc::new(CoordinatorTurnSubmitter::new(coordinator))
}

/// The port implementation. Holds the coordinator handle so
/// `ironclaw_conversations` does not have to.
pub(crate) struct CoordinatorTurnSubmitter {
    coordinator: Arc<dyn TurnCoordinator>,
}

impl CoordinatorTurnSubmitter {
    pub(crate) fn new(coordinator: Arc<dyn TurnCoordinator>) -> Self {
        Self { coordinator }
    }
}

#[async_trait]
impl ConversationTurnSubmitter for CoordinatorTurnSubmitter {
    async fn submit_conversation_turn(
        &self,
        submission: ConversationTurnSubmission,
    ) -> Result<SubmitTurnResponse, TurnSubmissionError> {
        self.coordinator
            .submit_turn(coordinator_submit_request(submission))
            .await
            .map_err(turn_submission_error)
    }
}

/// Build the kernel request. An inbound submission never requests a model, a
/// run id, or a parent/spawn-tree lineage, so those stay at their empty values;
/// the product context is resolved from the classification the conversation
/// orchestration decided, never re-derived from the adapter identity.
fn coordinator_submit_request(submission: ConversationTurnSubmission) -> SubmitTurnRequest {
    let is_trusted_trigger = matches!(
        submission.classification,
        ConversationInboundClassification::TrustedTrigger
    );
    let mut product_context = product_context::resolve_inbound(
        inbound_classification(submission.classification),
        submission.origin_adapter,
        submission.surface_type,
        submission.scope.product_owner(&submission.actor),
    );
    if is_trusted_trigger {
        product_context.execution_policy = submission.execution_policy;
    }
    SubmitTurnRequest {
        subagent_activation_provenance: None,
        requested_model: None,
        scope: submission.scope,
        actor: submission.actor,
        accepted_message_ref: submission.accepted_message_ref,
        requested_run_profile: submission.requested_run_profile,
        output_contract: None,
        idempotency_key: submission.idempotency_key,
        received_at: submission.received_at,
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: Some(product_context),
    }
}

fn inbound_classification(
    classification: ConversationInboundClassification,
) -> product_context::InboundClassification {
    match classification {
        ConversationInboundClassification::TrustedTrigger => {
            product_context::InboundClassification::TrustedTrigger
        }
        ConversationInboundClassification::TrustedOther => {
            product_context::InboundClassification::TrustedOther
        }
        ConversationInboundClassification::Untrusted => {
            product_context::InboundClassification::Untrusted
        }
    }
}

/// Project a coordinator failure onto the port's vocabulary.
///
/// Total over `TurnError` by construction (no wildcard arm), and the rendered
/// cause is carried verbatim so nothing is lost server-side. The retry class is
/// **not** derivable from the category: `Conflict` covers both
/// `TurnError::Conflict` (retryable) and `LeaseMismatch`/`InvalidTransition`/
/// `RunNotRetryable` (permanent), which is why the port carries the two axes
/// separately.
pub(crate) fn turn_submission_error(error: TurnError) -> TurnSubmissionError {
    let detail = error.to_string();
    let (category, retry) = match &error {
        TurnError::ThreadBusy(_) => (
            TurnSubmissionErrorCategory::ThreadBusy,
            TurnSubmissionRetry::RetryableAfterKeyRotation,
        ),
        TurnError::Unavailable { .. } => (
            TurnSubmissionErrorCategory::Unavailable,
            TurnSubmissionRetry::RetryableAfterKeyRotation,
        ),
        TurnError::AdmissionRejected(rejection) => match rejection.reason {
            AdmissionRejectionReason::TenantLimit => (
                TurnSubmissionErrorCategory::AdmissionRejected,
                TurnSubmissionRetry::RetryableAfterKeyRotation,
            ),
            AdmissionRejectionReason::Unavailable => (
                TurnSubmissionErrorCategory::Unavailable,
                TurnSubmissionRetry::RetryableAfterKeyRotation,
            ),
            AdmissionRejectionReason::ProfileRejected => (
                TurnSubmissionErrorCategory::InvalidRequest,
                TurnSubmissionRetry::Permanent,
            ),
            // A thread that spent its autonomous-wake budget is parked pending
            // human attention, not permanently refused: the trigger poller may
            // retry, and a human activation restores the budget.
            AdmissionRejectionReason::SystemWakeStreak => (
                TurnSubmissionErrorCategory::AdmissionRejected,
                TurnSubmissionRetry::RetryableAfterKeyRotation,
            ),
            AdmissionRejectionReason::Policy | AdmissionRejectionReason::Unauthorized => (
                TurnSubmissionErrorCategory::Unauthorized,
                TurnSubmissionRetry::Permanent,
            ),
        },
        TurnError::CapacityExceeded { .. } => (
            TurnSubmissionErrorCategory::CapacityExceeded,
            TurnSubmissionRetry::RetryableWithSameKey,
        ),
        TurnError::Conflict { .. } => (
            TurnSubmissionErrorCategory::Conflict,
            TurnSubmissionRetry::RetryableWithSameKey,
        ),
        TurnError::ScopeNotFound => (
            TurnSubmissionErrorCategory::ScopeNotFound,
            TurnSubmissionRetry::Permanent,
        ),
        TurnError::Unauthorized => (
            TurnSubmissionErrorCategory::Unauthorized,
            TurnSubmissionRetry::Permanent,
        ),
        TurnError::InvalidRequest { .. } | TurnError::InvalidRunOriginAdapter => (
            TurnSubmissionErrorCategory::InvalidRequest,
            TurnSubmissionRetry::Permanent,
        ),
        TurnError::RunNotRetryable { .. }
        | TurnError::InvalidTransition { .. }
        | TurnError::LeaseMismatch => (
            TurnSubmissionErrorCategory::Conflict,
            TurnSubmissionRetry::Permanent,
        ),
    };
    TurnSubmissionError::new(category, retry, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::UserId;
    use ironclaw_host_api::turn::{
        AcceptedMessageRef, EventCursor, IdempotencyKey, RunOriginAdapter, TurnActor,
        TurnOriginKind, TurnRunId, TurnScope, TurnStatus, TurnSurfaceType,
    };
    use ironclaw_turns::{AdmissionRejection, ThreadBusy, TurnCapacityResource};

    /// Every `TurnError` the coordinator can return, with the class the port
    /// must put it in. This is the totality proof: the mapping has no wildcard
    /// arm, so a new `TurnError` variant fails to compile there, and this table
    /// pins that each existing one keeps its class. The three classes are the
    /// whole port error vocabulary the conversation orchestration branches on —
    /// `RetryableAfterKeyRotation` rotates the submit idempotency key,
    /// `RetryableWithSameKey` retries on the same key, `Permanent` never
    /// rotates and classifies as a submit rejection.
    fn turn_error_class_table() -> Vec<(TurnError, TurnSubmissionErrorCategory, TurnSubmissionRetry)>
    {
        vec![
            (
                TurnError::ThreadBusy(ThreadBusy {
                    active_run_id: TurnRunId::new(),
                    status: TurnStatus::Running,
                    event_cursor: EventCursor(7),
                }),
                TurnSubmissionErrorCategory::ThreadBusy,
                TurnSubmissionRetry::RetryableAfterKeyRotation,
            ),
            (
                TurnError::Unavailable {
                    reason: "turn store unavailable".to_string(),
                },
                TurnSubmissionErrorCategory::Unavailable,
                TurnSubmissionRetry::RetryableAfterKeyRotation,
            ),
            (
                TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::TenantLimit,
                )),
                TurnSubmissionErrorCategory::AdmissionRejected,
                TurnSubmissionRetry::RetryableAfterKeyRotation,
            ),
            (
                TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::Unavailable,
                )),
                TurnSubmissionErrorCategory::Unavailable,
                TurnSubmissionRetry::RetryableAfterKeyRotation,
            ),
            (
                TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::ProfileRejected,
                )),
                TurnSubmissionErrorCategory::InvalidRequest,
                TurnSubmissionRetry::Permanent,
            ),
            (
                TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::SystemWakeStreak,
                )),
                TurnSubmissionErrorCategory::AdmissionRejected,
                TurnSubmissionRetry::RetryableAfterKeyRotation,
            ),
            (
                TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::Policy,
                )),
                TurnSubmissionErrorCategory::Unauthorized,
                TurnSubmissionRetry::Permanent,
            ),
            (
                TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::Unauthorized,
                )),
                TurnSubmissionErrorCategory::Unauthorized,
                TurnSubmissionRetry::Permanent,
            ),
            (
                TurnError::CapacityExceeded {
                    resource: TurnCapacityResource::SubmitTurn,
                    cap: 1,
                },
                TurnSubmissionErrorCategory::CapacityExceeded,
                TurnSubmissionRetry::RetryableWithSameKey,
            ),
            (
                TurnError::Conflict {
                    reason: "cas mismatch".to_string(),
                },
                TurnSubmissionErrorCategory::Conflict,
                TurnSubmissionRetry::RetryableWithSameKey,
            ),
            (
                TurnError::ScopeNotFound,
                TurnSubmissionErrorCategory::ScopeNotFound,
                TurnSubmissionRetry::Permanent,
            ),
            (
                TurnError::Unauthorized,
                TurnSubmissionErrorCategory::Unauthorized,
                TurnSubmissionRetry::Permanent,
            ),
            (
                TurnError::InvalidRequest {
                    reason: "bad request".to_string(),
                },
                TurnSubmissionErrorCategory::InvalidRequest,
                TurnSubmissionRetry::Permanent,
            ),
            (
                TurnError::InvalidRunOriginAdapter,
                TurnSubmissionErrorCategory::InvalidRequest,
                TurnSubmissionRetry::Permanent,
            ),
            (
                TurnError::RunNotRetryable {
                    run_id: TurnRunId::new(),
                },
                TurnSubmissionErrorCategory::Conflict,
                TurnSubmissionRetry::Permanent,
            ),
            (
                TurnError::InvalidTransition {
                    from: TurnStatus::Queued,
                    to: TurnStatus::Completed,
                },
                TurnSubmissionErrorCategory::Conflict,
                TurnSubmissionRetry::Permanent,
            ),
            (
                TurnError::LeaseMismatch,
                TurnSubmissionErrorCategory::Conflict,
                TurnSubmissionRetry::Permanent,
            ),
        ]
    }

    #[test]
    fn conversation_turn_submitter_maps_every_turn_error_to_its_class() {
        for (error, expected_category, expected_retry) in turn_error_class_table() {
            let rendered = error.to_string();
            let expected_status = error.adapter_status_code();
            let mapped = turn_submission_error(error);
            assert_eq!(
                mapped.category(),
                expected_category,
                "category drifted for: {rendered}"
            );
            assert_eq!(
                mapped.retry(),
                expected_retry,
                "retry class drifted for: {rendered}"
            );
            assert_eq!(
                mapped.adapter_status_code(),
                expected_status,
                "port status must equal the kernel's for: {rendered}"
            );
            assert_eq!(
                mapped.to_string(),
                rendered,
                "the coordinator's rendered cause must be carried verbatim"
            );
        }
    }

    #[test]
    fn conversation_turn_submitter_covers_every_turn_error_variant() {
        // A discriminant census, so adding a `TurnError` variant without a row
        // in the table above fails here rather than silently losing coverage.
        let covered = turn_error_class_table()
            .into_iter()
            .map(|(error, _, _)| std::mem::discriminant(&error))
            .collect::<Vec<_>>();
        let distinct = covered
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            distinct.len(),
            12,
            "TurnError has 12 variants; the class table must name every one \
             (AdmissionRejected appears six times, once per rejection reason)"
        );
    }

    fn submission(
        classification: ConversationInboundClassification,
        adapter: &str,
        surface_type: Option<TurnSurfaceType>,
    ) -> ConversationTurnSubmission {
        let tenant = ironclaw_host_api::ids::TenantId::new("tenant").expect("tenant id");
        let thread = ironclaw_host_api::ids::ThreadId::new("thread").expect("thread id");
        ConversationTurnSubmission {
            scope: TurnScope::new(tenant, None, None, thread),
            actor: TurnActor::new(UserId::new("alice").expect("user id")),
            accepted_message_ref: AcceptedMessageRef::new("message:1").expect("message ref"),
            requested_run_profile: None,
            idempotency_key: IdempotencyKey::new("key:1").expect("idempotency key"),
            received_at: chrono::Utc::now(),
            classification,
            origin_adapter: RunOriginAdapter::new(adapter).expect("adapter"),
            surface_type,
            execution_policy: None,
        }
    }

    /// The trust half of the port: only a `TrustedTrigger` classification can
    /// mint a `ScheduledTrigger` origin, and an adapter literally named
    /// `"trigger"` arriving untrusted must NOT. This is the composition-side
    /// half of the guard `ironclaw_conversations`'
    /// `untrusted_trigger_adapter_records_product_inbound_not_scheduled_trigger`
    /// holds on the classification itself.
    #[test]
    fn conversation_turn_submitter_mints_scheduled_trigger_only_for_trusted_trigger() {
        let trusted = coordinator_submit_request(submission(
            ConversationInboundClassification::TrustedTrigger,
            "trigger",
            None,
        ));
        assert_eq!(
            trusted.product_context.as_ref().map(|c| c.origin),
            Some(TurnOriginKind::ScheduledTrigger)
        );

        for classification in [
            ConversationInboundClassification::Untrusted,
            ConversationInboundClassification::TrustedOther,
        ] {
            let request = coordinator_submit_request(submission(classification, "trigger", None));
            let context = request.product_context.expect("product context");
            assert_eq!(
                context.origin,
                TurnOriginKind::Inbound,
                "{classification:?} with adapter_kind='trigger' must record Inbound origin, \
                 not ScheduledTrigger"
            );
            assert_eq!(
                context.adapter.as_ref().map(RunOriginAdapter::as_str),
                Some("trigger"),
                "the adapter identity must still be carried"
            );
        }
    }

    #[test]
    fn conversation_turn_submitter_carries_surface_type_and_leaves_lineage_empty() {
        let request = coordinator_submit_request(submission(
            ConversationInboundClassification::Untrusted,
            "slack",
            Some(TurnSurfaceType::Channel),
        ));
        assert_eq!(
            request
                .product_context
                .as_ref()
                .and_then(|c| c.surface_type),
            Some(TurnSurfaceType::Channel)
        );
        assert!(request.requested_model.is_none());
        assert!(request.requested_run_id.is_none());
        assert!(request.parent_run_id.is_none());
        assert!(request.spawn_tree_root_run_id.is_none());
        assert_eq!(request.subagent_depth, 0);
    }
}
