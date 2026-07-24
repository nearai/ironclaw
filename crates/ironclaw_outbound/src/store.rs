use std::collections::HashSet;

use async_trait::async_trait;
use ironclaw_event_projections::ProjectionCursor;
use ironclaw_turns::{ReplyTargetBindingRef, TurnScope};

use crate::{
    AdvanceSubscriptionCursorRequest, ClaimDeliveryAttemptForSendRequest,
    LoadSubscriptionCursorRequest, OutboundDeliveryAttempt, OutboundError, OutboundPushCandidate,
    OutboundPushKind, OutboundPushPlan, OutboundPushTargetRequest, ProjectionSubscriptionRecord,
    RecoverInterruptedDeliveryRequest, RunDeliveryCleanupRecord, RunDeliveryCleanupRequest,
    RunFinalReplyHandoffRecord, RunFinalReplyTargetRecord, RunFinalReplyTargetRequest,
    ThreadNotificationPolicy, UpdateDeliveryStatusRequest,
};

#[async_trait]
pub trait OutboundStateStore: Send + Sync {
    async fn put_run_delivery_cleanup(
        &self,
        record: RunDeliveryCleanupRecord,
    ) -> Result<(), OutboundError>;

    async fn load_run_delivery_cleanup(
        &self,
        request: RunDeliveryCleanupRequest,
    ) -> Result<Vec<RunDeliveryCleanupRecord>, OutboundError>;

    async fn complete_run_delivery_cleanup(
        &self,
        record: &RunDeliveryCleanupRecord,
    ) -> Result<(), OutboundError>;

    /// Persist the minimal completed-run projection key used to resume final
    /// reply delivery after a process crash.
    async fn put_run_final_reply_handoff(
        &self,
        record: RunFinalReplyHandoffRecord,
    ) -> Result<(), OutboundError>;

    async fn list_pending_run_final_reply_handoffs(
        &self,
        limit: usize,
    ) -> Result<Vec<RunFinalReplyHandoffRecord>, OutboundError>;

    /// Continue a stable `(event_cursor, run_id)` scan after `after`.
    ///
    /// The production filesystem store overrides this with an indexed keyset
    /// query so callers may delete settled rows between pages without
    /// shifting or skipping later records. The default preserves compatibility
    /// for test/error stores that only implement the original first-page API.
    async fn list_pending_run_final_reply_handoffs_after(
        &self,
        after: Option<&RunFinalReplyHandoffRecord>,
        limit: usize,
    ) -> Result<Vec<RunFinalReplyHandoffRecord>, OutboundError> {
        let mut records = self.list_pending_run_final_reply_handoffs(limit).await?;
        if let Some(after) = after {
            records.retain(|record| {
                (record.event_cursor, record.run_id) > (after.event_cursor, after.run_id)
            });
        }
        Ok(records)
    }

    async fn complete_run_final_reply_handoff(
        &self,
        record: &RunFinalReplyHandoffRecord,
    ) -> Result<(), OutboundError>;

    async fn load_run_final_reply_handoff_cursor(
        &self,
    ) -> Result<ironclaw_turns::EventCursor, OutboundError>;

    async fn advance_run_final_reply_handoff_cursor(
        &self,
        cursor: ironclaw_turns::EventCursor,
    ) -> Result<(), OutboundError>;

    /// Seal the final-reply destination for one run.
    ///
    /// Implementations must make an identical retry idempotent and reject a
    /// different record for the same run. A run target is not a user-wide
    /// preference and must not mutate communication defaults.
    async fn put_run_final_reply_target(
        &self,
        record: RunFinalReplyTargetRecord,
    ) -> Result<(), OutboundError>;

    /// Load a run target only for the exact actor and turn scope.
    ///
    /// Missing and unauthorized records are deliberately indistinguishable.
    async fn load_run_final_reply_target(
        &self,
        request: RunFinalReplyTargetRequest,
    ) -> Result<Option<RunFinalReplyTargetRecord>, OutboundError>;

