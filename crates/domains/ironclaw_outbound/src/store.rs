use std::collections::HashSet;

use async_trait::async_trait;
use ironclaw_event_projections::ProjectionCursor;
use ironclaw_host_api::turn::{ReplyTargetBindingRef, TurnRunId, TurnScope};

use crate::{
    AdvanceReplyPublicationRequest, ClaimDeliveryAttemptForSendRequest,
    ClaimReplyPublicationLeaseRequest, LoadSubscriptionCursorRequest, OpenReplyPublicationRequest,
    OutboundDeliveryAttempt, OutboundDeliveryId, OutboundError, OutboundPushCandidate,
    OutboundPushKind, OutboundPushPlan, OutboundPushTargetRequest, ProjectionSubscriptionRecord,
    RecoverInterruptedDeliveryRequest, ReleaseReplyPublicationLeaseRequest, ReplyPublicationClaim,
    ReplyPublicationRecord, RunDeliveryCleanupRecord, RunDeliveryCleanupRequest,
    SettleReplyPublicationRequest, ThreadNotificationPolicy, UpdateDeliveryStatusRequest,
};

#[async_trait]
pub trait OutboundStateStorePort: Send + Sync {
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
    ///
    /// An attempt carrying a reply publication substate is never an
    /// interrupted one-shot send: recovery returns `Ok(false)` and leaves the
    /// row untouched. Publications recover through the publisher re-resolving
    /// open records on the run's terminal signal, never by being marked
    /// `Unknown`.
    async fn recover_interrupted_delivery_attempt(
        &self,
        request: RecoverInterruptedDeliveryRequest,
    ) -> Result<bool, OutboundError>;