    async fn put_thread_notification_policy(
        &self,
        policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError>;

    async fn load_thread_notification_policy(
        &self,
        scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError>;

    async fn plan_push_targets(
        &self,
        request: OutboundPushTargetRequest,
    ) -> Result<OutboundPushPlan, OutboundError> {
        let policy = self
            .load_thread_notification_policy(request.scope.clone())
            .await?;
        plan_push_targets_from_policy(request, &policy)
    }

    async fn upsert_subscription(
        &self,
        record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError>;

    /// Load a cursor only for the exact authorized actor/scope/thread tuple.
    ///
    /// Returns `Ok(None)` for missing rows and for rows with a mismatched
    /// actor/scope/thread. The indistinguishable `None` preserves
    /// anti-enumeration semantics: callers cannot learn whether a
    /// subscription id exists outside their authorized tuple.
    async fn load_subscription_cursor(
        &self,
        request: LoadSubscriptionCursorRequest,
    ) -> Result<Option<ProjectionCursor>, OutboundError>;

    async fn advance_subscription_cursor(
        &self,
        request: AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError>;

    async fn record_delivery_attempt(
        &self,
        attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError>;

    /// Atomically reserve the one allowed vendor-egress drive for a prepared
    /// attempt. Returns `true` only to the caller that persisted the
    /// `Prepared -> Sending` transition.
    async fn claim_delivery_attempt_for_send(
        &self,
        request: ClaimDeliveryAttemptForSendRequest,
    ) -> Result<bool, OutboundError>;

    /// Crash recovery for an interrupted send. Re-reads the attempt inside the
    /// store's CAS and transitions `Sending -> Unknown` only when it is still
    /// `Sending`. Returns `Ok(true)` only for the caller that persisted that
    /// transition and `Ok(false)` when the attempt already advanced past
    /// `Sending`, so a stale recovery list snapshot can never overwrite a
    /// terminal `Delivered`/`Failed` result a different worker wrote after
    /// completing egress. Unlike [`Self::update_delivery_status`] (an
    /// unconditional setter used for forward egress-result writes), this
    /// transition re-verifies the source state under the same CAS read.
    async fn recover_interrupted_delivery_attempt(
        &self,
        request: RecoverInterruptedDeliveryRequest,
    ) -> Result<bool, OutboundError>;

    async fn update_delivery_status(
        &self,
        request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError>;

    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError>;
}

fn plan_push_targets_from_policy(
    request: OutboundPushTargetRequest,
    policy: &ThreadNotificationPolicy,
) -> Result<OutboundPushPlan, OutboundError> {
    if policy.scope != request.scope {
        return Err(OutboundError::InvalidRequest {
            reason: "notification policy scope does not match request",
        });
    }

    let mut seen = HashSet::<ReplyTargetBindingRef>::new();
    let mut candidates = Vec::new();
    if request.kind == OutboundPushKind::FinalReply {
        push_candidate(
            &request,
            request.reply_target.clone(),
            &mut seen,
            &mut candidates,
        );
    }

    for target in &policy.targets {
        let allowed = match request.kind {
            OutboundPushKind::FinalReply => target.final_replies,
            OutboundPushKind::Progress
            | OutboundPushKind::GateRequired
            | OutboundPushKind::AuthPrompt
            | OutboundPushKind::DeliveryStatus => target.progress,
        };
        if allowed {
            push_candidate(&request, target.target.clone(), &mut seen, &mut candidates);
        }
    }
    Ok(OutboundPushPlan { candidates })
}

fn push_candidate(
    request: &OutboundPushTargetRequest,
    target: ReplyTargetBindingRef,
    seen: &mut HashSet<ReplyTargetBindingRef>,
    candidates: &mut Vec<OutboundPushCandidate>,
) {
    if !seen.insert(target.clone()) {
        return;
    }
    candidates.push(OutboundPushCandidate {
        tenant_id: request.scope.tenant_id.clone(),
        agent_id: request.scope.agent_id.clone(),
        project_id: request.scope.project_id.clone(),
        thread_id: request.scope.thread_id.clone(),
        turn_run_id: request.turn_run_id,
        target,
        kind: request.kind,
        projection_ref: request.projection_ref.clone(),
        requires_reply_target_revalidation: true,
    });
}