    async fn update_delivery_status(
        &self,
        request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError>;

    /// Load one attempt by its durable id under the exact caller scope.
    /// Implementations should use the point-addressed row; the default keeps
    /// lightweight test doubles source-compatible.
    async fn load_delivery_attempt(
        &self,
        scope: TurnScope,
        delivery_id: OutboundDeliveryId,
    ) -> Result<Option<OutboundDeliveryAttempt>, OutboundError> {
        Ok(self
            .list_delivery_attempts(scope)
            .await?
            .into_iter()
            .find(|attempt| attempt.delivery_id == delivery_id))
    }

    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError>;

    // ── Progressive reply publication (design 2026-08-31 §5) ────────────────
    //
    // Every operation below is one compare-and-swap on the attempt row. The
    // publication substate is invisible through the attempt operations above
    // except that `recover_interrupted_delivery_attempt` skips such rows.
    // The operations are host-internal: only the delivery coordinator's
    // publication lane calls them, never a channel adapter or product caller.

    /// Open a publication on a `Prepared` attempt, creating the row with the
    /// substate (`fence 0`, no lease, revisions `0`, `Active`) when absent.
    /// Idempotent: the same delivery id with the same target returns the
    /// existing record unchanged. A different target under the same id — or a
    /// plain attempt row without a substate — is
    /// [`OutboundError::ReplyPublicationTargetMismatch`]; an attempt that is
    /// not `Prepared` is [`OutboundError::InvalidRequest`] and writes nothing.
    async fn open_reply_publication(
        &self,
        request: OpenReplyPublicationRequest,
    ) -> Result<ReplyPublicationRecord, OutboundError>;

    /// Acquire (or re-enter) the publication claim — the atomic ownership a
    /// publisher must hold before any provider egress. A settled publication
    /// returns [`ReplyPublicationClaim::Settled`]; a live lease held by
    /// another owner [`ReplyPublicationClaim::Held`]; a live lease held by
    /// the same owner is re-entered with the same fence and an extended
    /// expiry (re-entry is also the heartbeat — there is no separate renew).
    /// Otherwise (no lease, or an expired one) the lease is set to
    /// `now + ttl`, the fence is bumped, and the attempt moves
    /// `Prepared -> Sending` if it is still `Prepared` (a `Sending` attempt
    /// is left alone). A row without a substate is
    /// [`OutboundError::ReplyPublicationNotFound`]; a zero `ttl` is
    /// [`OutboundError::InvalidRequest`].
    async fn claim_reply_publication_lease(
        &self,
        request: ClaimReplyPublicationLeaseRequest,
    ) -> Result<ReplyPublicationClaim, OutboundError>;

    /// Record progress. Refused once settled
    /// ([`OutboundError::ReplyPublicationSettled`]), on a stale fence
    /// ([`OutboundError::StaleReplyPublisher`] — how a worker that lost a
    /// takeover is rejected), or without a lease
    /// ([`OutboundError::ReplyPublicationLeaseRequired`]). Revisions are
    /// monotonic — `desired_revision` and `published_revision` never move
    /// backwards and `published_revision <= desired_revision`
    /// ([`OutboundError::ReplyPublicationRevisionRegressed`]). The terminal
    /// revision is set once (at most the desired revision) and every later
    /// request must carry the same value. `generation` and `evidence` are
    /// stored as given; `checkpoint: None` keeps the previous checkpoint.
    async fn advance_reply_publication(
        &self,
        request: AdvanceReplyPublicationRequest,
    ) -> Result<ReplyPublicationRecord, OutboundError>;

    /// End the publication one-way. Needs the current fence and an `Active`
    /// status, not a live lease. `Delivered` additionally requires the
    /// terminal revision to be known and applied
    /// ([`OutboundError::ReplyPublicationNotTerminal`]); `Unknown` and
    /// `Failed(kind)` are always allowed while `Active`. Clears the lease and
    /// writes the matching attempt status (`Delivered` / `Unknown` / `Failed`
    /// with its failure kind).
    async fn settle_reply_publication(
        &self,
        request: SettleReplyPublicationRequest,
    ) -> Result<ReplyPublicationRecord, OutboundError>;

    /// Drop the lease under the current fence without settling; the fence is
    /// kept, so the next claim still bumps it. A stale fence is
    /// [`OutboundError::StaleReplyPublisher`]; no lease is a no-op.
    async fn release_reply_publication_lease(
        &self,
        request: ReleaseReplyPublicationLeaseRequest,
    ) -> Result<(), OutboundError>;

    /// Point read under the exact caller scope. `Ok(None)` for a missing row,
    /// a row outside the scope, and a plain attempt without a substate alike.
    async fn load_reply_publication(
        &self,
        scope: TurnScope,
        delivery_id: OutboundDeliveryId,
    ) -> Result<Option<ReplyPublicationRecord>, OutboundError>;

    /// Every publication in `scope` whose target run id is `run_id`, in any
    /// status, ordered like [`Self::list_delivery_attempts`].
    async fn list_reply_publications(
        &self,
        scope: TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<ReplyPublicationRecord>, OutboundError>;

    /// Every publication still `Active` anywhere in the caller's tenant —
    /// the boot-time crash-recovery read, served from the existing tenant
    /// index over the delivery rows. `scope` authorizes the read and names
    /// the tenant; rows from every thread of that tenant are returned so a
    /// restarted publisher can resume work the journal already acknowledged.
    /// Because `/outbound` is a per-user mount, one call sees the owner
    /// subtree `scope` resolves to; a sweep passes each owner scope it wants
    /// covered.
    async fn list_open_reply_publications(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<ReplyPublicationRecord>, OutboundError>;
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
    if matches!(
        request.kind,
        OutboundPushKind::FinalReply | OutboundPushKind::ModelDelivery
    ) {
        push_candidate(
            &request,
            request.reply_target.clone(),
            &mut seen,
            &mut candidates,
        );
    }

    for target in &policy.targets {
        let allowed = match request.kind {
            OutboundPushKind::FinalReply | OutboundPushKind::ModelDelivery => target.final_replies,
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
